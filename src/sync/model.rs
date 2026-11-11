//! The three things two-way sync compares, per path:
//!
//! - [`Base`]: the saved files: what sync (or a download) recorded when ILIAS and your disk
//!   were last in sync, files and folders, by their path **relative to the download
//!   folder** (with `/` between folders, on every system). Kept next to the session as
//!   `<host>.base.json`. Entries go as soon as a file is gone on both sides, so the base
//!   always matches what is really there. Opening a saved file and the "saved" marks use
//!   it too.
//! - [`Remote`]: ILIAS now, from the crawl: the places (courses and folders) with their
//!   rights and items, and the files in them.
//! - [`Local`]: your disk now, from the scan of those places' folders.
//!
//! Only [`Base`] and [`Local::read`] touch the disk; the [`diff`](super::diff) is pure.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use color_eyre::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::local;
use crate::ilias::{Item, Kind, Page, Rights, sanitize};

/// A path relative to the download folder, folders separated by `/`: `DEMO/Unterlagen/a.pdf`.
pub type Rel = String;

/// `root/a/b` → `a/b`; `None` outside `root` (or not UTF-8).
pub fn rel(root: &Path, path: &Path) -> Option<Rel> {
    let below = path.strip_prefix(root).ok()?;
    let parts = below
        .components()
        .map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The folder `rel` is in (`""` for the download folder itself).
pub fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The last part of `rel`: the file or folder name.
pub fn name(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

/// Whether `rel` is `dir` or below it.
pub fn within(rel: &str, dir: &str) -> bool {
    rel == dir
        || rel
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The local folder of an ILIAS place (a page's or file's path from the course down), as
/// downloads name it.
pub fn rel_dir(path: &[String]) -> Rel {
    path.iter()
        .map(|p| sanitize(p))
        .collect::<Vec<_>>()
        .join("/")
}

/// Size and modification time (ms) of a file; `None` if it isn't there.
pub fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_millis() as u64))
}

/// SHA-256 of a file's content, in hex; `None` if it can't be read.
pub fn hash_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// Your copy compared with what was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disk {
    Missing,
    Untouched,
    Edited,
    /// Changed, saved without a hash: edited or only touched can't be told.
    Unsure,
}

/// A file as sync last saw it on both sides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Its reference id on ILIAS.
    pub id: u64,
    /// Its version on ILIAS (size and date from the listing, dates absolute).
    #[serde(default)]
    pub version: String,
    /// Size and modification time of your copy then.
    #[serde(default)]
    pub len: u64,
    #[serde(default)]
    pub modified_ms: u64,
    /// SHA-256 of your copy then (hex); empty for entries saved before hashes were.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

impl FileEntry {
    /// The file at `path` as it is now, as ILIAS' file `id` at `version`.
    pub fn stamp(path: &Path, id: u64, version: String) -> Self {
        let (len, modified_ms) = stat(path).unwrap_or((0, 0));
        Self {
            id,
            version,
            len,
            modified_ms,
            hash: hash_file(path).unwrap_or_default(),
        }
    }

    /// Your copy `now` (with its hash where size and time alone can't tell) compared with
    /// this entry.
    pub fn compare(&self, now: Option<&LocalFile>) -> Disk {
        match now {
            None => Disk::Missing,
            Some(f) if self.len > 0 && (f.len, f.modified_ms) == (self.len, self.modified_ms) => {
                Disk::Untouched
            }
            Some(f)
                if !self.hash.is_empty()
                    && f.len == self.len
                    && f.hash.as_deref() == Some(self.hash.as_str()) =>
            {
                Disk::Untouched
            }
            Some(_) if self.hash.is_empty() => Disk::Unsure,
            Some(_) => Disk::Edited,
        }
    }

    /// The copy at `path` compared with this entry now: the executor's re-check.
    pub fn now(&self, path: &Path) -> Disk {
        let now = stat(path).map(|(len, modified_ms)| LocalFile {
            len,
            modified_ms,
            hash: (len == self.len).then(|| hash_file(path)).flatten(),
        });
        self.compare(now.as_ref())
    }
}

/// A folder sync saved (or created) on your disk for an ILIAS course or folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderEntry {
    /// Its reference id on ILIAS; unknown for folders taken over from the old index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

/// The base of one download folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entries {
    /// The download folder these paths are below.
    pub root: PathBuf,
    #[serde(default)]
    pub files: BTreeMap<Rel, FileEntry>,
    #[serde(default)]
    pub folders: BTreeMap<Rel, FolderEntry>,
}

