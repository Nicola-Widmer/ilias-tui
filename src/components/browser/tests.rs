use std::collections::{HashMap, HashSet};

use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::ilias::{Block, Item, Page, Todo, Urgency};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn item(kind: &str, ref_id: u64, title: &str) -> Item {
    let url = format!("https://ilias.example/goto.php/{kind}/{ref_id}");
    Item {
        kind: kind.into(),
        id: Id::Ref(ref_id),
        title: title.into(),
        url,
        ..Default::default()
    }
}

fn load(browser: &mut Browser, item: Option<Item>, page: Page, deep: bool) {
    browser.update(&Action::Load {
        item: item.clone(),
        url: None,
        refresh: false,
        deep,
    });
    browser.update(&Action::Loaded {
        item,
        page,
        refresh: false,
        deep,
    });
}

#[test]
fn marks_what_is_new_since_the_last_visit() {
    let mut browser = Browser::new(HashSet::new(), &Default::default());
    let courses = vec![item("crs", 1, "Algebra"), item("crs", 2, "Biologie")];
    load(&mut browser, None, Page::from_items(courses), false);
    browser.update(&Action::NewCounts(HashMap::from([(
        1,
        crate::seen::Counts(std::collections::BTreeMap::from([
            (crate::seen::Group::Files, 2),
            (crate::seen::Group::Folders, 1),
            (crate::seen::Group::Posts, 2),
        ])),
    )])));
    let shown = screen(&mut browser);
    assert!(
        shown.contains("Algebra  ● 3 new  ● new posts in 2 forums"),
        "{shown}"
    );
    assert!(!shown.contains("Biologie  ●"), "{shown}");

    let files = vec![item("file", 10, "Folien"), item("file", 11, "Skript")];
    load(
        &mut browser,
        Some(item("crs", 1, "Algebra")),
        Page::from_items(files),
        false,
    );
    browser.update(&Action::NewItems(vec![11]));
    let shown = screen(&mut browser);
    assert!(shown.contains("Skript  ● new"), "{shown}");
    assert!(!shown.contains("Folien  ●"), "{shown}");
}

#[test]
fn hidden_kinds_of_news_are_neither_marked_nor_counted() {
    use crate::seen::{Counts, Group};
    let config = crate::config::Config {
        hide_new: [Group::Posts].into(),
        ..Default::default()
    };
    let mut browser = Browser::new(HashSet::new(), &config);
    let page = Page::from_items(vec![item("frm", 1, "Forum"), item("file", 2, "Skript")]);
    load(&mut browser, Some(item("crs", 9, "DEMO")), page, false);
    browser.update(&Action::NewItems(vec![1, 2]));
    let shown = screen(&mut browser);
    assert!(shown.contains("Skript  ● new"), "{shown}");
    assert!(!shown.contains("new posts"), "{shown}");

    // Switched on again in the settings: it shows at once.
    browser.update(&Action::SetConfig(crate::config::Config::default()));
    browser.update(&Action::NewCounts(HashMap::from([(
        2,
        Counts(std::collections::BTreeMap::from([(Group::Posts, 1)])),
    )])));
    let shown = screen(&mut browser);
    assert!(shown.contains("Forum  ● new posts"), "{shown}");
}

#[test]
fn the_top_bar_shows_the_version() {
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![item("crs", 1, "DEMO")]),
        false,
    );
    let shown = screen(&mut browser);
    assert!(shown.contains(&format!("v{}", crate::VERSION)), "{shown}");
}

#[test]
fn a_asks_before_syncing_all_courses_from_anywhere() {
    let mut browser = Browser::new(HashSet::new(), &Default::default());
    assert!(matches!(
        browser.handle_key_event(KeyCode::Char('A').into()),
        Some(Action::Error(_))
    ));

    let (a, b) = (item("crs", 1, "Algebra"), item("crs", 2, "Biologie"));
    // Algebra twice, as under Favourites and its semester.
    let courses = vec![a.clone(), item("webr", 3, "Link"), b, a.clone()];
    load(&mut browser, None, Page::from_items(courses), false);
    load(
        &mut browser,
        Some(a),
        Page::from_items(vec![item("fold", 4, "Woche 1")]),
        false,
    );

    let Some(Action::Ask { question, then }) = browser.handle_key_event(KeyCode::Char('A').into())
    else {
        panic!("expected a question");
    };
    assert!(question.starts_with("Sync all 2 courses"), "{question}");
    assert!(!question.contains("missing"), "{question}");
    let Action::Sync(roots) = *then else {
        panic!("expected a sync");
    };
    let ids: Vec<u64> = roots.iter().map(|r| r.key()).collect();
    assert_eq!(ids, [1, 2]);

    // The question follows the settings.
    browser.update(&Action::SetConfig(crate::config::Config {
        missing_only: true,
        ..Default::default()
    }));
    let Some(Action::Ask { question, .. }) = browser.handle_key_event(KeyCode::Char('A').into())
    else {
        panic!("expected a question");
    };
    assert!(question.contains("every missing file"), "{question}");
}

