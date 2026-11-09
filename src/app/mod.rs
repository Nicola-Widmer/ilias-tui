//! Event loop: routes keys to components, runs side effects on worker threads and
//! broadcasts every action to all components. What each action does is in `effects.rs`.

use std::path::{Path, PathBuf};
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
use crate::seen::Seen;
use crate::{auth, open, session, sync};

pub struct App {
    client: Arc<Client>,
    config: Config,
    download_dir: PathBuf,
    downloads: Downloads,
    cache: Cache,
    /// What's new since your last visit, until you open it.
    seen: Seen,
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
    /// The session check at startup is still running: pages show from the cache, and
    /// nothing is refreshed until it says whether the session works.
    checking: bool,
    tx: Sender<Action>,
    rx: Receiver<Action>,
    should_quit: bool,
}

impl App {
    pub fn new(client: Client, config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let downloads = Downloads::load(&client.base);
        let cache = Cache::new(&client.base);
        let seen = Seen::load(&client.base);
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
            seen,
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
            checking: false,
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
        self.start()?;
        loop {
            // Actions first, so that even the first frame shows what they bring (the cached
            // My courses), not an empty screen.
            while let Ok(action) = self.rx.try_recv() {
                self.dispatch(action)?;
            }
            if self.should_quit {
                break;
            }
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
        }
        session::save(&self.client.base, &self.client.cookies())
    }

    /// Shows My courses at once (from the cache) and checks the session in the background,
    /// renewing it silently if possible. Then the page on screen is refreshed (`LoggedIn`),
    /// or Chrome opens for the login (`Login`), or the app goes offline (`Offline`).
    fn start(&mut self) -> Result<()> {
        self.tx.send(Action::NewCounts(self.seen.counts()))?;
        if self.client.is_offline() {
            // Started offline on purpose: the browser then shows My courses from the cache.
            self.tx.send(Action::Offline)?;
            return Ok(());
        }
        self.checking = true;
        self.tx.send(Action::Load {
            item: None,
            url: None,
            refresh: false,
            deep: false,
        })?;
        self.spawn(|client, _| {
            Ok(match client.check_session() {
                Ok(true) => Action::LoggedIn,
                Ok(false) => Action::Login,
                // Can't reach ILIAS: work with what is cached.
                Err(_) => {
                    client.set_offline(true);
                    Action::Offline
                }
            })
        });
        Ok(())
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
        handled.or(
            (crate::keymap::command(key) == Some(crate::keymap::Command::Quit))
                .then_some(Action::Quit),
        )
    }

    /// Runs what `action` needs (a request, a file, a job), then hands it to every
    /// component.
    fn dispatch(&mut self, action: Action) -> Result<()> {
        if let Some(refusal) = self.refusal(&action) {
            self.tx.send(Action::Error(refusal.into()))?;
            return Ok(());
        }
        // The session check (or a login) has answered.
        if matches!(action, Action::LoggedIn | Action::Login | Action::Offline) {
            self.checking = false;
        }
        if self.effect(&action)? == Broadcast::No {
            return Ok(());
        }
        let tx = self.tx.clone();
        for component in self.components() {
            if let Some(follow_up) = component.update(&action) {
                tx.send(follow_up)?;
            }
        }
        Ok(())
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

mod effects;
#[cfg(test)]
mod tests;

use effects::Broadcast;
