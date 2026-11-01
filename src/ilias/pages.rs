//! The seam between the client's logic and the network: getting one ILIAS page. The client
//! sends every page request through [`Pages`], after the write guard
//! ([`super::write::is_write`]); file downloads and the Keycloak probe go to the network
//! directly.

use color_eyre::Result;
use url::Url;

/// A fetched HTML page and the URL it ended up at (after redirects).
#[derive(Debug)]
pub struct Fetched {
    pub url: Url,
    pub html: String,
}

pub trait Pages: Send + Sync {
    /// GETs `url`, following redirects. HTTP errors (4xx, 5xx) are errors.
    fn get(&self, url: &Url) -> Result<Fetched>;
}

/// The real adapter: HTTPS with the session's cookie jar.
pub struct Https(pub reqwest::blocking::Client);

impl Pages for Https {
    fn get(&self, url: &Url) -> Result<Fetched> {
        let response = self.0.get(url.clone()).send()?.error_for_status()?;
        let url = response.url().clone();
        Ok(Fetched {
            url,
            html: response.text()?,
        })
    }
}

#[cfg(test)]
pub use fake::{FakeIlias, LOGGED_OUT, reply};

#[cfg(test)]
mod fake {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    type Respond = dyn Fn(&Url) -> Result<Fetched> + Send + Sync;

    /// An in-memory ILIAS for tests: answers with `respond` and records every URL it was
    /// asked for, in order.
    pub struct FakeIlias {
        respond: Box<Respond>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeIlias {
        pub fn new(respond: impl Fn(&Url) -> Result<Fetched> + Send + Sync + 'static) -> Self {
            Self {
                respond: Box::new(respond),
                requests: Mutex::new(Vec::new()),
            }
        }

        /// The URLs requested so far, relative to `base` where possible.
        pub fn requests(&self, base: &Url) -> Vec<String> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|url| url.strip_prefix(base.as_str()).unwrap_or(url).to_string())
                .collect()
        }
    }

    /// What ILIAS shows when you are not logged in (any page with this link).
    pub const LOGGED_OUT: &str = r#"<a href="login.php?cmd=force_login">Anmelden</a>"#;

    /// A page served at `url` itself (no redirect).
    pub fn reply(url: &Url, html: &str) -> Result<Fetched> {
        Ok(Fetched {
            url: url.clone(),
            html: html.into(),
        })
    }

    impl Pages for FakeIlias {
        fn get(&self, url: &Url) -> Result<Fetched> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(url.to_string());
            (self.respond)(url)
        }
    }
}
