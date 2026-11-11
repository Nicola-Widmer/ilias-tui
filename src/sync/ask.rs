//! Two-way sync's brake: the review is the question before sending, and only a run that
//! sends more than usual asks once more.

use std::collections::HashMap;

use super::Op;
use crate::ilias::{Item, Kind};

/// More files than this to upload in one run (new ones, those in new folders, replaced
/// ones), or to move to the trash, and sync asks once more before sending.
pub const MAX_FILES: usize = 20;
/// More files than this of one folder replaced or moved to the trash, and sync asks once
/// more.
pub const MAX_IN_FOLDER: usize = 5;

/// The question before moving `paths` (gone on ILIAS, untouched on your disk; below the
/// download folder) to your computer's trash.
pub fn local_trash(paths: &[String]) -> String {
    let mut lines: Vec<String> = paths.iter().take(8).cloned().collect();
    if paths.len() > 8 {
        lines.push(format!("and {} more (see the report)", paths.len() - 8));
    }
    let files = match paths.len() {
        1 => "this file".to_string(),
        n => format!("these {n} files"),
    };
    format!(
        "Deleted on ILIAS, unchanged on your disk. Move {files} to your Trash?\n{}",
        lines.join("\n")
    )
}

/// Why `ops` are more than usual, for a second question; `None` if they aren't.
pub fn brake(ops: &[Op]) -> Option<String> {
    let mut uploads = 0;
    let mut trash = 0;
    // A whole folder is more than its one line in the review: always asked about.
    let mut folders: Vec<&str> = Vec::new();
    let mut touched: HashMap<u64, (&Item, usize)> = HashMap::new();
    for op in ops {
        match op {
            Op::Upload { .. } | Op::Replace { .. } | Op::Rename { .. } => uploads += 1,
            Op::MkdirRemote { files, .. } => uploads += files.len(),
            Op::TrashRemote { item, .. } if item.kind == Kind::Folder => {
                trash += 1;
                folders.push(&item.title);
            }
            Op::TrashRemote { .. } => trash += 1,
            Op::Download { .. } | Op::TrashLocal { .. } | Op::Conflict { .. } | Op::Keep { .. } => {
            }
        }
        if let Op::Replace { place, .. }
        | Op::Rename { place, .. }
        | Op::TrashRemote { place, .. } = op
        {
            touched.entry(place.key()).or_insert((place, 0)).1 += 1;
        }
    }
    if !folders.is_empty() {
        return Some(format!(
            "Move these folders, with everything in them, to the ILIAS trash?\n{}",
            folders.join("\n")
        ));
    }
    if uploads > MAX_FILES || trash > MAX_FILES {
        return Some(format!(
            "That is a lot at once: {uploads} files to upload and {trash} to move to the trash \
             (more than {MAX_FILES}). If you moved or renamed a folder, say no. Send them all?"
        ));
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
    use super::*;
    use crate::ilias::Id;

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

    fn trash(id: u64) -> Op {
        Op::TrashRemote {
            place: folder(),
            item: file(id),
            path: format!("{id}.pdf"),
            base: "v1".into(),
            files: 1,
        }
    }

    #[test]
    fn asks_once_more_for_more_than_20_files() {
        let uploads: Vec<Op> = (0..=MAX_FILES)
            .map(|n| Op::Upload {
                place: folder(),
                path: format!("{n}.pdf"),
                from: None,
            })
            .collect();
        assert!(brake(&uploads[..MAX_FILES]).is_none());
        assert!(brake(&uploads).unwrap().contains("21 files to upload"));
        // The files in a new folder count too.
        let folder_of = |n: usize| Op::MkdirRemote {
            place: folder(),
            dir: "Eigenes".into(),
            folders: Vec::new(),
            files: (0..n).map(|i| format!("Eigenes/{i}.md")).collect(),
        };
        assert!(brake(&[folder_of(MAX_FILES)]).is_none());
        assert!(brake(&[folder_of(MAX_FILES + 1)]).is_some());
    }

    #[test]
    fn asks_once_more_for_more_than_5_files_of_one_folder_whatever_its_size() {
        let replace = |id| Op::Replace {
            place: folder(),
            file: file(id),
            path: format!("{id}.pdf"),
            base: "v1".into(),
        };
        // One file of a small folder: no second question.
        assert!(brake(&[replace(1)]).is_none());
        let mut ops: Vec<Op> = (1..=4).map(replace).collect();
        ops.push(trash(5));
        assert!(brake(&ops).is_none(), "5 in one folder");
        ops.push(replace(6));
        assert!(
            brake(&ops)
                .unwrap()
                .contains("6 files in Unterlagen would be replaced or moved to the trash")
        );
    }

    #[test]
    fn names_the_files_for_the_trash_on_your_disk() {
        assert_eq!(
            local_trash(&["DEMO/Alt.pdf".into()]),
            "Deleted on ILIAS, unchanged on your disk. Move this file to your Trash?\nDEMO/Alt.pdf"
        );
        let many: Vec<String> = (0..10).map(|n| format!("{n}.pdf")).collect();
        let question = local_trash(&many);
        assert!(
            question.contains("these 10 files")
                && question.ends_with("and 2 more (see the report)")
        );
    }

    #[test]
    fn a_folder_to_the_trash_is_always_asked_about() {
        let folder_trash = Op::TrashRemote {
            place: folder(),
            item: Item {
                kind: Kind::Folder,
                id: Id::Ref(5),
                title: "Unterlagen › Woche 1".into(),
                ..Item::default()
            },
            path: "DEMO/Unterlagen/Woche 1".into(),
            base: String::new(),
            files: 2,
        };
        assert_eq!(
            brake(&[folder_trash]).unwrap(),
            "Move these folders, with everything in them, to the ILIAS trash?\nUnterlagen › Woche 1"
        );
    }
}
