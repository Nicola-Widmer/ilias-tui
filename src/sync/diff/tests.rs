//! The three-way table, one test per row and per rule: no files, no ILIAS, only the
//! snapshots.

use std::path::Path;

use super::*;
use crate::ilias::{Id, Rights};
use crate::sync::model::{FolderEntry, Remote};
use pretty_assertions::assert_eq;

const DIR: &str = "DEMO/Unterlagen";
const V1: &str = "4 Bytes";
const V2: &str = "5 Bytes";

fn two_way() -> Rules {
    Rules {
        max_mb: 1.0,
        missing_only: false,
        two_way: true,
    }
}

fn one_way() -> Rules {
    Rules {
        two_way: false,
        ..two_way()
    }
}

fn item(kind: &str, id: u64, title: &str) -> Item {
    Item {
        kind: kind.into(),
        id: Id::Ref(id),
        title: title.into(),
        ..Item::default()
    }
}

/// ILIAS' file `id` called `title` at `version` (its size), in `dir`.
fn file(id: u64, title: &str, version: &str, dir: &str) -> Item {
    Item {
        properties: vec!["pdf".into(), version.into()],
        path: dir.split('/').map(str::to_string).collect(),
        ..item("file", id, title)
    }
}

struct World {
    base: Base,
    remote: Remote,
    local: Local,
}

impl World {
    /// The course DEMO (1) and its folder Unterlagen (2), both on your disk, where you may
    /// do everything.
    fn new() -> Self {
        let mut world = Self {
            base: Base::new(Path::new("/dl")),
            remote: Remote::default(),
            local: Local::default(),
        };
        world.place(
            item("crs", 1, "DEMO"),
            "DEMO",
            vec![item("fold", 2, "Unterlagen")],
        );
        world.place(item("fold", 2, "Unterlagen"), DIR, Vec::new());
        for dir in ["DEMO", DIR] {
            world.local.folders.insert(dir.into());
            world.base.change(|e| e.record_folder(dir, None)).unwrap();
        }
        world
    }

    fn place(&mut self, place: Item, dir: &str, items: Vec<Item>) {
        let keys: Vec<u64> = items.iter().map(Item::key).collect();
        self.remote.places.push(Place {
            item: place,
            dir: dir.into(),
            rights: Rights {
                upload: true,
                create: vec![Kind::Folder],
                delete: keys.clone(),
                versions: keys,
                ..Rights::default()
            },
            items,
        });
    }

    fn rights(&mut self, dir: &str, change: impl FnOnce(&mut Rights)) {
        let place = self
            .remote
            .places
            .iter_mut()
            .find(|p| p.dir == dir)
            .unwrap();
        change(&mut place.rights);
    }

    /// ILIAS lists `file` in the place of its folder.
    fn on_ilias(&mut self, file: Item) {
        let dir = file.path.join("/");
        let place = self
            .remote
            .places
            .iter_mut()
            .find(|p| p.dir == dir)
            .unwrap();
        place.items.push(file.clone());
        place.rights.delete.push(file.key());
        place.rights.versions.push(file.key());
        self.remote.add_file(file);
    }

    /// Sync saved `path` as ILIAS' file `id` at `version`, with content `hash`.
    fn saved(&mut self, path: &str, id: u64, version: &str, hash: &str) {
        let entry = crate::sync::model::FileEntry {
            id,
            version: version.into(),
            len: 4,
            modified_ms: 1,
            hash: hash.into(),
        };
        self.base.change(|e| e.record_file(path, entry)).unwrap();
    }

    /// `path` on your disk as saved.
    fn untouched(&mut self, path: &str) {
        self.disk(path, 4, 1, None);
    }

    fn edited(&mut self, path: &str) {
        self.disk(path, 6, 2, Some("edited"));
    }

    fn disk(&mut self, path: &str, len: u64, modified_ms: u64, hash: Option<&str>) {
        let file = LocalFile {
            len,
            modified_ms,
            hash: hash.map(str::to_string),
        };
        self.local.files.insert(path.into(), file);
    }

