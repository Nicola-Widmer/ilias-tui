use super::*;
use crate::sync::journal::Journal;
use crate::sync::model::Base;
use crate::sync::ops::{Reason, Why};
use crate::sync::report::Section;
use crate::sync::run::Done;
use pretty_assertions::assert_eq;
use std::sync::atomic::{AtomicUsize, Ordering};

fn file(size: &str, date: &str) -> Item {
    Item {
        kind: "file".into(),
        id: Id::Ref(1),
        title: "Blatt 3".into(),
        properties: vec!["pdf".into(), size.into(), date.into()],
        ..Item::default()
    }
}

const V1: &str = "1.2 MB · 2026-11-01T10:00:00";

const RULES: Rules = Rules {
    max_mb: 50.0,
    missing_only: false,
    two_way: false,
};

const TWO_WAY: Rules = Rules {
    two_way: true,
    ..RULES
};

/// The base and journal of a sync into `dir`.
struct World {
    base: Base,
    journal: Journal,
}

impl World {
    fn new(dir: &Path) -> Self {
        Self {
            base: Base::new(dir),
            journal: Journal::in_file(dir.join("journal.jsonl")),
        }
    }

    fn sync(&mut self, client: &Client, roots: &[Item], cache: &Cache, rules: Rules) -> Summary {
        sync(
            client,
            roots,
            &mut self.base,
            &mut self.journal,
            cache,
            rules,
            |_| {},
        )
        .unwrap()
    }

    /// Records `path` as saved from ILIAS' file `id` at `version`, as it is now.
    fn saved(&mut self, path: &Path, id: u64, version: &str) {
        self.base.record(path, id, version.into()).unwrap();
    }
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
    let mut world = World::new(&dir);
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO.101".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    world.sync(&client, &[course], &cache, RULES);
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
    let mut world = World::new(&dir);
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    let mut run = || {
        world
            .sync(&client, std::slice::from_ref(&course), &cache, RULES)
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
    let mut world = World::new(&dir);
    world.saved(&path, 9, "1.2 MB");
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
    let summary = world.sync(&client, &[course], &cache, rules);
    // Nothing fetched (that would have failed: there is no network here), nothing touched.
    let counts = summary.counts();
    assert_eq!((counts.unchanged, counts.new, counts.updated), (1, 0, 0));
    assert_eq!((counts.failed, counts.beside), (0, 0));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my copy");
    assert_eq!(world.base.file("Blatt 3.pdf").unwrap().version, "1.2 MB");
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
    let mut world = World::new(&dir);
    world.saved(&path, 9, "1.2 MB");
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
    let summary = world.sync(&client, &roots, &cache, RULES);
    assert_eq!(summary.counts().unchanged, 1, "{summary:?}");
    assert_eq!(summary.counts().failed, 0, "{summary:?}");
    // Course and group page, each read once.
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn two_way_sync_plans_what_to_send_and_sends_nothing_by_itself() {
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
            <span class="il_ItemProperty">1.2 MB</span></div>
            <ul><li><button class="btn btn-link" data-action="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilFileVersionsGUI&amp;ref_id=9">Versions</button></li></ul></div></div>
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
    let mut world = World::new(&dir);
    world.saved(&saved_copy, 9, "1.2 MB");
    std::fs::write(&saved_copy, "from ILIAS, with my notes").unwrap();
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO.101".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Item::default()
    };
    let cache = Cache::in_dir(dir.join("cache"));
    let summary = world.sync(&client, &[course], &cache, TWO_WAY);
    assert!(ilias.posts(&base).is_empty());
    let report = summary.report(50.0);
    let section = |section: Section| {
        report
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, lines)| lines.clone())
            .unwrap_or_default()
    };
    assert_eq!(section(Section::Upload), ["DEMO.101/Notizen.md"]);
    assert_eq!(section(Section::Replace), ["DEMO.101/Blatt 3.pdf"]);
    // The course's folder is a saved folder now.
    assert!(world.base.has_folder("DEMO.101"));
    assert!(
        summary.text(50.0).contains("2 to send to ILIAS"),
        "{}",
        summary.text(50.0)
    );
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
    let blatt = file("1.3 MB", "2. Nov 2026, 09:00");
    let summary = Summary {
        ops: vec![
            Op::Keep {
                path: "DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".into(),
                reason: Reason::TooBig,
            },
            Op::Conflict {
                path: "DEMO.101/Blatt 3.pdf".into(),
                why: Why::Both {
                    file: blatt.clone(),
                    version: "v2".into(),
                },
            },
        ],
        done: vec![(
            Op::Conflict {
                path: "DEMO.101/Blatt 3.pdf".into(),
                why: Why::Both {
                    file: blatt,
                    version: "v2".into(),
                },
            },
            Done::Ok {
                saved: Some("DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf".into()),
            },
        )],
        ..Summary::default()
    };
    assert_eq!(
        summary.report(50.0),
        [
            (
                Section::Beside,
                vec!["DEMO.101/Blatt 3 (ILIAS 2026-11-02).pdf".to_string()]
            ),
            (
                Section::TooBig { max_mb: 50.0 },
                vec!["DEMO.101 › Aufzeichnungen › Lecture 1 (107.9 MB)".to_string()]
            ),
        ]
    );
    assert!(Summary::default().report(50.0).is_empty());
    assert_eq!(
        summary.text(50.0),
        "Synced: 0 new, 0 updated, 1 edited by you (new version saved beside), 0 unchanged, 1 over 50 MB skipped"
    );
}

