//! The settings page, toggled with `c`: every setting of `config.rs`, one per row. Changes
//! are saved right away; environment variables (`ILIAS_*`) still win over them.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::config::{self, Config};
use crate::keymap::{self, Command};
use crate::seen::Group;
use crate::theme;

/// One row of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    DownloadDir,
    SyncMaxMb,
    MissingOnly,
    TwoWay,
    NerdFont,
    Theme,
    Mouse,
    Cache,
    Offline,
    DryRun,
    Url,
    /// Whether this kind of news is marked and counted.
    ShowNew(Group),
}

const FIELDS: [Field; 15] = [
    Field::DownloadDir,
    Field::SyncMaxMb,
    Field::MissingOnly,
    Field::TwoWay,
    Field::NerdFont,
    Field::Theme,
    Field::Mouse,
    Field::Cache,
    Field::Offline,
    Field::DryRun,
    Field::Url,
    Field::ShowNew(Group::Files),
    Field::ShowNew(Group::Folders),
    Field::ShowNew(Group::Posts),
    Field::ShowNew(Group::Other),
];

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::DownloadDir => "Download folder",
            Field::SyncMaxMb => "Sync size limit",
            Field::MissingOnly => "Sync missing files only",
            Field::TwoWay => "Two-way sync",
            Field::NerdFont => "Nerd Font icons",
            Field::Theme => "Theme",
            Field::Mouse => "Mouse support",
            Field::Cache => "Page cache",
            Field::Offline => "Always start offline",
            Field::DryRun => "Dry run",
            Field::Url => "ILIAS URL",
            Field::ShowNew(Group::Files) => "What's new: files",
            Field::ShowNew(Group::Folders) => "What's new: folders",
            Field::ShowNew(Group::Posts) => "What's new: forum posts",
            Field::ShowNew(Group::Other) => "What's new: other",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Field::DownloadDir => "Where downloaded and synced files are saved.",
            Field::SyncMaxMb => "Sync skips files bigger than this (lecture recordings).",
            Field::MissingOnly => {
                "Sync downloads every file that isn't on disk, also the ones you deleted. \
                 Files you have are never replaced, and no new versions are saved beside them."
            }
            Field::TwoWay => {
                "Sync also uploads your new files to ILIAS, after listing them and asking. \
                 Edited and deleted files are only listed so far."
            }
            Field::NerdFont => "Off: plain Unicode symbols, for fonts without Nerd Font glyphs.",
            Field::Theme => {
                "Enter switches between dark, light and hub. For your own colours, add e.g. \
                 \"colors\": { \"accent\": \"#ff8800\" } to config.json and restart."
            }
            Field::Mouse => "Off: the terminal's own text selection works without a modifier key.",
            Field::Cache => "Keeps opened pages for instant display and offline use.",
            Field::Offline => "Start with cached pages and saved files only (L goes online).",
            Field::DryRun => "Never send favourites changes; only say what would have happened.",
            Field::Url => "The ILIAS root, the folder that contains login.php.",
            Field::ShowNew(Group::Files) => "Mark new and changed files (● new) and count them.",
            Field::ShowNew(Group::Folders) => {
                "Mark new and changed folders, courses, groups and sessions, and count them."
            }
            Field::ShowNew(Group::Posts) => {
                "Mark forums and threads with new posts (● new posts) and count them."
            }
            Field::ShowNew(Group::Other) => {
                "Mark everything else that is new or changed (exercises, tests, links, …) \
                 and count it."
            }
        }
    }

    /// The environment variable that wins over this setting.
    fn env(self) -> &'static str {
        match self {
            Field::DownloadDir => "ILIAS_DOWNLOAD_DIR",
            Field::SyncMaxMb => "ILIAS_SYNC_MAX_MB",
            Field::MissingOnly | Field::Theme | Field::ShowNew(_) => "",
            Field::TwoWay => "ILIAS_TWO_WAY",
            Field::NerdFont => "ILIAS_NERD_FONT",
            Field::Mouse => "ILIAS_MOUSE",
            Field::Cache => "ILIAS_CACHE",
            Field::Offline => "ILIAS_OFFLINE",
            Field::DryRun => "ILIAS_DRY_RUN",
            Field::Url => "ILIAS_URL",
        }
    }

    /// Changes take effect only after a restart.
    fn at_next_start(self) -> bool {
        matches!(
            self,
            Field::Mouse | Field::Cache | Field::Offline | Field::Url
        )
    }

    /// On/off, for the toggles; `None` for the rows that are typed.
    fn toggle(self, config: &Config) -> Option<bool> {
        Some(match self {
            Field::MissingOnly => config.missing_only,
            Field::TwoWay => config.two_way(),
            Field::NerdFont => config.nerd_font(),
            Field::Mouse => config.mouse(),
            Field::Cache => config.cache(),
            Field::Offline => config.offline(),
            Field::DryRun => config.dry_run(),
            Field::ShowNew(group) => config.shows_new(group),
            Field::DownloadDir | Field::SyncMaxMb | Field::Theme | Field::Url => return None,
        })
    }

    fn set_toggle(self, config: &mut Config, on: bool) {
        match self {
            Field::MissingOnly => config.missing_only = on,
            Field::TwoWay => config.two_way = on,
            Field::NerdFont => config.nerd_font = Some(on),
            Field::Mouse => config.mouse = Some(on),
            Field::Cache => config.cache = Some(on),
            Field::Offline => config.offline = on,
            Field::DryRun => config.dry_run = on,
            Field::ShowNew(group) if on => {
                config.hide_new.remove(&group);
            }
            Field::ShowNew(group) => {
                config.hide_new.insert(group);
            }
            Field::DownloadDir | Field::SyncMaxMb | Field::Theme | Field::Url => {}
        }
    }

    /// Chosen here, i.e. not the default.
    fn is_custom(self, config: &Config) -> bool {
        match self {
            Field::DownloadDir => config.download_dir.is_some(),
            Field::SyncMaxMb => config.sync_max_mb.is_some(),
            Field::MissingOnly => config.missing_only,
            Field::TwoWay => config.two_way,
            Field::NerdFont => config.nerd_font.is_some(),
            Field::Theme => config.theme.is_some() || !config.colors.is_empty(),
            Field::Mouse => config.mouse.is_some(),
            Field::Cache => config.cache.is_some(),
            Field::Offline => config.offline,
            Field::DryRun => config.dry_run,
            Field::Url => config.url.is_some(),
            Field::ShowNew(group) => !config.shows_new(group),
        }
    }

    fn reset(self, config: &mut Config) {
        let default = Config::default();
        match self {
            Field::DownloadDir => config.download_dir = default.download_dir,
            Field::SyncMaxMb => config.sync_max_mb = default.sync_max_mb,
            Field::MissingOnly => config.missing_only = default.missing_only,
            Field::TwoWay => config.two_way = default.two_way,
            Field::NerdFont => config.nerd_font = default.nerd_font,
            // The colours were written by hand; they stay until removed by hand.
            Field::Theme => config.theme = default.theme,
            Field::Mouse => config.mouse = default.mouse,
            Field::Cache => config.cache = default.cache,
            Field::Offline => config.offline = default.offline,
            Field::DryRun => config.dry_run = default.dry_run,
            Field::Url => config.url = default.url,
            Field::ShowNew(group) => {
                config.hide_new.remove(&group);
            }
        }
    }

    /// Stores what was typed into a text row.
    fn parse(self, config: &mut Config, input: &str) -> Result<(), String> {
        match self {
            Field::DownloadDir => config.download_dir = Some(config::parse_dir(input)?),
            Field::SyncMaxMb => config.sync_max_mb = Some(config::parse_mb(input)?),
            Field::Url => config.url = Some(config::parse_url(input)?),
            _ => {}
        }
        Ok(())
    }
}