    fn diff(&self, rules: Rules) -> Plan {
        diff(&self.base, &self.remote, &self.local, &rules)
    }

    fn ops(&self, rules: Rules) -> Vec<Op> {
        self.diff(rules).ops
    }

    fn folder(&self) -> Item {
        item("fold", 2, "Unterlagen")
    }
}

fn path(name: &str) -> String {
    format!("{DIR}/{name}")
}

fn keep(path: &str, reason: Reason) -> Op {
    Op::Keep {
        path: path.into(),
        reason,
    }
}

// Disk: new file.

#[test]
fn a_new_file_is_uploaded_where_ilias_lets_you() {
    let mut w = World::new();
    w.disk(&path("Notizen.md"), 4, 9, Some("mine"));
    let upload = Op::Upload {
        place: w.folder(),
        path: path("Notizen.md"),
        from: None,
    };
    assert_eq!(w.ops(two_way()), [upload]);
    w.rights(DIR, |r| r.upload = false);
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Notizen.md"), Reason::NotAllowed)]
    );
}

#[test]
fn a_new_file_named_like_an_ilias_file_sync_did_not_save_is_a_conflict() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.disk(&path("Blatt 3.pdf"), 4, 9, Some("from the browser"));
    let ops = w.ops(two_way());
    assert!(ops.contains(&Op::Conflict {
        path: path("Blatt 3.pdf"),
        why: Why::SameName
    }));
    // ILIAS' one is saved (beside, under a free name), yours is not sent.
    assert!(
        ops.iter()
            .any(|op| matches!(op, Op::Download { replace: None, .. }))
    );
    assert!(!ops.iter().any(Op::writes), "{ops:?}");
}

#[test]
fn a_new_file_over_the_size_limit_is_not_sent() {
    let mut w = World::new();
    w.disk(&path("Video.mp4"), 2_000_000, 9, Some("big"));
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Video.mp4"), Reason::TooBigToSend)]
    );
}

#[test]
fn a_copy_of_a_saved_file_is_a_conflict() {
    let mut w = World::new();
    w.on_ilias(file(11, "Skript", V1, DIR));
    w.saved(&path("Skript.pdf"), 11, V1, "script");
    w.untouched(&path("Skript.pdf"));
    w.disk(&path("Skript (Kopie).pdf"), 4, 9, Some("script"));
    assert_eq!(
        w.ops(two_way()),
        [Op::Conflict {
            path: path("Skript (Kopie).pdf"),
            why: Why::CopyOf(path("Skript.pdf")),
        }]
    );
}

#[test]
fn two_ilias_folders_sharing_one_local_folder_are_a_conflict() {
    let mut w = World::new();
    w.place(item("fold", 3, "Unterlagen"), DIR, Vec::new());
    w.disk(&path("Notizen.md"), 4, 9, Some("mine"));
    w.local.folders.insert(path("Eigenes"));
    assert_eq!(
        w.ops(two_way()),
        [Op::Conflict {
            path: DIR.into(),
            why: Why::SharedFolder
        }]
    );
}

// Disk: new folder.

#[test]
fn a_new_folder_is_created_with_what_is_in_it() {
    let mut w = World::new();
    for folder in ["Eigenes", "Eigenes/Woche 1"] {
        w.local.folders.insert(path(folder));
    }
    w.disk(&path("Eigenes/a.md"), 1, 9, None);
    w.disk(&path("Eigenes/Woche 1/b.md"), 1, 9, None);
    w.disk(&path("Eigenes/Woche 1/big.mp4"), 2_000_000, 9, None);
    assert_eq!(
        w.ops(two_way()),
        [
            keep(&path("Eigenes/Woche 1/big.mp4"), Reason::TooBigToSend),
            Op::MkdirRemote {
                place: w.folder(),
                dir: path("Eigenes"),
                folders: vec![path("Eigenes/Woche 1")],
                files: vec![path("Eigenes/Woche 1/b.md"), path("Eigenes/a.md")],
            },
        ]
    );
    w.rights(DIR, |r| r.create.clear());
    assert!(
        w.ops(two_way())
            .contains(&keep(&path("Eigenes"), Reason::NotAllowed))
    );
}

