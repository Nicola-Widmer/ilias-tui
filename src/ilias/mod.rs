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
use pages::{Form, Https, Pages};
pub use parse::{
    Block, Id, Item, Kind, Page, Post, Rights, Todo, Urgency, parse_page, parse_target,
};

/// The "My courses and groups" page.
const MEMBERSHIPS: &str = "ilias.php?baseClass=ilmembershipoverviewgui";

/// Upper bound of pages fetched by one deep search, to stay polite to the server.
const MAX_CRAWL: usize = 500;

/// Upper bound of "Show more" requests for one page (each brings the next batch of items).
const MAX_SHOW_MORE: usize = 100;

/// Shown on a page whose "Show more" list couldn't be loaded in full.
const CUT_SHORT: &str = "⚠ ILIAS cut a list on this page short (\"Show more\") and not all of it could be loaded: open it in the browser (o) to see everything.";

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
    /// Dry run (settings or `ILIAS_DRY_RUN`): favourites changes are only pretended.
    dry_run: AtomicBool,
    /// Offline mode: nothing at all is sent; every request fails with [`OFFLINE`].
    offline: AtomicBool,
    /// The course list page the session check fetched; the next course list uses it
    /// instead of fetching it again (at startup, right after).
    checked_courses: Mutex<Option<Fetched>>,
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
            dry_run: AtomicBool::new(crate::config::Config::load().dry_run()),
            offline: AtomicBool::new(false),
            checked_courses: Mutex::new(None),
        })
    }

    pub fn set_dry_run(&self, dry_run: bool) {
        self.dry_run.store(dry_run, Ordering::Relaxed);
    }

    /// Dry run: every write only says what it would have sent.
    pub fn is_dry_run(&self) -> bool {
        self.dry_run.load(Ordering::Relaxed)
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
            dry_run: AtomicBool::new(false),
            ..Self::new(base, CookieStore::default()).unwrap()
        }
    }

    pub fn url(&self, path: &str) -> Result<Url> {
        Ok(self.base.join(path)?)
    }

    /// GETs a page. Refused if it looks like a write (the write guard); writes go through
    /// `write.rs`.
    pub fn get(&self, url: &Url) -> Result<Fetched> {
        self.request(url, None, false)
    }

    /// The one place page requests leave the client: a GET, or a POST of `form`. Nothing is
    /// sent offline, and a request that looks like a write is refused unless `write` is
    /// set, which only `write.rs` does.
    fn request(&self, url: &Url, form: Option<&[Form]>, write: bool) -> Result<Fetched> {
        self.ensure_online()?;
        if !write && write::is_write(url) {
            bail!("refused to request {url}: it would change something on ILIAS");
        }
        match form {
            None => self.pages.get(url),
            Some(form) => self.pages.post(url, form),
        }
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
    /// It reads the course list, which the next [`Client::course_list`] or
    /// [`Client::my_courses`] then uses instead of asking again.
    pub fn check_session(&self) -> Result<bool> {
        let url = self.url(MEMBERSHIPS)?;
        let mut page = self.get(&url)?;
        if !self.is_logged_in(&page) {
            if !self.silent_login()? {
                return Ok(false);
            }
            page = self.get(&url)?;
        }
        let logged_in = self.is_logged_in(&page);
        if logged_in {
            *self
                .checked_courses
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(page);
        }
        Ok(logged_in)
    }

    /// The course list page the session check just read, once.
    fn take_checked_courses(&self) -> Option<Fetched> {
        self.checked_courses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// GETs a page that requires a login, refreshing the session once if it expired.
    pub fn get_authed(&self, url: &Url) -> Result<Fetched> {
        self.authed(url, false)
    }

    fn authed(&self, url: &Url, write: bool) -> Result<Fetched> {
        let page = self.request(url, None, write)?;
        if self.is_logged_in(&page) {
            return Ok(page);
        }
        if self.silent_login()? {
            let page = self.request(url, None, write)?;
            if self.is_logged_in(&page) {
                return Ok(page);
            }
        }
        bail!("session expired: press L to log in again")
    }

    /// The page of a container; `url` selects another tab of it. Lists that ILIAS cut short
    /// behind "Show more" come complete.
    pub fn page(&self, item: &Item, url: Option<&str>) -> Result<Page> {
        let fetched = self.get_authed(&Url::parse(url.unwrap_or(&item.url))?)?;
        let mut page = parse_page(&fetched.html, &fetched.url);
        self.show_all(&mut page, &fetched)?;
        page.remove(item.id);
        Ok(page)
    }

    /// Loads what the page's "Show more" buttons hide, one batch per request, as the button
    /// would. If a list can't be completed, the page says so.
    fn show_all(&self, page: &mut Page, fetched: &Fetched) -> Result<()> {
        let mut pending = parse::show_more(&fetched.html, &fetched.url);
        let mut complete = !parse::has_show_more(&fetched.html) || !pending.is_empty();
        let mut requests = 0;
        while let Some(more) = pending.pop() {
            requests += 1;
            if requests > MAX_SHOW_MORE {
                complete = false;
                break;
            }
            let Some(after) = more.last_shown() else {
                complete = false;
                continue;
            };
            let answer = self.more_items(&more)?;
            let batch = parse_page(&answer.html, &more.url);
            let items = batch.items().cloned().collect();
            page.rights.delete.extend(batch.rights.delete);
            if page.insert_after(after, items) == 0 {
                continue;
            }
            for mut next in parse::show_more(&answer.html, &more.url) {
                next.shown.splice(0..0, more.shown.iter().cloned());
                pending.push(next);
            }
        }
        if !complete {
            page.blocks.push(Block::Text(CUT_SHORT.into()));
        }
        Ok(())
    }

    /// The next items of a list cut short behind "Show more". ILIAS wants a POST of the
    /// items already shown, but it only reads. It is the one POST outside `write.rs`, so on
    /// top of the write guard it only goes to this ILIAS, with the one command the button
    /// uses.
    fn more_items(&self, more: &parse::ShowMore) -> Result<Fetched> {
        self.ensure_online()?;
        let url = &more.url;
        let command = url.query_pairs().find(|(k, _)| k == "cmd").map(|(_, v)| v);
        if url.origin() != self.base.origin()
            || command.as_deref() != Some("renderBlockAsynch")
            || write::is_write(url)
        {
            bail!("refused to request {url}: it is not a \"Show more\" list");
        }
        let ids: Vec<Form> = more
            .shown
            .iter()
            .map(|id| Form::Field {
                name: "ids[]",
                value: id,
            })
            .collect();
        let answer = self.request(url, Some(&ids), false)?;
        if !self.is_logged_in(&answer) {
            bail!("session expired: press L to log in again");
        }
        Ok(answer)
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
        self.descendants_with_pages(root, progress, &|_, _| {})
    }

    /// Like [`Client::descendants`], and hands every container page it reads (the root's
    /// included) to `pages`, e.g. to keep them for offline use.
    pub fn descendants_with_pages(
        &self,
        root: Option<&Item>,
        progress: impl Fn(usize) + Sync,
        pages: &(dyn Fn(&Item, &Page) + Sync),
    ) -> Result<Vec<Item>> {
        let top = match root {
            Some(item) => {
                let page = self.page(item, None)?;
                pages(item, &page);
                page.items().cloned().collect()
            }
            None => self.course_list()?,
        };
        // 1. Fetch every container below `root`, one tree level at a time, in parallel.
        let mut queued: HashSet<Id> = root.map(|r| r.id).into_iter().collect();
        let mut children: HashMap<Id, Vec<Item>> = HashMap::new();
        let mut frontier: Vec<Item> = top
            .iter()
            .filter(|i| crawlable(i) && queued.insert(i.id))
            .cloned()
            .collect();
        let fetched = AtomicUsize::new(0);
        let done = || progress(fetched.fetch_add(1, Ordering::Relaxed) + 1);
        while !frontier.is_empty() {
            frontier.truncate(MAX_CRAWL.saturating_sub(children.len()));
            let results = self.fetch_children(&frontier, &done, pages)?;
            frontier = results
                .iter()
                .flat_map(|(_, items)| items)
                .filter(|i| crawlable(i) && queued.insert(i.id))
                .cloned()
                .collect();
            children.extend(results);
        }

        // 2. Walk the tree in display order, turning titles into paths.
        Ok(walk(root, top, |item| children.get(&item.id).cloned()))
    }

    /// Fetches the children of all `items`, `PARALLEL` requests at a time. ILIAS handles
    /// concurrent requests of one session fine (measured: 6 folders in 0.7 s instead of 4 s).
    fn fetch_children(
        &self,
        items: &[Item],
        done: &(dyn Fn() + Sync),
        pages: &(dyn Fn(&Item, &Page) + Sync),
    ) -> Result<Vec<(Id, Vec<Item>)>> {
        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::new());
        thread::scope(|scope| {
            for _ in 0..PARALLEL.min(items.len()) {
                scope.spawn(|| {
                    while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let result = self.page(item, None).map(|page| {
                            pages(item, &page);
                            (item.id, page.items().cloned().collect())
                        });
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
        // Files listed in folders link straight to `cmd=sendfile`, those of assignments to
        // `cmd=downloadFile`, handed-in ones to `cmd=download…&delivered=`, a participant's
        // submission to `cmd=downloadReturned`; otherwise use the permalink.
        let direct = [
            "cmd=sendfile",
            "cmd=downloadFile",
            "&delivered=",
            "cmd=downloadReturned",
        ];
        let url = if direct.iter().any(|d| item.url.contains(d)) {
            Url::parse(&item.url)?
        } else {
            self.url(&format!("goto.php?target=file_{}_download", item.key()))?
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
    local_folder(dir, &item.path)
}

/// The local folder of an ILIAS place (a page's or file's path from the course down).
pub fn local_folder(dir: &Path, path: &[String]) -> PathBuf {
    path.iter()
        .fold(dir.to_path_buf(), |dir, folder| dir.join(sanitize(folder)))
}

/// Whether ILIAS' title for a file is the file named `name`: ILIAS may leave out the
/// extension, and a saved copy's name is sanitised.
pub fn lists_as(title: &str, name: &str) -> bool {
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str());
    [title.to_string(), sanitize(title)]
        .iter()
        .any(|title| title == name || stem == Some(title.as_str()))
}

/// A file or folder name that is safe on every system: no path separators or characters
/// Windows refuses, no leading dots, no trailing dots or spaces (Windows drops them), and
/// none of Windows' device names (`CON`, `NUL`, `COM1`, …).
pub(crate) fn sanitize(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_start_matches('.').trim_end_matches(['.', ' ']);
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let device = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|d| stem.eq_ignore_ascii_case(d))
        || (stem.len() == 4
            && ["COM", "LPT"]
                .iter()
                .any(|d| stem.get(..3).is_some_and(|s| s.eq_ignore_ascii_case(d)))
            && stem.as_bytes()[3].is_ascii_digit());
    match name {
        "" => "download".into(),
        name if device => format!("_{name}"),
        name => name.into(),
    }
}

pub(crate) fn unique_path(dir: &Path, name: &str) -> PathBuf {
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

/// Containers whose children deep search and sync look at. Categories lead out of the
/// course into the rest of the repository; forum threads hold posts, not files, and the files
/// of assignments are read on their pages.
pub fn crawlable(item: &Item) -> bool {
    item.is_container()
        && !matches!(
            item.kind,
            Kind::Category
                | Kind::Forum
                | Kind::Thread
                | Kind::Exercise
                | Kind::Assignment
                | Kind::Submission
        )
}

/// Everything below `root` in display order, starting from its items `top`; `children`
/// gives the items of a crawlable container, if known. Titles become paths relative to
/// `root`, e.g. `Unterlagen › Woche 3 › Folien.pdf`.
pub fn walk(
    root: Option<&Item>,
    top: Vec<Item>,
    children: impl Fn(&Item) -> Option<Vec<Item>>,
) -> Vec<Item> {
    let mut seen: HashSet<Id> = root.map(|r| r.id).into_iter().collect();
    let mut stack: Vec<(String, Item)> =
        top.into_iter().rev().map(|i| (String::new(), i)).collect();
    let mut found = Vec::new();
    while let Some((parent, item)) = stack.pop() {
        if !seen.insert(item.id) {
            continue;
        }
        let path = if parent.is_empty() {
            item.title.clone()
        } else {
            format!("{parent} › {}", item.title)
        };
        if let Some(items) = crawlable(&item).then(|| children(&item)).flatten() {
            stack.extend(items.into_iter().rev().map(|c| (path.clone(), c)));
        }
        found.push(Item {
            title: path,
            ..item
        });
    }
    found
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
            Path::new("/dl/DEMO.101/Woche 1_ Intro/_")
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
            id: Id::Ref(7),
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
        // Also safe on Windows.
        assert_eq!(
            sanitize("Was <ist> \"das\"? | a*b.pdf"),
            "Was _ist_ _das__ _ a_b.pdf"
        );
        assert_eq!(sanitize("Notizen. "), "Notizen");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize("COM1"), "_COM1");
        assert_eq!(sanitize("Console.txt"), "Console.txt");
        assert_eq!(sanitize("a€.pdf"), "a€.pdf", "4 bytes, not a device name");
        assert_eq!(
            sanitize("Blatt 0, Student (student)"),
            "Blatt 0, Student (student)"
        );
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
            id: Id::Ref(1),
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

    /// A list block as ILIAS 10 renders it: the items, then a "Show more" button if `more`.
    fn list_block(ids: &[u64], more: bool) -> String {
        let items: String = ids
            .iter()
            .map(|id| {
                format!(
                    r#"<li class="ilCLI ilObjListRow"><div class="ilContainerListItemOuter" data-list-item-id="lg_div_{id}_pref_9"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id={id}">Exam {id}</a></div></li>"#
                )
            })
            .collect();
        let button = match more {
            true => format!(
                r#"<li class="ilContainerShowMore"><button class="btn btn-default" id="b{}">Show More</button></li>
                <script>il.Container.initShowMore('b{}', '_other', 'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=9&type=_other&cmdMode=asynch');</script>"#,
                ids[0], ids[0]
            ),
            false => String::new(),
        };
        format!(
            r#"<div class="ilContainerBlock"><div class="ilContainerBlockHeader"><h2>Content</h2></div><ul class="ilContainerItemsContainer">{items}{button}</ul></div>"#
        )
    }

    #[test]
    fn loads_lists_cut_short_behind_show_more() {
        let answers = Arc::new(AtomicUsize::new(0));
        let counter = answers.clone();
        let (client, ilias) = fake(move |url| match url.path() {
            "/ilias/goto.php/fold/9" => reply(
                url,
                &format!(
                    r#"<ol class="breadcrumb"><li><a href="goto.php/crs/1">DEMO</a></li></ol><div id="il_center_col">{}</div>"#,
                    list_block(&[1, 2], true)
                ),
            ),
            "/ilias/ilias.php" => match counter.fetch_add(1, Ordering::SeqCst) {
                0 => reply(url, &list_block(&[3, 4], true)),
                _ => reply(url, &list_block(&[5], false)),
            },
            other => Err(eyre!("unexpected request {other}")),
        });
        let folder = Item {
            kind: "fold".into(),
            id: Id::Ref(9),
            url: "https://ilias.example/ilias/goto.php/fold/9".into(),
            ..Item::default()
        };
        let page = client.page(&folder, None).unwrap();
        let titles: Vec<&str> = page.items().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Exam 1", "Exam 2", "Exam 3", "Exam 4", "Exam 5"]);
        assert!(
            page.items().all(|i| i.path == ["DEMO"]),
            "saved in the folder"
        );
        assert!(!page.blocks.iter().any(|b| matches!(b, Block::Text(_))));
        // Each request names every item shown so far, as the button does.
        let more = "ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=9&type=_other&cmdMode=asynch";
        assert_eq!(
            ilias.posts(&base()),
            [
                format!("{more} <- ids[]=lg_div_1_pref_9&ids[]=lg_div_2_pref_9"),
                format!(
                    "{more} <- ids[]=lg_div_1_pref_9&ids[]=lg_div_2_pref_9&ids[]=lg_div_3_pref_9&ids[]=lg_div_4_pref_9"
                ),
            ]
        );
    }

    #[test]
    fn says_so_when_a_list_stays_cut_short() {
        // A button whose address can't be read.
        let (client, ilias) = fake(|url| {
            reply(
                url,
                r#"<div id="il_center_col"><div class="ilContainerBlock"><ul class="ilContainerItemsContainer">
                <li><div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/file/1">Exam</a></div></li>
                <li class="ilContainerShowMore"><button>Show More</button></li></ul></div></div>"#,
            )
        });
        let page = client
            .page(
                &Item::default(),
                Some("https://ilias.example/ilias/goto.php/fold/9"),
            )
            .unwrap();
        assert_eq!(page.items().count(), 1);
        assert_eq!(page.blocks.last(), Some(&Block::Text(CUT_SHORT.into())));
        assert!(ilias.posts(&base()).is_empty());
    }

    #[test]
    fn show_more_only_asks_this_ilias() {
        let (client, ilias) = fake(|url| reply(url, ""));
        for link in [
            "https://evil.example/ilias/ilias.php?cmd=renderBlockAsynch",
            "https://ilias.example/ilias/ilias.php?cmd=addToDesk&item_ref_id=1",
            "https://ilias.example/ilias/ilias.php?cmd=renderBlockAsynch&rtoken=abc",
        ] {
            let more = parse::ShowMore {
                url: Url::parse(link).unwrap(),
                shown: vec!["lg_div_1_pref_9".into()],
            };
            let error = client.more_items(&more).unwrap_err();
            assert!(error.to_string().contains("refused"), "{link}: {error}");
        }
        assert!(ilias.posts(&base()).is_empty());
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
