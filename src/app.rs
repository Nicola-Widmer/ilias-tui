//! Event loop: routes keys to components, runs side effects on worker threads and
//! broadcasts every action to all components.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use color_eyre::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};

use crate::action::Action;
use crate::cache::Cache;
use crate::components::{
    Browser, Component, Confirm, Filter, HandIn, Help, Report, Settings, StatusBar,
};
use crate::config::Config;
use crate::downloads::Downloads;
use crate::ilias::{Client, Item, Page};
use crate::{auth, open, session, sync};

pub struct App {
    client: Arc<Client>,
    config: Config,
    download_dir: PathBuf,
    downloads: Downloads,
    cache: Cache,
    confirm: Confirm,
    hand_in: HandIn,
    report: Report,
    filter: Filter,
    browser: Browser,
    status_bar: StatusBar,
    help: Help,
    settings: Settings,
    /// A sync is running. Only one at a time: each works from the download index as it
    /// was when it started, so a second one would save the first one's files again.
    syncing: Arc<AtomicBool>,
    /// A deep search is reading ILIAS. Only one at a time: each reads every folder below,
    /// and a second one would double the load on the server for a result nobody sees.
    searching: Arc<AtomicBool>,
    tx: Sender<Action>,
    rx: Receiver<Action>,
    should_quit: bool,
}

impl App {
    pub fn new(client: Client, config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let downloads = Downloads::load(&client.base);
        let cache = Cache::new(&client.base);
        let browser = Browser::new(downloads.saved().collect(), sync::Rules::new(&config));
        let download_dir = config.download_dir();
        let settings = Settings::new(config.clone(), download_dir.clone());
        crate::icons::set_nerd_font(config.nerd_font());
        crate::theme::apply(config.palette().0);
        Self {
            client: Arc::new(client),
            config,
            download_dir,
            downloads,
            cache,
            confirm: Confirm::default(),
            hand_in: HandIn::default(),
            report: Report::default(),
            filter: Filter::default(),
            browser,
            status_bar: StatusBar::default(),
            help: Help::default(),
            settings,
            syncing: Arc::default(),
            searching: Arc::default(),
            tx,
            rx,
            should_quit: false,
        }
    }

