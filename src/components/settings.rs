//! The settings page, toggled with `c`. Changes are saved right away (see `config.rs`).

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::{config, theme};

pub struct Settings {
    visible: bool,
    /// Where files go now.
    download_dir: PathBuf,
    /// Whether the folder was chosen here (otherwise it's the default or the env variable).
    custom: bool,
    /// `ILIAS_DOWNLOAD_DIR` is set, so it wins at the next start.
    env: bool,
    /// The folder being typed, while editing.
    input: Option<String>,
    error: Option<String>,
}

impl Settings {
    pub fn new(download_dir: PathBuf, custom: bool) -> Self {
        Self {
            visible: false,
            download_dir,
            custom,
            env: std::env::var_os("ILIAS_DOWNLOAD_DIR").is_some(),
            input: None,
            error: None,
        }
    }

    fn edit_key(&mut self, key: KeyEvent) -> Option<Action> {
        let input = self.input.as_mut()?;
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => match config::parse_dir(input) {
                Ok(dir) => {
                    self.input = None;
                    return Some(Action::SetDownloadDir(Some(dir)));
                }
                Err(e) => self.error = Some(e),
            },
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
            return (key.code == KeyCode::Char('c')).then_some(Action::ToggleSettings);
        }
        self.error = None;
        if self.input.is_some() {
            return self.edit_key(key);
        }
        // While open, the page swallows every key.
        Some(match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                self.input = Some(config::display(&self.download_dir));
                Action::Handled
            }
            KeyCode::Char('r') if self.custom => Action::SetDownloadDir(None),
            KeyCode::Esc | KeyCode::Char('c' | 'q') => Action::ToggleSettings,
            _ => Action::Handled,
        })
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        // A click closes the page (unless a folder is being typed).
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
            Action::SetDownloadDir(dir) => self.custom = dir.is_some(),
            Action::DownloadDirSet(dir) => self.download_dir.clone_from(dir),
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }
        let mut text = vec![Line::styled(
            "Download folder",
            Style::new().fg(theme::YELLOW).add_modifier(Modifier::BOLD),
        )];
        match &self.input {
            Some(input) => text.push(Line::from(vec![
                Span::styled(format!("  {input}"), Style::new().fg(theme::FG)),
                Span::styled("▏", Style::new().fg(theme::ACCENT)),
            ])),
            None => text.push(Line::from(vec![
                Span::styled(
                    format!("  {}", config::display(&self.download_dir)),
                    Style::new().fg(theme::FG),
                ),
                Span::styled(if self.custom { "" } else { "  (default)" }, theme::muted()),
            ])),
        }
        if self.env {
            text.push(Line::styled(
                "  ILIAS_DOWNLOAD_DIR is set: it wins again at the next start.",
                Style::new().fg(theme::YELLOW),
            ));
        }
        if let Some(error) = &self.error {
            text.push(Line::styled(
                format!("  {error}"),
                Style::new().fg(theme::RED),
            ));
        }
        let hint = match (&self.input, self.custom) {
            (Some(_), _) => " ↵ save  esc cancel ",
            (None, true) => " ↵ change  r reset  esc close ",
            (None, false) => " ↵ change  esc close ",
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::ACCENT))
            .title(Line::styled(
                format!(" {}  Settings ", theme::gear()),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(hint, theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(Style::new().bg(theme::BAR_BG));
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false }).block(block);
        // Long folders wrap: make the box tall enough for every wrapped line
        // (the width inside the borders and padding is 4 less).
        let width = area.width.min(76);
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

    #[test]
    fn typing_a_folder_sets_it() {
        let mut s = Settings::new(PathBuf::from("/old"), false);
        assert!(
            key(&mut s, KeyCode::Char('x')).is_none(),
            "closed: keys pass"
        );
        assert!(matches!(
            key(&mut s, KeyCode::Char('c')),
            Some(Action::ToggleSettings)
        ));
        assert!(s.visible);
        assert!(
            matches!(key(&mut s, KeyCode::Char('r')), Some(Action::Handled)),
            "no reset of the default"
        );

        key(&mut s, KeyCode::Enter);
        assert_eq!(s.input.as_deref(), Some("/old"));
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
            matches!(set, Some(Action::SetDownloadDir(Some(ref d))) if d == &PathBuf::from("/newq"))
        );
        assert!(s.custom && s.input.is_none());

        assert!(matches!(
            key(&mut s, KeyCode::Char('r')),
            Some(Action::SetDownloadDir(None))
        ));
        assert!(matches!(
            key(&mut s, KeyCode::Esc),
            Some(Action::ToggleSettings)
        ));
        assert!(!s.visible);
    }

    #[test]
    fn long_folders_are_shown_in_full() {
        let dir = format!("/{}/end-of-path", "very-long-folder-name/".repeat(8));
        let mut s = Settings::new(PathBuf::from(&dir), true);
        key(&mut s, KeyCode::Char('c'));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
        terminal.draw(|frame| s.draw(frame, frame.area())).unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(screen.contains("end-of-path"), "{screen}");
        assert!(screen.contains("esc close"), "{screen}");
    }

    #[test]
    fn relative_folders_are_refused() {
        let mut s = Settings::new(PathBuf::from("/old"), false);
        key(&mut s, KeyCode::Char('c'));
        key(&mut s, KeyCode::Enter);
        s.input = Some("Uni".into());
        assert!(matches!(key(&mut s, KeyCode::Enter), Some(Action::Handled)));
        assert!(s.error.is_some() && s.input.is_some());
    }
}