impl Entries {
    /// Records `rel` as ILIAS' file `entry.id`; any other path recorded for that file goes
    /// (it is here now).
    pub fn record_file(&mut self, rel: &str, entry: FileEntry) {
        self.files.retain(|path, e| e.id != entry.id || path == rel);
        self.files.insert(rel.to_string(), entry);
    }

    pub fn forget_file(&mut self, rel: &str) {
        self.files.remove(rel);
    }

    pub fn record_folder(&mut self, rel: &str, id: Option<u64>) {
        let entry = self.folders.entry(rel.to_string()).or_default();
        entry.id = id.or(entry.id);
    }

    /// Forgets the folder `rel` and everything recorded below it.
    pub fn forget_below(&mut self, rel: &str) {
        self.files.retain(|path, _| !within(path, rel));
        self.folders.retain(|path, _| !within(path, rel));
    }
}

/// The saved files: see the module docs. One per download folder in the file, so going back
/// to an earlier download folder finds what was saved there.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    file: Option<PathBuf>,
    entries: Entries,
}

impl Base {
    /// The base of this ILIAS for the download folder `root`. The first time it is taken
    /// over from the old saved-files list (`<host>.downloads.json`).
    pub fn load(url: &Url, root: &Path) -> Self {
        let old = crate::session::host_file(url, "downloads");
        Self::open(session_file(url), root, old.filter(|_| !cfg!(test)))
    }

    fn open(file: Option<PathBuf>, root: &Path, old: Option<PathBuf>) -> Self {
        let stored = read(file.as_deref()).into_iter().find(|e| e.root == root);
        let taken_over = stored.is_none();
        let mut base = Self {
            entries: stored.unwrap_or_else(|| migrate(old.as_deref(), root)),
            file,
        };
        if taken_over {
            // Once; an unwritable file only means it is taken over again next time.
            let _ = base.change(|_| {});
        }
        base
    }

    /// A base in `file` (tests), taken over from the old list `old` the first time.
    #[cfg(test)]
    pub fn in_file(file: PathBuf, root: &Path, old: &Path) -> Self {
        Self::open(Some(file), root, Some(old.to_path_buf()))
    }

    /// A base that is only in memory (tests).
    #[cfg(test)]
    pub fn new(root: &Path) -> Self {
        Self {
            file: None,
            entries: Entries {
                root: root.to_path_buf(),
                ..Entries::default()
            },
        }
    }

    pub fn root(&self) -> &Path {
        &self.entries.root
    }

    /// The full path of `rel`.
    pub fn abs(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|p| !p.is_empty())
            .fold(self.root().to_path_buf(), |path, part| path.join(part))
    }

    pub fn rel(&self, path: &Path) -> Option<Rel> {
        rel(self.root(), path)
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &FileEntry)> {
        self.entries.files.iter().map(|(p, e)| (p.as_str(), e))
    }

    pub fn file(&self, rel: &str) -> Option<&FileEntry> {
        self.entries.files.get(rel)
    }

    /// Where ILIAS' file `id` is recorded.
    pub fn find(&self, id: u64) -> Option<(&str, &FileEntry)> {
        self.files().find(|(_, e)| e.id == id)
    }

    /// The saved copy of ILIAS' file `id`, if it is still there.
    pub fn path(&self, id: u64) -> Option<PathBuf> {
        let (rel, _) = self.find(id)?;
        Some(self.abs(rel)).filter(|p| p.exists())
    }

    /// The ids of the files whose saved copy is still there.
    pub fn saved(&self) -> Vec<u64> {
        let here = |(rel, e): (&str, &FileEntry)| self.abs(rel).exists().then_some(e.id);
        self.files().filter_map(here).collect()
    }

    pub fn folders(&self) -> impl Iterator<Item = (&str, &FolderEntry)> {
        self.entries.folders.iter().map(|(p, e)| (p.as_str(), e))
    }

    pub fn has_folder(&self, rel: &str) -> bool {
        self.entries.folders.contains_key(rel)
    }

    /// Applies `change` and writes the base. The file is read again first and wins for
    /// everything else, so a sync in the background and the TUI don't undo each other.
    pub fn change(&mut self, change: impl FnOnce(&mut Entries)) -> Result<()> {
        let mut all = read(self.file.as_deref());
        if let Some(on_disk) = all.iter().find(|e| e.root == self.entries.root) {
            self.entries = on_disk.clone();
        }
        change(&mut self.entries);
        let Some(file) = &self.file else {
            return Ok(());
        };
        all.retain(|e| e.root != self.entries.root);
        all.push(self.entries.clone());
        crate::session::write_json(file, &all)
    }

    /// Records a file just saved (or sent) at `path`, if it is in the download folder.
    pub fn record(&mut self, path: &Path, id: u64, version: String) -> Result<()> {
        let Some(rel) = self.rel(path) else {
            return Ok(());
        };
        let entry = FileEntry::stamp(path, id, version);
        self.change(|e| e.record_file(&rel, entry))
    }
}

