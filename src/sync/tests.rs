use super::*;
use pretty_assertions::assert_eq;

fn file(size: &str, date: &str) -> Item {
    Item {
        kind: "file".into(),
        id: Id::Ref(1),
        title: "Blatt 3".into(),
        properties: vec!["pdf".into(), size.into(), date.into()],
        ..Item::default()
    }
}

fn saved(version: &str) -> Saved {
    Saved {
        path: PathBuf::from("/dl/DEMO.101/Blatt 3.pdf"),
        version: version.into(),
        len: 1,
        modified_ms: 1,
        hash: String::new(),
    }
}

const V1: &str = "1.2 MB · 1. Nov 2026, 10:00";

const RULES: Rules = Rules {
    max_mb: 50.0,
    missing_only: false,
    two_way: false,
};

#[test]
fn missing_only_restores_deleted_files_and_leaves_the_rest() {
    let rules = Rules {
        missing_only: true,
        ..RULES
    };
    let same = file("1.2 MB", "1. Nov 2026, 10:00");
    let changed = file("1.3 MB", "2. Nov 2026, 09:00");
    let plan = |item: &Item, local| plan(item, Some((&saved(V1), local)), rules);
    // Deleted by you: back, changed on ILIAS or not.
    assert_eq!(plan(&same, Local::Missing), Plan::New);
    assert_eq!(plan(&changed, Local::Missing), Plan::New);
    // On disk: never replaced, no new version beside it.
    assert_eq!(plan(&changed, Local::Untouched), Plan::Unchanged);
    assert_eq!(plan(&changed, Local::Edited), Plan::Unchanged);
    assert_eq!(super::plan(&same, None, rules), Plan::New);
    let recording = file("107.9 MB", "19. Sep 2026, 23:33");
    assert_eq!(
        super::plan(&recording, Some((&saved(V1), Local::Missing)), rules),
        Plan::TooBig
    );
}

#[test]
fn reads_the_version_and_size_from_the_listing() {
    let item = file("1.2 MB", "1. Nov 2026, 10:00");
    assert_eq!(version(&item), V1);
    assert_eq!(size_mb("107.9 MB"), Some(107.9));
    assert_eq!(size_mb("512 KB"), Some(0.512));
    assert_eq!(size_mb("6,92 MB"), Some(6.92));
    assert_eq!(size_mb("pdf"), None);
}

#[test]
fn never_overwrites_your_edits() {
    let changed = file("1.3 MB", "2. Nov 2026, 09:00");
    let path = PathBuf::from("/dl/DEMO.101/Blatt 3.pdf");
    let plan = |item: &Item, local| plan(item, Some((&saved(V1), local)), RULES);
    assert_eq!(plan(&changed, Local::Edited), Plan::Beside(path.clone()));
    assert_eq!(plan(&changed, Local::Untouched), Plan::Update(path));
    let same = file("1.2 MB", "1. Nov 2026, 10:00");
    assert_eq!(plan(&same, Local::Edited), Plan::Unchanged);
    assert_eq!(plan(&same, Local::Untouched), Plan::Unchanged);
}

#[test]
fn respects_deletions_until_ilias_changes_the_file() {
    let same = file("1.2 MB", "1. Nov 2026, 10:00");
    let changed = file("1.3 MB", "2. Nov 2026, 09:00");
    assert_eq!(
        plan(&same, Some((&saved(V1), Local::Missing)), RULES),
        Plan::Deleted
    );
    assert_eq!(
        plan(&changed, Some((&saved(V1), Local::Missing)), RULES),
        Plan::New
    );
}

#[test]
fn skips_big_files_and_adopts_old_downloads() {
    let recording = file("107.9 MB", "19. Sep 2026, 23:33");
    assert_eq!(plan(&recording, None, RULES), Plan::TooBig);
    let generous = Rules {
        max_mb: 200.0,
        ..RULES
    };
    assert_eq!(plan(&recording, None, generous), Plan::New);
    // Saved before versions were recorded: no duplicate, no overwrite.
    let old = file("1.2 MB", "1. Nov 2026, 10:00");
    assert_eq!(
        plan(&old, Some((&saved(""), Local::Edited)), RULES),
        Plan::Adopt
    );
}