fn screen(browser: &mut Browser) -> String {
    let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
    terminal.draw(|f| browser.draw(f, f.area())).unwrap();
    format!("{:?}", terminal.backend().buffer())
}

#[test]
fn links_in_page_text_can_be_selected_and_opened() {
    let mut browser = Browser::default();
    let zoom = Item {
        description: "https://zoom.us/j/1".into(),
        url: "https://zoom.us/j/1".into(),
        ..item("link", 99, "Zoom-Meeting")
    };
    let page = Page {
        blocks: vec![
            Block::Text("Online: Zoom-Meeting".into()),
            Block::Items {
                title: String::new(),
                items: vec![zoom.clone()],
            },
            Block::Items {
                title: "Inhalt".into(),
                items: vec![item("file", 2, "Skript")],
            },
        ],
        ..Page::default()
    };
    load(&mut browser, Some(item("crs", 1, "DEMO")), page, false);
    let shown = screen(&mut browser);
    assert!(shown.contains("Zoom-Meeting"), "{shown}");
    assert!(shown.contains("https://zoom.us/j/1"), "{shown}");

    // The link is the first entry; j moves on to the file and k back.
    let opens = |browser: &mut Browser, key: KeyCode| match browser.handle_key_event(key.into()) {
        Some(Action::OpenInBrowser(item)) => item.url,
        other => panic!("expected the browser, got {other:?}"),
    };
    assert_eq!(opens(&mut browser, KeyCode::Enter), "https://zoom.us/j/1");
    browser.handle_key_event(KeyCode::Char('j').into());
    assert!(matches!(
        browser.handle_key_event(KeyCode::Enter.into()),
        Some(Action::Download { .. })
    ));
    browser.handle_key_event(KeyCode::Char('k').into());
    assert!(matches!(
        browser.handle_key_event(KeyCode::Char('y').into()),
        Some(Action::CopyLink(item)) if item.key() == 99
    ));
    assert_eq!(
        opens(&mut browser, KeyCode::Char('o')),
        "https://zoom.us/j/1"
    );
}

#[test]
fn going_back_keeps_the_filter_of_that_list() {
    let mut browser = Browser::default();
    let mut filter = super::super::Filter::default();
    let courses = vec![
        item("crs", 1, "Algebra"),
        item("crs", 2, "Analysis"),
        item("crs", 3, "Biologie"),
    ];
    load(&mut browser, None, Page::from_items(courses), false);
    // `/an` finds Analysis.
    for key in [KeyCode::Char('/'), KeyCode::Char('a'), KeyCode::Char('n')] {
        if let Some(action) = filter.handle_key_event(key.into()) {
            browser.update(&action);
        }
    }
    filter.handle_key_event(KeyCode::Enter.into());
    assert_eq!(browser.current().unwrap().visible.len(), 1, "Analysis");
    let course = browser.current().unwrap().selected().unwrap().clone();
    assert_eq!(course.title, "Analysis");

    // Into the course: its list starts unfiltered.
    load(
        &mut browser,
        Some(course),
        Page::from_items(vec![item("fold", 4, "Woche 1")]),
        false,
    );
    assert!(browser.update(&Action::Filter(String::new())).is_none());
    filter.update(&Action::Filter(String::new()));
    assert!(!filter.is_visible());

    // Back: My courses comes back with "an", in the bar too, the selection unchanged.
    let back = browser.handle_key_event(KeyCode::Char('h').into()).unwrap();
    assert!(matches!(&back, Action::Filter(q) if q == "an"));
    browser.update(&back);
    filter.update(&back);
    assert!(filter.is_visible());
    let level = browser.current().unwrap();
    assert_eq!(level.selected().map(|i| i.title.as_str()), Some("Analysis"));
    assert_eq!(level.visible.len(), 1);
}