fn read(file: Option<&Path>) -> Vec<Entries> {
    crate::session::read_json(file).unwrap_or_default()
}

/// `<host>.base.json` next to the session; tests never touch it.
fn session_file(url: &Url) -> Option<PathBuf> {
    match cfg!(test) {
        true => None,
        false => crate::session::host_file(url, "base"),
    }
}

/// An entry of the old saved-files list: `{ "<ref id>": { "path", "version", "len",
/// "modified_ms", "hash" } }`, or only the path in its first versions.
#[derive(Deserialize)]
#[serde(untagged)]
enum Old {
    Saved {
        path: PathBuf,
        #[serde(default)]
        version: String,
        #[serde(default)]
        len: u64,
        #[serde(default)]
        modified_ms: u64,
        #[serde(default)]
        hash: String,
    },
    Path(PathBuf),
}

/// The base taken over from the old saved-files list (absolute paths, files only): the
/// entries in `root`, with their versions made absolute and the folders they are in.
/// Entries elsewhere (saved before the download folder changed) stay out.
pub fn migrate(old: Option<&Path>, root: &Path) -> Entries {
    let mut entries = Entries {
        root: root.to_path_buf(),
        ..Entries::default()
    };
    let old: std::collections::HashMap<String, Old> =
        crate::session::read_json(old).unwrap_or_default();
    let mut saved: Vec<(u64, FileEntry, PathBuf)> = old
        .into_iter()
        .filter_map(|(key, old)| {
            let (path, entry) = match old {
                Old::Saved {
                    path,
                    version,
                    len,
                    modified_ms,
                    hash,
                } => {
                    let version = saved_version(&version, modified_ms);
                    (
                        path,
                        FileEntry {
                            id: 0,
                            version,
                            len,
                            modified_ms,
                            hash,
                        },
                    )
                }
                Old::Path(path) => (path, FileEntry::default()),
            };
            let id = key.parse().ok()?;
            Some((id, FileEntry { id, ..entry }, path))
        })
        .collect();
    // Two old entries for one path (a stale one among them): the newer copy wins.
    saved.sort_by_key(|(id, e, _)| (e.modified_ms, *id));
    for (_, entry, path) in saved {
        let Some(path) = rel(root, &path) else {
            continue;
        };
        let mut dir = parent(&path);
        while !dir.is_empty() {
            entries.folders.entry(dir.to_string()).or_default();
            dir = parent(dir);
        }
        entries.files.insert(path, entry);
    }
    entries
}

/// A version recorded by an older ilias-tui, comparable with [`super::version`]: ILIAS'
/// relative dates ("Today, 12:41") are read on the day the copy was saved.
pub fn saved_version(version: &str, modified_ms: u64) -> String {
    let day = jiff::Timestamp::from_millisecond(modified_ms as i64)
        .ok()
        .filter(|_| modified_ms > 0)
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date())
        .unwrap_or_else(|| jiff::Zoned::now().date());
    super::absolute(version, day)
}

/// A course or folder the sync read, and its folder on your disk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub item: Item,
    pub dir: Rel,
    pub rights: Rights,
    /// What ILIAS lists in it, titles as listed.
    pub items: Vec<Item>,
}

impl Place {
    /// Whether it lists something saved under the name `name` here (a file by its title,
    /// with or without extension; anything else by its folder name).
    pub fn lists_file(&self, name: &str) -> bool {
        self.items
            .iter()
            .any(|i| i.kind == Kind::File && crate::ilias::lists_as(&i.title, name))
    }

    pub fn lists_folder(&self, name: &str) -> bool {
        self.items
            .iter()
            .any(|i| sanitize(&i.title).eq_ignore_ascii_case(name))
    }
}

