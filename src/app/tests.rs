use super::*;
use crate::ilias::Id;
use crate::ilias::Item;
use url::Url;

fn app(cache_dir: &std::path::Path) -> App {
    // No test reaches the network: every page request fails.
    let offline =
        crate::ilias::pages::FakeIlias::new(|url| Err(color_eyre::eyre::eyre!("offline: {url}")));
    app_with(cache_dir, Arc::new(offline))
}

fn app_with(cache_dir: &std::path::Path, ilias: Arc<crate::ilias::pages::FakeIlias>) -> App {
    let base = Url::parse("https://ilias.example/ilias/").unwrap();
    let client = Client::with_pages(base, ilias);
    let mut app = App::new(client, Config::default());
    app.cache = Cache::in_dir(cache_dir.to_path_buf());
    app.seen = Seen::default();
    app
}

fn load(item: Option<Item>) -> Action {
    Action::Load {
        item,
        url: None,
        refresh: false,
        deep: false,
    }
}

/// Shows My courses (with one course) from the cache, without the background refresh,
/// which would go to the network.
fn show_my_courses(app: &mut App) {
    let courses = Page::from_items(vec![Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        favorite_toggle: Some(
            "https://ilias.example/ilias/ilias.php?cmd=addToDesk&item_ref_id=1".into(),
        ),
        ..Default::default()
    }]);
    app.cache.put(&Cache::key(None, false), &courses);
    app.dispatch(load(None)).unwrap();
    let shown = app.rx.try_recv().unwrap();
    assert!(matches!(shown, Action::Loaded { refresh: false, .. }));
    app.dispatch(shown).unwrap();
    assert!(matches!(
        app.rx.try_recv(),
        Ok(Action::Load { refresh: true, .. })
    ));
    while app.rx.try_recv().is_ok() {}
}