pub struct Settings {
    visible: bool,
    config: Config,
    /// Where files go now (chosen here, the env variable or the default).
    download_dir: PathBuf,
    /// The row that is selected.
    selected: usize,
    /// The value being typed, while editing.
    input: Option<String>,
    error: Option<String>,
}

impl Settings {
    pub fn new(config: Config, download_dir: PathBuf) -> Self {
        Self {
            visible: false,
            config,
            download_dir,
            selected: 0,
            input: None,
            error: None,
        }
    }

    fn field(&self) -> Field {
        FIELDS[self.selected]
    }

    /// The value as shown (what applies, environment variables included).
    fn value(&self, field: Field) -> String {
        if let Some(on) = field.toggle(&self.config) {
            return if on { "on" } else { "off" }.into();
        }
        match field {
            Field::DownloadDir => config::display(&self.download_dir),
            Field::SyncMaxMb => format!("{} MB", self.config.sync_max_mb()),
            Field::Theme => match self.config.colors.len() {
                0 => self.config.theme().to_string(),
                n => format!("{} + {n} custom colours", self.config.theme()),
            },
            Field::Url => self.config.url(),
            _ => String::new(),
        }
    }

    /// What editing a text row starts with.
    fn initial_input(&self, field: Field) -> String {
        match field {
            Field::SyncMaxMb => self.config.sync_max_mb().to_string(),
            _ => self.value(field),
        }
    }

