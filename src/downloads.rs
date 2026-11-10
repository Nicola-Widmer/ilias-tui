//! Remembers which ILIAS files were saved where, and what they looked like then, so opening
//! a file again uses the local copy and sync can tell whether you edited it.
//! Stored next to the session as `<ilias host>.downloads.json`:
//! `{ "<item key>": { "path": …, "version": …, "len": …, "modified_ms": …, "hash": … } }`
//! (older versions stored only the path; such entries count as possibly edited). Each entry
//! is the **base** of two-way sync: what ILIAS and your disk had when the two were last in
//! sync.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use color_eyre::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::session;

/// A saved file and how it looked right after the download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub path: PathBuf,
    /// The file's version on ILIAS when it was saved (size and date from the listing).
    #[serde(default)]
    pub version: String,
    /// Size and modification time right after saving; any difference means you edited it.
    #[serde(default)]
    pub len: u64,
    #[serde(default)]
    pub modified_ms: u64,
    /// SHA-256 of the content right after saving (hex), so a file whose modification time
    /// changed but whose content didn't is still untouched. Empty for entries saved before
    /// hashes were recorded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
}

/// What became of a saved file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Local {
    /// Gone (you deleted or moved it).
    Missing,
    /// Exactly as saved.
    Untouched,
    /// Changed since it was saved (or unknown, for old entries): never overwrite it.
    Edited,
}

impl Saved {
    fn stamp(path: PathBuf, version: String) -> Self {
        let (len, modified_ms) = stat(&path).unwrap_or((0, 0));
        let hash = hash_file(&path).unwrap_or_default();
        Self {
            path,
            version,
            len,
            modified_ms,
            hash,
        }
    }

    pub fn local(&self) -> Local {
        match stat(&self.path) {
            None => Local::Missing,
            Some(now) if self.len > 0 && now == (self.len, self.modified_ms) => Local::Untouched,
            // Touched (copied, synced by another tool, …) but the same content.
            Some((len, _))
                if len == self.len
                    && !self.hash.is_empty()
                    && hash_file(&self.path).is_some_and(|h| h == self.hash) =>
            {
                Local::Untouched
            }
            Some(_) => Local::Edited,
        }
    }

    /// Whether an edit can be told for sure: the content hash was recorded.
    pub fn has_hash(&self) -> bool {
        !self.hash.is_empty()
    }
}

/// SHA-256 of a file's content, in hex; `None` if it can't be read.
pub fn hash_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        match file.read(&mut buffer).ok()? {
            0 => break,
            n => hasher.update(&buffer[..n]),
        }
    }
    Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_millis() as u64))
}

/// Old index files map ref ids to bare paths.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Saved(Saved),
    Path(PathBuf),
}

pub struct Downloads {
    file: Option<PathBuf>,
    entries: HashMap<u64, Saved>,
}

impl Downloads {
    /// Loads the index; a missing or unreadable file means "nothing saved yet".
    pub fn load(base: &Url) -> Self {
        Self::from_file(session::host_file(base, "downloads"))
    }

    fn from_file(file: Option<PathBuf>) -> Self {
        let entries = session::read_json::<HashMap<String, Stored>>(file.as_deref())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, stored)| {
                let saved = match stored {
                    Stored::Saved(saved) => saved,
                    Stored::Path(path) => Saved {
                        path,
                        version: String::new(),
                        len: 0,
                        modified_ms: 0,
                        hash: String::new(),
                    },
                };
                Some((id.parse().ok()?, saved))
            })
            .collect();
        Self { file, entries }
    }

    #[cfg(test)]
    pub fn in_file(file: PathBuf) -> Self {
        Self::from_file(Some(file))
    }

    /// The saved copy of `key`, if it still exists on disk.
    pub fn get(&self, key: u64) -> Option<&PathBuf> {
        self.entries
            .get(&key)
            .map(|s| &s.path)
            .filter(|p| p.exists())
    }

    pub fn entry(&self, key: u64) -> Option<&Saved> {
        self.entries.get(&key)
    }

    /// Every entry, saved copy on disk or not.
    pub fn entries(&self) -> impl Iterator<Item = (u64, &Saved)> + '_ {
        self.entries.iter().map(|(key, saved)| (*key, saved))
    }

    /// Ref ids of all files that are still on disk.
    pub fn saved(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries
            .iter()
            .filter(|(_, s)| s.path.exists())
            .map(|(id, _)| *id)
    }

    /// Records a file that was just written to `path` (its current state becomes "untouched").
    pub fn record(&mut self, key: u64, path: PathBuf, version: String) -> Result<()> {
        let saved = Saved::stamp(path, version);
        self.update(key, |entry| *entry = Some(saved))
    }

    /// Notes that ILIAS's `version` is known, without touching the local file's state (it
    /// stays "edited" if it was).
    pub fn set_version(&mut self, key: u64, version: String) -> Result<()> {
        self.update(key, |entry| {
            if let Some(saved) = entry {
                saved.version = version;
            }
        })
    }

    /// Records the content hash of an entry saved before hashes were, if its file is still
    /// exactly as saved (size and modification time); otherwise nothing changes.
    pub fn add_hash(&mut self, key: u64) -> Result<()> {
        let untouched = self.entries.get(&key).is_some_and(|s| {
            !s.has_hash() && s.len > 0 && stat(&s.path) == Some((s.len, s.modified_ms))
        });
        if !untouched {
            return Ok(());
        }
        self.update(key, |entry| {
            if let Some(saved) = entry {
                saved.hash = hash_file(&saved.path).unwrap_or_default();
            }
        })
    }

    /// Applies a change to one entry and writes the index. The file is read again first
    /// and wins for every other entry, so a sync running in the background and the TUI
    /// neither drop nor undo each other's entries.
    fn update(&mut self, key: u64, change: impl FnOnce(&mut Option<Saved>)) -> Result<()> {
        if let Some(file) = &self.file {
            // Overlaid, not replaced: an unreadable file must not wipe what is known here.
            let on_disk = Self::from_file(Some(file.clone())).entries;
            self.entries.extend(on_disk);
        }
        let mut entry = self.entries.remove(&key);
        change(&mut entry);
        if let Some(saved) = entry {
            self.entries.insert(key, saved);
        }
        let Some(file) = &self.file else {
            return Ok(());
        };
        let json: HashMap<String, &Saved> = self
            .entries
            .iter()
            .map(|(id, s)| (id.to_string(), s))
            .collect();
        session::write_json(file, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_copy_of_the_index_does_not_undo_newer_entries() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-index-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let index = dir.join("index.json");
        let mut tui = Downloads::in_file(index.clone());
        tui.record(1, dir.join("Blatt 1.pdf"), "v1".into()).unwrap();

        // A sync, with its own copy of the index, saves a newer version of file 1.
        let mut sync = Downloads::in_file(index.clone());
        sync.record(1, dir.join("Blatt 1 (1).pdf"), "v2".into())
            .unwrap();

        // The TUI, still knowing the old entry, saves another file.
        tui.record(2, dir.join("Blatt 2.pdf"), "v1".into()).unwrap();
        let on_disk = Downloads::in_file(index);
        assert_eq!(on_disk.entry(1).unwrap().version, "v2");
        assert_eq!(on_disk.entry(2).unwrap().version, "v1");
        let _ = fs::remove_dir_all(dir);
    }
}
