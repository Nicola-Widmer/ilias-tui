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

/// Which of `paths` (files gone on ILIAS) may go to your computer's trash now: those still
/// exactly as sync saved them (by content). The others are returned with why not.
pub fn trashable(
    paths: &[PathBuf],
    downloads: &crate::downloads::Downloads,
) -> (Vec<PathBuf>, Vec<String>) {
    let (mut ok, mut skipped) = (Vec::new(), Vec::new());
    for path in paths {
        let saved = downloads.entries().find(|(_, s)| &s.path == path);
        match saved.map(|(_, s)| s.local()) {
            Some(crate::downloads::Local::Untouched) => ok.push(path.clone()),
            Some(crate::downloads::Local::Missing) => {}
            Some(crate::downloads::Local::Edited) => {
                skipped.push(format!("{}: changed since the sync; kept", path.display()))
            }
            None => skipped.push(format!("{}: not saved by sync; kept", path.display())),
        }
    }
    (ok, skipped)
}

/// Moves `path` to your computer's trash, from where it can be put back. On macOS through
/// the file manager API, which needs no permission (asking Finder would need Automation
/// rights and plays its sound).
pub fn to_trash(path: &Path) -> Result<(), trash::Error> {
    #[allow(unused_mut)]
    let mut trash = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(path)
}

/// What a new local folder holds, all the way down (at most [`MAX_DEPTH`] folders deep and
/// [`MAX_ENTRIES`] entries): its folders and files, each folder before what is in it, and
/// the files over `max_bytes`, which are left out.
pub fn walk(dir: &Path, max_bytes: u64) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut contents, mut too_big) = (Vec::new(), Vec::new());
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((at, depth)) = stack.pop() {
        let found = scan(&at);
        for (file, len) in found.files {
            match len > max_bytes {
                true => too_big.push(file),
                false => contents.push(file),
            }
        }
        for folder in found.folders.into_iter().rev() {
            if depth < MAX_DEPTH {
                contents.push(folder.clone());
                stack.push((folder, depth + 1));
            }
        }
        if contents.len() > MAX_ENTRIES {
            contents.truncate(MAX_ENTRIES);
            break;
        }
    }
    // Each folder before what is in it.
    contents.sort();
    (contents, too_big)
}

/// How deep [`walk`] goes, and how many entries it takes at most.
pub const MAX_DEPTH: usize = 10;
pub const MAX_ENTRIES: usize = 1000;

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
    fn only_files_still_as_saved_may_go_to_the_trash() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-trashable-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut downloads = crate::downloads::Downloads::in_file(dir.join("index.json"));
        let (same, edited, other) = (dir.join("a.pdf"), dir.join("b.pdf"), dir.join("c.pdf"));
        for (key, path) in [(1, &same), (2, &edited)] {
            fs::write(path, "as saved").unwrap();
            downloads.record(key, path.clone(), "v1".into()).unwrap();
        }
        fs::write(&edited, "with my notes").unwrap();
        fs::write(&other, "never synced").unwrap();
        let (ok, kept) = trashable(&[same.clone(), edited, other], &downloads);
        assert_eq!(ok, [same]);
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(
            kept[0].contains("changed since the sync") && kept[1].contains("not saved by sync")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn walks_a_new_folder_all_the_way_down() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Eigenes/Woche 1")).unwrap();
        fs::write(dir.join("Eigenes/a.md"), "a").unwrap();
        fs::write(dir.join("Eigenes/Woche 1/b.md"), "b").unwrap();
        fs::write(dir.join("Eigenes/Woche 1/big.mp4"), "0123456789").unwrap();
        fs::write(dir.join("Eigenes/.DS_Store"), "x").unwrap();
        let (contents, too_big) = walk(&dir.join("Eigenes"), 5);
        assert_eq!(
            contents,
            [
                dir.join("Eigenes/Woche 1"),
                dir.join("Eigenes/Woche 1/b.md"),
                dir.join("Eigenes/a.md"),
            ]
        );
        assert_eq!(too_big, [dir.join("Eigenes/Woche 1/big.mp4")]);
        let _ = fs::remove_dir_all(dir);
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