    fn changed(&self, change: impl FnOnce(&mut Config)) -> Action {
        let mut config = self.config.clone();
        change(&mut config);
        if config == self.config {
            Action::Handled
        } else {
            Action::SetConfig(config)
        }
    }

    fn edit_key(&mut self, key: KeyEvent) -> Option<Action> {
        let field = self.field();
        let input = self.input.as_mut()?;
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => {
                let mut config = self.config.clone();
                match field.parse(&mut config, input) {
                    Ok(()) => {
                        self.input = None;
                        return Some(self.changed(|c| *c = config));
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) => input.push(c),
            _ => {}
        }
        Some(Action::Handled)
    }
}

impl Component for Settings {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.visible {
            return (keymap::command(key) == Some(Command::Settings))
                .then_some(Action::ToggleSettings);
        }
        self.error = None;
        if self.input.is_some() {
            return self.edit_key(key);
        }
        let field = self.field();
        // While open, the page swallows every key.
        Some(match key.code {
            KeyCode::Enter | KeyCode::Char(' ' | 'e') if field == Field::Theme => {
                let at = theme::THEMES.iter().position(|t| *t == self.config.theme());
                let next = at.map_or(0, |i| (i + 1) % theme::THEMES.len());
                self.changed(|c| c.theme = Some(theme::THEMES[next].into()))
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1) % FIELDS.len();
                Action::Handled
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.selected = (self.selected + FIELDS.len() - 1) % FIELDS.len();
                Action::Handled
            }
            KeyCode::Enter | KeyCode::Char(' ' | 'e') => match field.toggle(&self.config) {
                Some(on) => self.changed(|c| field.set_toggle(c, !on)),
                None => {
                    self.input = Some(self.initial_input(field));
                    Action::Handled
                }
            },
            KeyCode::Char('r') => self.changed(|c| field.reset(c)),
            KeyCode::Esc | KeyCode::Char('c' | 'q') => Action::ToggleSettings,
            _ => Action::Handled,
        })
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        // A click closes the page (unless a value is being typed).
        (self.visible && self.input.is_none() && matches!(mouse.kind, MouseEventKind::Down(_)))
            .then_some(Action::ToggleSettings)
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        match action {
            Action::ToggleSettings => {
                self.visible = !self.visible;
                self.input = None;
                self.error = None;
            }
            Action::SetConfig(config) => self.config = config.clone(),
            Action::DownloadDirSet(dir) => self.download_dir.clone_from(dir),
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }
        let label_width = FIELDS.iter().map(|f| f.label().len()).max().unwrap_or(0);
        let mut text = Vec::new();
        for (i, &field) in FIELDS.iter().enumerate() {
            let selected = i == self.selected;
            let label_style = if selected {
                theme::accent_bold()
            } else {
                Style::new().fg(theme::colors().fg)
            };
            let mut spans = vec![
                Span::styled(if selected { "› " } else { "  " }, theme::accent_bold()),
                Span::styled(format!("{:label_width$}  ", field.label()), label_style),
            ];
            match (&self.input, selected) {
                (Some(input), true) => {
                    spans.push(Span::styled(
                        input.clone(),
                        Style::new().fg(theme::colors().fg),
                    ));
                    spans.push(Span::styled("▏", Style::new().fg(theme::colors().accent)));
                }
                _ => {
                    let value = self.value(field);
                    let color = match field.toggle(&self.config) {
                        Some(true) => theme::colors().green,
                        Some(false) => theme::colors().red,
                        None => theme::colors().fg,
                    };
                    spans.push(Span::styled(
                        value,
                        Style::new().fg(color).add_modifier(Modifier::BOLD),
                    ));
                    if !field.is_custom(&self.config) {
                        spans.push(Span::styled("  (default)", theme::muted()));
                    }
                }
            }
            text.push(Line::from(spans));
        }

