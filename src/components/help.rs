//! Key binding overlay, toggled with `?`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Padding, Row, Table};

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
        let mut sections: Vec<Section> = keymap::SECTIONS
            .iter()
            .map(|(title, bindings)| Section {
                title,
                keys: bindings
                    .iter()
                    .map(|b| (keymap::label(b), b.help))
                    .collect(),
            })
            .collect();
        sections.push(Section {
            title: "Mouse",
            keys: MOUSE
                .iter()
                .map(|(k, what)| (k.to_string(), *what))
                .collect(),
        });
        // Two columns side by side where the terminal is wide enough, so it stays short.
        let columns = if area.width >= 2 * COLUMN + 2 { 2 } else { 1 };
        let width = (area.width.saturating_sub(4) / columns).min(COLUMN - 4);
        let text = (width.saturating_sub(KEYS + 1) as usize).max(10);
        let halves = split(&sections, columns as usize, text);
        let tables: Vec<Vec<Row>> = halves.iter().map(|part| rows(part, text)).collect();
        let tallest = halves
            .iter()
            .map(|part| part.iter().map(|s| s.lines(text)).sum::<usize>())
            .max()
            .unwrap_or(0)
            // The first section has no blank line above it.
            .saturating_sub(1) as u16;
        let area = area.centered(
            Constraint::Max(columns * (width + 1) + 3),
            Constraint::Length(tallest + 4),
        );
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(
                format!(
                    " {}  Keys  ·  ilias-tui v{} ",
                    theme::keyboard(),
                    crate::VERSION
                ),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(" any key to close ", theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let parts = Layout::horizontal(vec![Constraint::Fill(1); tables.len()])
            .spacing(1)
            .split(inner);
        for (rows, part) in tables.into_iter().zip(parts.iter()) {
            let table = Table::new(rows, [Constraint::Length(KEYS), Constraint::Fill(1)]);
            frame.render_widget(table, *part);
        }
    }
}

/// The widest a column of the overlay gets, and the room for the keys in it.
const COLUMN: u16 = 76;
const KEYS: u16 = 16;

/// A group of keys: its title (`Edit · where ILIAS lets you` puts a note after the dot) and
/// the keys with what they do.
struct Section<'a> {
    title: &'a str,
    keys: Vec<(String, &'a str)>,
}

impl Section<'_> {
    /// Lines it takes: a blank, the title, then each key with its text wrapped to `text`.
    fn lines(&self, text: usize) -> usize {
        2 + self
            .keys
            .iter()
            .map(|(_, what)| wrap(what, text).len())
            .sum::<usize>()
    }
}

/// The sections in `columns` runs of about equal height, in order.
fn split<'a, 'b>(
    sections: &'b [Section<'a>],
    columns: usize,
    text: usize,
) -> Vec<&'b [Section<'a>]> {
    if columns < 2 {
        return vec![sections];
    }
    let total: usize = sections.iter().map(|s| s.lines(text)).sum();
    let mut lines = 0;
    let at = sections
        .iter()
        .position(|s| {
            lines += s.lines(text);
            lines * 2 >= total
        })
        .map_or(sections.len(), |i| i + 1);
    vec![&sections[..at], &sections[at..]]
}

fn rows<'a>(sections: &[Section<'a>], text: usize) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            rows.push(Row::new([""]));
        }
        let (name, note) = section
            .title
            .split_once(" · ")
            .map_or((section.title, None), |(n, note)| (n, Some(note)));
        let name = Span::styled(
            name,
            Style::new()
                .fg(theme::colors().yellow)
                .add_modifier(Modifier::BOLD),
        );
        // A note goes beside the name, in the text column.
        let note = Span::styled(note.unwrap_or_default(), theme::muted());
        rows.push(Row::new([Cell::from(name), Cell::from(note)]));
        for (key, what) in &section.keys {
            let lines = wrap(what, text);
            let height = lines.len() as u16;
            let text: Vec<Line> = lines
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(theme::colors().fg)))
                .collect();
            rows.push(
                Row::new([
                    Cell::from(Span::styled(format!("  {key}"), theme::accent_bold())),
                    Cell::from(Text::from(text)),
                ])
                .height(height),
            );
        }
    }
    rows
}

/// `text` in lines of at most `width` characters, broken between words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let line = lines.last_mut().expect("never empty");
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(word.to_string());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn shown(width: u16, height: u16) -> Vec<String> {
        let mut help = Help::default();
        help.update(&Action::ToggleHelp);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| help.draw(f, f.area())).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn every_key_and_its_whole_text_fits() {
        for (width, height) in [(80, 70), (170, 40)] {
            let screen = shown(width, height).join("\n");
            for (title, bindings) in keymap::SECTIONS {
                assert!(
                    screen.contains(title.split(" · ").next().unwrap()),
                    "{title}"
                );
                for binding in *bindings {
                    // Wrapped texts continue on the next line: each word is there.
                    for word in binding.help.split(' ') {
                        assert!(screen.contains(word), "{width}: {word} of {}", binding.help);
                    }
                }
            }
            assert!(
                screen.contains("where ILIAS lets you, asks first"),
                "{screen}"
            );
        }
    }

    #[test]
    fn a_wide_terminal_gets_two_columns_and_a_short_overlay() {
        let narrow = shown(80, 70);
        let wide = shown(170, 40);
        let used = |screen: &[String]| screen.iter().filter(|l| !l.trim().is_empty()).count();
        assert!(used(&wide) < used(&narrow) - 15, "{}", wide.join("\n"));
        // Navigate on the left, App on the right, on the same lines.
        assert!(wide.iter().any(|l| l.contains("Navigate")));
        let navigate = wide.iter().position(|l| l.contains("Navigate")).unwrap();
        assert!(!wide[navigate].contains("App"));
    }

    #[test]
    fn wraps_between_words() {
        assert_eq!(
            wrap("Move the selected item", 10),
            ["Move the", "selected", "item"]
        );
        assert_eq!(wrap("short", 10), ["short"]);
    }
}
