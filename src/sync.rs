//! Sync: saves the files of whole courses or folders into the download folder, in ILIAS's
//! folder structure, and keeps them up to date.
//!
//! - Only files up to a size limit (default 50 MB, settings or `ILIAS_SYNC_MAX_MB`); bigger
//!   ones (lecture recordings) are skipped and counted.
//! - A file is fetched again only if its version on ILIAS (size and date in the folder
//!   listing) changed, so an unchanged course costs just the folder pages.
//! - Files you edited (size or modification time differ from right after the download) are
//!   never overwritten; a newer ILIAS version is saved beside them instead.
//! - Files you deleted stay deleted until they change on ILIAS.
//! - With "missing files only" (settings), it's simpler: every file that isn't on disk is
//!   downloaded (also the ones you deleted), and every file that is stays as it is.
//! - Each file is saved once per sync, even if ILIAS lists it in several places: "My
//!   courses" shows a course both under Favourites and its semester, a group both on its
//!   own and inside its course. Files are saved by their ILIAS path, so every listing
//!   would save the same file into the same folder again (`Blatt 3 (1).pdf`, …).
//! - Every course and folder page read on the way is cached, so synced courses can be
//!   browsed offline.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use color_eyre::Result;

use crate::cache::Cache;
use crate::downloads::{Downloads, Local, Saved};
use crate::ilias::{Client, Id, Item, Kind, Page};

/// Downloads running at the same time; polite to the server, still much faster than one.
const PARALLEL: usize = 3;

/// How sync decides, from the settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Files bigger than this many MB are skipped.
    pub max_mb: f64,
    /// Fetch only files missing on disk (deleted ones too); never touch existing ones.
    pub missing_only: bool,
}

impl Rules {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            max_mb: config.sync_max_mb(),
            missing_only: config.missing_only,
        }
    }

    /// What sync will do, for the question before syncing everything.
    pub fn describe(&self) -> String {
        if self.missing_only {
            format!(
                "Saves every missing file up to {} MB (also ones you deleted); files you have stay as they are.",
                self.max_mb
            )
        } else {
            format!("Saves every file up to {} MB.", self.max_mb)
        }
    }
}

impl Default for Rules {
    fn default() -> Self {
        Self::new(&crate::config::Config::default())
    }
}

/// What sync does with one file.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Not saved yet (or deleted and since changed on ILIAS): download it.
    New,
    /// Changed on ILIAS, your copy is untouched: replace it.
    Update(PathBuf),
    /// Changed on ILIAS, but you edited your copy: save the new version beside it.
    Beside(PathBuf),
    /// Saved before this index knew versions: assume it is current, remember the version.
    Adopt,
    Unchanged,
    /// You deleted it and it hasn't changed since.
    Deleted,
    TooBig,
}

