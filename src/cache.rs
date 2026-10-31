//! On-disk cache of parsed pages, for instant display while the fresh page loads.
//! One JSON file per page in the system cache folder (`~/Library/Caches/ch.ilias-tui.ilias-tui/`
//! on macOS, `~/.cache/ilias-tui/` on Linux), which backup tools usually skip.
//! `ILIAS_CACHE=0` turns it off; entries older than [`MAX_AGE`] are removed on startup.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use url::Url;

use crate::ilias::Page;
use crate::session;

/// Cached pages older than this are deleted; course pages rarely matter after a month.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone)]
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Cache {
    pub fn new(base: &Url) -> Self {
        let Ok(dirs) = session::dirs() else {
            return Self { dir: None };
        };
        // Earlier versions kept the cache next to the session; it now lives in the cache folder.
        let _ = fs::remove_dir_all(dirs.data_dir().join("cache"));
        let disabled = matches!(
            std::env::var("ILIAS_CACHE").as_deref(),
            Ok("0" | "false" | "no" | "off")
        );
        let dir = base
            .host_str()
            .filter(|_| !disabled)
            .map(|host| dirs.cache_dir().join(host))
            .filter(|dir| {
                session::private_dir(dirs.cache_dir()).is_ok() && fs::create_dir_all(dir).is_ok()
            });
        let cache = Self { dir };
        cache.remove_older_than(MAX_AGE);
        cache
    }

    #[cfg(test)]
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    /// Cache key for a page: what was requested (`url`, a deep search or "My courses").
    pub fn key(url: Option<&str>, deep: bool) -> String {
        format!("{deep}|{}", url.unwrap_or("courses"))
    }

    fn file(&self, key: &str) -> Option<PathBuf> {
        // FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`.
        let hash = key.bytes().fold(0xcbf29ce484222325_u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        Some(self.dir.as_ref()?.join(format!("{hash:016x}.json")))
    }

    pub fn get(&self, key: &str) -> Option<Page> {
        let json = fs::read_to_string(self.file(key)?).ok()?;
        serde_json::from_str(&json).ok()
    }

    /// Best effort: a failed write only means a slower next visit.
    pub fn put(&self, key: &str, page: &Page) {
        let Some(file) = self.file(key) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(page) {
            let _ = fs::write(file, json);
        }
    }

    fn remove_older_than(&self, max_age: Duration) {
        let Some(entries) = self.dir.as_ref().and_then(|d| fs::read_dir(d).ok()) else {
            return;
        };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > max_age);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    pub fn clear(&self) {
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_old_entries_only() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-cache-{}", std::process::id()));
        let cache = Cache::in_dir(dir.clone());
        cache.put("old", &Page::default());
        cache.put("new", &Page::default());
        let old = fs::File::options()
            .write(true)
            .open(cache.file("old").unwrap())
            .unwrap();
        old.set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(60))
            .unwrap();

        cache.remove_older_than(MAX_AGE);
        assert!(cache.get("old").is_none());
        assert!(cache.get("new").is_some());
        cache.clear();
    }
}
