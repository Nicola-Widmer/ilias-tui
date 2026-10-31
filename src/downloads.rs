//! Remembers where each ILIAS file was saved, so opening it again uses the local copy.
//! Stored next to the session as `<ilias host>.downloads.json`: `{ "<ref_id>": "<path>" }`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use color_eyre::Result;
use url::Url;

use crate::session;

pub struct Downloads {
    file: Option<PathBuf>,
    paths: HashMap<u64, PathBuf>,
}

impl Downloads {
    /// Loads the index; a missing or unreadable file means "nothing saved yet".
    pub fn load(base: &Url) -> Self {
        let file = session::path(base)
            .ok()
            .map(|p| p.with_extension("downloads.json"));
        let paths = file
            .as_ref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|json| serde_json::from_str::<HashMap<String, PathBuf>>(&json).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, path)| Some((id.parse().ok()?, path)))
            .collect();
        Self { file, paths }
    }

    /// The saved copy of `ref_id`, if it still exists on disk.
    pub fn get(&self, ref_id: u64) -> Option<&PathBuf> {
        self.paths.get(&ref_id).filter(|p| p.exists())
    }

    /// Ref ids of all files that are still on disk.
    pub fn saved(&self) -> impl Iterator<Item = u64> + '_ {
        self.paths
            .iter()
            .filter(|(_, p)| p.exists())
            .map(|(id, _)| *id)
    }

    pub fn insert(&mut self, ref_id: u64, path: PathBuf) -> Result<()> {
        self.paths.insert(ref_id, path);
        let Some(file) = &self.file else {
            return Ok(());
        };
        let json: HashMap<String, &PathBuf> = self
            .paths
            .iter()
            .map(|(id, p)| (id.to_string(), p))
            .collect();
        fs::write(file, serde_json::to_string_pretty(&json)?)?;
        Ok(())
    }
}
