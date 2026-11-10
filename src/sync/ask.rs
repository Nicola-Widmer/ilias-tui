//! Two-way sync's brake: the review is the question before sending, and only a run that
//! sends more than usual asks once more.

use std::collections::HashMap;

use super::Change;
use crate::ilias::Item;

/// More files than this to upload in one run (new ones, those in new folders, replaced
/// ones), or to move to the trash, and sync asks once more before sending.
pub const MAX_FILES: usize = 20;
/// More files than this of one folder replaced or moved to the trash, and sync asks once
/// more.
pub const MAX_IN_FOLDER: usize = 5;

/// The question before moving `paths` (gone on ILIAS, untouched on your disk) to your
/// computer's trash; `dir` shortens them.
pub fn local_trash(paths: &[std::path::PathBuf], dir: &std::path::Path) -> String {
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.strip_prefix(dir).unwrap_or(p).display().to_string())
        .collect();
    let mut lines: Vec<String> = names.iter().take(8).cloned().collect();
    if names.len() > 8 {
        lines.push(format!("and {} more (see the report)", names.len() - 8));
    }
    let files = match names.len() {
        1 => "this file".to_string(),
        n => format!("these {n} files"),
    };
    format!(
        "Deleted on ILIAS, unchanged on your disk. Move {files} to your Trash?\n{}",
        lines.join("\n")
    )
}

/// Why `changes` are more than usual, for a second question; `None` if they aren't.
pub fn brake(changes: &[Change]) -> Option<String> {
    let in_folders: usize = changes
        .iter()
        .map(|c| match c {
            Change::NewFolder { contents, .. } => contents.iter().filter(|p| !p.is_dir()).count(),
            _ => 0,
        })
        .sum();
    let count = |f: fn(&Change) -> bool| changes.iter().filter(|c| f(c)).count();
    let uploads = count(|c| {
        matches!(
            c,
            Change::Upload { .. } | Change::Replace { .. } | Change::Rename { .. }
        )
    }) + in_folders;
    let trash = count(|c| matches!(c, Change::Trash { .. }));
    if uploads > MAX_FILES || trash > MAX_FILES {
        return Some(format!(
            "That is a lot at once: {uploads} files to upload and {trash} to move to the trash \
             (more than {MAX_FILES}). If you moved or renamed a folder, say no. Send them all?"
        ));
    }
    let mut touched: HashMap<u64, (&Item, usize)> = HashMap::new();
    for change in changes {
        if let Change::Replace { place, .. }
        | Change::Rename { place, .. }
        | Change::Trash { place, .. } = change
        {
            touched.entry(place.key()).or_insert((place, 0)).1 += 1;
        }
    }
    let mut places: Vec<_> = touched.into_values().collect();
    places.sort_by_key(|(place, _)| place.title.clone());
    places
        .into_iter()
        .find(|(_, n)| *n > MAX_IN_FOLDER)
        .map(|(place, n)| {
            format!(
                "That is a lot for one folder: {n} files in {} would be replaced or moved to the \
             trash (more than {MAX_IN_FOLDER}). Send them all?",
                place.title
            )
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::ilias::{Id, Kind};

    fn folder() -> Item {
        Item {
            kind: Kind::Folder,
            id: Id::Ref(89),
            title: "Unterlagen".into(),
            ..Item::default()
        }
    }

    fn file(id: u64) -> Item {
        Item {
            kind: Kind::File,
            id: Id::Ref(id),
            ..Item::default()
        }
    }

    #[test]
    fn asks_once_more_for_more_than_20_files() {
        let uploads: Vec<Change> = (0..=MAX_FILES)
            .map(|n| Change::Upload {
                place: folder(),
                path: PathBuf::from(format!("/dl/{n}.pdf")),
                renamed_from: None,
            })
            .collect();
        assert!(brake(&uploads[..MAX_FILES]).is_none());
        assert!(brake(&uploads).unwrap().contains("21 files to upload"));
        // The files in a new folder count too.
        let folder_of = |n: usize| Change::NewFolder {
            place: folder(),
            dir: PathBuf::from("/dl/Eigenes"),
            contents: (0..n)
                .map(|i| PathBuf::from(format!("/dl/Eigenes/{i}.md")))
                .collect(),
        };
        assert!(brake(&[folder_of(MAX_FILES)]).is_none());
        assert!(brake(&[folder_of(MAX_FILES + 1)]).is_some());
    }

    #[test]
    fn asks_once_more_for_more_than_5_files_of_one_folder_whatever_its_size() {
        let replace = |id| Change::Replace {
            place: folder(),
            file: file(id),
            path: PathBuf::from(format!("/dl/{id}.pdf")),
            base: "v1".into(),
        };
        // One file of a small folder: no second question.
        assert!(brake(&[replace(1)]).is_none());
        let mut changes: Vec<Change> = (1..=4).map(replace).collect();
        changes.push(Change::Trash {
            place: folder(),
            file: file(5),
            path: PathBuf::from("/dl/5.pdf"),
            base: "v1".into(),
        });
        assert!(brake(&changes).is_none(), "5 in one folder");
        changes.push(replace(6));
        assert!(
            brake(&changes)
                .unwrap()
                .contains("6 files in Unterlagen would be replaced or moved to the trash")
        );
    }

    #[test]
    fn names_the_files_for_the_trash_on_your_disk() {
        let dir = std::path::Path::new("/dl");
        assert_eq!(
            local_trash(&[PathBuf::from("/dl/DEMO/Alt.pdf")], dir),
            "Deleted on ILIAS, unchanged on your disk. Move this file to your Trash?\nDEMO/Alt.pdf"
        );
        let many: Vec<PathBuf> = (0..10)
            .map(|n| PathBuf::from(format!("/dl/{n}.pdf")))
            .collect();
        let question = local_trash(&many, dir);
        assert!(
            question.contains("these 10 files")
                && question.ends_with("and 2 more (see the report)")
        );
    }
}