#[test]
fn a_folder_ilias_lists_or_would_name_differently_is_not_created() {
    let mut w = World::new();
    w.place(
        item("fold", 2, "Unterlagen"),
        DIR,
        vec![item("exc", 6, "Blatt 0: Abgabe")],
    );
    w.remote.places.remove(1);
    w.local.folders.insert(path("Blatt 0_ Abgabe"));
    w.local.folders.insert(path("Entwurf."));
    assert_eq!(
        w.ops(two_way()),
        [Op::Conflict {
            path: path("Entwurf."),
            why: Why::ComesBackAs("Entwurf".into()),
        }]
    );
}

// Saved files, by their state on both sides.

#[test]
fn unchanged_by_you_and_changed_on_ilias_is_downloaded_in_place_of_your_copy() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V2, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.untouched(&path("Blatt 3.pdf"));
    for rules in [two_way(), one_way()] {
        assert_eq!(
            w.ops(rules),
            [Op::Download {
                file: file(9, "Blatt 3", V2, DIR),
                version: V2.into(),
                dir: DIR.into(),
                replace: Some(path("Blatt 3.pdf")),
            }]
        );
    }
}

#[test]
fn edited_by_you_and_unchanged_on_ilias_replaces_ilias_file() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.edited(&path("Blatt 3.pdf"));
    assert_eq!(
        w.ops(two_way()),
        [Op::Replace {
            place: w.folder(),
            file: file(9, "Blatt 3", V1, DIR),
            path: path("Blatt 3.pdf"),
            base: V1.into(),
        }]
    );
    w.rights(DIR, |r| r.versions.clear());
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Blatt 3.pdf"), Reason::NotAllowed)]
    );
}

#[test]
fn a_touched_file_with_the_same_content_is_not_edited() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.disk(&path("Blatt 3.pdf"), 4, 7, Some("h"));
    assert_eq!(w.ops(two_way()), []);
}

#[test]
fn edited_by_you_and_changed_on_ilias_is_a_conflict_with_ilias_copy_beside() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V2, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.edited(&path("Blatt 3.pdf"));
    let conflict = Op::Conflict {
        path: path("Blatt 3.pdf"),
        why: Why::Both {
            file: file(9, "Blatt 3", V2, DIR),
            version: V2.into(),
        },
    };
    assert_eq!(w.ops(two_way()), std::slice::from_ref(&conflict));
    assert_eq!(w.ops(one_way()), [conflict]);
}

#[test]
fn a_rename_in_the_same_folder_renames_the_ilias_file() {
    let mut w = World::new();
    w.on_ilias(file(10, "Blatt 4", V1, DIR));
    w.saved(&path("Blatt 4.pdf"), 10, V1, "four");
    w.disk(&path("Sheet 4.pdf"), 4, 9, Some("four"));
    let rename = Op::Rename {
        place: w.folder(),
        file: file(10, "Blatt 4", V1, DIR),
        from: path("Blatt 4.pdf"),
        path: path("Sheet 4.pdf"),
        base: V1.into(),
    };
    assert_eq!(w.ops(two_way()), [rename]);
    // Without Versions there: uploaded as new, the old one to the trash.
    w.rights(DIR, |r| r.versions.clear());
    let ops = w.ops(two_way());
    assert!(
        matches!(ops.as_slice(), [Op::TrashRemote { .. }, Op::Upload { .. }]),
        "{ops:?}"
    );
}

#[test]
fn a_file_moved_to_another_folder_is_uploaded_there_and_the_old_one_trashed() {
    let mut w = World::new();
    let week = "DEMO/Unterlagen/Woche 1";
    w.place(item("fold", 5, "Woche 1"), week, Vec::new());
    w.local.folders.insert(week.into());
    w.base.change(|e| e.record_folder(week, Some(5))).unwrap();
    w.on_ilias(file(10, "Blatt 4", V1, DIR));
    w.saved(&path("Blatt 4.pdf"), 10, V1, "four");
    w.disk(&format!("{week}/Blatt 4.pdf"), 4, 9, Some("four"));
    assert_eq!(
        w.ops(two_way()),
        [
            Op::TrashRemote {
                place: w.folder(),
                item: file(10, "Blatt 4", V1, DIR),
                path: path("Blatt 4.pdf"),
                base: V1.into(),
                files: 1,
            },
            Op::Upload {
                place: item("fold", 5, "Woche 1"),
                path: format!("{week}/Blatt 4.pdf"),
                from: None,
            },
        ]
    );
}