#[test]
fn navigates_into_containers_and_back() {
    let course = item("crs", 1, "Informatik 1");
    let file = item("file", 2, "Skript.pdf");
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![course.clone()]),
        false,
    );

    let open = browser.handle_key_event(KeyCode::Enter.into());
    assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if *i == course));
    load(
        &mut browser,
        Some(course),
        Page::from_items(vec![file.clone()]),
        false,
    );
    assert!(
        matches!(browser.handle_key_event(KeyCode::Enter.into()), Some(Action::Download { item: ref i, open: true }) if *i == file)
    );

    let screen = screen(&mut browser);
    assert!(
        screen.contains("My courses") && screen.contains("Informatik 1"),
        "{screen}"
    );
    assert!(screen.contains("Skript.pdf"), "{screen}");

    browser.handle_key_event(KeyCode::Esc.into());
    assert_eq!(browser.current().unwrap().title(), "My courses");
}

#[test]
fn marks_saved_files_and_d_saves_without_opening() {
    let file = item("file", 2, "Skript");
    let mut browser = Browser::new(HashSet::from([2]), &Default::default());
    load(
        &mut browser,
        None,
        Page::from_items(vec![file.clone()]),
        false,
    );
    assert!(screen(&mut browser).contains("saved"));

    let d = browser.handle_key_event(KeyCode::Char('d').into());
    assert!(matches!(d, Some(Action::Download { open: false, .. })));

    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![file.clone()]),
        false,
    );
    assert!(!screen(&mut browser).contains("saved"));
    browser.update(&Action::Downloaded {
        item: file,
        path: "/tmp/Skript.pdf".into(),
        open: true,
    });
    assert!(screen(&mut browser).contains("saved"));
}

fn click(browser: &mut Browser, hit: Hit) -> Option<Action> {
    let (rect, _) = *browser
        .hits
        .iter()
        .find(|(_, h)| *h == hit)
        .expect("hit drawn");
    browser.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + 1,
        row: rect.y,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    })
}

#[test]
fn mouse_selects_opens_switches_tabs_and_goes_back() {
    let course = item("crs", 1, "DEMO");
    let mut file = item("file", 3, "Skript");
    file.properties = vec!["pdf".into()];
    let page = Page {
        todo: Vec::new(),
        rights: Default::default(),
        path: Vec::new(),
        tabs: vec![
            crate::ilias::parse::Tab {
                title: "Inhalt".into(),
                url: "u1".into(),
                active: true,
            },
            crate::ilias::parse::Tab {
                title: "Info".into(),
                url: "u2".into(),
                active: false,
            },
        ],
        blocks: vec![Block::Items {
            title: "Unterlagen".into(),
            items: vec![item("fold", 2, "Woche 1"), file],
        }],
    };
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![course.clone()]),
        false,
    );
    load(&mut browser, Some(course), page, false);
    screen(&mut browser);

    // Single click selects, a second click on the same item opens it.
    assert!(click(&mut browser, Hit::Item(1)).is_none());
    assert_eq!(browser.current().unwrap().selected().unwrap().key(), 3);
    let open = click(&mut browser, Hit::Item(1));
    assert!(matches!(open, Some(Action::Download { open: true, .. })));

    // Wheel moves the selection.
    browser.handle_mouse_event(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    });
    assert_eq!(browser.current().unwrap().selected().unwrap().key(), 2);

    let tab = click(&mut browser, Hit::Tab(1));
    assert!(matches!(tab, Some(Action::Load { url: Some(ref u), .. }) if u == "u2"));
    assert!(
        click(&mut browser, Hit::Tab(0)).is_none(),
        "active tab: nothing to do"
    );

    // Clicking "My courses" in the breadcrumb goes back.
    click(&mut browser, Hit::Crumb(0));
    assert_eq!(browser.stack.len(), 1);
}

#[test]
fn drops_stale_responses() {
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![item("crs", 1, "A")]),
        false,
    );
    browser.update(&Action::Load {
        item: Some(item("crs", 1, "A")),
        url: None,
        refresh: false,
        deep: false,
    });
    browser.handle_key_event(KeyCode::Esc.into()); // user gave up waiting
    browser.update(&Action::Loaded {
        item: Some(item("crs", 1, "A")),
        page: Page::default(),
        refresh: false,
        deep: false,
    });
    assert_eq!(browser.stack.len(), 1);
}

