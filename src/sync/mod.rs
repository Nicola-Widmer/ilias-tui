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
//! - Two-way sync (settings, off by default) also decides what to send to ILIAS: your new
//!   and edited files, and deletions to propose for the trash.
//!
//! One model for both: the [`model::Base`], ILIAS now ([`model::Remote`]) and your disk now
//! ([`model::Local`]) go into the pure [`diff::diff`], which gives one [`ops::Op`] per path;
//! the [`run`] executor carries them out. A sync runs what doesn't write to ILIAS (the
//! downloads) straight away; two-way, what writes waits for the review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use color_eyre::Result;

use crate::cache::Cache;
use crate::ilias::{Client, Id, Item, Kind, Page};

pub mod ask;
pub mod diff;
pub mod journal;
pub mod local;
pub mod model;
pub mod ops;
pub mod report;
pub mod run;

pub use ops::Op;
pub use report::{Review, Summary};

use model::Remote;
use ops::Settle;

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

    pub fn max_bytes(&self) -> u64 {
        (self.max_mb * 1e6) as u64
    }
}

impl Default for Rules {
    fn default() -> Self {
        Self::new(&crate::config::Config::default())
    }
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

/// Syncs every file below `roots` into the download folder (the base's root): reads ILIAS,
/// your disk and the base, decides every path ([`diff::diff`]), settles the base and runs
/// what doesn't write to ILIAS (the downloads). What would write is in the summary's
/// review. Every course and folder page read on the way goes into `cache`, so the synced
/// courses can be browsed offline. `progress` gets a status line now and then.
pub fn sync(
    client: &Client,
    roots: &[Item],
    base: &mut model::Base,
    journal: &mut journal::Journal,
    cache: &Cache,
    rules: Rules,
    progress: impl Fn(String) + Sync,
) -> Result<Summary> {
    let places = match roots {
        [] => String::new(),
        [one] => one.title.clone(),
        [one, rest @ ..] => format!("{} and {} more", one.title, rest.len()),
    };
    let (remote, unseen) = crawl(client, roots, cache, &progress)?;
    let local = model::Local::read(base, &remote, rules.two_way);
    let plan = diff::diff(base, &remote, &local, &rules);
    settle(base, &plan.settle)?;
    // Straight away: what only saves from ILIAS. What writes waits for the review, your
    // Trash for its question.
    let now: Vec<Op> = (plan.ops.iter())
        .filter(|op| !op.writes() && !matches!(op, Op::TrashLocal { .. }))
        .cloned()
        .collect();
    let done = run::run(client, &now, base, journal, &progress);
    // Folders the downloads just made are saved folders too.
    let made: Vec<Settle> = (remote.places.iter())
        .filter(|p| !base.has_folder(&p.dir) && base.abs(&p.dir).is_dir())
        .map(|p| Settle::Folder {
            path: p.dir.clone(),
            id: p.item.key(),
        })
        .collect();
    settle(base, &made)?;
    Ok(Summary {
        review: Review::new(&plan.ops, client.is_dry_run()),
        ops: plan.ops,
        unchanged: plan.unchanged,
        done,
        saved: base.saved(),
        unseen,
        two_way: rules.two_way,
        places,
    })
}

/// Reads ILIAS below `roots`: the places and files, and what's new on each page.
fn crawl(
    client: &Client,
    roots: &[Item],
    cache: &Cache,
    progress: &(impl Fn(String) + Sync),
) -> Result<(Remote, Vec<FoundNew>)> {
    let remote = Mutex::new(Remote::default());
    let mut unseen = Vec::new();
    // Ids of the containers read so far.
    let mut read: std::collections::HashSet<Id> = std::collections::HashSet::new();
    let mut files = Vec::new();
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
            remote
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .add_page(item, page);
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
        for mut item in items.into_iter().filter(|i| i.kind == Kind::File) {
            // Titles are paths below the root; name the course too.
            item.title = format!("{} › {}", root.title, item.title);
            files.push(item);
        }
    }
    let mut remote = remote.into_inner().unwrap_or_else(PoisonError::into_inner);
    for file in files {
        remote.add_file(file);
    }
    Ok((remote, unseen))
}

/// Changes to the base alone, in one write.
fn settle(base: &mut model::Base, settle: &[Settle]) -> Result<()> {
    if settle.is_empty() {
        return Ok(());
    }
    base.change(|e| {
        for change in settle {
            match change {
                Settle::Forget(path) => e.forget_file(path),
                Settle::ForgetFolder(path) => {
                    e.folders.remove(path);
                }
                Settle::Adopt { path, version } => {
                    if let Some(entry) = e.files.get_mut(path) {
                        entry.version.clone_from(version);
                    }
                }
                Settle::Hash { path, hash } => {
                    if let Some(entry) = e.files.get_mut(path) {
                        entry.hash.clone_from(hash);
                    }
                }
                Settle::Folder { path, id } => e.record_folder(path, Some(*id)),
            }
        }
    })
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

/// `…/Blatt 3.pdf` → `…/Blatt 3 (ILIAS 2026-11-02).pdf`, or `… (ILIAS 2026-11-02) (1).pdf`
/// if that exists.
pub fn beside_name(path: &Path, date: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let name = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem} (ILIAS {date}).{ext}"),
        None => format!("{stem} (ILIAS {date})"),
    };
    crate::ilias::unique_path(path.parent().unwrap_or(Path::new(".")), &name)
}

#[cfg(test)]
mod tests;
