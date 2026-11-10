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
//! - Only copies in the download folder count: after it changed, files are saved again
//!   there, and the old copies are left alone.
//! - With "missing files only" (settings), it's simpler: every file that isn't on disk is
//!   downloaded (also the ones you deleted), and every file that is stays as it is.
//! - Each file is saved once per sync, even if ILIAS lists it in several places: "My
//!   courses" shows a course both under Favourites and its semester, a group both on its
//!   own and inside its course. Files are saved by their ILIAS path, so every listing
//!   would save the same file into the same folder again (`Blatt 3 (1).pdf`, …).
//! - Every course and folder page read on the way is cached, so synced courses can be
//!   browsed offline.
//! - Two-way sync (settings, off by default) also plans what to send to ILIAS: your new
//!   and edited files, and deletions to propose for the trash ([`review`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use color_eyre::Result;

use crate::cache::Cache;
use crate::downloads::{Downloads, Local, Saved};
use crate::ilias::{Client, Id, Item, Kind, Page};

pub mod ask;
pub mod journal;
mod local;
pub mod push;
pub mod review;

use review::{Candidates, Place};
pub use review::{Change, Review};

/// The report's section of proposed deletions, which the review shows as checkboxes.
pub const PROPOSALS: &str = "Deleted by you: to the ILIAS trash (Space unticks)";

/// Downloads running at the same time; polite to the server, still much faster than one.
const PARALLEL: usize = 3;

/// How sync decides, from the settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Files bigger than this many MB are skipped.
    pub max_mb: f64,
    /// Fetch only files missing on disk (deleted ones too); never touch existing ones.
    pub missing_only: bool,
    /// Also plan what to send to ILIAS (your new and edited files, deletions to propose).
    pub two_way: bool,
}

impl Rules {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            max_mb: config.sync_max_mb(),
            missing_only: config.missing_only,
            two_way: config.two_way(),
        }
    }

    /// What sync will do, for the question before syncing everything.
    pub fn describe(&self) -> String {
        let saves = if self.missing_only {
            format!(
                "Saves every missing file up to {} MB (also ones you deleted); files you have stay as they are.",
                self.max_mb
            )
        } else {
            format!("Saves every file up to {} MB.", self.max_mb)
        };
        match self.two_way {
            true => format!("{saves} Then shows what it would send to ILIAS."),
            false => saves,
        }
    }

    fn max_bytes(&self) -> u64 {
        (self.max_mb * 1e6) as u64
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
    /// Two-way: you edited it, ILIAS hasn't changed it: your copy replaces ILIAS' file.
    Replace(PathBuf),
    /// Two-way: you deleted it, ILIAS hasn't changed it: propose the trash.
    Trash(PathBuf),
    /// Two-way: changed on disk, but saved before content hashes were recorded, so whether
    /// you edited it can't be told: left as it is.
    Unsure(PathBuf),
}

