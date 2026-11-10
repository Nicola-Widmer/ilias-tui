//! Two-way sync's plan for ILIAS: what your disk has that ILIAS doesn't, decided three-way
//! from the **base** (what the saved-files list recorded when both were last in sync), your
//! disk now and ILIAS now. Nothing here sends anything; the [`Review`] is shown first.
//!
//! | Your disk vs base | ILIAS vs base | Here                                        |
//! | ----------------- | ------------- | ------------------------------------------- |
//! | new               | –             | [`Change::Upload`], where ILIAS lets you    |
//! | edited            | unchanged     | [`Change::Replace`]                      |
//! | deleted           | unchanged     | [`Change::Trash`], a proposal (unticked)    |
//! | any               | gone          | kept, listed in [`Review::gone`]            |
//!
//! The rows that download (ILIAS changed) are decided per file in [`super::plan`]. Anything
//! unclear (a file of the same name on ILIAS, two ILIAS folders sharing one local folder, a
//! copy of another saved file) becomes a conflict: nothing is sent, both copies stay.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::local;
use crate::downloads::Downloads;
use crate::ilias::{Item, Kind, Rights, lists_as, sanitize};

/// A course or folder the sync read, and its folder on your disk. Only pages with a place
/// below a course or group have one (elsewhere it would be the download folder itself).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    pub item: Item,
    pub dir: PathBuf,
    pub rights: Rights,
    /// What ILIAS lists in it.
    pub items: Vec<Item>,
}

/// One write two-way sync proposes.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A file only on your disk: upload it into `place`. `renamed_from`: the saved file
    /// with the same content that is gone from your disk (so it is not proposed for the
    /// trash).
    Upload {
        place: Item,
        path: PathBuf,
        renamed_from: Option<PathBuf>,
    },
    /// You edited `path`, ILIAS still has the version saved (`base`): upload it as a new
    /// version of `file`.
    Replace {
        place: Item,
        file: Item,
        path: PathBuf,
        base: String,
    },
    /// You deleted `path`, ILIAS still has the version saved (`base`): move `file` into the
    /// ILIAS trash, only if you tick it.
    Trash {
        place: Item,
        file: Item,
        path: PathBuf,
        base: String,
    },
    /// A folder only on your disk: create it in `place`, then what is in it: `contents`,
    /// its folders and files all the way down, each folder before what is in it.
    NewFolder {
        place: Item,
        dir: PathBuf,
        contents: Vec<PathBuf>,
    },
}

impl Change {
    /// The local file or folder it is about.
    pub fn path(&self) -> &Path {
        match self {
            Change::Upload { path, .. }
            | Change::Replace { path, .. }
            | Change::Trash { path, .. } => path,
            Change::NewFolder { dir, .. } => dir,
        }
    }
}

/// What two-way sync would send, and what it leaves alone and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Review {
    pub changes: Vec<Change>,
    /// Unclear cases, with why: nothing sent, both copies kept.
    pub conflicts: Vec<String>,
    /// Saved files ILIAS no longer lists: kept on your disk.
    pub gone: Vec<PathBuf>,
    /// New or edited files where ILIAS doesn't let you upload.
    pub local_only: Vec<PathBuf>,
    /// Changed on your disk, but saved before content hashes were recorded: whether you
    /// edited them can't be told, so they are not sent.
    pub unsure: Vec<PathBuf>,
    /// New files over the sync size limit.
    pub too_big: Vec<PathBuf>,
    /// How many files ILIAS lists in each place (by key), for the mass-change brake.
    pub listed: HashMap<u64, usize>,
    /// Dry run: two-way sync stops after the plan.
    pub dry_run: bool,
}

impl Review {
    /// What sync sends after the review (besides the ticked proposals): uploads of new
    /// files, new folders with what is in them, your edited files in place of ILIAS' ones.
    pub fn sendable(&self) -> Vec<Change> {
        self.uploads()
            .chain(self.new_folders())
            .chain(self.replacements())
            .cloned()
            .collect()
    }

    pub fn uploads(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::Upload { .. }))
    }

    pub fn replacements(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::Replace { .. }))
    }

    pub fn proposals(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::Trash { .. }))
    }

    pub fn new_folders(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::NewFolder { .. }))
    }
}

/// What the per-file plan left for the review.
#[derive(Debug, Default)]
pub struct Candidates {
    /// Edited by you, unchanged on ILIAS: (file, your copy).
    pub edited: Vec<(Item, PathBuf)>,
    /// Deleted by you, unchanged on ILIAS.
    pub deleted: Vec<(Item, PathBuf)>,
    pub unsure: Vec<PathBuf>,
}

