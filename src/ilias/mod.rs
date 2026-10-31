//! HTTP client for an ILIAS installation, authenticated with the session cookies.

pub mod parse;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use color_eyre::{Result, eyre::bail};
use cookie_store::CookieStore;
use reqwest::blocking::Response;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest_cookie_store::CookieStoreMutex;
use url::Url;

pub use parse::{Block, Item, Page, parse_page, parse_target};

/// The "My courses and groups" page.
const MEMBERSHIPS: &str = "ilias.php?baseClass=ilmembershipoverviewgui";

/// Upper bound of pages fetched by one deep search, to stay polite to the server.
const MAX_CRAWL: usize = 500;

/// Concurrent requests during a deep search; low enough to stay polite to the server.
const PARALLEL: usize = 6;

/// A fetched HTML page and the URL it ended up at (after redirects).
pub struct Fetched {
    pub url: Url,
    pub html: String,
}

pub struct Client {
    /// ILIAS root, always ending in `/`, e.g. `https://elearning.hslu.ch/ilias/`.
    pub base: Url,
    http: reqwest::blocking::Client,
    jar: Arc<CookieStoreMutex>,
}

impl Client {
    pub fn new(base: Url, cookies: CookieStore) -> Result<Self> {
        let jar = Arc::new(CookieStoreMutex::new(cookies));
        let http = reqwest::blocking::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(concat!("ilias-tui/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base, http, jar })
    }

    pub fn url(&self, path: &str) -> Result<Url> {
        Ok(self.base.join(path)?)
    }

    pub fn get(&self, url: &Url) -> Result<Fetched> {
        let response = self.http.get(url.clone()).send()?.error_for_status()?;
        let url = response.url().clone();
        Ok(Fetched {
            url,
            html: response.text()?,
        })
    }

    /// Anonymous ILIAS pages carry a "force_login" link; redirects to Keycloak/edu-ID or the
    /// login page also mean we are not logged in.
    pub fn is_logged_in(&self, page: &Fetched) -> bool {
        page.url.host_str() == self.base.host_str()
            && !page.url.path().ends_with("login.php")
            && !page.html.contains("cmd=force_login")
    }

    /// Ends the ILIAS session on the server via its logout link (which carries a one-time
    /// token, so it has to be read from a page). Returns whether a session was ended.
    pub fn logout(&self) -> Result<bool> {
        let fetched = self.get(&self.url(MEMBERSHIPS)?)?;
        if !self.is_logged_in(&fetched) {
            return Ok(false);
        }
        let Some(url) = parse::logout_link(&fetched.html, &fetched.url) else {
            bail!("no logout link found on the ILIAS page");
        };
        self.get(&url)?;
        let after = self.get(&self.url(MEMBERSHIPS)?)?;
        Ok(!self.is_logged_in(&after))
    }

    /// Whether the login server (HSLU's Keycloak) would still log us in without a password.
    /// Follows the OIDC redirects but stops right before the login code would be handed back
    /// to ILIAS, so no new ILIAS session is created.
    pub fn login_server_session_active(&self) -> Result<bool> {
        let ilias_host = self.base.host_str().map(str::to_owned);
        let http = reqwest::blocking::Client::builder()
            .cookie_provider(self.jar.clone())
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                let url = attempt.url();
                let back_with_code = url.host_str() == ilias_host.as_deref()
                    && url.query().is_some_and(|q| q.contains("code="));
                if back_with_code {
                    attempt.stop()
                } else if attempt.previous().len() > 10 {
                    attempt.error("too many redirects")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        let response = http.get(self.url("openidconnect.php")?).send()?;
        let location = response.headers().get(reqwest::header::LOCATION);
        Ok(response.status().is_redirection()
            && location
                .and_then(|l| l.to_str().ok())
                .is_some_and(|l| l.contains("code=")))
    }

    /// Re-runs the OIDC flow with the saved Keycloak cookies. Succeeds without any user
    /// interaction as long as the Keycloak SSO session is still alive.
    pub fn silent_login(&self) -> Result<bool> {
        let page = self.get(&self.url("openidconnect.php")?)?;
        Ok(self.is_logged_in(&page))
    }

    /// Checks the current session and refreshes it silently if needed.
    /// Checks the session (refreshing it silently if needed). On success returns "My courses",
    /// which had to be fetched for the check anyway, so the TUI can show it right away.
    pub fn check_session(&self) -> Result<Option<Page>> {
        let url = self.url(MEMBERSHIPS)?;
        let mut fetched = self.get(&url)?;
        if !self.is_logged_in(&fetched) {
            if !self.silent_login()? {
                return Ok(None);
            }
            fetched = self.get(&url)?;
            if !self.is_logged_in(&fetched) {
                return Ok(None);
            }
        }
        Ok(Some(parse_page(&fetched.html, &fetched.url)))
    }

    /// GETs a page that requires a login, refreshing the session once if it expired.
    pub fn get_authed(&self, url: &Url) -> Result<Fetched> {
        let page = self.get(url)?;
        if self.is_logged_in(&page) {
            return Ok(page);
        }
        if self.silent_login()? {
            let page = self.get(url)?;
            if self.is_logged_in(&page) {
                return Ok(page);
            }
        }
        bail!("session expired: press L to log in again")
    }

    /// "My courses and groups", grouped like on ILIAS (by semester).
    pub fn courses(&self) -> Result<Page> {
        let fetched = self.get_authed(&self.url(MEMBERSHIPS)?)?;
        Ok(parse_page(&fetched.html, &fetched.url))
    }

    /// The page of a container; `url` selects another tab of it.
    pub fn page(&self, item: &Item, url: Option<&str>) -> Result<Page> {
        let fetched = self.get_authed(&Url::parse(url.unwrap_or(&item.url))?)?;
        let mut page = parse_page(&fetched.html, &fetched.url);
        page.remove(item.ref_id);
        Ok(page)
    }

    pub fn children(&self, item: &Item) -> Result<Vec<Item>> {
        Ok(self.page(item, None)?.items().cloned().collect())
    }

    /// Collects everything below `root` (`None` = all courses), in display order. Titles
    /// become paths relative to `root`, e.g. `Unterlagen › Woche 3 › Folien.pdf`.
    /// `progress` is called with the number of containers fetched so far.
    pub fn descendants(
        &self,
        root: Option<&Item>,
        progress: impl Fn(usize) + Sync,
    ) -> Result<Vec<Item>> {
        let top = match root {
            Some(item) => self.children(item)?,
            None => self.courses()?.items().cloned().collect(),
        };
        // Categories lead out of the course into the rest of the repository.
        let crawlable = |i: &Item| i.is_container() && i.kind != "cat";

        // 1. Fetch every container below `root`, one tree level at a time, in parallel.
        let mut queued: HashSet<u64> = root.map(|r| r.ref_id).into_iter().collect();
        let mut children: HashMap<u64, Vec<Item>> = HashMap::new();
        let mut frontier: Vec<Item> = top
            .iter()
            .filter(|i| crawlable(i) && queued.insert(i.ref_id))
            .cloned()
            .collect();
        let fetched = AtomicUsize::new(0);
        let done = || progress(fetched.fetch_add(1, Ordering::Relaxed) + 1);
        while !frontier.is_empty() {
            frontier.truncate(MAX_CRAWL.saturating_sub(children.len()));
            let results = self.fetch_children(&frontier, &done)?;
            frontier = results
                .iter()
                .flat_map(|(_, items)| items)
                .filter(|i| crawlable(i) && queued.insert(i.ref_id))
                .cloned()
                .collect();
            children.extend(results);
        }

        // 2. Walk the tree in display order, turning titles into paths.
        let mut seen: HashSet<u64> = root.map(|r| r.ref_id).into_iter().collect();
        let mut stack: Vec<(String, Item)> =
            top.into_iter().rev().map(|i| (String::new(), i)).collect();
        let mut found = Vec::new();
        while let Some((parent, item)) = stack.pop() {
            if !seen.insert(item.ref_id) {
                continue;
            }
            let path = if parent.is_empty() {
                item.title.clone()
            } else {
                format!("{parent} › {}", item.title)
            };
            if let Some(items) = children.get(&item.ref_id) {
                stack.extend(items.iter().rev().map(|c| (path.clone(), c.clone())));
            }
            found.push(Item {
                title: path,
                ..item
            });
        }
        Ok(found)
    }

    /// Fetches the children of all `items`, `PARALLEL` requests at a time. ILIAS handles
    /// concurrent requests of one session fine (measured: 6 folders in 0.7 s instead of 4 s).
    fn fetch_children(
        &self,
        items: &[Item],
        done: &(dyn Fn() + Sync),
    ) -> Result<Vec<(u64, Vec<Item>)>> {
        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::new());
        thread::scope(|scope| {
            for _ in 0..PARALLEL.min(items.len()) {
                scope.spawn(|| {
                    while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let result = self.children(item).map(|c| (item.ref_id, c));
                        done();
                        results
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(result);
                    }
                });
            }
        });
        results
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .collect()
    }

    /// Downloads a file object into `dir`, never overwriting existing files.
    pub fn download(&self, item: &Item, dir: &Path) -> Result<PathBuf> {
        // Files listed in folders link straight to `cmd=sendfile`; otherwise use the permalink.
        let url = if item.url.contains("cmd=sendfile") {
            Url::parse(&item.url)?
        } else {
            self.url(&format!("goto.php?target=file_{}_download", item.ref_id))?
        };
        let mut response = self.send_file_request(&url)?;
        if is_html(&response) && self.silent_login()? {
            response = self.send_file_request(&url)?;
        }
        if is_html(&response) {
            bail!("ILIAS returned a page instead of the file (no access or session expired)");
        }
        let name = filename(&response).unwrap_or_else(|| item.title.clone());
        std::fs::create_dir_all(dir)?;
        let path = unique_path(dir, &sanitize(&name));
        io::copy(&mut response, &mut File::create(&path)?)?;
        crate::open::quarantine(&path);
        Ok(path)
    }

    fn send_file_request(&self, url: &Url) -> Result<Response> {
        Ok(self.http.get(url.clone()).send()?.error_for_status()?)
    }

    pub fn cookies(&self) -> CookieStore {
        self.jar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_cookies(&self, cookies: CookieStore) {
        *self.jar.lock().unwrap_or_else(PoisonError::into_inner) = cookies;
    }
}

fn is_html(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"))
}

fn filename(response: &Response) -> Option<String> {
    let header = response.headers().get(CONTENT_DISPOSITION)?.to_str().ok()?;
    filename_from_disposition(header)
}

fn filename_from_disposition(header: &str) -> Option<String> {
    let params = header.split(';').map(str::trim);
    let mut plain = None;
    for param in params {
        if let Some(value) = param.strip_prefix("filename*=") {
            // RFC 5987: UTF-8''percent-encoded
            let encoded = value.split_once("''")?.1;
            return Some(
                percent_encoding::percent_decode_str(encoded)
                    .decode_utf8()
                    .ok()?
                    .into(),
            );
        }
        if let Some(value) = param.strip_prefix("filename=") {
            plain = Some(value.trim_matches('"').to_string());
        }
    }
    plain
}

fn sanitize(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    match name.trim_start_matches('.') {
        "" => "download".into(),
        trimmed => trimmed.into(),
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        return path;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_content_disposition() {
        assert_eq!(
            filename_from_disposition(r#"attachment; filename="Skript.pdf""#).as_deref(),
            Some("Skript.pdf")
        );
        assert_eq!(
            filename_from_disposition(
                r#"attachment; filename="a.pdf"; filename*=UTF-8''%C3%9Cbung%201.pdf"#
            )
            .as_deref(),
            Some("Übung 1.pdf")
        );
        assert_eq!(filename_from_disposition("inline"), None);
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("../etc/passwd"), "_etc_passwd");
        assert_eq!(sanitize("..."), "download");
        assert_eq!(sanitize("Woche 1: Intro.pdf"), "Woche 1_ Intro.pdf");
    }
}