#[test]
fn an_unclear_rename_among_same_content_files_is_an_upload_and_the_gone_ones_go_to_the_trash() {
    let mut w = World::new();
    for (id, name) in [
        (20, "Pruefung 12"),
        (21, "Pruefung 13"),
        (22, "Pruefung 14"),
    ] {
        w.on_ilias(file(id, name, V1, DIR));
        w.saved(&path(&format!("{name}.pdf")), id, V1, "template");
    }
    w.untouched(&path("Pruefung 12.pdf"));
    // 13 renamed and 14 deleted, or the other way round.
    w.disk(&path("Exam 13.pdf"), 4, 9, Some("template"));
    let ops = w.ops(two_way());
    let trashed = ops
        .iter()
        .filter(|op| matches!(op, Op::TrashRemote { .. }))
        .count();
    assert_eq!(trashed, 2, "{ops:?}");
    assert!(
        ops.contains(&Op::Upload {
            place: w.folder(),
            path: path("Exam 13.pdf"),
            from: Some(path("Pruefung 13.pdf")),
        }),
        "{ops:?}"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, Op::Conflict { .. } | Op::Rename { .. }))
    );
}

#[test]
fn a_saved_file_gone_on_both_sides_does_not_make_a_rename_unclear() {
    let mut w = World::new();
    // Archiv (11) is in the ILIAS trash and gone from your disk; Archiv_2 (12), the same
    // content, you renamed to Archiv_3.
    w.saved(&path("Archiv.pdf"), 11, V1, "same");
    w.on_ilias(file(12, "Archiv_2", V1, DIR));
    w.saved(&path("Archiv_2.pdf"), 12, V1, "same");
    w.disk(&path("Archiv_3.pdf"), 4, 9, Some("same"));
    let plan = w.diff(two_way());
    assert!(plan.settle.contains(&Settle::Forget(path("Archiv.pdf"))));
    let ops: Vec<&Op> = plan
        .ops
        .iter()
        .filter(|op| !matches!(op, Op::Keep { .. }))
        .collect();
    assert_eq!(
        ops,
        [&Op::Rename {
            place: w.folder(),
            file: file(12, "Archiv_2", V1, DIR),
            from: path("Archiv_2.pdf"),
            path: path("Archiv_3.pdf"),
            base: V1.into(),
        }]
    );
}

#[test]
fn deleted_by_you_and_unchanged_on_ilias_is_proposed_for_the_trash() {
    let mut w = World::new();
    w.on_ilias(file(10, "Blatt 4", V1, DIR));
    w.saved(&path("Blatt 4.pdf"), 10, V1, "four");
    assert_eq!(
        w.ops(two_way()),
        [Op::TrashRemote {
            place: w.folder(),
            item: file(10, "Blatt 4", V1, DIR),
            path: path("Blatt 4.pdf"),
            base: V1.into(),
            files: 1,
        }]
    );
    w.rights(DIR, |r| r.delete.clear());
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Blatt 4.pdf"), Reason::Deleted)]
    );
    // One-way: not downloaded again.
    assert_eq!(
        w.ops(one_way()),
        [keep(&path("Blatt 4.pdf"), Reason::Deleted)]
    );
}

/// Unterlagen › Woche 1 (5) with a.pdf (20) and b.pdf (21), saved and then deleted.
fn deleted_week() -> World {
    let mut w = World::new();
    let week = "DEMO/Unterlagen/Woche 1";
    w.remote.places[1].items.push(item("fold", 5, "Woche 1"));
    w.remote.places[1].rights.delete.push(5);
    w.place(item("fold", 5, "Woche 1"), week, Vec::new());
    w.base.change(|e| e.record_folder(week, Some(5))).unwrap();
    for (id, name) in [(20, "a"), (21, "b")] {
        w.on_ilias(file(id, name, V1, week));
        w.saved(&format!("{week}/{name}.pdf"), id, V1, name);
    }
    w
}