/// The file's version as ILIAS lists it now: size and last change, e.g. `6.92 MB ·
/// 2026-09-17T14:50:00`. Other properties (type, "new" marks, …) don't count. ILIAS writes
/// recent dates relative to today ("Today, 14:50"), which changes overnight; they are
/// written out with today's date, so the version stays the same.
pub fn version(item: &Item) -> String {
    let listed = item
        .properties
        .iter()
        .filter(|p| size_mb(p).is_some() || p.contains(", ") || p.starts_with("Version"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    absolute(&listed, jiff::Zoned::now().date())
}

/// `version` with every date in it made absolute, "Today" being `day`. Dates that are
/// absolute already stay as they are.
pub fn absolute(version: &str, day: jiff::civil::Date) -> String {
    version
        .split(" · ")
        .map(|part| match crate::ilias::parse::parse_date(part, day) {
            Some(date) => date.to_string(),
            None => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The version recorded for a saved file, comparable with [`version`]: one recorded with
/// ILIAS' relative dates (by older versions) reads them on the day the copy was saved.
pub fn saved_version(saved: &Saved) -> String {
    let day = jiff::Timestamp::from_millisecond(saved.modified_ms as i64)
        .ok()
        .filter(|_| saved.modified_ms > 0)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date())
        .unwrap_or_else(|| jiff::Zoned::now().date());
    absolute(&saved.version, day)
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
    let changed = saved_version(saved) != version;
    let path = || saved.path.clone();
    match local {
        _ if saved.version.is_empty() && local != Local::Missing => Plan::Adopt,
        Local::Missing if rules.missing_only => fetch(Plan::New),
        Local::Edited if rules.two_way && !changed && !saved.has_hash() => Plan::Unsure(path()),
        Local::Edited if rules.two_way && !changed => Plan::Replace(path()),
        _ if rules.missing_only => Plan::Unchanged,
        Local::Missing if changed => fetch(Plan::New),
        Local::Missing if rules.two_way => Plan::Trash(path()),
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
    /// Two-way: what to send to ILIAS, once you agree.
    pub review: Review,
    pub two_way: bool,
}

/// New items a sync found on one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundNew {
    /// The synced course or folder.
    pub root: u64,
    /// The folders between `root` and `container`, from the top.
    pub path: Vec<u64>,
    pub container: u64,
    pub new: crate::seen::New,
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
        let review = &self.review;
        let (trash, renamed) = (review.proposals().count(), review.renames().count());
        let kept = self.deleted.saturating_sub(trash + renamed);
        if trash > 0 {
            parts.push(format!("{trash} deleted by you (to the ILIAS trash)"));
        }
        if kept > 0 && self.two_way {
            parts.push(format!(
                "{kept} deleted by you (ILIAS doesn't let you delete them there)"
            ));
        } else if kept > 0 {
            parts.push(format!("{kept} deleted by you (not downloaded again)"));
        }
        let to_send = self.review.changes.len();
        if to_send > 0 {
            parts.push(format!("{to_send} to send to ILIAS (see the report)"));
        }
        if to_send > 0 && self.review.dry_run {
            parts.push("dry run: nothing will be sent".into());
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
        let shorten = |path: &Path| path.strip_prefix(dir).unwrap_or(path).display().to_string();
        let review = &self.review;
        let changes = |changes: &mut dyn Iterator<Item = &Change>| {
            changes
                .map(|c| match c {
                    Change::Upload {
                        renamed_from: Some(old),
                        path,
                        ..
                    } => format!("{} (renamed from {})", shorten(path), shorten(old)),
                    Change::Rename { from, path, .. } => {
                        format!("{} (was {})", shorten(path), shorten(from))
                    }
                    Change::NewFolder { dir, contents, .. } => {
                        let files = contents.iter().filter(|p| !p.is_dir()).count();
                        format!("{}/ ({files} files in it)", shorten(dir))
                    }
                    c => shorten(c.path()),
                })
                .collect()
        };
        let paths = |paths: &[PathBuf]| paths.iter().map(|p| shorten(p)).collect();
        // Conflicts name full paths; shorten them too.
        let prefix = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
        let conflicts = review
            .conflicts
            .iter()
            .map(|c| c.replace(&prefix, ""))
            .collect();
        [
            (
                "Changed on ILIAS, kept your edited copy; new version saved as".to_string(),
                self.beside.iter().map(|p| shorten(p)).collect(),
            ),
            (
                "New on your disk: to upload".to_string(),
                changes(&mut review.uploads()),
            ),
            (
                "Edited by you: to replace on ILIAS (its earlier versions are deleted)".to_string(),
                changes(&mut review.replacements()),
            ),
            (
                "Renamed by you: to rename on ILIAS (its earlier versions are deleted)".to_string(),
                changes(&mut review.renames()),
            ),
            (PROPOSALS.to_string(), changes(&mut review.proposals())),
            (
                "New folders on your disk: to create on ILIAS, with what is in them".to_string(),
                changes(&mut review.new_folders()),
            ),
            (
                "Conflicts: nothing sent, both copies kept".to_string(),
                conflicts,
            ),
            (
                "Gone on ILIAS, kept on your disk".to_string(),
                paths(&review.gone),
            ),
            (
                "Only on your disk: ILIAS doesn't let you upload or change it there".to_string(),
                paths(&review.local_only),
            ),
            (
                "Changed on your disk, saved by an older version: can't tell if edited, not sent"
                    .to_string(),
                paths(&review.unsure),
            ),
            (
                format!("Not sent: bigger than {max_mb} MB"),
                paths(&review.too_big),
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
    let mut summary = Summary {
        two_way: rules.two_way,
        ..Summary::default()
    };
    let mut work: Vec<(Item, Plan)> = Vec::new();
    let mut unseen = Vec::new();
    // Ref ids of the containers read and the files planned so far.
    let mut read: HashSet<Id> = HashSet::new();
    let mut planned: HashSet<Id> = HashSet::new();
    // Two-way: the places read, the files ILIAS lists, and what the per-file plan leaves.
    let places = Mutex::new(Vec::new());
    let mut on_ilias = HashSet::new();
    let mut candidates = Candidates::default();
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
            if rules.two_way && !page.path.is_empty() {
                let place = Place {
                    item: item.clone(),
                    dir: crate::ilias::local_folder(dir, &page.path),
                    rights: page.rights.clone(),
                    items: page.items().cloned().collect(),
                };
                places
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(place);
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
        for (container, new) in found.into_inner().unwrap_or_else(PoisonError::into_inner) {
            unseen.push(FoundNew {
                root: root.key(),
                path: path_below(root.key(), container, &parents),
                container,
                new,
            });
        }
        read.extend(items.iter().filter(|i| i.kind != Kind::File).map(|i| i.id));
        on_ilias.extend(items.iter().filter(|i| i.kind == Kind::File).map(Item::key));
        let files = items
            .into_iter()
            .filter(|i| i.kind == Kind::File && planned.insert(i.id));
        for mut item in files {
            // Titles are paths below the root; name the course too.
            item.title = format!("{} › {}", root.title, item.title);
            let saved = base(downloads, item.key(), dir).map(|s| (s, s.local()));
            let has_hash = saved.is_some_and(|(s, _)| s.has_hash());
            match plan(&item, saved, rules) {
                Plan::Unchanged => {
                    if !has_hash {
                        downloads.add_hash(item.key())?;
                    }
                    summary.unchanged += 1;
                }
                Plan::Deleted => summary.deleted += 1,
                Plan::Replace(path) => {
                    summary.unchanged += 1;
                    candidates.edited.push((item, path));
                }
                Plan::Trash(path) => {
                    summary.deleted += 1;
                    candidates.deleted.push((item, path));
                }
                Plan::Unsure(path) => {
                    summary.unchanged += 1;
                    candidates.unsure.push(path);
                }
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
    if rules.two_way {
        let places = places.into_inner().unwrap_or_else(PoisonError::into_inner);
        summary.review =
            review::decide(&places, &on_ilias, candidates, downloads, rules.max_bytes());
        summary.review.dry_run = client.is_dry_run();
    }
    Ok(summary)
}

/// What sync saved of file `key` in the download folder `dir`, if anything. A copy saved
/// elsewhere (before the download folder changed) is left as it is: it is neither updated
/// nor taken as deleted, and the file is saved again in `dir`.
fn base<'a>(downloads: &'a Downloads, key: u64, dir: &Path) -> Option<&'a Saved> {
    downloads
        .entry(key)
        .filter(|saved| saved.path.starts_with(dir))
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
mod tests;