#[test]
fn filters_and_selects_within_matches() {
    let mut browser = Browser::default();
    let mut described = item("file", 3, "Woche 2 › Lecture_02");
    described.description = "Folien".into();
    let items = vec![
        item("fold", 1, "Woche 1"),
        item("file", 2, "Woche 1 › Notizen"),
        described,
    ];
    load(&mut browser, None, Page::from_items(items), false);

    // Matches the description too.
    browser.update(&Action::Filter("folien 2".into()));
    let screen = screen(&mut browser);
    assert!(screen.contains("1 of 3"), "{screen}");
    assert!(!screen.contains("Woche 1"), "{screen}");
    assert!(
        matches!(browser.handle_key_event(KeyCode::Enter.into()), Some(Action::Download { item: ref i, .. }) if i.key() == 3)
    );
}

#[test]
fn deep_search_opens_search_level() {
    let course = item("crs", 1, "DEMO");
    let mut browser = Browser::default();
    load(&mut browser, Some(course.clone()), Page::default(), false);
    let action = browser.handle_key_event(KeyCode::Char('s').into());
    assert!(matches!(action, Some(Action::Load { deep: true, .. })));
    load(
        &mut browser,
        Some(course),
        Page::from_items(vec![item("file", 2, "A › B.pdf")]),
        true,
    );
    assert!(screen(&mut browser).contains("Search in DEMO"));
}

#[test]
fn renders_course_page_like_ilias() {
    let mut file = item("file", 9, "Lecture_01");
    file.properties = vec!["pdf".into(), "1.2 MB".into()];
    let page = Page {
        todo: Vec::new(),
        rights: Default::default(),
        path: Vec::new(),
        tabs: vec![
            crate::ilias::parse::Tab {
                title: "Inhalt".into(),
                url: "u1".into(),
                active: true,
            },
            crate::ilias::parse::Tab {
                title: "Info".into(),
                url: "u2".into(),
                active: false,
            },
        ],
        blocks: vec![
            Block::Text("Willkommen im Modul DEMO".into()),
            Block::Items {
                title: "Unterlagen".into(),
                items: vec![item("fold", 8, "Übungen"), file],
            },
        ],
    };
    let mut browser = Browser::default();
    load(&mut browser, Some(item("crs", 7, "DEMO")), page, false);

    let screen = screen(&mut browser);
    for expected in [
        "Inhalt",
        "Info",
        "Willkommen im Modul",
        "Unterlagen",
        "Übungen",
        "pdf  ·  1.2 MB",
    ] {
        assert!(
            screen.contains(expected),
            "missing {expected:?} in {screen}"
        );
    }
    // Text rows are skipped: the first selectable row is the folder.
    assert_eq!(browser.current().unwrap().selected().unwrap().key(), 8);
    browser.handle_key_event(KeyCode::Down.into());
    assert_eq!(browser.current().unwrap().selected().unwrap().key(), 9);

    let tab = browser.handle_key_event(KeyCode::Tab.into());
    assert!(
        matches!(tab, Some(Action::Load { url: Some(ref u), refresh: false, .. }) if u == "u2")
    );
    // The other tab (here from the cache) replaces the page instead of opening on top.
    let info = Page::from_items(vec![item("file", 10, "Syllabus")]);
    load(
        &mut browser,
        Some(item("crs", 7, "DEMO")),
        info.clone(),
        false,
    );
    assert_eq!(browser.stack.len(), 1);
    assert_eq!(browser.current().unwrap().page, info);
}

