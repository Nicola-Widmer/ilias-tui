//! Your side of two-way sync: the files and folders in the local folder of a synced course
//! or folder, without what sync never sends (the ignore list).

use std::fs;
use std::path::{Path, PathBuf};

/// Files and folders two-way sync never sends: system and editor leftovers (`.DS_Store`,
/// `Thumbs.db`, Office's `~$…` lock files, `*.tmp`), hidden files and folders (`.git/`
/// among them), and the copies sync itself saves beside your edited files.
pub fn ignored(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("Thumbs.db")
        || name.starts_with("~$")
        || name.to_ascii_lowercase().ends_with(".tmp")
        || is_beside_copy(name)
}

/// `Blatt 3 (ILIAS 2026-11-02).pdf`, also `… (ILIAS 2026-11-02) (1).pdf`: a new ILIAS
/// version sync saved beside your edited copy (see `beside_name`).
fn is_beside_copy(name: &str) -> bool {
    name.match_indices(" (ILIAS ").any(|(at, marker)| {
        let rest = &name[at + marker.len()..];
        rest.get(10..11) == Some(")")
            && rest
                .get(..10)
                .is_some_and(|date| date.parse::<jiff::civil::Date>().is_ok())
    })
}

/// What a local folder holds, without the ignored names and without links (which could
/// lead anywhere).
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// Files and their size in bytes.
    pub files: Vec<(PathBuf, u64)>,
    pub folders: Vec<PathBuf>,
}

/// The files and folders directly in `dir`, sorted by name; nothing if it doesn't exist.
pub fn scan(dir: &Path) -> Found {
    let mut found = Found::default();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // `file_type` doesn't follow links, so a link is neither a file nor a folder here.
        let (Ok(kind), false) = (entry.file_type(), ignored(name)) else {
            continue;
        };
        if kind.is_file() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            found.files.push((entry.path(), len));
        } else if kind.is_dir() {
            found.folders.push(entry.path());
        }
    }
    found.files.sort();
    found.folders.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_system_files_hidden_files_and_copies_saved_beside() {
        for name in [
            ".DS_Store",
            "Thumbs.db",
            "~$Bericht.docx",
            "upload.TMP",
            ".git",
            ".notes.md",
            "Blatt 3 (ILIAS 2026-11-02).pdf",
            "Blatt 3 (ILIAS 2026-11-02) (1).pdf",
        ] {
            assert!(ignored(name), "{name}");
        }
        for name in [
            "Blatt 3.pdf",
            "Zusammenfassung (ILIAS).md",
            "Bericht (ILIAS 2026).pdf",
            "notes",
        ] {
            assert!(!ignored(name), "{name}");
        }
    }

    #[test]
    fn scans_one_folder_without_what_is_ignored() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Woche 1")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("notes.txt"), "12345").unwrap();
        fs::write(dir.join(".DS_Store"), "x").unwrap();
        fs::write(dir.join("Woche 1/deeper.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("notes.txt"), dir.join("link.txt")).unwrap();
        assert_eq!(
            scan(&dir),
            Found {
                files: vec![(dir.join("notes.txt"), 5)],
                folders: vec![dir.join("Woche 1")],
            }
        );
        assert_eq!(scan(&dir.join("missing")), Found::default());
        let _ = fs::remove_dir_all(dir);
    }
}
