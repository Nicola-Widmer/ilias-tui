//! The `/` search bar. Filters the current list as you type.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::Component;
use crate::action::Action;
use crate::keymap::{self, Command};
use crate::theme;

#[derive(Default)]
pub struct Filter {
    query: String,
    /// Keys go into the query while editing; otherwise only `/` and `Esc` are handled.
    editing: bool,
}

impl Filter {
    /// Whether the bar takes up a line on screen.
    pub fn is_visible(&self) -> bool {
        self.editing || !self.query.is_empty()
    }

    fn changed(&self) -> Option<Action> {
        Some(Action::Filter(self.query.clone()))
    }
}

impl Component for Filter {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.editing {
            return match key.code {
                _ if keymap::command(key) == Some(Command::Filter) => {
                    self.editing = true;
                    self.changed()
                }
                // First Esc clears an active filter; the next one goes back.
                KeyCode::Esc if !self.query.is_empty() => {
                    self.query.clear();
                    self.changed()
                }
                _ => None,
            };
        }
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                self.query.clear();
            }
            // Stop typing and let the list open the selected match with the same Enter.
            KeyCode::Enter => {
                self.editing = false;
                return None;
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char(c) => self.query.push(c),
            // Let arrow keys through so the list can be navigated while typing.
            _ => return None,
        }
        self.changed()
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        match action {
            Action::Filter(query) => self.query.clone_from(query),
            // Deep search results open the bar right away.
            Action::Loaded {
                refresh: false,
                deep: true,
                ..
            } => self.editing = true,
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled(
            format!(" {}  ", theme::search()),
            theme::accent_bold(),
        )];
        if self.query.is_empty() && self.editing {
            spans.push(Span::styled("type to filter…", theme::muted()));
        } else {
            spans.push(Span::styled(
                self.query.clone(),
                Style::new().fg(theme::colors().fg),
            ));
        }
        if self.editing {
            spans.push(Span::styled("▏", Style::new().fg(theme::colors().accent)));
        }
        let hint = if self.editing {
            "↵ open  esc clear "
        } else {
            "esc clear "
        };
        let [left, right] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(hint.chars().count() as u16),
        ])
        .areas(area);
        let bar = Style::new()
            .bg(theme::colors().selected_bg)
            .fg(theme::colors().fg);
        frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), left);
        frame.render_widget(
            Paragraph::new(hint).style(bar.fg(theme::colors().muted)),
            right,
        );
    }
}

/// Case-insensitive; every whitespace-separated word of `query` must occur in `title`.
pub fn matches(title: &str, query: &str) -> bool {
    let title = title.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| title.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_all_words_in_any_order() {
        assert!(matches("Unterlagen › Woche 3 › Folien.pdf", "folien woche"));
        assert!(matches("Anything", ""));
        assert!(!matches("Unterlagen › Woche 3", "woche 4"));
    }

    #[test]
    fn typing_emits_filter_actions_and_esc_clears() {
        let mut filter = Filter::default();
        assert!(filter.handle_key_event(KeyCode::Char('x').into()).is_none());
        filter.handle_key_event(KeyCode::Char('/').into());
        filter.handle_key_event(KeyCode::Char('s').into());
        let action = filter.handle_key_event(KeyCode::Char('w').into());
        assert!(matches!(action, Some(Action::Filter(ref q)) if q == "sw"));
        // Enter ends typing but isn't consumed, so the list opens the selection.
        assert!(filter.handle_key_event(KeyCode::Enter.into()).is_none());
        assert!(filter.is_visible());
        let action = filter.handle_key_event(KeyCode::Esc.into());
        assert!(matches!(action, Some(Action::Filter(ref q)) if q.is_empty()));
        assert!(!filter.is_visible());
    }
}