#[test]
fn h_hands_in_only_on_assignments_that_take_files() {
    let tab = |url: &str| crate::ilias::parse::Tab {
        title: "t".into(),
        url: url.into(),
        active: false,
    };
    let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('H').into());

    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![item("crs", 1, "DEMO")]),
        false,
    );
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));

    let page = Page {
        tabs: vec![
            tab("overview"),
            tab("x?cmdClass=ilExSubmissionFileGUI&ass_id=1"),
        ],
        ..Page::default()
    };
    load(&mut browser, Some(item("exc", 2, "Blatt 1")), page, false);
    let action = press(&mut browser);
    assert!(
        matches!(action, Some(Action::AskHandIn { ref assignment, ref url })
            if assignment.title == "Blatt 1" && url.ends_with("ass_id=1"))
    );

    // A text assignment: no such tab.
    let page = Page {
        tabs: vec![tab("x?cmdClass=ilExSubmissionTextGUI&ass_id=2")],
        ..Page::default()
    };
    load(&mut browser, Some(item("ass", 3, "Essay")), page, false);
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));

    // Handing in reloads the page on screen and marks the others.
    let handed_in = browser.update(&Action::HandedIn {
        assignment: item("ass", 3, "Essay"),
        file: Some("a.pdf".into()),
    });
    assert!(matches!(
        handed_in,
        Some(Action::Load { refresh: true, .. })
    ));
    assert!(browser.stack[0].out_of_date);
    let dry = browser.update(&Action::HandedIn {
        assignment: item("ass", 3, "Essay"),
        file: None,
    });
    assert!(dry.is_none(), "nothing changed");
}

#[test]
fn x_asks_before_deleting_a_handed_in_file() {
    let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('X').into());
    let mut handed_in = item("file", 9, "Bericht.pdf");
    handed_in.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=download&ass_id=1&delivered=2".into();
    let mut browser = Browser::default();
    load(
        &mut browser,
        Some(item("ass", 2, "Laborbericht")),
        Page::from_items(vec![item("file", 8, "Vorlage.pdf"), handed_in]),
        false,
    );
    // The assignment's own file can't be deleted.
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));

    browser.handle_key_event(KeyCode::Char('j').into());
    let Some(Action::Ask { question, then }) = press(&mut browser) else {
        panic!("asks first");
    };
    assert!(question.contains("Bericht.pdf") && question.contains("Laborbericht"));
    assert!(matches!(*then, Action::DeleteSubmission { ref file, .. } if file.key() == 9));

    // Inside the assignment's Submission, it is still about the assignment.
    let mut submission = item("sub", 3, "Submission");
    submission.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=submissionScreen&ass_id=1".into();
    let mut handed_in = item("file", 9, "Bericht.pdf");
    handed_in.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=download&ass_id=1&delivered=2".into();
    let page = Page {
        tabs: vec![crate::ilias::parse::Tab {
            title: "Submission".into(),
            url: submission.url.clone(),
            active: true,
        }],
        ..Page::from_items(vec![handed_in])
    };
    load(&mut browser, Some(submission), page, false);
    let Some(Action::Ask { then, .. }) = press(&mut browser) else {
        panic!("asks first");
    };
    assert!(
        matches!(*then, Action::DeleteSubmission { ref assignment, .. } if assignment.title == "Laborbericht")
    );
    let hand_in = browser.handle_key_event(KeyCode::Char('H').into());
    assert!(
        matches!(hand_in, Some(Action::AskHandIn { ref assignment, .. }) if assignment.title == "Laborbericht")
    );

    let deleted = browser.update(&Action::SubmissionDeleted {
        assignment: item("ass", 2, "Laborbericht"),
        file: item("file", 9, "Bericht.pdf"),
        deleted: true,
    });
    assert!(matches!(deleted, Some(Action::Load { refresh: true, .. })));
}

#[test]
fn todo_panel_gets_focus_with_tab_and_opens_assignments() {
    let mut browser = Browser::default();
    let todo = |assignment| Todo {
        assignment,
        course: Some("DEMO.101".into()),
        deadline: "3. Nov 2026, 23:55".into(),
        due: "due in 2 days".into(),
        urgency: Urgency::Soon,
    };
    let page = Page {
        todo: vec![
            todo(item("exc", 5, "Blatt 1")),
            todo(item("exc", 6, "Blatt 2")),
        ],
        ..Page::from_items(vec![item("crs", 1, "DEMO.101")])
    };
    browser.update(&Action::Load {
        item: None,
        url: None,
        refresh: false,
        deep: false,
    });
    browser.update(&Action::Loaded {
        item: None,
        page,
        refresh: false,
        deep: false,
    });
    let key = |b: &mut Browser, code| b.handle_key_event(KeyEvent::from(code));

    // The panel sits to the right of the list on a wide terminal.
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal.draw(|f| browser.draw(f, f.area())).unwrap();
    let screen = format!("{:?}", terminal.backend().buffer());
    assert!(screen.contains("To-Do"), "{screen}");
    assert!(screen.contains("due in 2 days"), "{screen}");

    assert!(key(&mut browser, KeyCode::Tab).is_none());
    assert!(browser.todo_focus);
    assert!(key(&mut browser, KeyCode::Char('j')).is_none());
    let open = key(&mut browser, KeyCode::Enter);
    assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "Blatt 2"));

    // Esc gives the keys back to the course list.
    key(&mut browser, KeyCode::Esc);
    assert!(!browser.todo_focus);
    let open = key(&mut browser, KeyCode::Enter);
    assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "DEMO.101"));

    // Double-clicking an entry opens it.
    assert!(browser.click(Hit::Todo(0)).is_none());
    let open = browser.click(Hit::Todo(0));
    assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "Blatt 1"));
}

