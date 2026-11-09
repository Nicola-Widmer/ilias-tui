//! The seam between the client's logic and the network: getting one ILIAS page, or posting a
//! form. The client sends every page request, GET or POST, through [`Pages`] from one
//! place (`Client::request`), after the offline check and the write guard
//! ([`super::write::is_write`]). File downloads and the Keycloak probe go to the network
//! directly.

use std::path::Path;

use color_eyre::Result;
use reqwest::blocking::multipart;
use url::Url;

/// A fetched HTML page and the URL it ended up at (after redirects).
#[derive(Debug)]
pub struct Fetched {
    pub url: Url,
    pub html: String,
}

/// A multipart form: what a browser sends for a form with a file input.
#[derive(Debug, Clone, Copy)]
pub enum Form<'a> {
    Field { name: &'a str, value: &'a str },
    File { name: &'a str, path: &'a Path },
}

pub trait Pages: Send + Sync {
    /// GETs `url`, following redirects. HTTP errors (4xx, 5xx) are errors.
    fn get(&self, url: &Url) -> Result<Fetched>;

    /// POSTs `form` (its fields in order) to `url`, following redirects. HTTP errors (4xx,
    /// 5xx) are errors.
    fn post(&self, url: &Url, form: &[Form]) -> Result<Fetched>;
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

    fn post(&self, url: &Url, form: &[Form]) -> Result<Fetched> {
        // Browsers send file names as they are; percent-encoded, ILIAS would keep the codes.
        let mut multipart = multipart::Form::new().percent_encode_noop();
        for field in form {
            multipart = match *field {
                Form::Field { name, value } => multipart.text(name.to_string(), value.to_string()),
                Form::File { name, path } => multipart.file(name.to_string(), path)?,
            };
        }
        let response = self
            .0
            .post(url.clone())
            .multipart(multipart)
            .send()?
            .error_for_status()?;
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
        posts: Mutex<Vec<String>>,
    }

    impl FakeIlias {
        pub fn new(respond: impl Fn(&Url) -> Result<Fetched> + Send + Sync + 'static) -> Self {
            Self {
                respond: Box::new(respond),
                requests: Mutex::new(Vec::new()),
                posts: Mutex::new(Vec::new()),
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

    impl FakeIlias {
        /// What was posted so far: `URL <- name=value&…`, with `name=@file` for a file.
        pub fn posts(&self, base: &Url) -> Vec<String> {
            self.posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|post| post.strip_prefix(base.as_str()).unwrap_or(post).to_string())
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

        fn post(&self, url: &Url, form: &[Form]) -> Result<Fetched> {
            let sent: Vec<String> = form
                .iter()
                .map(|field| match *field {
                    Form::Field { name, value } => format!("{name}={value}"),
                    Form::File { name, path } => {
                        let file = path.file_name().unwrap_or_default().to_string_lossy();
                        format!("{name}=@{file}")
                    }
                })
                .collect();
            let sent = sent.join("&");
            self.posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{url} <- {sent}"));
            (self.respond)(url)
        }
    }
}
