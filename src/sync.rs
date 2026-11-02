//! Sync: saves the files of whole courses or folders into the download folder, in ILIAS's
//! folder structure, and keeps them up to date.
//!
//! - Only files up to a size limit (default 50 MB, `ILIAS_SYNC_MAX_MB`); bigger ones (lecture
//!   recordings) are skipped and counted.
//! - A file is fetched again only if its version on ILIAS (size and date in the folder
//!   listing) changed, so an unchanged course costs just the folder pages.
//! - Files you edited (size or modification time differ from right after the download) are
//!   never overwritten; a newer ILIAS version is saved beside them instead.
//! - Files you deleted stay deleted until they change on ILIAS.
//! - Every course and folder page read on the way is cached, so synced courses can be
//!   browsed offline.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use color_eyre::Result;

use crate::cache::Cache;
use crate::downloads::{Downloads, Local, Saved};
use crate::ilias::{Client, Item, Page};

/// Files bigger than this many MB are skipped unless `ILIAS_SYNC_MAX_MB` says otherwise.
const DEFAULT_MAX_MB: f64 = 50.0;

/// Downloads running at the same time; polite to the server, still much faster than one.
const PARALLEL: usize = 3;

pub fn max_mb_from_env() -> f64 {
    std::env::var("ILIAS_SYNC_MAX_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|mb: &f64| *mb > 0.0)
        .unwrap_or(DEFAULT_MAX_MB)
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
pub fn plan(item: &Item, saved: Option<(&Saved, Local)>, max_mb: f64) -> Plan {
    let version = version(item);
    let too_big = item
        .properties
        .iter()
        .find_map(|p| size_mb(p))
        .is_some_and(|mb| mb > max_mb);
    let fetch = |plan| if too_big { Plan::TooBig } else { plan };
    let Some((saved, local)) = saved else {
        return fetch(Plan::New);
    };
    let changed = saved.version != version;
    match local {
        _ if saved.version.is_empty() && local != Local::Missing => Plan::Adopt,
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
    max_mb: f64,
    progress: impl Fn(String) + Sync,
) -> Result<Summary> {
    let mut summary = Summary::default();
    let mut work: Vec<(Item, Plan)> = Vec::new();
    let keep = |item: &Item, page: &Page| cache.put(&Cache::key(Some(&item.url), false), page);
    for root in roots {
        let files = client.descendants_with_pages(
            Some(root),
            |n| progress(format!("Sync: reading {} … {n} folders", root.title)),
            &keep,
        )?;
        for mut item in files.into_iter().filter(|i| i.kind == "file") {
            // Titles are paths below the root; name the course too.
            item.title = format!("{} › {}", root.title, item.title);
            let saved = downloads.entry(item.ref_id).map(|s| (s, s.local()));
            match plan(&item, saved, max_mb) {
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
                    downloads.set_version(item.ref_id, version(&item))?;
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
    Ok(summary)
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
            .record(item.ref_id, path, version(item))
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
                .set_version(item.ref_id, version(item))?;
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
            ref_id: 1,
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
        let plan = |item: &Item, local| plan(item, Some((&saved(V1), local)), 50.0);
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
            plan(&same, Some((&saved(V1), Local::Missing)), 50.0),
            Plan::Deleted
        );
        assert_eq!(
            plan(&changed, Some((&saved(V1), Local::Missing)), 50.0),
            Plan::New
        );
    }

    #[test]
    fn skips_big_files_and_adopts_old_downloads() {
        let recording = file("107.9 MB", "19. Sep 2026, 23:33");
        assert_eq!(plan(&recording, None, 50.0), Plan::TooBig);
        assert_eq!(plan(&recording, None, 200.0), Plan::New);
        // Saved before versions were recorded: no duplicate, no overwrite.
        let old = file("1.2 MB", "1. Nov 2026, 10:00");
        assert_eq!(
            plan(&old, Some((&saved(""), Local::Edited)), 50.0),
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
            ref_id: 1,
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
            50.0,
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