#[test]
fn a_deleted_folder_goes_to_the_trash_as_one() {
    let w = deleted_week();
    assert_eq!(
        w.ops(two_way()),
        [Op::TrashRemote {
            place: w.folder(),
            item: item("fold", 5, "Woche 1"),
            path: "DEMO/Unterlagen/Woche 1".into(),
            base: String::new(),
            files: 2,
        }]
    );
}

#[test]
fn a_deleted_folder_with_a_file_changed_on_ilias_stays_and_its_files_are_decided_alone() {
    let mut w = deleted_week();
    w.remote.files[1].version = V2.into();
    let ops = w.ops(two_way());
    assert!(
        matches!(ops.as_slice(), [Op::TrashRemote { files: 1, item, .. }, Op::Download { .. }] if item.kind == Kind::File),
        "{ops:?}"
    );
    // Not allowed to delete the folder: its files alone.
    let mut w = deleted_week();
    w.remote.places[1].rights.delete.clear();
    let ops = w.ops(two_way());
    assert_eq!(ops.len(), 2, "{ops:?}");
    assert!(
        ops.iter()
            .all(|op| matches!(op, Op::TrashRemote { item, .. } if item.kind == Kind::File))
    );
    // A course never goes to the trash.
    let mut w = World::new();
    w.local.folders.clear();
    w.base = Base::new(Path::new("/dl"));
    w.base.change(|e| e.record_folder("DEMO", Some(1))).unwrap();
    assert_eq!(w.ops(two_way()), []);
    // A saved folder you deleted, empty on ILIAS: to the trash too.
    w.base.change(|e| e.record_folder(DIR, Some(2))).unwrap();
    assert!(matches!(
        w.ops(two_way()).as_slice(),
        [Op::TrashRemote { files: 0, .. }]
    ));
}

#[test]
fn deleted_by_you_and_changed_on_ilias_is_downloaded_again() {
    let mut w = World::new();
    w.on_ilias(file(10, "Blatt 4", V2, DIR));
    w.saved(&path("Blatt 4.pdf"), 10, V1, "four");
    let ops = w.ops(two_way());
    assert!(
        matches!(ops.as_slice(), [Op::Download { replace: None, .. }]),
        "{ops:?}"
    );
}

#[test]
fn unchanged_by_you_and_gone_on_ilias_goes_to_your_trash() {
    let mut w = World::new();
    w.saved(&path("Alt.pdf"), 12, V1, "old");
    w.untouched(&path("Alt.pdf"));
    assert_eq!(
        w.ops(two_way()),
        [Op::TrashLocal {
            path: path("Alt.pdf")
        }]
    );
    assert_eq!(w.ops(one_way()), []);
}

#[test]
fn edited_by_you_and_gone_on_ilias_is_kept() {
    let mut w = World::new();
    w.saved(&path("Notizen.pdf"), 13, V1, "old");
    w.edited(&path("Notizen.pdf"));
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Notizen.pdf"), Reason::GoneEdited)]
    );
}

#[test]
fn gone_on_both_sides_is_forgotten() {
    let mut w = World::new();
    w.saved(&path("Alt.pdf"), 12, V1, "old");
    let plan = w.diff(two_way());
    assert_eq!(plan.ops, []);
    assert_eq!(plan.settle, [Settle::Forget(path("Alt.pdf"))]);
}

#[test]
fn a_change_without_a_hash_is_kept_never_sent() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "");
    w.edited(&path("Blatt 3.pdf"));
    assert_eq!(
        w.ops(two_way()),
        [keep(&path("Blatt 3.pdf"), Reason::CantTell)]
    );
    // Still as saved: it gets its hash.
    w.disk(&path("Blatt 3.pdf"), 4, 1, Some("now"));
    let plan = w.diff(two_way());
    assert_eq!(
        plan.settle,
        [Settle::Hash {
            path: path("Blatt 3.pdf"),
            hash: "now".into()
        }]
    );
}

