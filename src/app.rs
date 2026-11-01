//! Event loop: routes keys to components, runs side effects on worker threads and
//! broadcasts every action to all components.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use color_eyre::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::{DefaultTerminal, Frame};

use crate::action::Action;
use crate::cache::Cache;
use crate::components::{Browser, Component, Filter, Help, StatusBar};
use crate::downloads::Downloads;
use crate::ilias::{Client, Item, Page};
use crate::{auth, ilias, open, session};

pub struct App {
    client: Arc<Client>,
    download_dir: PathBuf,
    downloads: Downloads,
    cache: Cache,
    /// "My courses" from the startup session check, shown without fetching it again.
    initial: Option<Page>,
    filter: Filter,
    browser: Browser,
    status_bar: StatusBar,
    help: Help,
    tx: Sender<Action>,
    rx: Receiver<Action>,
    should_quit: bool,
}

impl App {
    pub fn new(client: Client, download_dir: PathBuf, courses: Page) -> Self {
        let (tx, rx) = mpsc::channel();
        let downloads = Downloads::load(&client.base);
        let cache = Cache::new(&client.base);
        let browser = Browser::new(downloads.saved().collect());
        Self {
            client: Arc::new(client),
            download_dir,
            downloads,
            cache,
            initial: Some(courses),
            filter: Filter::default(),
            browser,
            status_bar: StatusBar::default(),
            help: Help::default(),
            tx,
            rx,
            should_quit: false,
        }
    }

    /// Components from top-most to bottom-most; earlier ones get keys first.
    fn components(&mut self) -> [&mut dyn Component; 4] {
        [
            &mut self.help,
            &mut self.filter,
            &mut self.browser,
            &mut self.status_bar,
        ]
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
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
                let startup = (item.is_none() && !deep && !refresh)
                    .then(|| self.initial.take())
                    .flatten();
                if let Some(courses) = startup {
                    // The courses come from the session check; the To-Do list takes a few
                    // more requests, so until it arrives show last time's (if cached).
                    let shown = self.cache.get(&key).unwrap_or_else(|| courses.clone());
                    self.tx.send(Action::Loaded {
                        item: None,
                        page: shown,
                        refresh: false,
                        deep: false,
                    })?;
                    let cache = self.cache.clone();
                    self.spawn(move |client, _| {
                        let page = ilias::with_todo(courses, client.todo()?);
                        cache.put(&key, &page);
                        Ok(Action::Loaded {
                            item: None,
                            page,
                            refresh: true,
                            deep: false,
                        })
                    });
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
                self.downloads.insert(item.ref_id, path.clone())?;
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
            Action::Login => self.spawn(|client, _| {
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
        self.spawn(move |client, tx| {
            let page = match (&item, deep) {
                (_, true) => Page::from_items(client.descendants(item.as_ref(), |n| {
                    let _ = tx.send(Action::Progress(format!("Searching… {n} folders")));
                })?),
                (Some(item), false) => client.page(item, url.as_deref())?,
                (None, false) => client.courses()?,
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
        self.help.draw(frame, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Item;
    use cookie_store::CookieStore;
    use url::Url;

    fn app(cache_dir: &std::path::Path) -> App {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let client = Client::new(base, CookieStore::default()).unwrap();
        let courses = Page::from_items(vec![Item {
            kind: "crs".into(),
            ref_id: 1,
            title: "DEMO".into(),
            ..Default::default()
        }]);
        let mut app = App::new(client, PathBuf::from("/tmp"), courses);
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

    #[test]
    fn one_enter_in_the_filter_opens_the_match() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-filter-{}", std::process::id()));
        let mut app = app(&dir);
        app.dispatch(load(None)).unwrap();
        while let Ok(action) = app.rx.try_recv() {
            app.dispatch(action).unwrap();
        }
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
    fn startup_page_and_cache_hits_need_no_request() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-test-{}", std::process::id()));
        let mut app = app(&dir);

        // "My courses" comes from the startup session check.
        app.dispatch(load(None)).unwrap();
        let Ok(Action::Loaded {
            item: None,
            page,
            refresh: false,
            ..
        }) = app.rx.try_recv()
        else {
            panic!("expected the startup page");
        };
        assert_eq!(page.items().next().unwrap().title, "DEMO");

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
}
