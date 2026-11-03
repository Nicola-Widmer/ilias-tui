//! Settings changed in the app (the settings page, `c`), saved as `config.json` in the app's
//! config folder. Environment variables still win over them at startup.

use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::Result;
use serde::{Deserialize, Serialize};

use crate::session;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Where files are saved; `None` means [`default_download_dir`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
}

impl Config {
    fn path() -> Result<PathBuf> {
        Ok(session::dirs()?.config_dir().join("config.json"))
    }

    /// The saved settings, or the defaults if there are none (or they are unreadable).
    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(dir) = path.parent() {
            session::private_dir(dir)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// `ILIAS_DOWNLOAD_DIR`, else the saved folder, else `~/Downloads/ILIAS`.
    pub fn download_dir(&self) -> PathBuf {
        std::env::var_os("ILIAS_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .or_else(|| self.download_dir.clone())
            .unwrap_or_else(default_download_dir)
    }
}

pub fn default_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ILIAS")
}

fn home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// A folder typed by the user: `~` stands for the home folder, and it must be absolute.
pub fn parse_dir(input: &str) -> Result<PathBuf, String> {
    let input = input.trim();
    let path = match input.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home()
            .ok_or("no home folder")?
            .join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    };
    if input.is_empty() || !path.is_absolute() {
        return Err("Use a full path, e.g. ~/Documents/ILIAS".into());
    }
    Ok(path)
}

/// `path` with the home folder shown as `~`.
pub fn display(path: &Path) -> String {
    match home().and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_home_relative_and_absolute_folders() {
        let home = home().unwrap();
        assert_eq!(parse_dir("~/Uni/ILIAS"), Ok(home.join("Uni/ILIAS")));
        assert_eq!(parse_dir(" /tmp/x "), Ok(PathBuf::from("/tmp/x")));
        assert!(parse_dir("Uni").is_err());
        assert!(parse_dir("").is_err());
        assert_eq!(display(&home.join("Uni/ILIAS")), "~/Uni/ILIAS");
        assert_eq!(display(Path::new("/tmp/x")), "/tmp/x");
    }

    #[test]
    fn unset_folder_is_left_out_of_the_file() {
        assert_eq!(serde_json::to_string(&Config::default()).unwrap(), "{}");
    }
}