// Rules.

#[test]
fn new_on_ilias_is_downloaded_unless_too_big() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.on_ilias(file(10, "Aufzeichnung", "107.9 MB", DIR));
    assert_eq!(
        w.ops(one_way()),
        [
            Op::Download {
                file: file(9, "Blatt 3", V1, DIR),
                version: V1.into(),
                dir: DIR.into(),
                replace: None,
            },
            keep("Aufzeichnung (107.9 MB)", Reason::TooBig),
        ]
    );
}

#[test]
fn an_entry_without_a_version_is_adopted() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V2, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, "", "");
    w.untouched(&path("Blatt 3.pdf"));
    let plan = w.diff(two_way());
    assert_eq!(
        plan.settle,
        [Settle::Adopt {
            path: path("Blatt 3.pdf"),
            version: V2.into()
        }]
    );
    assert_eq!(w.ops(two_way()), []);
}

#[test]
fn missing_only_restores_deleted_files_and_leaves_the_rest() {
    let rules = Rules {
        missing_only: true,
        ..one_way()
    };
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.on_ilias(file(10, "Blatt 4", V2, DIR));
    w.saved(&path("Blatt 4.pdf"), 10, V1, "h4");
    w.edited(&path("Blatt 4.pdf"));
    let ops = w.ops(rules);
    assert!(
        matches!(ops.as_slice(), [Op::Download { file, replace: None, .. }] if file.key() == 9),
        "{ops:?}"
    );
}

#[test]
fn files_of_places_not_read_are_left_alone() {
    let mut w = World::new();
    w.saved("OTHER/Skript.pdf", 30, V1, "s");
    let plan = w.diff(two_way());
    assert!(plan.ops.is_empty() && !plan.settle.iter().any(|s| matches!(s, Settle::Forget(_))));
}

#[test]
fn one_way_sends_nothing_and_says_nothing_about_what_two_way_would() {
    let mut w = World::new();
    w.on_ilias(file(9, "Blatt 3", V1, DIR));
    w.saved(&path("Blatt 3.pdf"), 9, V1, "h");
    w.edited(&path("Blatt 3.pdf"));
    w.saved(&path("Alt.pdf"), 12, V1, "old");
    w.untouched(&path("Alt.pdf"));
    w.disk(&path("Notizen.md"), 4, 9, Some("mine"));
    w.local.folders.insert(path("Eigenes"));
    let plan = w.diff(one_way());
    assert!(!plan.ops.iter().any(Op::writes), "{plan:?}");
    assert_eq!(plan.ops, [], "{plan:?}");
    assert_eq!(
        plan.unchanged, 1,
        "edited, unchanged on ILIAS: counted as unchanged"
    );
}

#[test]
fn a_saved_folder_gone_on_ilias_is_not_created_again_its_files_go_to_your_trash() {
    let mut w = World::new();
    let old = path("Alt");
    w.base.change(|e| e.record_folder(&old, Some(7))).unwrap();
    w.local.folders.insert(old.clone());
    w.saved(&format!("{old}/a.pdf"), 40, V1, "a");
    w.untouched(&format!("{old}/a.pdf"));
    assert_eq!(
        w.ops(two_way()),
        [Op::TrashLocal {
            path: format!("{old}/a.pdf")
        }]
    );
    // Gone from your disk too: forgotten.
    w.local.folders.remove(&old);
    w.local.files.clear();
    let settle = w.diff(two_way()).settle;
    assert!(settle.contains(&Settle::ForgetFolder(old)), "{settle:?}");
}

#[test]
fn the_places_folders_on_your_disk_become_saved_folders() {
    let mut w = World::new();
    w.base = Base::new(Path::new("/dl"));
    let settle = w.diff(one_way()).settle;
    assert_eq!(
        settle,
        [
            Settle::Folder {
                path: "DEMO".into(),
                id: 1
            },
            Settle::Folder {
                path: DIR.into(),
                id: 2
            },
        ]
    );
    assert_eq!(FolderEntry::default().id, None);
}
