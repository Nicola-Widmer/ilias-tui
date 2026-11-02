//! HTTP client for an ILIAS installation, authenticated with the session cookies. Page
//! requests go through the [`pages::Pages`] seam, behind the write guard.

mod my_courses;
pub mod pages;
pub mod parse;
pub mod write;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use color_eyre::{Result, eyre::bail};
use cookie_store::CookieStore;
use reqwest::blocking::Response;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest_cookie_store::CookieStoreMutex;
use url::Url;

pub use pages::Fetched;
use pages::{Https, Pages};
pub use parse::{Block, Item, Page, Post, parse_page, parse_target};

/// The "My courses and groups" page.
const MEMBERSHIPS: &str = "ilias.php?baseClass=ilmembershipoverviewgui";

/// Upper bound of pages fetched by one deep search, to stay polite to the server.
const MAX_CRAWL: usize = 500;

/// Concurrent requests during a deep search; low enough to stay polite to the server.
const PARALLEL: usize = 6;

pub struct Client {
    /// ILIAS root, always ending in `/`, e.g. `https://elearning.hslu.ch/ilias/`.
    pub base: Url,
    /// Where page requests go (the network, or a fake ILIAS in tests).
    pages: Arc<dyn Pages>,
    /// For file downloads and the Keycloak probe, which bypass `pages`.
    http: reqwest::blocking::Client,
    jar: Arc<CookieStoreMutex>,
    /// `ILIAS_DRY_RUN`: favourites changes are only pretended.
    dry_run: bool,
    /// Offline mode: nothing at all is sent; every request fails with [`OFFLINE`].
    offline: AtomicBool,
}

/// The error of every request in offline mode.
pub const OFFLINE: &str = "offline: not connected to ILIAS";

impl Client {
    pub fn new(base: Url, cookies: CookieStore) -> Result<Self> {
        let jar = Arc::new(CookieStoreMutex::new(cookies));
        let http = reqwest::blocking::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(concat!("ilias-tui/", env!("CARGO_PKG_VERSION")))
            // An unreachable server shows quickly (offline detection at startup); slow
            // pages and big downloads still have all the time they need.
            .connect_timeout(Duration::from_secs(8))
            .build()?;
        Ok(Self {
            base,
            pages: Arc::new(Https(http.clone())),
            http,
            jar,
            dry_run: write::dry_run_from_env(),
            offline: AtomicBool::new(false),
        })
    }