#[test]
fn f_asks_before_changing_favourites() {
    let mut course = item("crs", 1, "DEMO.101");
    course.favorite = true;
    course.favorite_toggle =
        Some("https://ilias.example/ilias.php?cmd=removeFromDesk&item_ref_id=1".into());
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![course, item("crs", 2, "DEMO.102")]),
        false,
    );
    assert!(screen(&mut browser).contains("★"));

    let action = browser.handle_key_event(KeyEvent::from(KeyCode::Char('f')));
    let Some(Action::Ask { question, then }) = action else {
        panic!("expected a question, got {action:?}");
    };
    assert!(question.starts_with("Remove “DEMO.101”"), "{question}");
    assert!(matches!(*then, Action::ToggleFavorite(ref i) if i.key() == 1));

    // Without a menu link there is nothing to send.
    browser.handle_key_event(KeyEvent::from(KeyCode::Char('j')));
    let action = browser.handle_key_event(KeyEvent::from(KeyCode::Char('f')));
    assert!(matches!(action, Some(Action::Error(_))));
}

#[test]
fn hidden_pages_reload_when_shown_again_after_a_favourites_change() {
    let mut browser = Browser::default();
    let course = item("crs", 1, "DEMO.101");
    load(
        &mut browser,
        None,
        Page::from_items(vec![course.clone()]),
        false,
    );
    let file = item("file", 5, "Lecture_01");
    load(
        &mut browser,
        Some(course.clone()),
        Page::from_items(vec![file.clone()]),
        false,
    );

    // The page on screen reloads right away …
    let now = browser.update(&Action::FavoriteToggled {
        item: file,
        dry_run: false,
    });
    assert!(
        matches!(now, Some(Action::Load { item: Some(ref i), refresh: true, .. }) if i.key() == 1)
    );
    // … while a refresh of hidden My courses is not applied behind the course.
    browser.update(&Action::Loaded {
        item: None,
        page: Page::default(),
        refresh: true,
        deep: false,
    });
    assert_eq!(browser.current().unwrap().items[0].key(), 5);

    // Going back shows My courses and reloads it, once.
    let back = browser
        .handle_key_event(KeyEvent::from(KeyCode::Esc))
        .unwrap();
    let reload = browser.update(&back);
    assert!(matches!(
        reload,
        Some(Action::Load {
            item: None,
            refresh: true,
            ..
        })
    ));
    assert!(browser.update(&Action::Filter(String::new())).is_none());
}

#[test]
fn a_dry_run_marks_nothing_out_of_date() {
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![item("crs", 1, "DEMO.101")]),
        false,
    );
    let change = Action::FavoriteToggled {
        item: item("crs", 1, "DEMO.101"),
        dry_run: true,
    };
    assert!(browser.update(&change).is_none());
}

