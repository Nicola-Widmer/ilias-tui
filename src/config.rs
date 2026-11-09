//! Settings changed in the app (the settings page, `c`), saved as `config.json` in the app's
//! config folder. Environment variables (`ILIAS_*`) still win over them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::Result;
use serde::{Deserialize, Serialize};

use crate::seen::Group;
use crate::session;
use crate::theme::{self, Palette};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Where files are saved; `None` means [`default_download_dir`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
    /// Sync skips files bigger than this many MB; `None` means [`DEFAULT_SYNC_MAX_MB`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_max_mb: Option<f64>,
    /// Sync downloads only files that are missing on disk, including ones you deleted, and
    /// leaves every file that is there alone (no updates, no new versions beside).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing_only: bool,
    /// Sync also sends your new and edited files to ILIAS (after showing what and asking).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub two_way: bool,
    /// `None` means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nerd_font: Option<bool>,
    /// One of [`theme::THEMES`]; `None` means the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Colours on top of the theme, by name: `"accent": "#ff8800"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
    /// Always start offline (cached pages and saved files only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offline: bool,
    /// Never send favourites changes; only say what would have happened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// The ILIAS root URL; `None` means [`DEFAULT_URL`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Kinds of news not to show (marks and counts); none by default.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hide_new: BTreeSet<Group>,
}

pub const DEFAULT_URL: &str = "https://elearning.hslu.ch/ilias/";
pub const DEFAULT_SYNC_MAX_MB: f64 = 50.0;