    /// Offline mode: from now on nothing is sent to ILIAS.
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::Relaxed);
    }

    pub fn is_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    fn ensure_online(&self) -> Result<()> {
        if self.is_offline() {
            bail!(OFFLINE);
        }
        Ok(())
    }

    /// A client whose page requests go to `pages` (a fake ILIAS).
    #[cfg(test)]
    pub fn with_pages(base: Url, pages: Arc<dyn Pages>) -> Self {
        Self {
            pages,
            dry_run: false,
            ..Self::new(base, CookieStore::default()).unwrap()
        }
    }

    pub fn url(&self, path: &str) -> Result<Url> {
        Ok(self.base.join(path)?)
    }

    /// GETs a page. Refused if it looks like a write (the write guard); writes go through
    /// `write.rs`.
    pub fn get(&self, url: &Url) -> Result<Fetched> {
        self.request(url, false)
    }

    /// The one place page requests leave the client. Only `write.rs` passes `write: true`.
    fn request(&self, url: &Url, write: bool) -> Result<Fetched> {
        self.ensure_online()?;
        if !write && write::is_write(url) {
            bail!("refused to request {url}: it would change something on ILIAS");
        }
        self.pages.get(url)
    }

    /// Anonymous ILIAS pages carry a "force_login" link; redirects to Keycloak/edu-ID or the
    /// login page also mean we are not logged in.
    pub fn is_logged_in(&self, page: &Fetched) -> bool {
        page.url.host_str() == self.base.host_str()
            && !page.url.path().ends_with("login.php")
            && !page.html.contains("cmd=force_login")
    }

    /// Whether the login server (HSLU's Keycloak) would still log us in without a password.
    /// Follows the OIDC redirects but stops right before the login code would be handed back
    /// to ILIAS, so no new ILIAS session is created.
    pub fn login_server_session_active(&self) -> Result<bool> {
        self.ensure_online()?;
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

    /// The session check: whether the saved session works, renewing it silently if needed.
    pub fn check_session(&self) -> Result<bool> {
        let url = self.url(MEMBERSHIPS)?;
        if self.is_logged_in(&self.get(&url)?) {
            return Ok(true);
        }
        Ok(self.silent_login()? && self.is_logged_in(&self.get(&url)?))
    }

    /// GETs a page that requires a login, refreshing the session once if it expired.
    pub fn get_authed(&self, url: &Url) -> Result<Fetched> {
        self.authed(url, false)
    }

    fn authed(&self, url: &Url, write: bool) -> Result<Fetched> {
        let page = self.request(url, write)?;
        if self.is_logged_in(&page) {
            return Ok(page);
        }
        if self.silent_login()? {
            let page = self.request(url, write)?;
            if self.is_logged_in(&page) {
                return Ok(page);
            }
        }
        bail!("session expired: press L to log in again")
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
            None => self.course_list()?,
        };
        // Categories lead out of the course into the rest of the repository; forum threads
        // hold posts, not files.
        let crawlable =
            |i: &Item| i.is_container() && !matches!(i.kind.as_str(), "cat" | "frm" | "thr");

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
        self.ensure_online()?;
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
        let dir = folder_for(dir, item);
        std::fs::create_dir_all(&dir)?;
        let path = unique_path(&dir, &sanitize(&name));
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

/// Same folders as on ILIAS: `<download dir>/<course>/<folder>/…`.
fn folder_for(dir: &Path, item: &Item) -> PathBuf {
    item.path
        .iter()
        .fold(dir.to_path_buf(), |dir, folder| dir.join(sanitize(folder)))
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
    fn saves_files_in_their_ilias_folders() {
        let item = Item {
            path: vec!["DEMO.101".into(), "Woche 1: Intro".into(), "../..".into()],
            ..Item::default()
        };
        assert_eq!(
            folder_for(Path::new("/dl"), &item),
            Path::new("/dl/DEMO.101/Woche 1_ Intro/_..")
        );
        assert_eq!(
            folder_for(Path::new("/dl"), &Item::default()),
            Path::new("/dl")
        );
    }

    #[test]
    fn only_sends_favourites_links_for_the_item() {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::new(base, CookieStore::default()).unwrap();
        let item = |link: &str| Item {
            ref_id: 7,
            title: "DEMO".into(),
            favorite_toggle: Some(link.into()),
            ..Item::default()
        };
        for link in [
            // another item
            "https://ilias.example/ilias/ilias.php?cmd=addToDesk&item_ref_id=8",
            // another command
            "https://ilias.example/ilias/ilias.php?cmd=delete&item_ref_id=7",
            // another server
            "https://evil.example/ilias/ilias.php?cmd=addToDesk&item_ref_id=7",
        ] {
            let error = client.toggle_favorite(&item(link)).unwrap_err();
            assert!(error.to_string().contains("unexpected"), "{link}: {error}");
        }
        assert!(client.toggle_favorite(&Item::default()).is_err());
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("../etc/passwd"), "_etc_passwd");
        assert_eq!(sanitize("..."), "download");
        assert_eq!(sanitize("Woche 1: Intro.pdf"), "Woche 1_ Intro.pdf");
    }

    use super::pages::{FakeIlias, LOGGED_OUT, reply};
    use color_eyre::eyre::eyre;
    use std::sync::atomic::AtomicBool;

    fn base() -> Url {
        Url::parse("https://ilias.example/ilias/").unwrap()
    }

    fn fake(
        respond: impl Fn(&Url) -> Result<Fetched> + Send + Sync + 'static,
    ) -> (Client, Arc<FakeIlias>) {
        let fake = Arc::new(FakeIlias::new(respond));
        (Client::with_pages(base(), fake.clone()), fake)
    }

    #[test]
    fn renews_an_expired_session_once_and_retries() {
        let logged_in = Arc::new(AtomicBool::new(false));
        let state = logged_in.clone();
        let (client, ilias) = fake(move |url| {
            if url.path().ends_with("openidconnect.php") {
                state.store(true, Ordering::SeqCst);
            }
            match state.load(Ordering::SeqCst) {
                true => reply(url, "<p>Inhalt</p>"),
                false => reply(url, LOGGED_OUT),
            }
        });
        let page = client
            .get_authed(&client.url("goto.php/crs/1").unwrap())
            .unwrap();
        assert_eq!(page.html, "<p>Inhalt</p>");
        assert_eq!(
            ilias.requests(&base()),
            ["goto.php/crs/1", "openidconnect.php", "goto.php/crs/1"]
        );
    }

    #[test]
    fn says_so_when_the_session_cannot_be_renewed() {
        let (client, ilias) = fake(|url| reply(url, LOGGED_OUT));
        let error = client
            .get_authed(&client.url("goto.php/crs/1").unwrap())
            .unwrap_err();
        assert!(
            error.to_string().contains("press L to log in again"),
            "{error}"
        );
        assert_eq!(
            ilias.requests(&base()).len(),
            2,
            "one page, one silent login, no retry"
        );
        assert!(!client.check_session().unwrap());
    }

    #[test]
    fn the_write_guard_refuses_writes_outside_the_write_path() {
        let (client, ilias) = fake(|url| reply(url, "<p>ok</p>"));
        for link in [
            "ilias.php?cmd=addToDesk&item_ref_id=1",
            "ilias.php?cmd=markPostRead&pos_pk=2",
            "ilias.php?cmd=join&ref_id=3",
            "ilias.php?baseClass=ilstartupgui&cmd=doLogout&rtoken=abc",
            "ilias.php?cmd=saveRating&rating=5&rtoken=abc",
        ] {
            let error = client.get_authed(&client.url(link).unwrap()).unwrap_err();
            assert!(error.to_string().contains("refused"), "{link}: {error}");
        }
        assert!(ilias.requests(&base()).is_empty(), "nothing reached ILIAS");
        // Reading is fine, also with a `cmd`.
        client
            .get_authed(&client.url("ilias.php?cmd=viewThread&thr_pk=1").unwrap())
            .unwrap();
    }

    #[test]
    fn deep_search_follows_folders_but_not_forums() {
        let link = |kind: &str, id: u32, title: &str| {
            format!(
                r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/{kind}/{id}">{title}</a></div>"#
            )
        };
        let (client, ilias) = fake(move |url| {
            let html = match url.path() {
                "/ilias/goto.php/crs/1" => {
                    [link("fold", 2, "Unterlagen"), link("frm", 3, "Forum")].concat()
                }
                "/ilias/goto.php/fold/2" => {
                    [link("file", 4, "Folien"), link("fold", 5, "Woche 1")].concat()
                }
                "/ilias/goto.php/fold/5" => link("file", 6, "Blatt 1"),
                other => return Err(eyre!("unexpected request {other}")),
            };
            reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
        });
        let course = Item {
            kind: "crs".into(),
            ref_id: 1,
            url: "https://ilias.example/ilias/goto.php/crs/1".into(),
            ..Item::default()
        };
        let found: Vec<String> = client
            .descendants(Some(&course), |_| {})
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(
            found,
            [
                "Unterlagen",
                "Unterlagen › Folien",
                "Unterlagen › Woche 1",
                "Unterlagen › Woche 1 › Blatt 1",
                "Forum"
            ]
        );
        assert_eq!(ilias.requests(&base()).len(), 3, "the forum is not crawled");
    }

    #[test]
    fn offline_sends_nothing() {
        let (client, ilias) = fake(|url| reply(url, "<p>ok</p>"));
        client.set_offline(true);
        let error = client
            .get_authed(&client.url("goto.php/crs/1").unwrap())
            .unwrap_err();
        assert_eq!(error.to_string(), OFFLINE);
        assert!(
            client
                .download(&Item::default(), Path::new("/nonexistent"))
                .is_err()
        );
        assert!(ilias.requests(&base()).is_empty());

        client.set_offline(false);
        assert!(
            client
                .get_authed(&client.url("goto.php/crs/1").unwrap())
                .is_ok()
        );
    }
}