    /// Components from top-most to bottom-most; earlier ones get keys first.
    fn components(&mut self) -> [&mut dyn Component; 8] {
        [
            &mut self.confirm,
            &mut self.hand_in,
            &mut self.report,
            &mut self.help,
            &mut self.settings,
            &mut self.filter,
            &mut self.browser,
            &mut self.status_bar,
        ]
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        if self.client.is_offline() {
            self.tx.send(Action::Offline)?;
        }
        self.tx.send(Action::Load {
            item: None,
            url: None,
            refresh: false,
            deep: false,
        })?;
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(50))? {
                let action = match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                    Event::Mouse(mouse) => self
                        .components()
                        .into_iter()
                        .find_map(|c| c.handle_mouse_event(mouse)),
                    _ => None,
                };
                if let Some(action) = action {
                    self.tx.send(action)?;
                }
            }
            while let Ok(action) = self.rx.try_recv() {
                self.dispatch(action)?;
            }
        }
        session::save(&self.client.base, &self.client.cookies())
    }

    fn handle_key(&mut self, key: event::KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::Quit);
        }
        let handled = self
            .components()
            .into_iter()
            .find_map(|c| c.handle_key_event(key));
        // `q` comes last so it can still be typed into the search bar.
        handled.or((key.code == KeyCode::Char('q')).then_some(Action::Quit))
    }

    fn dispatch(&mut self, action: Action) -> Result<()> {
        // Offline nothing can be changed on ILIAS; say so before asking the question.
        if self.client.is_offline()
            && matches!(
                action,
                Action::Ask { .. } | Action::AskHandIn { .. } | Action::Sync(_)
            )
        {
            self.tx.send(Action::Error(
                "Offline: nothing can be sent to or fetched from ILIAS. Press L to go online."
                    .into(),
            ))?;
            return Ok(());
        }
        let wants_sync = match &action {
            Action::Sync(_) => true,
            Action::Ask { then, .. } => matches!(**then, Action::Sync(_)),
            _ => false,
        };
        if wants_sync && self.syncing.load(Ordering::SeqCst) {
            self.tx.send(Action::Error(
                "A sync is already running; start the next one when it has finished.".into(),
            ))?;
            return Ok(());
        }
        // Offline, a deep search only reads the cache: no limit there.
        let wants_search =
            matches!(action, Action::Load { deep: true, .. }) && !self.client.is_offline();
        if wants_search && self.searching.load(Ordering::SeqCst) {
            self.tx.send(Action::Error(
                "A search is already running; start the next one when it has finished.".into(),
            ))?;
            return Ok(());
        }
        match &action {
            Action::Quit => self.should_quit = true,
            Action::Load {
                item,
                url,
                refresh,
                deep,
            } => {
                let (item, url, refresh, deep) = (item.clone(), url.clone(), *refresh, *deep);
                let key = Cache::key(
                    url.as_deref().or(item.as_ref().map(|i| i.url.as_str())),
                    deep,
                );
                if self.client.is_offline() {
                    // Offline: whatever the cache has, never a request.
                    let cached = if deep {
                        self.cache.descendants(item.as_ref()).map(Page::from_items)
                    } else {
                        self.cache.get(&key)
                    };
                    self.tx.send(match cached {
                        Some(page) => Action::Loaded {
                            item,
                            page,
                            refresh,
                            deep,
                        },
                        None => Action::Error(format!(
                            "Not available offline: {} was not opened before. Press L to go online.",
                            item.as_ref().map_or("My courses", |i| i.title.as_str())
                        )),
                    })?;
                } else if let Some(page) = (!refresh).then(|| self.cache.get(&key)).flatten() {
                    // Show the cached copy now and fetch the current one in the background.
                    self.tx.send(Action::Loaded {
                        item: item.clone(),
                        page,
                        refresh: false,
                        deep,
                    })?;
                    self.tx.send(Action::Load {
                        item,
                        url,
                        refresh: true,
                        deep,
                    })?;
                } else {
                    self.fetch(item, url, refresh, deep, key);
                }
            }
            // Opening a file that was saved before: use the local copy.
            Action::Download { item, open: true } if self.downloads.get(item.ref_id).is_some() => {
                let path = self.downloads.get(item.ref_id).cloned().unwrap_or_default();
                self.tx.send(Action::OpenFile(path))?;
                return Ok(());
            }
            Action::Download { item, open } => {
                let (item, open, dir) = (item.clone(), *open, self.download_dir.clone());
                self.spawn(move |client, _| {
                    let path = client.download(&item, &dir)?;
                    Ok(Action::Downloaded { item, path, open })
                });
            }
            Action::Downloaded { item, path, open } => {
                self.downloads
                    .record(item.ref_id, path.clone(), crate::sync::version(item))?;
                if *open {
                    self.tx.send(Action::OpenFile(path.clone()))?;
                }
            }
            // Files that could run code are only shown in the file manager.
            Action::OpenFile(path) if open::is_risky(path) => open::reveal(path)?,
            Action::OpenFile(path) => open::external(path.as_os_str())?,
            Action::OpenInBrowser(item) if open::is_ilias_link(&item.url, &self.client.base) => {
                open::external(item.url.as_ref())?
            }
            Action::OpenInBrowser(item) => {
                self.tx.send(Action::Error(format!(
                    "Not opening {}: not an ILIAS link",
                    item.url
                )))?;
            }
            Action::ToggleFavorite(item) => {
                let item = item.clone();
                self.spawn(move |client, _| {
                    let sent = client.toggle_favorite(&item)?;
                    Ok(Action::FavoriteToggled {
                        item,
                        dry_run: !sent,
                    })
                });
            }
            Action::HandIn {
                assignment,
                url,
                path,
            } => {
                let (assignment, url, path) = (assignment.clone(), url.clone(), path.clone());
                self.spawn(move |client, _| {
                    let file = client.hand_in(&url, &path)?;
                    Ok(Action::HandedIn { assignment, file })
                });
            }
            Action::DeleteSubmission { assignment, file } => {
                let (assignment, file) = (assignment.clone(), file.clone());
                self.spawn(move |client, _| {
                    let deleted = client.delete_submission(&file)?;
                    Ok(Action::SubmissionDeleted {
                        assignment,
                        file,
                        deleted,
                    })
                });
            }
            Action::Sync(roots) => {
                let (roots, dir) = (roots.clone(), self.download_dir.clone());
                let cache = self.cache.clone();
                let rules = sync::Rules::new(&self.config);
                self.syncing.store(true, Ordering::SeqCst);
                let syncing = Running(self.syncing.clone());
                self.spawn(move |client, tx| {
                    // Cleared when the job ends, whether it succeeds or fails.
                    let _syncing = syncing;
                    let mut downloads = Downloads::load(&client.base);
                    let summary =
                        sync::sync(client, &roots, &dir, &mut downloads, &cache, rules, |p| {
                            let _ = tx.send(Action::Progress(p));
                        })?;
                    Ok(Action::Synced {
                        summary: summary.text(rules.max_mb),
                        report: summary.report(&dir, rules.max_mb),
                        saved: summary.saved,
                    })
                });
            }
            // The sync wrote its own copy of the index; read it again.
            Action::Synced { .. } => self.downloads = Downloads::load(&self.client.base),
            Action::SetConfig(config) => {
                let dir_changed = config.download_dir != self.config.download_dir;
                self.config = config.clone();
                self.config.save()?;
                crate::icons::set_nerd_font(self.config.nerd_font());
                crate::theme::apply(self.config.palette().0);
                self.client.set_dry_run(self.config.dry_run());
                if dir_changed {
                    // Chosen here, it applies now, even over ILIAS_DOWNLOAD_DIR.
                    self.download_dir = (self.config.download_dir.clone())
                        .unwrap_or_else(|| self.config.download_dir());
                    self.tx
                        .send(Action::DownloadDirSet(self.download_dir.clone()))?;
                }
            }
            Action::Login => self.spawn(|client, _| {
                if client.is_offline() {
                    // Going online: the saved session may well still work.
                    client.set_offline(false);
                    match client.check_session() {
                        Ok(true) => return Ok(Action::LoggedIn),
                        Ok(false) => {}
                        Err(e) => {
                            client.set_offline(true);
                            return Err(e.wrap_err("Still offline: ILIAS can't be reached"));
                        }
                    }
                }
                client.set_cookies(auth::browser_login(&client.base)?);
                session::save(&client.base, &client.cookies())?;
                Ok(Action::LoggedIn)
            }),
            _ => {}
        }
        let tx = self.tx.clone();
        for component in self.components() {
            if let Some(follow_up) = component.update(&action) {
                tx.send(follow_up)?;
            }
        }
        Ok(())
    }

    /// Fetches a page in the background and caches it.
    fn fetch(
        &self,
        item: Option<Item>,
        url: Option<String>,
        refresh: bool,
        deep: bool,
        key: String,
    ) {
        let cache = self.cache.clone();
        let searching = deep.then(|| {
            self.searching.store(true, Ordering::SeqCst);
            Running(self.searching.clone())
        });
        self.spawn(move |client, tx| {
            // Cleared when the job ends, whether it succeeds or fails.
            let _searching = searching;
            let page = match (&item, deep) {
                (_, true) => Page::from_items(client.descendants(item.as_ref(), |n| {
                    let _ = tx.send(Action::Progress(format!("Searching… {n} folders")));
                })?),
                (Some(item), false) => client.page(item, url.as_deref())?,
                (None, false) => client.my_courses()?,
            };
            cache.put(&key, &page);
            Ok(Action::Loaded {
                item,
                page,
                refresh,
                deep,
            })
        });
    }

    /// Runs blocking network work off the UI thread. The job may send progress actions;
    /// its result is sent when it finishes, errors as [`Action::Error`].
    fn spawn(&self, job: impl FnOnce(&Client, &Sender<Action>) -> Result<Action> + Send + 'static) {
        let (client, tx) = (self.client.clone(), self.tx.clone());
        thread::spawn(move || {
            let action = job(&client, &tx).unwrap_or_else(|e| Action::Error(format!("{e:#}")));
            let _ = tx.send(action);
        });
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let colors = crate::theme::colors();
        frame.render_widget(
            ratatui::widgets::Block::new()
                .style(ratatui::style::Style::new().bg(colors.bg).fg(colors.fg)),
            area,
        );
        let filter_height = u16::from(self.filter.is_visible());
        let [main, filter, status] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(filter_height),
            Constraint::Length(1),
        ])
        .areas(area);
        self.browser.draw(frame, main);
        if filter_height > 0 {
            self.filter.draw(frame, filter);
        }
        self.status_bar.draw(frame, status);
        self.settings.draw(frame, area);
        self.help.draw(frame, area);
        self.report.draw(frame, area);
        self.hand_in.draw(frame, area);
        // Last, so the question is on top of everything.
        self.confirm.draw(frame, area);
    }
}

