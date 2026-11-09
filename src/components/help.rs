//! Key binding overlay, toggled with `?`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Row, Table};

use super::Component;
use crate::action::Action;
use crate::keymap::{self, Command};
use crate::theme;

/// The mouse, which has no place in the keymap.
const MOUSE: &[(&str, &str)] = &[
    ("click", "Select; double-click opens"),
    ("wheel", "Move selection"),
    ("tab / crumb", "Click to switch tab / go back"),
];

#[derive(Default)]
pub struct Help {
    visible: bool,
}

impl Component for Help {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        // While open, the overlay swallows every key; any key closes it.
        (self.visible || keymap::command(key) == Some(Command::Help)).then_some(Action::ToggleHelp)
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        // A click anywhere closes the overlay.
        (self.visible && matches!(mouse.kind, MouseEventKind::Down(_)))
            .then_some(Action::ToggleHelp)
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        if let Action::ToggleHelp = action {
            self.visible = !self.visible;
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }
        let sections = keymap::SECTIONS.iter().map(|(section, bindings)| {
            let keys: Vec<(String, &str)> = bindings
                .iter()
                .map(|b| (keymap::label(b), b.help))
                .collect();
            (*section, keys)
        });
        let mouse = MOUSE
            .iter()
            .map(|(k, what)| (k.to_string(), *what))
            .collect();
        let mut rows = Vec::new();
        for (i, (section, keys)) in sections.chain([("Mouse", mouse)]).enumerate() {
            if i > 0 {
                rows.push(Row::new([""]));
            }
            rows.push(Row::new([Span::styled(
                section,
                Style::new()
                    .fg(theme::colors().yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
            rows.extend(keys.into_iter().map(|(key, what)| {
                Row::new([
                    Span::styled(format!("  {key}"), theme::accent_bold()),
                    Span::styled(what, Style::new().fg(theme::colors().fg)),
                ])
            }));
        }
        let height = rows.len() as u16 + 4;
        let area = area.centered(Constraint::Max(76), Constraint::Length(height));
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(
                format!(" {}  Keys ", theme::keyboard()),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(" any key to close ", theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        let table = Table::new(rows, [Constraint::Length(16), Constraint::Fill(1)]).block(block);
        frame.render_widget(Clear, area);
        frame.render_widget(table, area);
    }
}