/// Decides what to send. `places`: every course and folder read; `on_ilias`: the keys of
/// every file they list; `downloads`: the base, after this sync's downloads; `max_bytes`:
/// the sync size limit.
pub fn decide(
    places: &[Place],
    on_ilias: &HashSet<u64>,
    candidates: Candidates,
    downloads: &Downloads,
    max_bytes: u64,
) -> Review {
    let mut review = Review {
        unsure: candidates.unsure,
        listed: places
            .iter()
            .map(|p| {
                let files = p.items.iter().filter(|i| i.kind == Kind::File).count();
                (p.item.key(), files)
            })
            .collect(),
        ..Review::default()
    };
    let place_of = |file: &Item| {
        places
            .iter()
            .find(|p| p.items.iter().any(|i| i.id == file.id))
    };
    let base = |key: u64| downloads.entry(key);

    for (file, path) in candidates.edited {
        match place_of(&file) {
            Some(place) if may_change(place, &file) => review.changes.push(Change::Replace {
                place: place.item.clone(),
                base: base(file.key())
                    .map(super::saved_version)
                    .unwrap_or_default(),
                file,
                path,
            }),
            _ => review.local_only.push(path),
        }
    }
    // Proposals for the trash, until a renamed copy turns up below.
    let mut trash: Vec<(Item, Item, PathBuf)> = Vec::new();
    for (file, path) in candidates.deleted {
        if let Some(place) = place_of(&file).filter(|p| p.rights.delete.contains(&file.key())) {
            trash.push((place.item.clone(), file, path));
        }
    }

    // Two ILIAS folders with the same name share one local folder: which one a file is
    // for can't be told.
    let mut sharing: HashMap<&Path, usize> = HashMap::new();
    for place in places {
        *sharing.entry(place.dir.as_path()).or_default() += 1;
    }
    let base_paths: HashSet<&Path> = downloads.entries().map(|(_, s)| s.path.as_path()).collect();
    // Saved files by content; several files can have the same.
    let mut by_hash: HashMap<&str, Vec<&Path>> = HashMap::new();
    for (_, saved) in downloads.entries().filter(|(_, s)| s.has_hash()) {
        by_hash
            .entry(saved.hash.as_str())
            .or_default()
            .push(saved.path.as_path());
    }
    let mut renamed: HashSet<&Path> = HashSet::new();
    let mut scanned = HashSet::new();
    for place in places {
        if !scanned.insert(&place.dir) {
            continue;
        }
        let found = local::scan(&place.dir);
        if sharing[place.dir.as_path()] > 1 {
            if !found.files.is_empty() || !found.folders.is_empty() {
                review.conflicts.push(format!(
                    "{}: several ILIAS folders are saved here, so nothing in it is sent",
                    place.dir.display()
                ));
            }
            continue;
        }
        for (path, len) in found.files {
            if base_paths.contains(path.as_path()) {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if len > max_bytes {
                review.too_big.push(path);
                continue;
            }
            if place
                .items
                .iter()
                .any(|i| i.kind == Kind::File && lists_as(&i.title, name))
            {
                review.conflicts.push(format!(
                    "{}: ILIAS has a file of this name here that sync didn't save; kept both, nothing sent",
                    path.display()
                ));
                continue;
            }
            let hash = crate::downloads::hash_file(&path).unwrap_or_default();
            let same = by_hash.get(hash.as_str()).map_or(&[][..], Vec::as_slice);
            // Renamed or moved: a saved file with this content is gone from its place (and
            // not taken by another rename yet).
            let gone = same
                .iter()
                .find(|saved| !saved.exists() && !renamed.contains(**saved));
            let renamed_from = match (gone, same.first()) {
                (Some(saved), _) => {
                    renamed.insert(saved);
                    // The old name is not proposed for the trash, nor any other missing file
                    // with this content: which one was renamed can't be told.
                    trash.retain(|(_, _, missing)| !same.contains(&missing.as_path()));
                    Some(saved.to_path_buf())
                }
                (None, Some(saved)) => {
                    review.conflicts.push(format!(
                        "{}: a copy of {}, which sync saved; not sent",
                        path.display(),
                        saved.display()
                    ));
                    continue;
                }
                (None, None) => None,
            };
            if place.rights.upload {
                review.changes.push(Change::Upload {
                    place: place.item.clone(),
                    path,
                    renamed_from,
                });
            } else {
                review.local_only.push(path);
            }
        }
        for folder in found.folders {
            let name = folder
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            // Saved from ILIAS: a folder sync read, or anything else listed here (an
            // exercise's submissions, say).
            let known = places.iter().any(|p| p.dir == folder)
                || place
                    .items
                    .iter()
                    .any(|i| sanitize(&i.title).eq_ignore_ascii_case(name));
            if known {
                continue;
            }
            if !place.rights.may_create(&Kind::Folder) {
                review.local_only.push(folder);
            } else if sanitize(name) != name {
                // Sync would save the ILIAS folder under another name.
                review.conflicts.push(format!(
                    "{}: a folder of this name would come back as {}; not sent",
                    folder.display(),
                    sanitize(name)
                ));
            } else {
                let (contents, too_big) = local::walk(&folder, max_bytes);
                review.too_big.extend(too_big);
                review.changes.push(Change::NewFolder {
                    place: place.item.clone(),
                    dir: folder,
                    contents,
                });
            }
        }
    }
    review
        .changes
        .extend(trash.into_iter().map(|(place, file, path)| {
            Change::Trash {
                base: base(file.key())
                    .map(super::saved_version)
                    .unwrap_or_default(),
                place,
                file,
                path,
            }
        }));

    // Saved files in these folders that ILIAS no longer lists.
    let dirs: HashSet<&Path> = places.iter().map(|p| p.dir.as_path()).collect();
    review.gone = downloads
        .entries()
        .filter(|(key, saved)| {
            !on_ilias.contains(key)
                && saved.path.parent().is_some_and(|d| dirs.contains(d))
                && saved.path.exists()
        })
        .map(|(_, saved)| saved.path.clone())
        .collect();
    review.gone.sort();
    review
}

/// Whether ILIAS lets you replace `file` in `place`: its menu there has Versions (where
/// "Replace File" is).
fn may_change(place: &Place, file: &Item) -> bool {
    place.rights.versions.contains(&file.key())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::ilias::Id;
    use pretty_assertions::assert_eq;

    fn item(kind: &str, id: u64, title: &str) -> Item {
        Item {
            kind: kind.into(),
            id: Id::Ref(id),
            title: title.into(),
            ..Item::default()
        }
    }

    /// A folder `Unterlagen` (ref id 2) at `dir`, listing `items`, where you may upload,
    /// create folders and delete `delete`.
    fn place(dir: &Path, items: Vec<Item>, delete: Vec<u64>) -> Place {
        Place {
            item: item("fold", 2, "Unterlagen"),
            dir: dir.to_path_buf(),
            rights: Rights {
                upload: true,
                create: vec![Kind::Folder],
                versions: items.iter().map(Item::key).collect(),
                delete,
                ..Rights::default()
            },
            items,
        }
    }

    /// A fresh folder per test, with a saved-files list in it.
    fn setup(name: &str) -> (PathBuf, Downloads) {
        let dir =
            std::env::temp_dir().join(format!("ilias-tui-review-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Unterlagen")).unwrap();
        let downloads = Downloads::in_file(dir.join("index.json"));
        (dir.clone(), downloads)
    }

    #[test]
    fn uploads_new_files_and_leaves_out_what_is_ignored_or_too_big() {
        let (dir, downloads) = setup("new");
        let folder = dir.join("Unterlagen");
        fs::write(folder.join("Notizen.md"), "mine").unwrap();
        fs::write(folder.join("Thumbs.db"), "x").unwrap();
        fs::write(folder.join("~$Bericht.docx"), "x").unwrap();
        fs::write(folder.join("Video.mp4"), "0123456789").unwrap();
        let places = [place(&folder, Vec::new(), Vec::new())];
        let review = decide(
            &places,
            &HashSet::new(),
            Candidates::default(),
            &downloads,
            5,
        );
        assert_eq!(
            review.changes,
            [Change::Upload {
                place: places[0].item.clone(),
                path: folder.join("Notizen.md"),
                renamed_from: None,
            }]
        );
        assert_eq!(review.too_big, [folder.join("Video.mp4")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_where_ilias_lets_you() {
        let (dir, downloads) = setup("rights");
        let folder = dir.join("Unterlagen");
        fs::write(folder.join("Notizen.md"), "mine").unwrap();
        fs::create_dir_all(folder.join("Eigenes")).unwrap();
        let mut student = place(&folder, vec![item("file", 9, "Blatt 3")], Vec::new());
        student.rights = Rights::default();
        let candidates = Candidates {
            edited: vec![(item("file", 9, "Blatt 3"), folder.join("Blatt 3.pdf"))],
            deleted: vec![(item("file", 9, "Blatt 3"), folder.join("Blatt 3.pdf"))],
            unsure: Vec::new(),
        };
        let review = decide(
            &[student],
            &HashSet::from([9]),
            candidates,
            &downloads,
            1 << 20,
        );
        assert!(review.changes.is_empty(), "{review:?}");
        assert_eq!(
            review.local_only,
            [
                folder.join("Blatt 3.pdf"),
                folder.join("Notizen.md"),
                folder.join("Eigenes")
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposes_replacements_and_the_trash_with_the_base_version() {
        let (dir, mut downloads) = setup("versions");
        let folder = dir.join("Unterlagen");
        for (key, name) in [(9, "Blatt 3.pdf"), (10, "Blatt 4.pdf")] {
            fs::write(folder.join(name), name).unwrap();
            downloads
                .record(key, folder.join(name), "v1".into())
                .unwrap();
        }
        fs::remove_file(folder.join("Blatt 4.pdf")).unwrap();
        let files = vec![item("file", 9, "Blatt 3"), item("file", 10, "Blatt 4")];
        let places = [place(&folder, files.clone(), vec![10])];
        let candidates = Candidates {
            edited: vec![(files[0].clone(), folder.join("Blatt 3.pdf"))],
            deleted: vec![(files[1].clone(), folder.join("Blatt 4.pdf"))],
            unsure: Vec::new(),
        };
        let review = decide(
            &places,
            &HashSet::from([9, 10]),
            candidates,
            &downloads,
            1 << 20,
        );
        assert_eq!(
            review.changes,
            [
                Change::Replace {
                    place: places[0].item.clone(),
                    file: files[0].clone(),
                    path: folder.join("Blatt 3.pdf"),
                    base: "v1".into(),
                },
                Change::Trash {
                    place: places[0].item.clone(),
                    file: files[1].clone(),
                    path: folder.join("Blatt 4.pdf"),
                    base: "v1".into(),
                },
            ]
        );
        // Without a Delete for it on ILIAS, nothing is proposed.
        let places = [place(&folder, files.clone(), Vec::new())];
        let candidates = Candidates {
            deleted: vec![(files[1].clone(), folder.join("Blatt 4.pdf"))],
            ..Candidates::default()
        };
        let review = decide(
            &places,
            &HashSet::from([9, 10]),
            candidates,
            &downloads,
            1 << 20,
        );
        assert!(review.changes.is_empty(), "{review:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_renamed_file_is_uploaded_and_its_old_name_not_proposed_for_the_trash() {
        let (dir, mut downloads) = setup("rename");
        let folder = dir.join("Unterlagen");
        fs::write(folder.join("Blatt 4.pdf"), "sheet four").unwrap();
        downloads
            .record(10, folder.join("Blatt 4.pdf"), "v1".into())
            .unwrap();
        fs::rename(folder.join("Blatt 4.pdf"), folder.join("Sheet 4.pdf")).unwrap();
        let file = item("file", 10, "Blatt 4");
        let places = [place(&folder, vec![file.clone()], vec![10])];
        let candidates = Candidates {
            deleted: vec![(file, folder.join("Blatt 4.pdf"))],
            ..Candidates::default()
        };
        let review = decide(
            &places,
            &HashSet::from([10]),
            candidates,
            &downloads,
            1 << 20,
        );
        assert_eq!(
            review.changes,
            [Change::Upload {
                place: places[0].item.clone(),
                path: folder.join("Sheet 4.pdf"),
                renamed_from: Some(folder.join("Blatt 4.pdf")),
            }]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rename_is_found_among_files_with_the_same_content() {
        let (dir, mut downloads) = setup("same");
        let folder = dir.join("Unterlagen");
        for (key, name) in [
            (20, "Pruefung 12.pdf"),
            (21, "Pruefung 13.pdf"),
            (22, "Pruefung 14.pdf"),
        ] {
            fs::write(folder.join(name), "same exam template").unwrap();
            downloads
                .record(key, folder.join(name), "v1".into())
                .unwrap();
        }
        fs::rename(folder.join("Pruefung 13.pdf"), folder.join("Exam 13.pdf")).unwrap();
        fs::remove_file(folder.join("Pruefung 14.pdf")).unwrap();
        let (p13, p14) = (
            item("file", 21, "Pruefung 13"),
            item("file", 22, "Pruefung 14"),
        );
        let listed = vec![item("file", 20, "Pruefung 12"), p13.clone(), p14.clone()];
        let places = [place(&folder, listed, vec![21, 22])];
        // 13 renamed and 14 deleted, or the other way round: neither goes to the trash.
        let candidates = Candidates {
            deleted: vec![
                (p13, folder.join("Pruefung 13.pdf")),
                (p14, folder.join("Pruefung 14.pdf")),
            ],
            ..Candidates::default()
        };
        let review = decide(
            &places,
            &HashSet::from([20, 21, 22]),
            candidates,
            &downloads,
            1 << 20,
        );
        let [
            Change::Upload {
                path, renamed_from, ..
            },
        ] = review.changes.as_slice()
        else {
            panic!("{review:?}");
        };
        assert_eq!(path, &folder.join("Exam 13.pdf"));
        let old = renamed_from.as_ref().and_then(|p| p.file_name()).unwrap();
        assert!(
            old == "Pruefung 13.pdf" || old == "Pruefung 14.pdf",
            "{old:?}"
        );
        assert!(review.conflicts.is_empty(), "{review:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn anything_unclear_is_a_conflict_and_nothing_is_sent() {
        let (dir, mut downloads) = setup("conflicts");
        let folder = dir.join("Unterlagen");
        // ILIAS lists "Blatt 3", which sync never saved; you have a Blatt 3.pdf.
        fs::write(folder.join("Blatt 3.pdf"), "from the browser").unwrap();
        // A copy of a saved file that is still there.
        fs::write(folder.join("Skript.pdf"), "script").unwrap();
        downloads
            .record(11, folder.join("Skript.pdf"), "v1".into())
            .unwrap();
        fs::write(folder.join("Skript (Kopie).pdf"), "script").unwrap();
        let places = [place(
            &folder,
            vec![item("file", 9, "Blatt 3"), item("file", 11, "Skript")],
            Vec::new(),
        )];
        let review = decide(
            &places,
            &HashSet::from([9, 11]),
            Candidates::default(),
            &downloads,
            1 << 20,
        );
        assert!(review.changes.is_empty(), "{review:?}");
        assert_eq!(review.conflicts.len(), 2, "{review:?}");
        assert!(review.conflicts[0].contains("Blatt 3.pdf: ILIAS has a file of this name"));
        assert!(review.conflicts[1].contains("Skript (Kopie).pdf: a copy of"));

        // Two ILIAS folders called "Unterlagen" share the local folder.
        let mut other = place(&folder, Vec::new(), Vec::new());
        other.item.id = Id::Ref(3);
        let places = [place(&folder, Vec::new(), Vec::new()), other];
        let review = decide(
            &places,
            &HashSet::new(),
            Candidates::default(),
            &downloads,
            1 << 20,
        );
        assert!(review.changes.is_empty(), "{review:?}");
        assert!(
            review.conflicts[0].contains("several ILIAS folders"),
            "{review:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_and_reports_files_gone_on_ilias() {
        let (dir, mut downloads) = setup("gone");
        let folder = dir.join("Unterlagen");
        fs::write(folder.join("Alt.pdf"), "old").unwrap();
        downloads
            .record(12, folder.join("Alt.pdf"), "v1".into())
            .unwrap();
        let places = [place(&folder, Vec::new(), Vec::new())];
        let review = decide(
            &places,
            &HashSet::new(),
            Candidates::default(),
            &downloads,
            1 << 20,
        );
        assert_eq!(review.gone, [folder.join("Alt.pdf")]);
        assert!(review.changes.is_empty());
        assert!(folder.join("Alt.pdf").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn new_folders_are_ones_ilias_lists_nothing_for() {
        let (dir, downloads) = setup("folders");
        let folder = dir.join("Unterlagen");
        for name in ["Woche 1", "Blatt 0: Abgabe", "Eigenes", ".git"] {
            fs::create_dir_all(folder.join(name.replace(':', "_"))).unwrap();
        }
        fs::write(folder.join("Eigenes/Notizen.md"), "mine").unwrap();
        // ILIAS' folder "Entwurf." would be saved as "Entwurf": not sent.
        fs::create_dir_all(folder.join("Entwurf.")).unwrap();
        let listed = vec![
            item("fold", 5, "Woche 1"),
            item("exc", 6, "Blatt 0: Abgabe"),
        ];
        let mut places = vec![place(&folder, listed, Vec::new())];
        places.push(Place {
            item: item("fold", 5, "Unterlagen › Woche 1"),
            dir: folder.join("Woche 1"),
            ..Place::default()
        });
        let review = decide(
            &places,
            &HashSet::new(),
            Candidates::default(),
            &downloads,
            1 << 20,
        );
        assert_eq!(
            review.changes,
            [Change::NewFolder {
                place: places[0].item.clone(),
                dir: folder.join("Eigenes"),
                contents: vec![folder.join("Eigenes/Notizen.md")],
            }]
        );
        assert!(
            review.conflicts[0].contains("would come back as Entwurf"),
            "{review:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