/// Clears a "job is running" flag when dropped.
struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Item;
    use url::Url;

    fn app(cache_dir: &std::path::Path) -> App {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        // No test reaches the network: every page request fails.
        let offline = crate::ilias::pages::FakeIlias::new(|url| {
            Err(color_eyre::eyre::eyre!("offline: {url}"))
        });
        let client = Client::with_pages(base, std::sync::Arc::new(offline));
        let mut app = App::new(client, Config::default());
        app.cache = Cache::in_dir(cache_dir.to_path_buf());
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
            ref_id: 1,
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
    fn f_shows_the_question_on_screen() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-confirm-{}", std::process::id()));
        let mut app = app(&dir);
        show_my_courses(&mut app);
        let key = |c| ratatui::crossterm::event::KeyEvent::from(c);
        let ask = app.handle_key(key(KeyCode::Char('f'))).expect("f asks");
        app.dispatch(ask).unwrap();

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
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

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
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
            ref_id: 1,
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
            ref_id: 1,
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
        assert!(
            matches!(app.rx.try_recv(), Ok(Action::Error(ref e)) if e.contains("already running"))
        );
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

        // A search that fails (no network here) ends too: the next one may start.
        app.dispatch(search()).unwrap();
        assert!(app.searching.load(Ordering::SeqCst));
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
    fn cache_hits_need_no_request() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-test-{}", std::process::id()));
        let mut app = app(&dir);

        // My courses loads like any other page: the cached copy, then a refresh.
        show_my_courses(&mut app);

        // A cached page is shown right away, then refreshed in the background.
        let folder = Item {
            kind: "fold".into(),
            ref_id: 2,
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
            ref_id: id,
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
            ref_id: id,
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
}