#[test]
fn u_uploads_only_where_ilias_offers_it() {
    let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('u').into());
    let mut browser = Browser::default();
    load(
        &mut browser,
        None,
        Page::from_items(vec![item("crs", 1, "DEMO")]),
        false,
    );
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));

    // A student's folder: no upload form, and no hint for it.
    let files = Page::from_items(vec![item("file", 3, "Skript")]);
    load(
        &mut browser,
        Some(item("crs", 1, "DEMO")),
        files.clone(),
        false,
    );
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));
    assert!(!screen(&mut browser).contains("upload"));

    let editable = Page {
        rights: crate::ilias::Rights {
            upload: true,
            ..Default::default()
        },
        ..files
    };
    load(
        &mut browser,
        Some(item("fold", 2, "Unterlagen")),
        editable,
        false,
    );
    assert!(screen(&mut browser).contains("u upload"));
    let action = press(&mut browser);
    assert!(
        matches!(action, Some(Action::AskUpload { ref container, ref place })
            if container.title == "Unterlagen" && place.starts_with("DEMO ") && place.ends_with(" Unterlagen")),
        "{action:?}"
    );

    // An upload reloads the folder on screen.
    let uploaded = browser.update(&Action::Uploaded {
        container: item("fold", 2, "Unterlagen"),
        files: Some(vec!["a.pdf".into()]),
    });
    assert!(matches!(uploaded, Some(Action::Load { refresh: true, .. })));
}

#[test]
fn n_adds_a_folder_only_where_ilias_offers_it() {
    let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('n').into());
    let mut browser = Browser::default();
    let files = Page::from_items(vec![item("file", 3, "Skript")]);
    load(
        &mut browser,
        Some(item("crs", 1, "DEMO")),
        files.clone(),
        false,
    );
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));
    assert!(!screen(&mut browser).contains("new folder"));

    // Offered for wikis only: still no folder.
    let wikis = Page {
        rights: crate::ilias::Rights {
            create: vec![Kind::Other("wiki".into())],
            ..Default::default()
        },
        ..files.clone()
    };
    load(&mut browser, Some(item("crs", 1, "DEMO")), wikis, false);
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));

    let folders = Page {
        rights: crate::ilias::Rights {
            create: vec![Kind::Folder],
            ..Default::default()
        },
        ..files
    };
    load(&mut browser, Some(item("crs", 1, "DEMO")), folders, false);
    assert!(screen(&mut browser).contains("n new folder"));
    assert!(matches!(
        press(&mut browser),
        Some(Action::AskFolderName { ref container, .. }) if container.title == "DEMO"
    ));
    let created = browser.update(&Action::FolderCreated {
        container: item("crs", 1, "DEMO"),
        name: "Woche 5".into(),
        created: true,
    });
    assert!(matches!(created, Some(Action::Load { refresh: true, .. })));
}

#[test]
fn d_deletes_only_what_ilias_offers_to_delete_and_says_it_goes_to_the_trash() {
    let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('D').into());
    let mut browser = Browser::default();
    let mut page = Page::from_items(vec![item("file", 3, "Skript"), item("file", 4, "Folien")]);
    load(
        &mut browser,
        Some(item("crs", 1, "DEMO")),
        page.clone(),
        false,
    );
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));
    assert!(!screen(&mut browser).contains("delete"));

    page.rights.delete = vec![4];
    load(&mut browser, Some(item("crs", 1, "DEMO")), page, false);
    // Skript is selected: its menu has no Delete.
    assert!(matches!(press(&mut browser), Some(Action::Error(_))));
    browser.handle_key_event(KeyCode::Char('j').into());
    let Some(Action::Ask { question, then }) = press(&mut browser) else {
        panic!("expected the question");
    };
    assert_eq!(
        question,
        "Move “Folien” (File) from DEMO to the ILIAS trash? It can be restored from there in ILIAS."
    );
    assert!(matches!(*then, Action::Delete { ref item, ref container }
        if item.title == "Folien" && container.title == "DEMO"));
    let deleted = browser.update(&Action::Deleted {
        container: item("crs", 1, "DEMO"),
        item: item("file", 4, "Folien"),
        deleted: true,
    });
    assert!(matches!(deleted, Some(Action::Load { refresh: true, .. })));
}

