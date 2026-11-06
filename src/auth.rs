//! Interactive SSO login (ILIAS → Keycloak → Switch edu-ID) in a real Chrome window. An ILIAS
//! without a login server (the local test ILIAS, see `dev/README.md`) shows its own login page
//! there instead.
//!
//! The user logs in normally (including 2FA). Once Chrome lands back on ILIAS we copy the
//! cookies of ILIAS and of its login server (HSLU's Keycloak) into a [`CookieStore`]. The
//! Keycloak cookies are what later allow [`crate::ilias::Client::silent_login`] to get a fresh
//! ILIAS session without a browser. Cookies of other sites the login passed through (Switch
//! edu-ID) are left out: they could be used for other edu-ID services and aren't needed.

use std::collections::BTreeSet;
use std::thread;
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::bail, eyre::eyre};
use cookie_store::{CookieStore, RawCookie};
use headless_chrome::protocol::cdp::Network;
use headless_chrome::{Browser, LaunchOptions};
use url::Url;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub fn browser_login(base: &Url) -> Result<CookieStore> {
    let login_server = login_server(base)?;
    let own_login_page = login_server.host_str() == base.host_str();
    let browser = Browser::new(LaunchOptions {
        headless: false,
        window_size: Some((1000, 800)),
        idle_browser_timeout: LOGIN_TIMEOUT,
        ..Default::default()
    })
    .map_err(|e| eyre!("could not start Chrome/Chromium: {e}"))?;
    let tab = browser.new_tab().map_err(|e| eyre!(e))?;
    tab.navigate_to(base.join("openidconnect.php")?.as_str())
        .map_err(|e| eyre!(e))?;

    // Every URL (without query) Chrome visits; cookies are looked up for exactly these,
    // so path-scoped cookies like `/ilias/` and `/realms/hslu/` are included.
    let mut visited = BTreeSet::new();
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            bail!("login timed out after {} seconds", LOGIN_TIMEOUT.as_secs());
        }
        thread::sleep(Duration::from_millis(300));
        let Ok(mut url) = Url::parse(&tab.get_url()) else {
            continue;
        };
        if !url.scheme().starts_with("http") {
            continue;
        }
        let past_login = past_login(&url, base);
        url.set_query(None);
        url.set_fragment(None);
        let left_ilias = visited
            .iter()
            .any(|u: &Url| u.host_str() != base.host_str());
        visited.insert(url);
        if past_login && (left_ilias || own_login_page) {
            break;
        }
    }

    // The login server's page may have redirected on to edu-ID before we polled it.
    visited.insert(login_server.clone());
    let urls = visited.iter().map(Url::to_string).collect();
    let keep = [base.host_str(), login_server.host_str()];
    let cookies = tab
        .call_method(Network::GetCookies { urls: Some(urls) })
        .map_err(|e| eyre!(e))?
        .cookies;

    let mut store = CookieStore::default();
    for c in cookies {
        if !keep.iter().flatten().any(|h| cookie_matches(&c.domain, h)) {
            continue;
        }
        let host = c.domain.trim_start_matches('.').to_string();
        let mut raw = RawCookie::build((c.name, c.value))
            .path(c.path.clone())
            .secure(c.secure)
            .http_only(c.http_only);
        // A leading dot means "valid for subdomains too"; otherwise keep it host-only.
        if c.domain.starts_with('.') {
            raw = raw.domain(host.clone());
        }
        let request_url = Url::parse(&format!("https://{host}{}", c.path))?;
        // Rejected cookies (e.g. for unrelated domains) are simply skipped.
        let _ = store.insert_raw(&raw.build(), &request_url);
    }
    Ok(store)
}

/// Whether Chrome is on ILIAS, on anything but its login pages.
fn past_login(url: &Url, base: &Url) -> bool {
    url.host_str() == base.host_str()
        && !url.path().ends_with("openidconnect.php")
        && !url.path().ends_with("login.php")
        && !url
            .query()
            .is_some_and(|q| q.to_ascii_lowercase().contains("baseclass=ilstartupgui"))
}

/// Where ILIAS sends the browser to log in (e.g. HSLU's Keycloak), without following the
/// redirect. Returned without query, as used for cookie lookups.
fn login_server(base: &Url) -> Result<Url> {
    let http = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = http.get(base.join("openidconnect.php")?).send()?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .ok_or_else(|| {
            eyre!("ILIAS did not redirect to a login server (is OpenID Connect enabled?)")
        })?;
    let mut url = base.join(location)?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Whether a cookie for `domain` (leading dot = also subdomains) is sent to `host`.
fn cookie_matches(domain: &str, host: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_ends_past_the_login_pages_of_ilias() {
        let base = Url::parse("http://localhost:8080/").unwrap();
        let past = |url: &str| past_login(&Url::parse(url).unwrap(), &base);
        assert!(past(
            "http://localhost:8080/ilias.php?baseClass=ilDashboardGUI"
        ));
        assert!(!past("http://localhost:8080/openidconnect.php"));
        assert!(!past("http://localhost:8080/login.php"));
        assert!(!past(
            "http://localhost:8080/ilias.php?baseClass=ilstartupgui&cmd=showLoginPage"
        ));
        assert!(!past("https://keycloak.example/realms/hslu"));
    }

    #[test]
    fn keeps_only_ilias_and_login_server_cookies() {
        let keep = ["elearning.hslu.ch", "keycloak.hslu.ch"];
        let kept = |domain: &str| keep.iter().any(|h| cookie_matches(domain, h));
        assert!(kept("elearning.hslu.ch"));
        assert!(kept("keycloak.hslu.ch"));
        assert!(kept(".hslu.ch"));
        assert!(!kept("login.eduid.ch"));
        assert!(!kept(".eduid.ch"));
        assert!(!kept("evilhslu.ch"));
    }
}