#[test]
fn one_enter_in_the_filter_opens_the_match() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-filter-{}", std::process::id()));
    let mut app = app(&dir);
    show_my_courses(&mut app);
    let key = |c| ratatui::crossterm::event::KeyEvent::from(c);
    for code in [KeyCode::Char('/'), KeyCode::Char('d'), KeyCode::Char('e')] {
        let action = app.handle_key(key(code)).expect("filter handles typing");
        app.dispatch(action).unwrap();
    }
    let open = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "DEMO"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn letters_typed_into_the_filter_open_nothing() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-typing-{}", std::process::id()));
    let mut app = app(&dir);
    show_my_courses(&mut app);
    let key = |c| ratatui::crossterm::event::KeyEvent::from(c);
    // `c` opens the settings and `?` the help, but not while you type: "scripts?".
    for c in "/scripts?".chars() {
        let action = app
            .handle_key(key(KeyCode::Char(c)))
            .expect("the filter takes it");
        assert!(matches!(action, Action::Filter(_)), "{c}: {action:?}");
        app.dispatch(action).unwrap();
    }
    assert!(app.filter.is_editing());
    // Done typing: `c` is the settings key again.
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        app.handle_key(key(KeyCode::Char('c'))),
        Some(Action::ToggleSettings)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn f_shows_the_question_on_screen() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-confirm-{}", std::process::id()));
    let mut app = app(&dir);
    show_my_courses(&mut app);
    let key = |c| ratatui::crossterm::event::KeyEvent::from(c);
    let ask = app.handle_key(key(KeyCode::Char('f'))).expect("f asks");
    app.dispatch(ask).unwrap();

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = format!("{:?}", terminal.backend().buffer());
    assert!(
        screen.contains("Add “DEMO” to your ILIAS favourites?"),
        "{screen}"
    );

    // "n" closes it without sending anything.
    let answer = app.handle_key(key(KeyCode::Char('n')));
    assert!(matches!(answer, Some(Action::Cancelled)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn c_shows_the_settings_page() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-settings-{}", std::process::id()));
    let mut app = app(&dir);
    show_my_courses(&mut app);
    let open = app
        .handle_key(KeyCode::Char('c').into())
        .expect("c opens settings");
    app.dispatch(open).unwrap();

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = format!("{:?}", terminal.backend().buffer());
    assert!(screen.contains("Download folder"), "{screen}");

    // `q` closes the page instead of quitting.
    let close = app.handle_key(KeyCode::Char('q').into());
    assert!(matches!(close, Some(Action::ToggleSettings)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_one_sync_runs_at_a_time() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-onesync-{}", std::process::id()));
    let mut app = app(&dir);
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Default::default()
    };
    let refused = |app: &mut App, action: Action| {
        app.dispatch(action).unwrap();
        matches!(app.rx.try_recv(), Ok(Action::Error(ref e)) if e.contains("already running"))
    };

    // While one runs, a second one is refused, also before its question.
    app.syncing.store(true, Ordering::SeqCst);
    assert!(refused(&mut app, Action::Sync(vec![course.clone()])));
    let ask = Action::Ask {
        question: "Sync all?".into(),
        then: Box::new(Action::Sync(vec![course.clone()])),
    };
    assert!(refused(&mut app, ask));
    app.syncing.store(false, Ordering::SeqCst);

    // A sync that fails (no network here) ends too: the next one may start.
    app.dispatch(Action::Sync(vec![course])).unwrap();
    assert!(app.syncing.load(Ordering::SeqCst));
    loop {
        match app.rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Action::Error(e)) => {
                assert!(!e.contains("already running"), "{e}");
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("the sync never ended: {e}"),
        }
    }
    assert!(!app.syncing.load(Ordering::SeqCst));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_one_search_runs_at_a_time() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-onesearch-{}", std::process::id()));
    let mut app = app(&dir);
    let course = Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Default::default()
    };
    let search = || Action::Load {
        item: Some(course.clone()),
        url: None,
        refresh: false,
        deep: true,
    };

    // While one runs, a second one is refused; plain pages still load.
    app.searching.store(true, Ordering::SeqCst);
    app.dispatch(search()).unwrap();
    assert!(matches!(app.rx.try_recv(), Ok(Action::Error(ref e)) if e.contains("already running")));
    app.dispatch(load(Some(course.clone()))).unwrap();
    // It fetches in the background (and fails: no network); wait for that.
    match app.rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Action::Error(e)) => assert!(!e.contains("already running"), "{e}"),
        other => panic!("expected the page fetch to fail, got {other:?}"),
    }

    // Offline, searching only reads the cache: never refused.
    app.client.set_offline(true);
    app.dispatch(search()).unwrap();
    assert!(
        !matches!(app.rx.try_recv(), Ok(Action::Error(ref e)) if e.contains("already running"))
    );
    app.client.set_offline(false);
    app.searching.store(false, Ordering::SeqCst);

    // A search that fails ends too: the next one may start. Its request waits for the
    // gate, so it is still running when checked.
    let (open_gate, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let waits = crate::ilias::pages::FakeIlias::new(move |url| {
        let _ = gate.lock().unwrap().recv();
        Err(color_eyre::eyre::eyre!("offline: {url}"))
    });
    let base = url::Url::parse("https://ilias.example/ilias/").unwrap();
    app.client = Arc::new(Client::with_pages(base, Arc::new(waits)));
    app.dispatch(search()).unwrap();
    assert!(app.searching.load(Ordering::SeqCst));
    open_gate.send(()).unwrap();
    loop {
        match app.rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Action::Error(e)) => {
                assert!(!e.contains("already running"), "{e}");
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("the search never ended: {e}"),
        }
    }
    assert!(!app.searching.load(Ordering::SeqCst));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn asks_before_opening_a_link_to_another_site() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-links-{}", std::process::id()));
    let mut app = app(&dir);
    let link = |url: &str| Item {
        kind: "link".into(),
        url: url.into(),
        ..Item::default()
    };
    app.dispatch(Action::OpenInBrowser(link("https://zoom.us/j/1")))
        .unwrap();
    match app.rx.try_recv() {
        Ok(Action::Ask { question, then }) => {
            assert!(question.contains("https://zoom.us/j/1"), "{question}");
            assert!(matches!(*then, Action::OpenLink(ref u) if u == "https://zoom.us/j/1"));
        }
        other => panic!("expected a question, got {other:?}"),
    }
    // Also offline: opening another site sends nothing to ILIAS.
    app.client.set_offline(true);
    app.dispatch(Action::Ask {
        question: "Open?".into(),
        then: Box::new(Action::OpenLink("https://zoom.us/j/1".into())),
    })
    .unwrap();
    assert!(app.rx.try_recv().is_err(), "not refused");
    app.client.set_offline(false);

    // ILIAS links open at once.
    app.dispatch(Action::OpenInBrowser(link(
        "https://ilias.example/ilias/goto.php/fold/2",
    )))
    .unwrap();
    assert!(matches!(app.rx.try_recv(), Ok(Action::OpenLink(_))));
    // Anything but a web link is refused.
    app.dispatch(Action::OpenInBrowser(link("file:///etc/passwd")))
        .unwrap();
    assert!(matches!(app.rx.try_recv(), Ok(Action::Error(e)) if e.contains("not a web link")));
    let _ = std::fs::remove_dir_all(dir);
}