/// An on/off environment variable: `Some` only if it is set to something recognisable.
pub fn env_flag(name: &str) -> Option<bool> {
    match std::env::var(name).ok()?.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// A sync size limit typed by the user (or set in `ILIAS_SYNC_MAX_MB`): a positive number.
pub fn parse_mb(input: &str) -> Result<f64, String> {
    input
        .trim()
        .replace(',', ".")
        .parse()
        .ok()
        .filter(|mb: &f64| mb.is_finite() && *mb > 0.0)
        .ok_or_else(|| "Use a number of MB above 0, e.g. 50".into())
}

/// An ILIAS root URL typed by the user (or set in `ILIAS_URL`), ending in `/`.
pub fn parse_url(input: &str) -> Result<String, String> {
    let mut url = input.trim().to_string();
    if !url.ends_with('/') {
        url.push('/');
    }
    match url::Url::parse(&url) {
        Ok(u) if u.scheme() == "https" || u.scheme() == "http" => Ok(url),
        _ => Err("Use a full link, e.g. https://elearning.hslu.ch/ilias/".into()),
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
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
    /// Whether this kind of news is shown.
    pub fn shows_new(&self, group: Group) -> bool {
        !self.hide_new.contains(&group)
    }

    pub fn download_dir(&self) -> PathBuf {
        std::env::var_os("ILIAS_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .or_else(|| self.download_dir.clone())
            .unwrap_or_else(default_download_dir)
    }

    /// `ILIAS_SYNC_MAX_MB`, else the saved limit, else 50.
    pub fn sync_max_mb(&self) -> f64 {
        std::env::var("ILIAS_SYNC_MAX_MB")
            .ok()
            .and_then(|v| parse_mb(&v).ok())
            .or(self.sync_max_mb)
            .unwrap_or(DEFAULT_SYNC_MAX_MB)
    }

    /// The chosen theme with the custom colours, and the entries that can't be used.
    pub fn palette(&self) -> (Palette, Vec<String>) {
        Palette::build(self.theme.as_deref(), &self.colors)
    }

    pub fn theme(&self) -> &str {
        self.theme.as_deref().unwrap_or(theme::THEMES[0])
    }

    pub fn nerd_font(&self) -> bool {
        env_flag("ILIAS_NERD_FONT")
            .or(self.nerd_font)
            .unwrap_or(true)
    }

    pub fn mouse(&self) -> bool {
        env_flag("ILIAS_MOUSE").or(self.mouse).unwrap_or(true)
    }

    pub fn cache(&self) -> bool {
        env_flag("ILIAS_CACHE").or(self.cache).unwrap_or(true)
    }

    pub fn offline(&self) -> bool {
        env_flag("ILIAS_OFFLINE").unwrap_or(self.offline)
    }

    /// `ILIAS_TWO_WAY`, else the saved setting; off by default.
    pub fn two_way(&self) -> bool {
        env_flag("ILIAS_TWO_WAY").unwrap_or(self.two_way)
    }

    pub fn dry_run(&self) -> bool {
        env_flag("ILIAS_DRY_RUN").unwrap_or(self.dry_run)
    }

    /// `ILIAS_URL`, else the saved one, else the HSLU ILIAS; always ending in `/`.
    pub fn url(&self) -> String {
        std::env::var("ILIAS_URL")
            .ok()
            .or_else(|| self.url.clone())
            .map(|u| if u.ends_with('/') { u } else { format!("{u}/") })
            .unwrap_or_else(|| DEFAULT_URL.into())
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
        // `~/…`, and `~\…` on Windows.
        Some(rest) if rest.is_empty() || rest.starts_with(['/', std::path::MAIN_SEPARATOR]) => {
            home()
                .ok_or("no home folder")?
                .join(rest.trim_start_matches(['/', std::path::MAIN_SEPARATOR]))
        }
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
        // An absolute path on this system (`/tmp/x`, `C:\…\x`).
        let elsewhere = std::env::temp_dir().join("x");
        let typed = format!(" {} ", elsewhere.display());
        assert_eq!(parse_dir(&typed), Ok(elsewhere.clone()));
        assert!(parse_dir("Uni").is_err());
        assert!(parse_dir("").is_err());
        assert_eq!(display(&home.join("Uni/ILIAS")), "~/Uni/ILIAS");
        if !elsewhere.starts_with(&home) {
            assert_eq!(display(&elsewhere), elsewhere.display().to_string());
        }
    }

    #[test]
    fn unset_settings_are_left_out_of_the_file() {
        assert_eq!(serde_json::to_string(&Config::default()).unwrap(), "{}");
        let config = Config {
            missing_only: true,
            mouse: Some(false),
            ..Config::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, r#"{"missing_only":true,"mouse":false}"#);
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }

    #[test]
    fn reads_on_off_variables() {
        // A name no other test uses: the environment is shared by all tests.
        let name = "ILIAS_TEST_ENV_FLAG";
        for (value, expected) in [("1", Some(true)), ("off", Some(false)), ("maybe", None)] {
            // SAFETY: no other test reads or writes this variable.
            unsafe { std::env::set_var(name, value) };
            assert_eq!(env_flag(name), expected, "{value}");
        }
        unsafe { std::env::remove_var(name) };
        assert_eq!(env_flag(name), None);
    }

    #[test]
    fn unset_settings_fall_back_to_the_defaults() {
        let config = Config::default();
        if std::env::var_os("ILIAS_SYNC_MAX_MB").is_none() {
            assert_eq!(config.sync_max_mb(), DEFAULT_SYNC_MAX_MB);
        }
        if std::env::var_os("ILIAS_URL").is_none() {
            assert_eq!(config.url(), DEFAULT_URL);
            let custom = Config {
                url: Some("https://ilias.example/ilias".into()),
                ..Config::default()
            };
            assert_eq!(custom.url(), "https://ilias.example/ilias/");
        }
        if std::env::var_os("ILIAS_MOUSE").is_none() {
            assert!(config.mouse());
            let off = Config {
                mouse: Some(false),
                ..Config::default()
            };
            assert!(!off.mouse());
        }
    }

    #[test]
    fn parses_limits_and_links() {
        assert_eq!(parse_mb(" 120 "), Ok(120.0));
        assert_eq!(parse_mb("2,5"), Ok(2.5));
        assert!(parse_mb("0").is_err() && parse_mb("lots").is_err());
        assert_eq!(
            parse_url("https://ilias.example/ilias"),
            Ok("https://ilias.example/ilias/".into())
        );
        assert!(parse_url("ilias.example").is_err());
    }
}