#[test]
fn n_in_a_forum_starts_a_thread_and_r_replies_to_the_post_at_the_top() {
    use crate::action::Draft;
    let key = |browser: &mut Browser, c| browser.handle_key_event(KeyCode::Char(c).into());
    let mut browser = Browser::default();
    let mut threads = Page::from_items(vec![item("thr", 3, "Termin")]);
    load(
        &mut browser,
        Some(item("frm", 101, "Fragen")),
        threads.clone(),
        false,
    );
    assert!(matches!(key(&mut browser, 'n'), Some(Action::Error(_))));
    threads.rights.new_thread = true;
    load(
        &mut browser,
        Some(item("frm", 101, "Fragen")),
        threads,
        false,
    );
    assert!(screen(&mut browser).contains("n new thread"));
    assert!(matches!(
        key(&mut browser, 'n'),
        Some(Action::Compose(Draft::Thread { ref forum, .. })) if forum.title == "Fragen"
    ));

    let post = |id, author: &str| {
        Block::Post(crate::ilias::Post {
            id,
            author: format!("{author} · {}", author.to_lowercase()),
            title: "Termin".into(),
            body: vec!["Wann ist die Prüfung?".into(); 6],
            ..Default::default()
        })
    };
    let mut thread = Page {
        blocks: vec![post(6, "Ada"), post(7, "Bob")],
        ..Page::default()
    };
    thread.rights.reply = vec![7];
    let mut thr = item("thr", 3, "Termin");
    thr.id = Id::Thread(3);
    load(&mut browser, Some(thr), thread, false);
    // Ada's post is at the top, and has no Reply.
    assert!(!screen(&mut browser).contains("↩ R reply"));
    assert!(matches!(key(&mut browser, 'R'), Some(Action::Error(_))));
    // Scroll Bob's post to the top.
    for _ in 0..8 {
        key(&mut browser, 'j');
    }
    let shown = screen(&mut browser);
    assert!(
        shown
            .lines()
            .any(|line| line.contains("Bob · bob") && line.contains("↩ R reply")),
        "{shown}"
    );
    assert!(!shown.contains("Ada · ada"), "scrolled: {shown}");
    assert!(matches!(
        key(&mut browser, 'R'),
        Some(Action::Compose(Draft::Reply { post: 7, ref author, .. })) if author == "Bob"
    ));
}

#[test]
fn a_downloads_every_submission_and_d_one() {
    let key = |browser: &mut Browser, c| browser.handle_key_event(KeyCode::Char(c).into());
    let mut browser = Browser::default();
    load(
        &mut browser,
        Some(item("crs", 1, "DEMO")),
        Page::from_items(vec![item("file", 3, "Skript")]),
        false,
    );
    assert!(matches!(key(&mut browser, 'a'), Some(Action::Error(_))));

    let participant = |title: &str, url: &str| Item {
        kind: Kind::Participant,
        id: Id::local(title),
        title: title.into(),
        url: url.into(),
        ..Default::default()
    };
    let page = Page::from_items(vec![
        participant(
            "Muster, Anna",
            "https://ilias.example/x?cmd=downloadReturned&member_id=1",
        ),
        participant("Beispiel, Ben", ""),
    ]);
    load(&mut browser, Some(item("exc", 2, "Blatt 0")), page, false);
    let all = key(&mut browser, 'a');
    assert!(
        matches!(all, Some(Action::DownloadSubmissions(ref items)) if items.len() == 1 && items[0].title == "Muster, Anna"),
        "{all:?}"
    );
    assert!(matches!(
        key(&mut browser, 'd'),
        Some(Action::Download { open: false, .. })
    ));
    // Ben has nothing to download.
    key(&mut browser, 'j');
    assert!(key(&mut browser, 'd').is_none());
    assert!(matches!(
        browser.handle_key_event(KeyCode::Enter.into()),
        Some(Action::Error(_))
    ));
}

#[test]
fn shows_ilias_and_your_disk_as_they_are_after_a_push_or_the_trash() {
    let file = item("file", 2, "Skript");
    let mut browser = Browser::new(HashSet::from([2]), &Default::default());
    load(
        &mut browser,
        None,
        Page::from_items(vec![file.clone()]),
        false,
    );
    assert!(screen(&mut browser).contains("saved"));
    // Moved to your trash: no longer saved, and the page on screen is read again.
    let reload = browser.update(&Action::LocalTrashed {
        moved: 1,
        kept: Vec::new(),
        saved: Vec::new(),
    });
    assert!(
        matches!(reload, Some(Action::Load { refresh: true, .. })),
        "{reload:?}"
    );
    assert!(!screen(&mut browser).contains("saved"));
    // After a push: what is saved now (an uploaded file too), and a reload.
    let reload = browser.update(&Action::Pushed {
        summary: String::new(),
        report: Vec::new(),
        saved: vec![2],
    });
    assert!(
        matches!(reload, Some(Action::Load { refresh: true, .. })),
        "{reload:?}"
    );
    assert!(screen(&mut browser).contains("saved"));
}