/// Runs the app's actions until `done` says so; returns them in order.
fn pump(app: &mut App, done: impl Fn(&Action) -> bool) -> Vec<Action> {
    let mut seen = Vec::new();
    loop {
        let action = app
            .rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("stuck after {seen:?}: {e}"));
        seen.push(action.clone());
        app.dispatch(action).unwrap();
        if done(seen.last().unwrap()) {
            return seen;
        }
    }
}

fn cached_my_courses(app: &App) {
    let courses = Page::from_items(vec![Item {
        kind: "crs".into(),
        id: Id::Ref(1),
        title: "DEMO".into(),
        url: "https://ilias.example/ilias/goto.php/crs/1".into(),
        ..Default::default()
    }]);
    app.cache.put(&Cache::key(None, false), &courses);
}

#[test]
fn starts_from_the_cache_and_reads_the_course_list_once() {
    use crate::ilias::pages::{FakeIlias, reply};
    let dir = std::env::temp_dir().join(format!("ilias-tui-start-{}", std::process::id()));
    // ILIAS answers once the gate opens (its sender is dropped), like a slow server.
    let (open_gate, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let ilias = Arc::new(FakeIlias::new(move |url| {
        let _ = gate.lock().unwrap().recv();
        reply(url, r#"<div id="il_center_col"><p>Kurse</p></div>"#)
    }));
    let mut app = app_with(&dir, ilias.clone());
    cached_my_courses(&app);
    app.start().unwrap();
    // The cached copy shows while the session check still waits for ILIAS (its request
    // may or may not have left yet); nothing else was asked for.
    pump(&mut app, |a| matches!(a, Action::Loaded { .. }));
    let base = Url::parse("https://ilias.example/ilias/").unwrap();
    let courses = "ilias.php?baseClass=ilmembershipoverviewgui";
    assert!(ilias.requests(&base).iter().all(|r| r == courses));

    drop(open_gate);
    pump(&mut app, |a| {
        matches!(a, Action::Loaded { refresh: true, .. })
    });
    let mut requests = ilias.requests(&base);
    requests.sort();
    assert_eq!(
        requests,
        [
            "ilias.php?baseClass=ilDashboardGUI",
            "ilias.php?baseClass=ilmembershipoverviewgui"
        ],
        "the session check's course list is the one shown"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn starts_offline_when_ilias_cannot_be_reached() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-start-off-{}", std::process::id()));
    let mut app = app(&dir);
    cached_my_courses(&app);
    app.start().unwrap();
    pump(&mut app, |a| matches!(a, Action::Offline));
    assert!(app.client.is_offline());
    assert!(!app.checking);
    // My courses from the cache, with nothing sent.
    pump(&mut app, |a| matches!(a, Action::Loaded { .. }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cache_hits_need_no_request() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-test-{}", std::process::id()));
    let mut app = app(&dir);

    // My courses loads like any other page: the cached copy, then a refresh.
    show_my_courses(&mut app);

    // A cached page is shown right away, then refreshed in the background.
    let folder = Item {
        kind: "fold".into(),
        id: Id::Ref(2),
        url: "https://ilias.example/ilias/goto.php/fold/2".into(),
        ..Default::default()
    };
    let cached = Page::from_items(vec![]);
    app.cache
        .put(&Cache::key(Some(&folder.url), false), &cached);
    app.dispatch(load(Some(folder))).unwrap();
    assert!(
        matches!(app.rx.try_recv(), Ok(Action::Loaded { refresh: false, ref page, .. }) if *page == cached)
    );
    assert!(matches!(
        app.rx.try_recv(),
        Ok(Action::Load { refresh: true, .. })
    ));

    app.cache.clear();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn offline_deep_search_walks_the_cached_pages() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-deep-{}", std::process::id()));
    let mut app = app(&dir);
    app.client.set_offline(true);
    let item = |kind: &str, id: u64, title: &str| Item {
        kind: kind.into(),
        id: Id::Ref(id),
        title: title.into(),
        url: format!("https://ilias.example/ilias/goto.php/{kind}/{id}"),
        ..Default::default()
    };
    let course = item("crs", 1, "DEMO");
    let put = |app: &App, of: &Item, items: Vec<Item>| {
        app.cache
            .put(&Cache::key(Some(&of.url), false), &Page::from_items(items))
    };
    put(
        &app,
        &course,
        vec![item("fold", 2, "Woche 1"), item("fold", 4, "Woche 2")],
    );
    put(
        &app,
        &item("fold", 2, "Woche 1"),
        vec![item("file", 3, "Folien")],
    );

    app.dispatch(Action::Load {
        item: Some(course),
        url: None,
        refresh: false,
        deep: true,
    })
    .unwrap();
    let Ok(Action::Loaded {
        page, deep: true, ..
    }) = app.rx.try_recv()
    else {
        panic!("expected the search results");
    };
    let titles: Vec<String> = page.items().map(|i| i.title.clone()).collect();
    // Woche 2 was never opened: listed, but nothing below it.
    assert_eq!(titles, ["Woche 1", "Woche 1 › Folien", "Woche 2"]);

    // A course whose page isn't cached can't be searched.
    app.dispatch(Action::Load {
        item: Some(item("crs", 9, "Anderes")),
        url: None,
        refresh: false,
        deep: true,
    })
    .unwrap();
    assert!(matches!(app.rx.try_recv(), Ok(Action::Error(_))));

    app.cache.clear();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn offline_shows_cached_pages_and_never_refreshes() {
    let dir = std::env::temp_dir().join(format!("ilias-tui-offline-{}", std::process::id()));
    let mut app = app(&dir);
    app.client.set_offline(true);
    let folder = |id: u64| Item {
        kind: "fold".into(),
        id: Id::Ref(id),
        title: format!("Ordner {id}"),
        url: format!("https://ilias.example/ilias/goto.php/fold/{id}"),
        ..Default::default()
    };
    let cached = Page::from_items(vec![Item::default()]);
    app.cache
        .put(&Cache::key(Some(&folder(2).url), false), &cached);

    // Cached: shown, and no background refresh queued (also not for `r`).
    for refresh in [false, true] {
        app.dispatch(Action::Load {
            item: Some(folder(2)),
            url: None,
            refresh,
            deep: false,
        })
        .unwrap();
        assert!(
            matches!(app.rx.try_recv(), Ok(Action::Loaded { ref page, .. }) if *page == cached)
        );
        assert!(app.rx.try_recv().is_err(), "nothing else queued");
    }

    // Never opened: a clear message instead of a request.
    app.dispatch(load(Some(folder(3)))).unwrap();
    let Ok(Action::Error(message)) = app.rx.try_recv() else {
        panic!("expected an error");
    };
    assert!(
        message.contains("Not available offline: Ordner 3"),
        "{message}"
    );

    // Changing a favourite is refused before the question.
    app.dispatch(Action::Ask {
        question: "Add?".into(),
        then: Box::new(Action::ToggleFavorite(folder(2))),
    })
    .unwrap();
    assert!(matches!(app.rx.try_recv(), Ok(Action::Error(_))));
    let _ = std::fs::remove_dir_all(dir);
}
