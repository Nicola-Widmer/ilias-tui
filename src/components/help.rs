//! Key binding overlay, toggled with `?`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Row, Table};

use super::Component;
use crate::action::Action;
use crate::theme;

/// Key bindings by section, shown in the overlay.
pub const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigate",
        &[
            ("↑ ↓  k j", "Move selection"),
            ("g  G", "First / last item"),
            (
                "Enter  →  l",
                "Open folder or file (downloads it first if needed)",
            ),
            ("Esc  ←  h", "Clear filter, then go back"),
            ("Tab  S-Tab", "Next / previous tab (Inhalt, Info, …)"),
            ("Tab", "My courses: into / out of the To-Do panel"),
        ],
    ),
    (
        "Search",
        &[
            ("/", "Filter this list (Enter opens the match, Esc clears)"),
            ("s", "Search everything below here"),
        ],
    ),
    (
        "Items",
        &[
            ("d", "Download a fresh copy (no open)"),
            ("o", "Open in your web browser"),
            ("f", "Add to / remove from ILIAS favourites (asks first)"),
            ("r", "Reload"),
            ("Space", "Mark a course / folder for sync"),
            (
                "S",
                "Sync files (≤ 50 MB) of the marked or selected courses",
            ),
        ],
    ),
    (
        "Mouse",
        &[
            ("click", "Select; double-click opens"),
            ("wheel", "Move selection"),
            ("tab / crumb", "Click to switch tab / go back"),
        ],
    ),
    (
        "App",
        &[
            ("L", "Log in again / go online (opens Chrome if needed)"),
            ("c", "Settings (download folder)"),
            ("?", "Toggle this help"),
            ("q  Ctrl-c", "Quit"),
        ],
    ),
];

#[derive(Default)]
pub struct Help {
    visible: bool,
}

impl Component for Help {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        // While open, the overlay swallows every key; any key closes it.
        (self.visible || key.code == KeyCode::Char('?')).then_some(Action::ToggleHelp)
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
        let mut rows = Vec::new();
        for (i, (section, keys)) in KEYS.iter().enumerate() {
            if i > 0 {
                rows.push(Row::new([""]));
            }
            rows.push(Row::new([Span::styled(
                *section,
                Style::new().fg(theme::YELLOW).add_modifier(Modifier::BOLD),
            )]));
            rows.extend(keys.iter().map(|(key, what)| {
                Row::new([
                    Span::styled(format!("  {key}"), theme::accent_bold()),
                    Span::styled(*what, Style::new().fg(theme::FG)),
                ])
            }));
        }
        let height = rows.len() as u16 + 4;
        let area = area.centered(Constraint::Max(76), Constraint::Length(height));
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::ACCENT))
            .title(Line::styled(
                format!(" {}  Keys ", theme::keyboard()),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(" any key to close ", theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(Style::new().bg(theme::BAR_BG));
        let table = Table::new(rows, [Constraint::Length(16), Constraint::Fill(1)]).block(block);
        frame.render_widget(Clear, area);
        frame.render_widget(table, area);
    }
}
