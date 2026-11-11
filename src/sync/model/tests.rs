use std::fs;

use super::*;
use pretty_assertions::assert_eq;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ilias-tui-model-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn relative_paths_use_slashes_and_stay_in_the_download_folder() {
    let root = Path::new("/dl");
    assert_eq!(
        rel(root, Path::new("/dl/DEMO/Unterlagen/a.pdf")).as_deref(),
        Some("DEMO/Unterlagen/a.pdf")
    );
    assert_eq!(rel(root, Path::new("/elsewhere/a.pdf")), None);
    assert_eq!(rel(root, Path::new("/dl")), None);
    assert_eq!(parent("DEMO/Unterlagen/a.pdf"), "DEMO/Unterlagen");
    assert_eq!(parent("a.pdf"), "");
    assert_eq!(name("DEMO/Unterlagen/a.pdf"), "a.pdf");
    assert!(within("DEMO/Unterlagen/a.pdf", "DEMO/Unterlagen"));
    assert!(within("DEMO/Unterlagen", "DEMO/Unterlagen"));
    assert!(!within("DEMO/Unterlagen 2/a.pdf", "DEMO/Unterlagen"));
    assert_eq!(
        rel_dir(&["DEMO: 101".into(), "Woche 1".into()]),
        "DEMO_ 101/Woche 1"
    );
}

/// An old saved-files list in `file`, as older versions wrote it.
fn old_list(file: &Path, entries: serde_json::Value) {
    fs::write(file, entries.to_string()).unwrap();
}