        // About the selected row.
        let field = self.field();
        text.push(Line::default());
        text.push(Line::styled(field.help(), theme::muted()));
        if field == Field::Theme {
            if let Ok(path) = Config::path() {
                text.push(Line::styled(config::display(&path), theme::muted()));
            }
            let ignored = self.config.palette().1;
            if !ignored.is_empty() {
                text.push(Line::styled(
                    format!("Not usable in colors: {}", ignored.join(", ")),
                    Style::new().fg(theme::colors().yellow),
                ));
            }
        }
        if field.at_next_start() {
            text.push(Line::styled("Applies at the next start.", theme::muted()));
        }
        if !field.env().is_empty() && std::env::var_os(field.env()).is_some() {
            text.push(Line::styled(
                if field == Field::DownloadDir {
                    format!("{} is set: it wins again at the next start.", field.env())
                } else {
                    format!("{} is set: it wins over this setting.", field.env())
                },
                Style::new().fg(theme::colors().yellow),
            ));
        }
        if let Some(error) = &self.error {
            text.push(Line::styled(
                error.clone(),
                Style::new().fg(theme::colors().red),
            ));
        }

        let custom = field.is_custom(&self.config);
        let hint = match (&self.input, field.toggle(&self.config).is_some(), custom) {
            (Some(_), _, _) => " ↵ save  esc cancel ",
            (None, true, true) => " ↑↓ choose  ↵ toggle  r reset  esc close ",
            (None, true, false) => " ↑↓ choose  ↵ toggle  esc close ",
            (None, false, true) => " ↑↓ choose  ↵ change  r reset  esc close ",
            (None, false, false) => " ↑↓ choose  ↵ change  esc close ",
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(
                format!(" {}  Settings ", theme::gear()),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(hint, theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false }).block(block);
        // Long values wrap: make the box tall enough for every wrapped line
        // (the width inside the borders and padding is 4 less).
        let width = area.width.min(80);
        let height = paragraph.line_count(width.saturating_sub(4)) as u16;
        let area = area.centered(Constraint::Length(width), Constraint::Length(height));
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &mut Settings, code: KeyCode) -> Option<Action> {
        let action = s.handle_key_event(KeyEvent::from(code));
        if let Some(action) = &action {
            s.update(action);
        }
        action
    }

    /// An absolute folder on every system (`/tmp/old`, `C:\\…\\Temp\\old`).
    fn old() -> PathBuf {
        std::env::temp_dir().join("old")
    }

    fn open() -> Settings {
        let mut s = Settings::new(Config::default(), old());
        assert!(
            key(&mut s, KeyCode::Char('x')).is_none(),
            "closed: keys pass"
        );
        assert!(matches!(
            key(&mut s, KeyCode::Char('c')),
            Some(Action::ToggleSettings)
        ));
        assert!(s.visible);
        s
    }

    fn select(s: &mut Settings, field: Field) {
        while s.field() != field {
            key(s, KeyCode::Down);
        }
    }

    #[test]
    fn typing_a_folder_sets_it() {
        let mut s = open();
        assert!(
            matches!(key(&mut s, KeyCode::Char('r')), Some(Action::Handled)),
            "no reset of the default"
        );

        key(&mut s, KeyCode::Enter);
        assert_eq!(s.input, Some(config::display(&old())));
        for _ in 0..3 {
            key(&mut s, KeyCode::Backspace);
        }
        for c in "new".chars() {
            key(&mut s, KeyCode::Char(c));
        }
        // `q` is typed, not a close.
        key(&mut s, KeyCode::Char('q'));
        let set = key(&mut s, KeyCode::Enter);
        assert!(
            matches!(set, Some(Action::SetConfig(ref c)) if c.download_dir == Some(std::env::temp_dir().join("newq")))
        );
        assert!(s.input.is_none());

        assert!(matches!(
            key(&mut s, KeyCode::Char('r')),
            Some(Action::SetConfig(ref c)) if c.download_dir.is_none()
        ));
        assert!(matches!(
            key(&mut s, KeyCode::Esc),
            Some(Action::ToggleSettings)
        ));
        assert!(!s.visible);
    }

