//! HTTP client for an ILIAS installation, authenticated with the session cookies.

pub mod parse;
pub mod write;

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

pub use parse::{Block, Dashboard, Item, Page, Post, parse_page, parse_target};

/// The "My courses and groups" page.
const MEMBERSHIPS: &str = "ilias.php?baseClass=ilmembershipoverviewgui";

/// The dashboard; only its favourites and "To-Do" (upcoming deadlines) blocks are used.
const DASHBOARD: &str = "ilias.php?baseClass=ilDashboardGUI";

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
        Ok(Some(newest_first(parse_page(&fetched.html, &fetched.url))))
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

    /// "My courses and groups" with the favourites and the To-Do list, all fetched at the
    /// same time.
    pub fn courses(&self) -> Result<Page> {
        thread::scope(|scope| {
            let dashboard = scope.spawn(|| self.dashboard());
            let courses = self.memberships()?;
            // Without the dashboard, the courses are still worth showing.
            let dashboard = dashboard
                .join()
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            Ok(with_dashboard(courses, dashboard))
        })
    }

    /// "My courses and groups", grouped by year like on ILIAS, but newest year first.
    fn memberships(&self) -> Result<Page> {
        let fetched = self.get_authed(&self.url(MEMBERSHIPS)?)?;
        Ok(newest_first(parse_page(&fetched.html, &fetched.url)))
    }

    /// The favourites and the assignments still to hand in (the dashboard's "To-Do"
    /// block), soonest first. Each To-Do item's description says when it is due; its first
    /// property names the course.
    pub fn dashboard(&self) -> Result<Dashboard> {
        let fetched = self.get_authed(&self.url(DASHBOARD)?)?;
        let mut dashboard = parse::parse_dashboard(&fetched.html, &fetched.url);
        let items = &mut dashboard.todo;
        // The block only names the exercise; its page's breadcrumb names the course.
        let mut exercises: Vec<u64> = items.iter().map(|i| i.ref_id).collect();
        exercises.sort_unstable();
        exercises.dedup();
        let courses: HashMap<u64, String> = thread::scope(|scope| {
            let jobs: Vec<_> = exercises
                .iter()
                .map(|&id| {
                    scope.spawn(move || -> Result<_> {
                        let page = self.get_authed(&self.url(&format!("goto.php/exc/{id}"))?)?;
                        Ok((id, parse::course_title(&page.html)))
                    })
                })
                .collect();
            jobs.into_iter()
                .filter_map(|job| match job.join() {
                    Ok(Ok((id, Some(course)))) => Some((id, course)),
                    _ => None,
                })
                .collect()
        });
        let now = jiff::Zoned::now().datetime();
        let mut due: Vec<(Option<jiff::civil::DateTime>, Item)> = items
            .drain(..)
            .map(|mut item| {
                let deadline = parse::parse_date(&item.description, now.date());
                if let Some(deadline) = deadline {
                    item.description = format!("{} · {}", due_in(now, deadline), item.description);
                }
                if let Some(course) = courses.get(&item.ref_id) {
                    item.properties.insert(0, course.clone());
                }
                (deadline, item)
            })
            .collect();
        // Unknown dates last.
        due.sort_by_key(|(deadline, _)| (deadline.is_none(), *deadline));
        dashboard.todo = due.into_iter().map(|(_, item)| item).collect();
        Ok(dashboard)
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
            None => self.memberships()?.items().cloned().collect(),
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

/// Reverses the order of the year groups ("2024", "2025", …), leaving everything else in
/// place. ILIAS lists the oldest year first, but the current courses matter most.
fn newest_first(mut page: Page) -> Page {
    let is_year = |b: &Block| matches!(b, Block::Items { title, .. } if !title.is_empty() && title.chars().all(|c| c.is_ascii_digit()));
    let slots: Vec<usize> = (0..page.blocks.len())
        .filter(|&i| is_year(&page.blocks[i]))
        .collect();
    let mut years: Vec<Block> = slots.iter().map(|&i| page.blocks[i].clone()).collect();
    for &slot in &slots {
        page.blocks[slot] = years.pop().expect("one block per slot");
    }
    page
}

/// Puts the favourites above the courses and the To-Do list beside them.
pub fn with_dashboard(mut page: Page, dashboard: Dashboard) -> Page {
    if let Some(mut favorites) = dashboard.favorites {
        // Favourite tiles only carry the short title; the course list has the full name.
        if let Block::Items { items, .. } = &mut favorites {
            for favorite in items.iter_mut().filter(|f| f.description.is_empty()) {
                if let Some(known) = page.items().find(|i| i.ref_id == favorite.ref_id) {
                    favorite.description.clone_from(&known.description);
                }
            }
        }
        page.blocks.insert(0, favorites);
    }
    Page {
        todo: dashboard.todo,
        ..page
    }
}

/// `due in 3 days`, `due in 5 hours`, `overdue`.
fn due_in(now: jiff::civil::DateTime, deadline: jiff::civil::DateTime) -> String {
    let hours = now
        .until((jiff::Unit::Hour, deadline))
        .map_or(0, |span| span.get_hours());
    match hours {
        ..0 => "overdue".into(),
        0 => "due in less than an hour".into(),
        1 => "due in 1 hour".into(),
        2..24 => format!("due in {hours} hours"),
        24..48 => "due in 1 day".into(),
        _ => format!("due in {} days", hours / 24),
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
    fn lists_the_newest_year_first() {
        let group = |title: &str| Block::Items {
            title: title.into(),
            items: vec![Item::default()],
        };
        let page = Page {
            blocks: vec![
                Block::Heading("Meine Kurse und Gruppen".into()),
                group("2024"),
                group("2025"),
                group("2026"),
                group("Von Hand erstellte Kurse"),
            ],
            ..Page::default()
        };
        let titles: Vec<String> = newest_first(page)
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading(t) | Block::Items { title: t, .. } => t.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(
            titles,
            [
                "Meine Kurse und Gruppen",
                "2026",
                "2025",
                "2024",
                "Von Hand erstellte Kurse"
            ]
        );
    }

    #[test]
    fn says_when_something_is_due() {
        use jiff::civil::datetime;
        let now = datetime(2026, 11, 1, 14, 0, 0, 0);
        let due = |y, m, d, h| due_in(now, datetime(y, m, d, h, 0, 0, 0));
        assert_eq!(due(2026, 11, 5, 0), "due in 3 days");
        assert_eq!(due(2026, 11, 2, 13), "due in 23 hours");
        assert_eq!(due(2026, 11, 2, 14), "due in 1 day");
        assert_eq!(due(2026, 11, 1, 14), "due in less than an hour");
        assert_eq!(due(2026, 10, 31, 0), "overdue");
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
}
