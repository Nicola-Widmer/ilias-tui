//! Two-way sync's questions: what the review would send, named file by file, and the
//! mass-change brake, which asks once more when that is more than usual.

use std::collections::HashMap;
use std::path::Path;

use super::Change;
use crate::ilias::Item;
use crate::ilias::write::megabytes;

/// More files than this in one run, and sync asks once more before sending.
pub const MAX_FILES: usize = 20;
/// More than this share of a folder's files changed (replaced, to the trash), and sync
/// asks once more.
pub const MAX_SHARE: f64 = 0.3;

/// The question before sending `changes`: what goes where, with sizes.
pub fn question(changes: &[Change], dir: &Path) -> String {
    let short = |path: &Path| path.strip_prefix(dir).unwrap_or(path).display().to_string();
    let sized = |path: &Path| {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        format!("{} ({})", short(path), megabytes(size))
    };
    let list = |files: Vec<String>| {
        let mut shown = files.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
        if files.len() > 8 {
            shown.push_str(&format!(" and {} more (see the report)", files.len() - 8));
        }
        shown
    };
    let plural = |n: usize, one: &str, many: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    let new: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c, Change::Upload { .. }))
        .map(|c| sized(c.path()))
        .collect();
    let trash: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c, Change::Trash { .. }))
        .map(|c| short(c.path()))
        .collect();
    let edited: Vec<String> = changes
        .iter()
        .filter(|c| matches!(c, Change::Replace { .. }))
        .map(|c| sized(c.path()))
        .collect();
    let folders: Vec<String> = changes
        .iter()
        .filter_map(|c| match c {
            Change::NewFolder { dir, contents, .. } => {
                let files: Vec<String> = contents
                    .iter()
                    .filter(|p| !p.is_dir())
                    .map(|p| sized(p))
                    .collect();
                Some(match files.is_empty() {
                    true => format!("{}/ (no files)", short(dir)),
                    false => format!(
                        "{}/ with {} ({})",
                        short(dir),
                        plural(files.len(), "file", "files"),
                        list(files)
                    ),
                })
            }
            _ => None,
        })
        .collect();
    let mut parts = Vec::new();
    if !folders.is_empty() {
        parts.push(format!(
            "create {} on ILIAS and upload what is in them: {}",
            plural(folders.len(), "new folder", "new folders"),
            folders.join("; ")
        ));
    }
    if !new.is_empty() {
        parts.push(format!(
            "upload {} into their ILIAS folders: {}",
            plural(new.len(), "new file", "new files"),
            list(new)
        ));
    }
    if !edited.is_empty() {
        parts.push(format!(
            "replace {} on ILIAS with yours (ILIAS keeps no earlier versions): {}",
            plural(edited.len(), "edited file", "edited files"),
            list(edited)
        ));
    }
    if !trash.is_empty() {
        parts.push(format!(
            "move {} you deleted to the ILIAS trash (not deleted for good; ilias-tui never empties it): {}",
            plural(trash.len(), "file", "files"),
            list(trash)
        ));
    }
    let mut question = parts.join("; and ");
    if let Some(first) = question.get(..1) {
        question = first.to_uppercase() + &question[1..];
    }
    match changes.iter().any(|c| !matches!(c, Change::Trash { .. })) {
        true => format!("{question}? Everyone who can open them on ILIAS sees them."),
        false => format!("{question}?"),
    }
}