/// The file's version as ILIAS lists it: size and last change, e.g. `6.92 MB · 17. Sep 2026,
/// 14:50`. Other properties (type, "new" marks, …) don't count.
pub fn version(item: &Item) -> String {
    item.properties
        .iter()
        .filter(|p| size_mb(p).is_some() || p.contains(", ") || p.starts_with("Version"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `"107.9 MB"` → 107.9; `"512 KB"` → 0.512.
fn size_mb(property: &str) -> Option<f64> {
    let (number, unit) = property.trim().rsplit_once(' ')?;
    let number: f64 = number.replace(',', ".").parse().ok()?;
    let factor = match unit {
        "B" | "Bytes" => 1e-6,
        "KB" | "kB" => 1e-3,
        "MB" => 1.0,
        "GB" => 1e3,
        _ => return None,
    };
    Some(number * factor)
}

/// Decides what to do with `item`, given what was saved before and how the file on disk is.
pub fn plan(item: &Item, saved: Option<(&Saved, Local)>, rules: Rules) -> Plan {
    let version = version(item);
    let too_big = item
        .properties
        .iter()
        .find_map(|p| size_mb(p))
        .is_some_and(|mb| mb > rules.max_mb);
    let fetch = |plan| if too_big { Plan::TooBig } else { plan };
    let Some((saved, local)) = saved else {
        return fetch(Plan::New);
    };
    let changed = saved.version != version;
    match local {
        _ if saved.version.is_empty() && local != Local::Missing => Plan::Adopt,
        Local::Missing if rules.missing_only => fetch(Plan::New),
        _ if rules.missing_only => Plan::Unchanged,
        Local::Missing if changed => fetch(Plan::New),
        Local::Missing => Plan::Deleted,
        _ if !changed => Plan::Unchanged,
        Local::Untouched => fetch(Plan::Update(saved.path.clone())),
        Local::Edited => fetch(Plan::Beside(saved.path.clone())),
    }
}

#[derive(Debug, Default)]
pub struct Summary {
    pub new: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    /// Changed on ILIAS but edited by you: where the new version was saved beside.
    pub beside: Vec<PathBuf>,
    /// Skipped for their size: `Course › Folder › File (107.9 MB)`.
    pub too_big: Vec<String>,
    /// `Course › Folder › File: error`.
    pub failed: Vec<String>,
    /// Ref ids of files that are saved now.
    pub saved: Vec<u64>,
    /// What's new on the pages read, compared to their cached copies, for
    /// [`crate::seen::Seen::record`].
    pub unseen: Vec<FoundNew>,
}

/// New items a sync found on one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundNew {
    /// The synced course or folder.
    pub root: u64,
    /// The folders between `root` and `container`, from the top.
    pub path: Vec<u64>,
    pub container: u64,
    pub items: Vec<u64>,
}

impl Summary {
    /// One line for the status bar.
    pub fn text(&self, max_mb: f64) -> String {
        let mut parts = vec![
            format!("{} new", self.new),
            format!("{} updated", self.updated),
        ];
        if !self.beside.is_empty() {
            parts.push(format!(
                "{} edited by you (new version saved beside)",
                self.beside.len()
            ));
        }
        parts.push(format!("{} unchanged", self.unchanged));
        if self.deleted > 0 {
            parts.push(format!("{} deleted by you (left out)", self.deleted));
        }
        if !self.too_big.is_empty() {
            parts.push(format!("{} over {max_mb} MB skipped", self.too_big.len()));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        format!("Synced: {}", parts.join(", "))
    }

    /// The files worth a closer look, by section; empty if there are none. `dir` shortens
    /// the paths of new versions to where they are below the download folder.
    pub fn report(&self, dir: &Path, max_mb: f64) -> Vec<(String, Vec<String>)> {
        let shorten = |path: &PathBuf| path.strip_prefix(dir).unwrap_or(path).display().to_string();
        [
            (
                "Changed on ILIAS, kept your edited copy; new version saved as".to_string(),
                self.beside.iter().map(shorten).collect(),
            ),
            (
                format!("Skipped: bigger than {max_mb} MB"),
                self.too_big.clone(),
            ),
            ("Failed".to_string(), self.failed.clone()),
        ]
        .into_iter()
        .filter(|(_, lines): &(String, Vec<String>)| !lines.is_empty())
        .collect()
    }
}

/// Syncs every file below `roots` into `dir`. Every course and folder page read on the way
/// goes into `cache`, so the synced courses can be browsed offline. `progress` gets a
/// status line now and then.
pub fn sync(
    client: &Client,
    roots: &[Item],
    dir: &Path,
    downloads: &mut Downloads,
    cache: &Cache,
    rules: Rules,
    progress: impl Fn(String) + Sync,
) -> Result<Summary> {
    let mut summary = Summary::default();
    let mut work: Vec<(Item, Plan)> = Vec::new();
    let mut unseen = Vec::new();
    // Ref ids of the containers read and the files planned so far.
    let mut read: HashSet<Id> = HashSet::new();
    let mut planned: HashSet<Id> = HashSet::new();
    for root in roots {
        // Listed twice, or already read as part of an earlier root (a group in its course).
        if !read.insert(root.id) {
            continue;
        }
        // Which page listed each folder, to find the way down to the new items.
        let parents = Mutex::new(HashMap::new());
        let found = Mutex::new(Vec::new());
        let keep = |item: &Item, page: &Page| {
            let mut parents = parents.lock().unwrap_or_else(PoisonError::into_inner);
            for child in page.items().filter(|i| i.is_container()) {
                parents.entry(child.key()).or_insert(item.key());
            }
            let new = cache.update(&Cache::key(Some(&item.url), false), page);
            if !new.is_empty() {
                let mut found = found.lock().unwrap_or_else(PoisonError::into_inner);
                found.push((item.key(), new));
            }
        };
        let items = client.descendants_with_pages(
            Some(root),
            |n| progress(format!("Sync: reading {} … {n} folders", root.title)),
            &keep,
        )?;
        let parents = parents.into_inner().unwrap_or_else(PoisonError::into_inner);
        for (container, items) in found.into_inner().unwrap_or_else(PoisonError::into_inner) {
            unseen.push(FoundNew {
                root: root.key(),
                path: path_below(root.key(), container, &parents),
                container,
                items,
            });
        }
        read.extend(items.iter().filter(|i| i.kind != Kind::File).map(|i| i.id));
        let files = items
            .into_iter()
            .filter(|i| i.kind == Kind::File && planned.insert(i.id));
        for mut item in files {
            // Titles are paths below the root; name the course too.
            item.title = format!("{} › {}", root.title, item.title);
            let saved = downloads.entry(item.key()).map(|s| (s, s.local()));
            match plan(&item, saved, rules) {
                Plan::Unchanged => summary.unchanged += 1,
                Plan::Deleted => summary.deleted += 1,
                Plan::TooBig => {
                    let size = item.properties.iter().find(|p| size_mb(p).is_some());
                    summary.too_big.push(match size {
                        Some(size) => format!("{} ({size})", item.title),
                        None => item.title.clone(),
                    });
                }
                Plan::Adopt => {
                    downloads.set_version(item.key(), version(&item))?;
                    summary.unchanged += 1;
                }
                plan => work.push((item, plan)),
            }
        }
    }

    let total = work.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let downloads = Mutex::new(downloads);
    let summary = Mutex::new(summary);
    thread::scope(|scope| {
        for _ in 0..PARALLEL.min(total) {
            scope.spawn(|| {
                while let Some((item, plan)) = work.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let result = fetch(client, item, plan, dir, &downloads);
                    let mut summary = summary.lock().unwrap_or_else(PoisonError::into_inner);
                    match (result, plan) {
                        (Ok(_), Plan::New) => summary.new += 1,
                        (Ok(_), Plan::Update(_)) => summary.updated += 1,
                        (Ok(path), _) => summary.beside.push(path),
                        (Err(e), _) => summary.failed.push(format!("{}: {e:#}", item.title)),
                    }
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(format!("Sync: {n}/{total} files"));
                }
            });
        }
    });
    let mut summary = summary.into_inner().unwrap_or_else(PoisonError::into_inner);
    let downloads = downloads
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    summary.saved = downloads.saved().collect();
    summary.unseen = unseen;
    Ok(summary)
}

/// The folders between `root` and `container`, from the top, following each folder's
/// parent (the page that listed it).
fn path_below(root: u64, container: u64, parents: &HashMap<u64, u64>) -> Vec<u64> {
    let mut path = Vec::new();
    let mut at = container;
    while let Some(&parent) = parents.get(&at) {
        if parent == root || path.contains(&parent) || path.len() > 100 {
            break;
        }
        path.push(parent);
        at = parent;
    }
    path.reverse();
    path
}

/// Downloads one file and puts it where `plan` says; returns where it ended up.
fn fetch(
    client: &Client,
    item: &Item,
    plan: &Plan,
    dir: &Path,
    downloads: &Mutex<&mut Downloads>,
) -> Result<PathBuf> {
    // Always into a new file first; nothing existing is touched until it is complete.
    let fresh = client.download(item, dir)?;
    let record = |path: PathBuf| {
        downloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(item.key(), path, version(item))
    };
    match plan {
        Plan::Update(path) => {
            std::fs::rename(&fresh, path)?;
            record(path.clone())?;
            Ok(path.clone())
        }
        Plan::Beside(path) => {
            // Your edited file keeps its place and stays "edited"; only the version is new.
            let today = jiff::Zoned::now().date().to_string();
            let beside = beside_name(path, &today);
            std::fs::rename(&fresh, &beside)?;
            downloads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_version(item.key(), version(item))?;
            Ok(beside)
        }
        _ => {
            record(fresh.clone())?;
            Ok(fresh)
        }
    }
}

/// `…/Blatt 3.pdf` → `…/Blatt 3 (ILIAS 2026-11-02).pdf`, or `… (ILIAS 2026-11-02) (1).pdf`
/// if that exists.
fn beside_name(path: &Path, date: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let name = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem} (ILIAS {date}).{ext}"),
        None => format!("{stem} (ILIAS {date})"),
    };
    crate::ilias::unique_path(path.parent().unwrap_or(Path::new(".")), &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn file(size: &str, date: &str) -> Item {
        Item {
            kind: "file".into(),
            id: Id::Ref(1),
            title: "Blatt 3".into(),
            properties: vec!["pdf".into(), size.into(), date.into()],
            ..Item::default()
        }
    }

    fn saved(version: &str) -> Saved {
        Saved {
            path: PathBuf::from("/dl/DEMO.101/Blatt 3.pdf"),
            version: version.into(),
            len: 1,
            modified_ms: 1,
        }
    }

    const V1: &str = "1.2 MB · 1. Nov 2026, 10:00";

    const RULES: Rules = Rules {
        max_mb: 50.0,
        missing_only: false,
    };

    #[test]
    fn missing_only_restores_deleted_files_and_leaves_the_rest() {
        let rules = Rules {
            missing_only: true,
            ..RULES
        };
        let same = file("1.2 MB", "1. Nov 2026, 10:00");
        let changed = file("1.3 MB", "2. Nov 2026, 09:00");
        let plan = |item: &Item, local| plan(item, Some((&saved(V1), local)), rules);
        // Deleted by you: back, changed on ILIAS or not.
        assert_eq!(plan(&same, Local::Missing), Plan::New);
        assert_eq!(plan(&changed, Local::Missing), Plan::New);
        // On disk: never replaced, no new version beside it.
        assert_eq!(plan(&changed, Local::Untouched), Plan::Unchanged);
        assert_eq!(plan(&changed, Local::Edited), Plan::Unchanged);
        assert_eq!(super::plan(&same, None, rules), Plan::New);
        let recording = file("107.9 MB", "19. Sep 2026, 23:33");
        assert_eq!(
            super::plan(&recording, Some((&saved(V1), Local::Missing)), rules),
            Plan::TooBig
        );
    }

    #[test]
    fn reads_the_version_and_size_from_the_listing() {
        let item = file("1.2 MB", "1. Nov 2026, 10:00");
        assert_eq!(version(&item), V1);
        assert_eq!(size_mb("107.9 MB"), Some(107.9));
        assert_eq!(size_mb("512 KB"), Some(0.512));
        assert_eq!(size_mb("6,92 MB"), Some(6.92));
        assert_eq!(size_mb("pdf"), None);
    }

    #[test]
    fn never_overwrites_your_edits() {
        let changed = file("1.3 MB", "2. Nov 2026, 09:00");
        let path = PathBuf::from("/dl/DEMO.101/Blatt 3.pdf");
        let plan = |item: &Item, local| plan(item, Some((&saved(V1), local)), RULES);
        assert_eq!(plan(&changed, Local::Edited), Plan::Beside(path.clone()));
        assert_eq!(plan(&changed, Local::Untouched), Plan::Update(path));
        let same = file("1.2 MB", "1. Nov 2026, 10:00");
        assert_eq!(plan(&same, Local::Edited), Plan::Unchanged);
        assert_eq!(plan(&same, Local::Untouched), Plan::Unchanged);
    }

    #[test]
    fn respects_deletions_until_ilias_changes_the_file() {
        let same = file("1.2 MB", "1. Nov 2026, 10:00");
        let changed = file("1.3 MB", "2. Nov 2026, 09:00");
        assert_eq!(
            plan(&same, Some((&saved(V1), Local::Missing)), RULES),
            Plan::Deleted
        );
        assert_eq!(
            plan(&changed, Some((&saved(V1), Local::Missing)), RULES),
            Plan::New
        );
    }

    #[test]
    fn skips_big_files_and_adopts_old_downloads() {
        let recording = file("107.9 MB", "19. Sep 2026, 23:33");
        assert_eq!(plan(&recording, None, RULES), Plan::TooBig);
        let generous = Rules {
            max_mb: 200.0,
            ..RULES
        };
        assert_eq!(plan(&recording, None, generous), Plan::New);
        // Saved before versions were recorded: no duplicate, no overwrite.
        let old = file("1.2 MB", "1. Nov 2026, 10:00");
        assert_eq!(
            plan(&old, Some((&saved(""), Local::Edited)), RULES),
            Plan::Adopt
        );
    }

    #[test]
    fn names_the_new_version_beside_yours() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("Blatt 3.pdf");
        assert_eq!(
            beside_name(&mine, "2026-11-02"),
            dir.join("Blatt 3 (ILIAS 2026-11-02).pdf")
        );
        std::fs::write(dir.join("Blatt 3 (ILIAS 2026-11-02).pdf"), "x").unwrap();
        assert_eq!(
            beside_name(&mine, "2026-11-02"),
            dir.join("Blatt 3 (ILIAS 2026-11-02) (1).pdf")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notices_edits_and_deletions_on_disk() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Blatt 3.pdf");
        std::fs::write(&path, "original").unwrap();
        let mut downloads = Downloads::in_file(dir.join("index.json"));
        downloads.record(1, path.clone(), V1.into()).unwrap();
        let local = |d: &Downloads| d.entry(1).unwrap().local();
        assert_eq!(local(&downloads), Local::Untouched);

        // An annotation changes the file; recording only the new ILIAS version keeps it
        // "edited".
        std::fs::write(&path, "original + my notes").unwrap();
        assert_eq!(local(&downloads), Local::Edited);
        downloads.set_version(1, "newer".into()).unwrap();
        let reloaded = Downloads::in_file(dir.join("index.json"));
        assert_eq!(local(&reloaded), Local::Edited);
        assert_eq!(reloaded.entry(1).unwrap().version, "newer");

        std::fs::remove_file(&path).unwrap();
        assert_eq!(local(&reloaded), Local::Missing);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_old_index_files() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Skript.pdf");
        std::fs::write(&path, "x").unwrap();
        let index = dir.join("index.json");
        std::fs::write(&index, format!(r#"{{"9": {:?}}}"#, path)).unwrap();
        let downloads = Downloads::in_file(index);
        assert_eq!(downloads.get(9), Some(&path));
        // Unknown how it looked when saved: treated as edited, so never overwritten.
        assert_eq!(downloads.entry(9).unwrap().local(), Local::Edited);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_every_page_it_reads_for_offline_use() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        let folder = |id: u32, title: &str| {
            format!(
                r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/fold/{id}">{title}</a></div>"#
            )
        };
        let ilias = Arc::new(FakeIlias::new(move |url| {
            let html = match url.path() {
                "/ilias/goto.php/crs/1" => folder(2, "Unterlagen"),
                "/ilias/goto.php/fold/2" => folder(3, "Woche 1"),
                "/ilias/goto.php/fold/3" => String::new(),
                other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
            };
            reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
        }));
        let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::with_pages(base, ilias);
        let dir = std::env::temp_dir().join(format!("ilias-tui-synccache-{}", std::process::id()));
        let cache = Cache::in_dir(dir.join("cache"));
        let mut downloads = Downloads::in_file(dir.join("index.json"));
        let course = Item {
            kind: "crs".into(),
            id: Id::Ref(1),
            title: "DEMO.101".into(),
            url: "https://ilias.example/ilias/goto.php/crs/1".into(),
            ..Item::default()
        };
        sync(
            &client,
            &[course],
            &dir,
            &mut downloads,
            &cache,
            RULES,
            |_| {},
        )
        .unwrap();
        // The same keys the TUI uses when it opens these pages (offline too).
        for url in ["crs/1", "fold/2", "fold/3"] {
            let key = Cache::key(
                Some(&format!("https://ilias.example/ilias/goto.php/{url}")),
                false,
            );
            assert!(cache.get(&key).is_some(), "{url} cached");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_whats_new_with_the_folders_on_the_way() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let link = |kind: &str, id: u32| {
            format!(
                r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/{kind}/{id}">{kind} {id}</a>
                <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span><span class="il_ItemProperty">900 MB</span></div></div>"#
            )
        };
        // Course 1 › folder 2 › folder 3, whose file 10 is new in the second sync. (The
        // files are too big to download, so nothing goes to the network.)
        let second = Arc::new(AtomicBool::new(false));
        let later = second.clone();
        let ilias = Arc::new(FakeIlias::new(move |url| {
            let html = match url.path() {
                "/ilias/goto.php/crs/1" => link("fold", 2),
                "/ilias/goto.php/fold/2" => link("fold", 3),
                "/ilias/goto.php/fold/3" if later.load(Ordering::SeqCst) => {
                    [link("file", 9), link("file", 10)].concat()
                }
                "/ilias/goto.php/fold/3" => link("file", 9),
                other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
            };
            reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
        }));
        let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::with_pages(base, ilias);
        let dir = std::env::temp_dir().join(format!("ilias-tui-new-{}", std::process::id()));
        let cache = Cache::in_dir(dir.join("cache"));
        let mut downloads = Downloads::in_file(dir.join("index.json"));
        let course = Item {
            kind: "crs".into(),
            id: Id::Ref(1),
            title: "DEMO".into(),
            url: "https://ilias.example/ilias/goto.php/crs/1".into(),
            ..Item::default()
        };
        let mut run = || {
            sync(
                &client,
                std::slice::from_ref(&course),
                &dir,
                &mut downloads,
                &cache,
                RULES,
                |_| {},
            )
            .unwrap()
            .unseen
        };
        assert!(run().is_empty(), "nothing to compare with yet");
        second.store(true, Ordering::SeqCst);
        assert_eq!(
            run(),
            [FoundNew {
                root: 1,
                path: vec![2],
                container: 3,
                items: vec![10],
            }]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_only_leaves_files_on_disk_alone_even_when_ilias_changed_them() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        // The course lists one file, now 1.3 MB; the saved copy is of the 1.2 MB version.
        let ilias = Arc::new(FakeIlias::new(|url| match url.path() {
            "/ilias/goto.php/crs/1" => reply(
                url,
                r#"<div id="il_center_col"><div class="ilContainerListItemOuter">
                <a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Blatt 3</a>
                <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span>
                <span class="il_ItemProperty">1.3 MB</span></div></div></div>"#,
            ),
            other => Err(color_eyre::eyre::eyre!("unexpected {other}")),
        }));
        let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::with_pages(base, ilias);
        let dir = std::env::temp_dir().join(format!("ilias-tui-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Blatt 3.pdf");
        std::fs::write(&path, "my copy").unwrap();
        let mut downloads = Downloads::in_file(dir.join("index.json"));
        downloads.record(9, path.clone(), "1.2 MB".into()).unwrap();
        let course = Item {
            kind: "crs".into(),
            id: Id::Ref(1),
            title: "DEMO.101".into(),
            url: "https://ilias.example/ilias/goto.php/crs/1".into(),
            ..Item::default()
        };
        let rules = Rules {
            missing_only: true,
            ..RULES
        };
        let cache = Cache::in_dir(dir.join("cache"));
        let summary = sync(
            &client,
            &[course],
            &dir,
            &mut downloads,
            &cache,
            rules,
            |_| {},
        )
        .unwrap();
        // Nothing fetched (that would have failed: there is no network here), nothing touched.
        assert_eq!((summary.unchanged, summary.new, summary.updated), (1, 0, 0));
        assert!(summary.failed.is_empty() && summary.beside.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my copy");
        assert_eq!(downloads.entry(9).unwrap().local(), Local::Untouched);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_listed_in_several_places_is_synced_once() {
        use crate::ilias::pages::{FakeIlias, reply};
        use std::sync::Arc;
        // Course 1 holds group 2 (also listed on its own in My courses), which holds file 9.
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let ilias = Arc::new(FakeIlias::new(move |url| {
            counter.fetch_add(1, Ordering::SeqCst);
            let html = match url.path() {
                "/ilias/goto.php/crs/1" => {
                    r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/grp/2">Gruppe</a></div>"#
                }
                "/ilias/goto.php/grp/2" => {
                    r#"<div class="ilContainerListItemOuter">
                    <a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Blatt 3</a>
                    <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span>
                    <span class="il_ItemProperty">1.2 MB</span></div></div>"#
                }
                other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
            };
            reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
        }));
        let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::with_pages(base, ilias);
        let dir = std::env::temp_dir().join(format!("ilias-tui-twice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Saved before and current: nothing to download (there is no network here).
        let path = dir.join("Blatt 3.pdf");
        std::fs::write(&path, "x").unwrap();
        let mut downloads = Downloads::in_file(dir.join("index.json"));
        downloads.record(9, path, "1.2 MB".into()).unwrap();
        let container = |kind: &str, id: u64, title: &str| Item {
            kind: kind.into(),
            id: Id::Ref(id),
            title: title.into(),
            url: format!("https://ilias.example/ilias/goto.php/{kind}/{id}"),
            ..Item::default()
        };
        let (course, group) = (container("crs", 1, "Kurs"), container("grp", 2, "Gruppe"));
        // As "sync all" got them: the course twice, and the group inside it on its own.
        let roots = [course.clone(), group, course];
        let cache = Cache::in_dir(dir.join("cache"));
        let summary = sync(&client, &roots, &dir, &mut downloads, &cache, RULES, |_| {}).unwrap();
        assert_eq!(summary.unchanged, 1, "{summary:?}");
        assert!(summary.failed.is_empty(), "{summary:?}");
        // Course and group page, each read once.
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn says_what_sync_will_do() {
        assert_eq!(RULES.describe(), "Saves every file up to 50 MB.");
        let missing_only = Rules {
            missing_only: true,
            ..RULES
        };
        assert!(missing_only.describe().contains("also ones you deleted"));
    }

    #[test]
    fn reports_what_deserves_a_look() {
        let summary = Summary {
            new: 3,
            beside: vec![PathBuf::from("/dl/DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf")],
            too_big: vec!["DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".into()],
            ..Summary::default()
        };
        let report = summary.report(Path::new("/dl"), 50.0);
        assert_eq!(
            report,
            [
                (
                    "Changed on ILIAS, kept your edited copy; new version saved as".to_string(),
                    vec!["DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf".to_string()]
                ),
                (
                    "Skipped: bigger than 50 MB".to_string(),
                    vec!["DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".to_string()]
                ),
            ]
        );
        assert!(Summary::default().report(Path::new("/dl"), 50.0).is_empty());
        assert_eq!(
            summary.text(50.0),
            "Synced: 3 new, 0 updated, 1 edited by you (new version saved beside), 0 unchanged, 1 over 50 MB skipped"
        );
    }
}