#[test]
fn takes_over_the_old_list_once_keeping_out_copies_elsewhere() {
    let dir = temp("migrate");
    let root = dir.join("ILIAS");
    let sheet = root.join("DEMO/Unterlagen/Blatt 3.pdf");
    let old = dir.join("downloads.json");
    old_list(
        &old,
        serde_json::json!({
            "9": { "path": sheet, "version": "1.2 MB · 2026-11-01T10:00:00", "len": 5,
                   "modified_ms": 1700000000000u64, "hash": "ab12" },
            // Saved before the download folder changed: not part of the base.
            "10": { "path": dir.join("Old/Skript.pdf"), "version": "2 MB" },
            // The first versions stored only the path.
            "11": root.join("DEMO/alt.pdf"),
        }),
    );
    let file = dir.join("base.json");
    let base = Base::in_file(file.clone(), &root, &old);
    let ids: Vec<_> = base.files().map(|(p, e)| (p, e.id)).collect();
    assert_eq!(
        ids,
        [("DEMO/Unterlagen/Blatt 3.pdf", 9), ("DEMO/alt.pdf", 11)]
    );
    assert_eq!(
        base.file("DEMO/Unterlagen/Blatt 3.pdf"),
        Some(&FileEntry {
            id: 9,
            version: "1.2 MB · 2026-11-01T10:00:00".into(),
            len: 5,
            modified_ms: 1700000000000,
            hash: "ab12".into(),
        })
    );
    // The folders they are in, ids unknown until a sync reads them.
    assert_eq!(
        base.folders().map(|(p, _)| p).collect::<Vec<_>>(),
        ["DEMO", "DEMO/Unterlagen"]
    );
    assert_eq!(base.abs("DEMO/Unterlagen/Blatt 3.pdf"), sheet);

    // Written: the next load reads it, not the old list.
    old_list(&old, serde_json::json!({}));
    assert_eq!(Base::in_file(file.clone(), &root, &old), base);
    // Another download folder has a base of its own; the first one is kept for later.
    let mut other = Base::in_file(file.clone(), &dir.join("Old"), &old);
    other.change(|e| e.record_folder("x", None)).unwrap();
    assert_eq!(other.files().count(), 0);
    assert_eq!(Base::in_file(file, &root, &old), base);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_old_relative_date_is_read_on_the_day_the_copy_was_saved() {
    let now = jiff::Timestamp::now().as_millisecond() as u64;
    let today = jiff::Zoned::now().date();
    assert_eq!(
        saved_version("1 KB · Today, 10:00", now),
        format!("1 KB · {today}T10:00:00")
    );
    // Saved yesterday while ILIAS said "Today": the same as "Yesterday" today.
    let yesterday = now - 24 * 3600 * 1000;
    assert_eq!(
        saved_version("15 B · Today, 12:41", yesterday),
        super::super::absolute("15 B · Yesterday, 12:41", today)
    );
    // Absolute dates stay as they are.
    assert_eq!(
        saved_version("1.2 MB · 1. Nov 2026, 10:00", 0),
        "1.2 MB · 2026-11-01T10:00:00"
    );
}

#[test]
fn round_trip_and_one_path_per_ilias_file() {
    let dir = temp("roundtrip");
    let file = dir.join("base.json");
    let empty = dir.join("none.json");
    let mut base = Base::in_file(file.clone(), &dir, &empty);
    fs::write(dir.join("a.pdf"), "a").unwrap();
    base.record(&dir.join("a.pdf"), 7, "v1".into()).unwrap();
    base.change(|e| e.record_folder("Woche 1", Some(5)))
        .unwrap();
    assert_eq!(base.find(7).map(|(p, _)| p), Some("a.pdf"));
    assert!(!base.file("a.pdf").unwrap().hash.is_empty());

    // Renamed: recorded under the new path only.
    fs::rename(dir.join("a.pdf"), dir.join("b.pdf")).unwrap();
    base.record(&dir.join("b.pdf"), 7, "v1".into()).unwrap();
    assert_eq!(base.files().map(|(p, _)| p).collect::<Vec<_>>(), ["b.pdf"]);
    // Outside the download folder: not recorded.
    base.record(Path::new("/elsewhere/c.pdf"), 8, "v1".into())
        .unwrap();
    assert!(base.find(8).is_none());

    let loaded = Base::in_file(file, &dir, &empty);
    assert_eq!(loaded, base);
    assert_eq!(
        loaded.folders().next(),
        Some(("Woche 1", &FolderEntry { id: Some(5) }))
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_stale_copy_does_not_undo_newer_entries_and_folders_go_with_what_is_in_them() {
    let dir = temp("stale");
    let file = dir.join("base.json");
    let empty = dir.join("none.json");
    let mut tui = Base::in_file(file.clone(), &dir, &empty);
    let mut sync = Base::in_file(file.clone(), &dir, &empty);
    sync.change(|e| {
        e.record_file(
            "W/a.pdf",
            FileEntry {
                id: 1,
                ..FileEntry::default()
            },
        );
        e.record_file(
            "W/x/b.pdf",
            FileEntry {
                id: 2,
                ..FileEntry::default()
            },
        );
        e.record_folder("W", Some(5));
        e.record_folder("W/x", None);
        e.record_folder("W2", None);
    })
    .unwrap();
    // The TUI, not knowing these, records another file.
    tui.change(|e| {
        e.record_file(
            "c.pdf",
            FileEntry {
                id: 3,
                ..FileEntry::default()
            },
        )
    })
    .unwrap();
    assert_eq!(tui.files().count(), 3);
    tui.change(|e| e.forget_below("W")).unwrap();
    let on_disk = Base::in_file(file, &dir, &empty);
    assert_eq!(
        on_disk.files().map(|(p, _)| p).collect::<Vec<_>>(),
        ["c.pdf"]
    );
    assert_eq!(
        on_disk.folders().map(|(p, _)| p).collect::<Vec<_>>(),
        ["W2"]
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reads_the_disk_hashing_only_what_the_diff_needs() {
    let dir = temp("local");
    fs::create_dir_all(dir.join("DEMO/Eigenes/tief")).unwrap();
    fs::write(dir.join("DEMO/saved.pdf"), "saved").unwrap();
    fs::write(dir.join("DEMO/new.md"), "new").unwrap();
    fs::write(dir.join("DEMO/.DS_Store"), "x").unwrap();
    fs::write(dir.join("DEMO/Eigenes/tief/c.md"), "c").unwrap();
    let mut base = Base::new(&dir);
    base.record(&dir.join("DEMO/saved.pdf"), 1, "v".into())
        .unwrap();
    // Saved in a folder that is no place any more.
    base.change(|e| {
        e.record_file(
            "Alt/old.pdf",
            FileEntry {
                id: 2,
                len: 3,
                ..FileEntry::default()
            },
        )
    })
    .unwrap();
    let remote = Remote {
        places: vec![Place {
            dir: "DEMO".into(),
            ..Place::default()
        }],
        files: Vec::new(),
    };
    let local = Local::read(&base, &remote, true);
    assert_eq!(
        local.folders.iter().map(String::as_str).collect::<Vec<_>>(),
        ["DEMO", "DEMO/Eigenes", "DEMO/Eigenes/tief"]
    );
    let files: Vec<(&str, bool)> = local
        .files
        .iter()
        .map(|(p, f)| (p.as_str(), f.hash.is_some()))
        .collect();
    // Untouched (same size and time): no hash needed; new: hashed; in a new folder: not.
    assert_eq!(
        files,
        [
            ("DEMO/Eigenes/tief/c.md", false),
            ("DEMO/new.md", true),
            ("DEMO/saved.pdf", false),
        ]
    );
    // Touched, same size: hashed to tell.
    let now = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(dir.join("DEMO/saved.pdf"))
        .unwrap()
        .set_modified(now)
        .unwrap();
    let local = Local::read(&base, &remote, false);
    assert!(local.files["DEMO/saved.pdf"].hash.is_some());
    assert!(local.files["DEMO/new.md"].hash.is_none());
    let _ = fs::remove_dir_all(dir);
}
