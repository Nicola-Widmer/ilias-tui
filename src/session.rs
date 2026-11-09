//! Persists the cookie jar (ILIAS + Keycloak session cookies) in a file only the current
//! user can read, e.g. `~/Library/Application Support/ch.ilias-tui.ilias-tui/elearning.hslu.ch.json`.
//!
//! (The OS keychain was tried first, but macOS ties keychain items to the exact binary, so
//! every rebuild lost the session.)

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::PathBuf;

use color_eyre::{Result, eyre::eyre};
use cookie_store::CookieStore;
use directories::ProjectDirs;
use url::Url;

/// The app's standard folders (data, cache) for this OS.
pub fn dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("ch", "ilias-tui", "ilias-tui").ok_or_else(|| eyre!("no home directory"))
}

/// Creates `dir` (and parents) and makes it accessible to the current user only.
pub fn private_dir(dir: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

pub fn path(base: &Url) -> Result<PathBuf> {
    let dirs = dirs()?;
    let host = base.host_str().unwrap_or("ilias");
    Ok(dirs.data_dir().join(format!("{host}.json")))
}

/// Loads the saved cookies, or an empty jar if there are none (or they are unreadable).
/// Another file of this ILIAS next to the session, e.g. `elearning.hslu.ch.seen.json` for
/// `name` = `seen`.
pub fn host_file(base: &Url, name: &str) -> Option<PathBuf> {
    path(base)
        .ok()
        .map(|p| p.with_extension(format!("{name}.json")))
}

/// Reads such a file; `None` if it is missing or unreadable.
pub fn read_json<T: serde::de::DeserializeOwned>(file: Option<&std::path::Path>) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(file?).ok()?).ok()
}

/// Writes such a file, in the private folder.
pub fn write_json<T: serde::Serialize>(file: &std::path::Path, value: &T) -> Result<()> {
    if let Some(dir) = file.parent() {
        private_dir(dir)?;
    }
    fs::write(file, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

pub fn load(base: &Url) -> CookieStore {
    path(base)
        .and_then(|p| Ok(File::open(p)?))
        .ok()
        .and_then(|f| cookie_store::serde::json::load(BufReader::new(f)).ok())
        .unwrap_or_default()
}

pub fn save(base: &Url, store: &CookieStore) -> Result<()> {
    let path = path(base)?;
    if let Some(dir) = path.parent() {
        // Owner-only folder: also covers the saved-files list inside it.
        private_dir(dir)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path)?;
    // Session cookies (PHPSESSID, KEYCLOAK_IDENTITY, ...) are exactly the ones we need to keep.
    cookie_store::serde::json::save_incl_expired_and_nonpersistent(store, &mut file)
        .map_err(|e| eyre!(e))?;
    file.flush()?;
    Ok(())
}

pub fn clear(base: &Url) -> Result<()> {
    match fs::remove_file(path(base)?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}