#[test]
fn names_the_new_version_beside_yours() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-sync-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mine = dir.join("Blatt 3.pdf");
    assert_eq!(
        beside_name(&mine, "2026-11-02"),
        dir.join("Blatt 3 (ILIAS 2026-11-02).pdf")
    );
    std::fs::write(dir.join("Blatt 3 (ILIAS 2026-11-02).pdf"), "x").unwrap();
    assert_eq!(
        beside_name(&mine, "2026-11-02"),
        dir.join("Blatt 3 (ILIAS 2026-11-02) (1).pdf")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notices_edits_and_deletions_on_disk() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Blatt 3.pdf");
    std::fs::write(&path, "original").unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    downloads.record(1, path.clone(), V1.into()).unwrap();
    let local = |d: &Downloads| d.entry(1).unwrap().local();
    assert_eq!(local(&downloads), Local::Untouched);

    // An annotation changes the file; recording only the new ILIAS version keeps it
    // "edited".
    std::fs::write(&path, "original + my notes").unwrap();
    assert_eq!(local(&downloads), Local::Edited);
    downloads.set_version(1, "newer".into()).unwrap();
    let reloaded = Downloads::in_file(dir.join("index.json"));
    assert_eq!(local(&reloaded), Local::Edited);
    assert_eq!(reloaded.entry(1).unwrap().version, "newer");

    std::fs::remove_file(&path).unwrap();
    assert_eq!(local(&reloaded), Local::Missing);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_old_index_files() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Skript.pdf");
    std::fs::write(&path, "x").unwrap();
    let index = dir.join("index.json");
    std::fs::write(&index, format!(r#"{{"9": {:?}}}"#, path)).unwrap();
    let downloads = Downloads::in_file(index);
    assert_eq!(downloads.get(9), Some(&path));
    // Unknown how it looked when saved: treated as edited, so never overwritten.
    assert_eq!(downloads.entry(9).unwrap().local(), Local::Edited);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keeps_every_page_it_reads_for_offline_use() {
    use crate::ilias::pages::{FakeIlias, reply};
    use std::sync::Arc;
    let folder = |id: u32, title: &str| {
        format!(
            r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/fold/{id}">{title}</a></div>"#
        )
    };
    let ilias = Arc::new(FakeIlias::new(move |url| {
        let html = match url.path() {
            "/ilias/goto.php/crs/1" => folder(2, "Unterlagen"),
            "/ilias/goto.php/fold/2" => folder(3, "Woche 1"),
            "/ilias/goto.php/fold/3" => String::new(),
            other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
        };
        reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
    }));
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base, ilias);
    let dir = std::env::temp_dir().join(format!("ilias-tui-synccache-{}", std::process::id()));
    let cache = Cache::in_dir(dir.join("cache"));
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO.101".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    sync(
        &client,
        &[course],
        &dir,
        &mut downloads,
        &cache,
        RULES,
        |_| {},
    )
    .unwrap();
    // The same keys the TUI uses when it opens these pages (offline too).
    for url in ["crs/1", "fold/2", "fold/3"] {
        let key = Cache::key(
            Some(&format!("https://ilias.example/ilias/goto.php/{url}")),
            false,
        );
        assert!(cache.get(&key).is_some(), "{url} cached");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn finds_whats_new_with_the_folders_on_the_way() {
    use crate::ilias::pages::{FakeIlias, reply};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let link = |kind: &str, id: u32| {
        format!(
            r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/{kind}/{id}">{kind} {id}</a>
            <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span><span class="il_ItemProperty">900 MB</span></div></div>"#
        )
    };
    // Course 1 › folder 2 › folder 3, whose file 10 is new in the second sync. (The
    // files are too big to download, so nothing goes to the network.)
    let second = Arc::new(AtomicBool::new(false));
    let later = second.clone();
    let ilias = Arc::new(FakeIlias::new(move |url| {
        let html = match url.path() {
            "/ilias/goto.php/crs/1" => link("fold", 2),
            "/ilias/goto.php/fold/2" => link("fold", 3),
            "/ilias/goto.php/fold/3" if later.load(Ordering::SeqCst) => {
                [link("file", 9), link("file", 10)].concat()
            }
            "/ilias/goto.php/fold/3" => link("file", 9),
            other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
        };
        reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
    }));
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base, ilias);
    let dir = std::env::temp_dir().join(format!("ilias-tui-new-{}", std::process::id()));
    let cache = Cache::in_dir(dir.join("cache"));
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    let mut run = || {
        sync(
            &client,
            std::slice::from_ref(&course),
            &dir,
            &mut downloads,
            &cache,
            RULES,
            |_| {},
        )
        .unwrap()
        .unseen
    };
    assert!(run().is_empty(), "nothing to compare with yet");
    second.store(true, Ordering::SeqCst);
    assert_eq!(
        run(),
        [FoundNew {
            root: 1,
            path: vec![2],
            container: 3,
            new: crate::seen::New(std::collections::BTreeMap::from([(
                crate::seen::Group::Files,
                vec![10]
            )])),
        }]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_only_leaves_files_on_disk_alone_even_when_ilias_changed_them() {
    use crate::ilias::pages::{FakeIlias, reply};
    use std::sync::Arc;
    // The course lists one file, now 1.3 MB; the saved copy is of the 1.2 MB version.
    let ilias = Arc::new(FakeIlias::new(|url| match url.path() {
        "/ilias/goto.php/crs/1" => reply(
            url,
            r#"<div id="il_center_col"><div class="ilContainerListItemOuter">
            <a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Blatt 3</a>
            <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span>
            <span class="il_ItemProperty">1.3 MB</span></div></div></div>"#,
        ),
        other => Err(color_eyre::eyre::eyre!("unexpected {other}")),
    }));
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base, ilias);
    let dir = std::env::temp_dir().join(format!("ilias-tui-missing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Blatt 3.pdf");
    std::fs::write(&path, "my copy").unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    downloads.record(9, path.clone(), "1.2 MB".into()).unwrap();
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO.101".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    let rules = Rules {
        missing_only: true,
        ..RULES
    };
    let cache = Cache::in_dir(dir.join("cache"));
    let summary = sync(
        &client,
        &[course],
        &dir,
        &mut downloads,
        &cache,
        rules,
        |_| {},
    )
    .unwrap();
    // Nothing fetched (that would have failed: there is no network here), nothing touched.
    assert_eq!((summary.unchanged, summary.new, summary.updated), (1, 0, 0));
    assert!(summary.failed.is_empty() && summary.beside.is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my copy");
    assert_eq!(downloads.entry(9).unwrap().local(), Local::Untouched);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_listed_in_several_places_is_synced_once() {
    use crate::ilias::pages::{FakeIlias, reply};
    use std::sync::Arc;
    // Course 1 holds group 2 (also listed on its own in My courses), which holds file 9.
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let ilias = Arc::new(FakeIlias::new(move |url| {
        counter.fetch_add(1, Ordering::SeqCst);
        let html = match url.path() {
            "/ilias/goto.php/crs/1" => {
                r#"<div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="https://ilias.example/ilias/goto.php/grp/2">Gruppe</a></div>"#
            }
            "/ilias/goto.php/grp/2" => {
                r#"<div class="ilContainerListItemOuter">
                <a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Blatt 3</a>
                <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span>
                <span class="il_ItemProperty">1.2 MB</span></div></div>"#
            }
            other => return Err(color_eyre::eyre::eyre!("unexpected {other}")),
        };
        reply(url, &format!(r#"<div id="il_center_col">{html}</div>"#))
    }));
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base, ilias);
    let dir = std::env::temp_dir().join(format!("ilias-tui-twice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Saved before and current: nothing to download (there is no network here).
    let path = dir.join("Blatt 3.pdf");
    std::fs::write(&path, "x").unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    downloads.record(9, path, "1.2 MB".into()).unwrap();
    let container = |kind: &str, id: u64, title: &str| Item {
        kind: kind.into(),
        id: Id::Ref(id),
        title: title.into(),
        url: format!("https://ilias.example/ilias/goto.php/{kind}/{id}"),
        ..Item::default()
    };
    let (course, group) = (container("crs", 1, "Kurs"), container("grp", 2, "Gruppe"));
    // As "sync all" got them: the course twice, and the group inside it on its own.
    let roots = [course.clone(), group, course];
    let cache = Cache::in_dir(dir.join("cache"));
    let summary = sync(&client, &roots, &dir, &mut downloads, &cache, RULES, |_| {}).unwrap();
    assert_eq!(summary.unchanged, 1, "{summary:?}");
    assert!(summary.failed.is_empty(), "{summary:?}");
    // Course and group page, each read once.
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn says_what_sync_will_do() {
    assert_eq!(RULES.describe(), "Saves every file up to 50 MB.");
    let missing_only = Rules {
        missing_only: true,
        ..RULES
    };
    assert!(missing_only.describe().contains("also ones you deleted"));
}

#[test]
fn reports_what_deserves_a_look() {
    let summary = Summary {
        new: 3,
        beside: vec![PathBuf::from("/dl/DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf")],
        too_big: vec!["DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".into()],
        ..Summary::default()
    };
    let report = summary.report(Path::new("/dl"), 50.0);
    assert_eq!(
        report,
        [
            (
                "Changed on ILIAS, kept your edited copy; new version saved as".to_string(),
                vec!["DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf".to_string()]
            ),
            (
                "Skipped: bigger than 50 MB".to_string(),
                vec!["DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".to_string()]
            ),
        ]
    );
    assert!(Summary::default().report(Path::new("/dl"), 50.0).is_empty());
    assert_eq!(
        summary.text(50.0),
        "Synced: 3 new, 0 updated, 1 edited by you (new version saved beside), 0 unchanged, 1 over 50 MB skipped"
    );
}

const TWO_WAY: Rules = Rules {
    two_way: true,
    ..RULES
};

fn hashed(version: &str) -> Saved {
    Saved {
        hash: "ab12".into(),
        ..saved(version)
    }
}

#[test]
fn two_way_decides_every_row_from_base_disk_and_ilias() {
    let same = file("1.2 MB", "1. Nov 2026, 10:00");
    let changed = file("1.3 MB", "2. Nov 2026, 09:00");
    let path = PathBuf::from("/dl/DEMO.101/Blatt 3.pdf");
    let plan = |item: &Item, local| plan(item, Some((&hashed(V1), local)), TWO_WAY);
    // Your disk unchanged, ILIAS changed: download, as one-way.
    assert_eq!(plan(&changed, Local::Untouched), Plan::Update(path.clone()));
    // Edited, ILIAS unchanged: a new version for ILIAS.
    assert_eq!(plan(&same, Local::Edited), Plan::NewVersion(path.clone()));
    // Edited, ILIAS changed: a conflict; ILIAS' copy goes beside yours.
    assert_eq!(plan(&changed, Local::Edited), Plan::Beside(path.clone()));
    // Deleted, ILIAS unchanged: propose the trash.
    assert_eq!(plan(&same, Local::Missing), Plan::Trash(path.clone()));
    // Deleted, ILIAS changed: download again.
    assert_eq!(plan(&changed, Local::Missing), Plan::New);
    assert_eq!(plan(&same, Local::Untouched), Plan::Unchanged);
    // New on ILIAS: download.
    assert_eq!(super::plan(&same, None, TWO_WAY), Plan::New);
    // Saved before hashes were recorded: an edit can't be told from a touched file.
    assert_eq!(
        super::plan(&same, Some((&saved(V1), Local::Edited)), TWO_WAY),
        Plan::Unsure(path)
    );
    // One-way never plans writes.
    assert_eq!(
        super::plan(&same, Some((&hashed(V1), Local::Edited)), RULES),
        Plan::Unchanged
    );
    assert_eq!(
        super::plan(&same, Some((&hashed(V1), Local::Missing)), RULES),
        Plan::Deleted
    );
}

#[test]
fn a_touched_file_with_the_same_content_is_not_edited() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-hash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Blatt 3.pdf");
    std::fs::write(&path, "original").unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    downloads.record(1, path.clone(), V1.into()).unwrap();
    assert!(downloads.entry(1).unwrap().has_hash());
    // Copied over by another tool: a new modification time, the same content.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_eq!(downloads.entry(1).unwrap().local(), Local::Untouched);
    std::fs::write(&path, "0riginal").unwrap();
    assert_eq!(downloads.entry(1).unwrap().local(), Local::Edited);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_old_entry_gets_its_hash_while_the_file_is_as_saved() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-addhash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Skript.pdf");
    std::fs::write(&path, "x").unwrap();
    // As an older version recorded it: size and modification time, no hash.
    let meta = std::fs::metadata(&path).unwrap();
    let modified = meta.modified().unwrap();
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let entry = serde_json::json!({ "1": {
        "path": path, "version": V1, "len": meta.len(), "modified_ms": ms as u64
    }});
    std::fs::write(dir.join("index.json"), entry.to_string()).unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    assert!(!downloads.entry(1).unwrap().has_hash());
    downloads.add_hash(1).unwrap();
    assert!(downloads.entry(1).unwrap().has_hash());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn two_way_reports_what_it_would_send_and_sends_nothing() {
    use crate::ilias::pages::{FakeIlias, reply};
    use std::sync::Arc;
    // Course 1 lists file 9; the page has a breadcrumb (its place) and an upload form.
    let ilias = Arc::new(FakeIlias::new(|url| match url.path() {
        "/ilias/goto.php/crs/1" => reply(
            url,
            r#"<ol class="breadcrumb"><li><a href="goto.php/crs/1">DEMO.101</a></li></ol>
            <div id="il_center_col"><div class="ilContainerListItemOuter">
            <a class="il_ContainerItemTitle" href="https://ilias.example/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Blatt 3</a>
            <div class="il_ItemProperties"><span class="il_ItemProperty">pdf</span>
            <span class="il_ItemProperty">1.2 MB</span></div></div></div>
            <form enctype="multipart/form-data" method="post" action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjFileGUI&amp;cmd=post&amp;fallbackCmd=uploadFiles&amp;ref_id=1&amp;new_type=file"></form>"#,
        ),
        other => Err(color_eyre::eyre::eyre!("unexpected {other}")),
    }));
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base.clone(), ilias.clone());
    let dir = std::env::temp_dir().join(format!("ilias-tui-twoway-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let course_dir = dir.join("DEMO.101");
    std::fs::create_dir_all(&course_dir).unwrap();
    let saved_copy = course_dir.join("Blatt 3.pdf");
    std::fs::write(&saved_copy, "from ILIAS").unwrap();
    std::fs::write(course_dir.join("Notizen.md"), "mine").unwrap();
    std::fs::write(course_dir.join(".DS_Store"), "x").unwrap();
    let mut downloads = Downloads::in_file(dir.join("index.json"));
    downloads
        .record(9, saved_copy.clone(), "1.2 MB".into())
        .unwrap();
    std::fs::write(&saved_copy, "from ILIAS, with my notes").unwrap();
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO.101".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    let cache = Cache::in_dir(dir.join("cache"));
    let summary = sync(
        &client,
        &[course],
        &dir,
        &mut downloads,
        &cache,
        TWO_WAY,
        |_| {},
    )
    .unwrap();
    assert!(ilias.posts(&base).is_empty());
    let report = summary.report(&dir, 50.0);
    let section = |title: &str| {
        report
            .iter()
            .find(|(t, _)| t.starts_with(title))
            .map(|(_, lines)| lines.clone())
            .unwrap_or_default()
    };
    assert_eq!(section("Would upload: new"), ["DEMO.101/Notizen.md"]);
    assert_eq!(
        section("Would upload as a new version"),
        ["DEMO.101/Blatt 3.pdf"]
    );
    assert!(
        summary.text(50.0).contains("2 to send to ILIAS"),
        "{}",
        summary.text(50.0)
    );
    let _ = std::fs::remove_dir_all(dir);
}