    #[test]
    fn toggles_flip_and_reset() {
        let mut s = open();
        select(&mut s, Field::MissingOnly);
        let on = key(&mut s, KeyCode::Char(' '));
        assert!(matches!(on, Some(Action::SetConfig(ref c)) if c.missing_only));
        assert!(s.config.missing_only);
        let off = key(&mut s, KeyCode::Char('r'));
        assert!(matches!(off, Some(Action::SetConfig(ref c)) if !c.missing_only));
        // Up from the first row wraps to the last.
        select(&mut s, Field::DownloadDir);
        key(&mut s, KeyCode::Up);
        assert_eq!(s.field(), FIELDS[FIELDS.len() - 1]);
    }

    #[test]
    fn every_toggle_flips_and_resets() {
        let mut s = open();
        for field in FIELDS {
            let Some(before) = field.toggle(&s.config) else {
                continue;
            };
            // A variable set in this shell would win; nothing to see then.
            if !field.env().is_empty() && std::env::var_os(field.env()).is_some() {
                continue;
            }
            select(&mut s, field);
            assert!(matches!(
                key(&mut s, KeyCode::Enter),
                Some(Action::SetConfig(_))
            ));
            assert_eq!(field.toggle(&s.config), Some(!before), "{field:?}");
            assert!(field.is_custom(&s.config), "{field:?}");
            key(&mut s, KeyCode::Char('r'));
            assert_eq!(field.toggle(&s.config), Some(before), "{field:?}");
            assert!(!field.is_custom(&s.config), "{field:?}");
        }
        assert_eq!(s.config, Config::default());
    }

    #[test]
    fn enter_switches_the_theme_and_r_goes_back() {
        let mut s = open();
        select(&mut s, Field::Theme);
        assert_eq!(s.value(Field::Theme), "dark");
        for expected in ["light", "hub", "dark", "light"] {
            let Some(Action::SetConfig(config)) = key(&mut s, KeyCode::Enter) else {
                panic!("expected a change");
            };
            assert_eq!(config.theme.as_deref(), Some(expected));
        }
        assert!(s.input.is_none(), "not a typed row");

        s.config.colors.insert("accent".into(), "#ff8800".into());
        assert_eq!(s.value(Field::Theme), "light + 1 custom colours");
        let Some(Action::SetConfig(config)) = key(&mut s, KeyCode::Char('r')) else {
            panic!("expected a change");
        };
        assert_eq!(config.theme, None);
        assert_eq!(config.colors.len(), 1, "hand-written colours stay");
    }

    #[test]
    fn the_size_limit_must_be_a_number() {
        let mut s = open();
        select(&mut s, Field::SyncMaxMb);
        key(&mut s, KeyCode::Enter);
        s.input = Some("lots".into());
        assert!(matches!(key(&mut s, KeyCode::Enter), Some(Action::Handled)));
        assert!(s.error.is_some() && s.input.is_some());
        s.input = Some("120".into());
        let set = key(&mut s, KeyCode::Enter);
        assert!(matches!(set, Some(Action::SetConfig(ref c)) if c.sync_max_mb == Some(120.0)));
    }

    #[test]
    fn every_setting_is_shown() {
        let dir = format!("/{}/end-of-path", "very-long-folder-name/".repeat(8));
        let mut s = Settings::new(Config::default(), PathBuf::from(&dir));
        key(&mut s, KeyCode::Char('c'));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 40)).unwrap();
        terminal.draw(|frame| s.draw(frame, frame.area())).unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(screen.contains("end-of-path"), "{screen}");
        assert!(screen.contains("esc close"), "{screen}");
        for field in FIELDS {
            assert!(screen.contains(field.label()), "{screen}");
        }
    }

    #[test]
    fn relative_folders_are_refused() {
        let mut s = open();
        key(&mut s, KeyCode::Enter);
        s.input = Some("Uni".into());
        assert!(matches!(key(&mut s, KeyCode::Enter), Some(Action::Handled)));
        assert!(s.error.is_some() && s.input.is_some());
    }
}