fn trash(kind: Kind, files: usize) -> Op {
    Op::TrashRemote {
        place: Item::default(),
        item: Item {
            kind,
            ..Item::default()
        },
        path: "KW1".into(),
        base: String::new(),
        files,
    }
}

#[test]
fn says_what_becomes_of_the_files_you_deleted() {
    let kept = Op::Keep {
        path: "b.pdf".into(),
        reason: Reason::Deleted,
    };
    let proposal = trash(Kind::File, 1);
    let two_way = Summary {
        ops: vec![proposal.clone(), kept],
        two_way: true,
        review: Review::new(&[proposal], false),
        ..Summary::default()
    };
    let text = two_way.text(50.0);
    assert!(
        text.contains("1 deleted by you (to the ILIAS trash)"),
        "{text}"
    );
    assert!(
        text.contains("1 deleted by you (ILIAS doesn't let you delete them there)"),
        "{text}"
    );
    assert!(
        text.contains("1 to send to ILIAS (see the report)"),
        "{text}"
    );
    let one_way = Summary {
        ops: vec![
            Op::Keep {
                path: "a.pdf".into(),
                reason: Reason::Deleted,
            };
            2
        ],
        ..Summary::default()
    };
    assert!(
        one_way
            .text(50.0)
            .contains("2 deleted by you (not downloaded again)")
    );
}

#[test]
fn files_in_a_folder_for_the_trash_count_as_going_there() {
    let summary = Summary {
        ops: vec![trash(Kind::Folder, 2)],
        two_way: true,
        ..Summary::default()
    };
    let text = summary.text(50.0);
    assert!(
        text.contains("2 deleted by you (to the ILIAS trash)"),
        "{text}"
    );
    assert!(!text.contains("doesn't let you"), "{text}");
    let report = Summary {
        review: Review::new(&[trash(Kind::Folder, 2)], false),
        ..Summary::default()
    }
    .report(50.0);
    assert_eq!(
        report,
        [(
            Section::Proposals,
            vec!["KW1/ (the folder, with what is in it)".to_string()]
        )]
    );
}

#[test]
fn the_status_line_names_what_was_synced() {
    let one = Summary {
        places: "Test".into(),
        ..Summary::default()
    };
    assert!(
        one.text(50.0).starts_with("Synced Test: 0 new"),
        "{}",
        one.text(50.0)
    );
    assert!(Summary::default().text(50.0).starts_with("Synced: "));
}

#[test]
fn a_dry_run_reviews_nothing_and_the_rest_is_kept_for_the_review() {
    let upload = Op::Upload {
        place: Item::default(),
        path: "a.md".into(),
        from: None,
    };
    let ops = [
        upload.clone(),
        trash(Kind::File, 1),
        Op::TrashLocal {
            path: "b.pdf".into(),
        },
        Op::Keep {
            path: "c.pdf".into(),
            reason: Reason::CantTell,
        },
    ];
    let review = Review::new(&ops, true);
    assert_eq!(review.send, [upload]);
    assert_eq!(review.proposals, [trash(Kind::File, 1)]);
    assert_eq!(review.trash_local, ["b.pdf"]);
    assert!(review.dry_run);
}