/// A file on ILIAS now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteFile {
    /// As the crawl found it, the title being its path from the course (`DEMO › a`).
    pub item: Item,
    /// The place that lists it, if it was read.
    pub place: Option<usize>,
    /// Its version now ([`super::version`]).
    pub version: String,
    /// The local folder it is saved into.
    pub dir: Rel,
}

/// ILIAS now, as far as the sync read it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remote {
    pub places: Vec<Place>,
    pub files: Vec<RemoteFile>,
}

impl Remote {
    /// Notes a course or folder page the crawl read. Only pages below a course have a place
    /// (elsewhere it would be the download folder itself); each is noted once.
    pub fn add_page(&mut self, item: &Item, page: &Page) {
        if page.path.is_empty() || self.places.iter().any(|p| p.item.id == item.id) {
            return;
        }
        self.places.push(Place {
            item: item.clone(),
            dir: rel_dir(&page.path),
            rights: page.rights.clone(),
            items: page.items().cloned().collect(),
        });
    }

    /// Notes a file the crawl found (once, even if listed in several places).
    pub fn add_file(&mut self, item: Item) {
        if self.files.iter().any(|f| f.item.id == item.id) {
            return;
        }
        let place = self
            .places
            .iter()
            .position(|p| p.items.iter().any(|i| i.id == item.id));
        self.files.push(RemoteFile {
            version: super::version(&item),
            dir: rel_dir(&item.path),
            place,
            item,
        });
    }

    pub fn place_at(&self, dir: &str) -> impl Iterator<Item = &Place> {
        self.places.iter().filter(move |p| p.dir == dir)
    }

    pub fn file(&self, id: u64) -> Option<&RemoteFile> {
        self.files.iter().find(|f| f.item.key() == id)
    }
}

/// A file on your disk now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalFile {
    pub len: u64,
    pub modified_ms: u64,
    /// Its content hash, where the diff needs it: for new files, and for recorded ones whose
    /// size is the same but whose time isn't (or that have no hash yet).
    pub hash: Option<String>,
}

/// Your disk now, below the places' folders (without the ignore list and links).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Local {
    pub files: BTreeMap<Rel, LocalFile>,
    pub folders: BTreeSet<Rel>,
}

impl Local {
    /// Scans the folder of every place in `remote` (one level: [`local::scan`]), every
    /// other folder in them all the way down ([`local::walk`]), and every file in `base`.
    /// `hash_new`: also hash new files directly in a place's folder (two-way needs it to
    /// find renames and copies).
    pub fn read(base: &Base, remote: &Remote, hash_new: bool) -> Self {
        let mut local = Self::default();
        let places: HashSet<&str> = remote.places.iter().map(|p| p.dir.as_str()).collect();
        for dir in &places {
            let path = base.abs(dir);
            if !path.is_dir() {
                continue;
            }
            local.folders.insert(dir.to_string());
            let found = local::scan(&path);
            for (file, _) in found.files {
                local.add(base, &file, hash_new);
            }
            for folder in found.folders {
                let Some(rel) = base.rel(&folder) else {
                    continue;
                };
                local.folders.insert(rel.clone());
                if places.contains(rel.as_str()) {
                    continue;
                }
                for inside in local::walk(&folder, u64::MAX).0 {
                    match inside.is_dir() {
                        true => {
                            local.folders.extend(base.rel(&inside));
                        }
                        false => local.add(base, &inside, false),
                    }
                }
            }
        }
        let recorded: Vec<&str> = base.files().map(|(rel, _)| rel).collect();
        for rel in recorded {
            if !local.files.contains_key(rel) {
                local.add(base, &base.abs(rel), false);
            }
        }
        local
    }

    fn add(&mut self, base: &Base, path: &Path, hash_new: bool) {
        let (Some(rel), Some((len, modified_ms))) = (base.rel(path), stat(path)) else {
            return;
        };
        let needed = match base.file(&rel) {
            Some(e) if e.hash.is_empty() => {
                e.len > 0 && (len, modified_ms) == (e.len, e.modified_ms)
            }
            Some(e) => len == e.len && modified_ms != e.modified_ms,
            None => hash_new,
        };
        let hash = needed.then(|| hash_file(path)).flatten();
        self.files.insert(
            rel,
            LocalFile {
                len,
                modified_ms,
                hash,
            },
        );
    }
}

#[cfg(test)]
mod tests;