/// Why `changes` are more than usual (more than [`MAX_FILES`] files, or more than
/// [`MAX_SHARE`] of a folder's files on ILIAS replaced or trashed), for a second question;
/// `listed` is how many files ILIAS lists in each place (by key).
pub fn brake(changes: &[Change], listed: &HashMap<u64, usize>) -> Option<String> {
    let count = |f: fn(&Change) -> bool| changes.iter().filter(|c| f(c)).count();
    let in_folders: usize = changes
        .iter()
        .map(|c| match c {
            Change::NewFolder { contents, .. } => contents.iter().filter(|p| !p.is_dir()).count(),
            _ => 0,
        })
        .sum();
    let uploads =
        count(|c| matches!(c, Change::Upload { .. } | Change::Replace { .. })) + in_folders;
    let trash = count(|c| matches!(c, Change::Trash { .. }));
    if uploads > MAX_FILES || trash > MAX_FILES {
        return Some(format!(
            "That is a lot at once: {uploads} files to upload and {trash} to move to the trash \
             (more than {MAX_FILES}). If you moved or renamed a folder, say no. Send them all?"
        ));
    }
    let mut touched: HashMap<u64, (&Item, usize)> = HashMap::new();
    for change in changes {
        if let Change::Replace { place, .. } | Change::Trash { place, .. } = change {
            touched.entry(place.key()).or_insert((place, 0)).1 += 1;
        }
    }
    let mut places: Vec<_> = touched.into_values().collect();
    places.sort_by_key(|(place, _)| place.title.clone());
    places.into_iter().find_map(|(place, n)| {
        let files = listed.get(&place.key()).copied().unwrap_or(0);
        (files > 0 && n as f64 > MAX_SHARE * files as f64).then(|| {
            format!(
                "That is a lot for one folder: {n} of the {files} files in {} would be replaced \
                 or moved to the trash. Send them all?",
                place.title
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::ilias::{Id, Kind};
    use pretty_assertions::assert_eq;

    fn folder() -> Item {
        Item {
            kind: Kind::Folder,
            id: Id::Ref(89),
            title: "Unterlagen".into(),
            ..Item::default()
        }
    }

    #[test]
    fn asks_once_more_for_more_than_usual() {
        let uploads: Vec<Change> = (0..=MAX_FILES)
            .map(|n| Change::Upload {
                place: folder(),
                path: PathBuf::from(format!("/dl/{n}.pdf")),
                renamed_from: None,
            })
            .collect();
        let listed = HashMap::from([(89, 100)]);
        assert!(brake(&uploads[..MAX_FILES], &listed).is_none());
        assert!(
            brake(&uploads, &listed)
                .unwrap()
                .contains("21 files to upload")
        );
        // 4 of a folder's 10 files replaced or trashed.
        let file = |id| Item {
            kind: Kind::File,
            id: Id::Ref(id),
            ..Item::default()
        };
        let mut changes: Vec<Change> = (1..=3)
            .map(|id| Change::Replace {
                place: folder(),
                file: file(id),
                path: PathBuf::from(format!("/dl/{id}.pdf")),
                base: "v1".into(),
            })
            .collect();
        let listed = HashMap::from([(89, 10)]);
        assert!(brake(&changes, &listed).is_none(), "3 of 10");
        changes.push(Change::Trash {
            place: folder(),
            file: file(4),
            path: PathBuf::from("/dl/4.pdf"),
            base: "v1".into(),
        });
        assert!(
            brake(&changes, &listed)
                .unwrap()
                .contains("4 of the 10 files in Unterlagen")
        );
    }

    #[test]
    fn the_question_names_each_file_and_its_size() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-ask-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Notizen.md");
        std::fs::write(&file, "mine").unwrap();
        let upload = |path: &Path| {
            vec![Change::Upload {
                place: folder(),
                path: path.to_path_buf(),
                renamed_from: None,
            }]
        };
        let question = question(&upload(&file), &dir);
        assert_eq!(
            question,
            "Upload 1 new file into their ILIAS folders: Notizen.md (0.0 MB)? Everyone who can open them on ILIAS sees them."
        );
        let both = [
            upload(&file),
            vec![Change::Replace {
                place: folder(),
                file: Item::default(),
                path: file.clone(),
                base: "v1".into(),
            }],
        ]
        .concat();
        assert!(
            super::question(&both, &dir).contains(
                "; and replace 1 edited file on ILIAS with yours (ILIAS keeps no earlier versions): Notizen.md"
            ),
            "{}",
            super::question(&both, &dir)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
