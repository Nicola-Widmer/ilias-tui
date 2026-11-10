//! The sync report: after a sync that skipped big files, saved new versions beside your
//! edited copies or had failures, it lists them. Scrolls with j/k; any other key closes it.
//!
//! After a two-way sync with something to send it is the **review**: what Enter sends
//! (uploads, replaced and renamed files), the files you deleted as checkboxes, ticked
//! for the ILIAS trash unless you untick them (Space), and everything else that stays as
//! it is. It is the question before
//! sending: Enter sends ([`Action::AskPush`], which asks once more only for a lot at once);
//! Esc closes it and nothing is sent. In a dry run
//! it is a plain report.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::keymap::{self, Command};
use crate::sync::{Change, PROPOSALS};
use crate::theme;

#[derive(Default)]
pub struct Report {
    /// The report's sections while it is open.
    sections: Option<Vec<(String, Vec<String>)>>,
    scroll: u16,
    /// Review: what Enter sends besides the ticked proposals.
    send: Vec<Change>,
    /// Review: the proposed deletions and whether each is ticked.
    proposals: Vec<(Change, bool)>,
    /// Review: the proposal under the cursor.
    cursor: usize,
}

impl Report {
    fn scroll_by(&mut self, delta: i32) -> Option<Action> {
        self.scroll = (i32::from(self.scroll) + delta).clamp(0, u16::MAX.into()) as u16;
        Some(Action::Handled)
    }

    fn close(&mut self) -> Option<Action> {
        *self = Self::default();
        Some(Action::Handled)
    }

    /// A review with something to send or tick; else a plain report.
    fn reviewing(&self) -> bool {
        !self.send.is_empty() || !self.proposals.is_empty()
    }

    fn review_key(&mut self, key: KeyEvent) -> Option<Action> {
        let last = self.proposals.len().saturating_sub(1);
        match keymap::review_command(key) {
            Some(Command::Down) if self.proposals.is_empty() => self.scroll_by(1),
            Some(Command::Up) if self.proposals.is_empty() => self.scroll_by(-1),
            Some(Command::Down) => {
                self.cursor = (self.cursor + 1).min(last);
                Some(Action::Handled)
            }
            Some(Command::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                Some(Action::Handled)
            }
            Some(Command::Tick) => {
                if let Some((_, ticked)) = self.proposals.get_mut(self.cursor) {
                    *ticked = !*ticked;
                }
                Some(Action::Handled)
            }
            Some(Command::Send) => {
                let mut changes = std::mem::take(&mut self.send);
                changes.extend(
                    self.proposals
                        .drain(..)
                        .filter(|(_, ticked)| *ticked)
                        .map(|(change, _)| change),
                );
                *self = Self::default();
                Some(match changes.is_empty() {
                    true => Action::Cancelled,
                    false => Action::AskPush(changes),
                })
            }
            Some(Command::Back) => {
                *self = Self::default();
                Some(Action::Cancelled)
            }
            _ => match key.code {
                KeyCode::PageDown => self.scroll_by(10),
                KeyCode::PageUp => self.scroll_by(-10),
                // Nothing closes a review by accident.
                _ => Some(Action::Handled),
            },
        }
    }

    /// The lines to show, and on which of them each proposal is.
    fn lines(&self) -> (Vec<Line<'static>>, Vec<usize>) {
        let mut lines = Vec::new();
        let mut rows = Vec::new();
        let colors = theme::colors();
        if self.reviewing() {
            lines.push(Line::styled(
                "Enter sends all of it; files you deleted go to the ILIAS trash unless you untick them (Space). Esc sends nothing.",
                theme::muted(),
            ));
        }
        for (title, items) in self.sections.iter().flatten() {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::styled(
                format!("{title} ({})", items.len()),
                Style::new().fg(colors.yellow).add_modifier(Modifier::BOLD),
            ));
            if title == PROPOSALS && !self.proposals.is_empty() {
                for (i, (item, (_, ticked))) in items.iter().zip(&self.proposals).enumerate() {
                    let mark = if *ticked { "[x]" } else { "[ ]" };
                    let mut style = Style::new().fg(colors.fg);
                    if i == self.cursor {
                        style = style.bg(colors.selected_bg).add_modifier(Modifier::BOLD);
                    }
                    rows.push(lines.len());
                    lines.push(Line::styled(format!("  {mark} {item}"), style));
                }
                continue;
            }
            lines.extend(
                items
                    .iter()
                    .map(|item| Line::styled(format!("  {item}"), Style::new().fg(colors.fg))),
            );
        }
        (lines, rows)
    }
}

impl Component for Report {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        self.sections.as_ref()?;
        if self.reviewing() {
            return self.review_key(key);
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(10),
            KeyCode::PageUp => self.scroll_by(-10),
            _ => self.close(),
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        self.sections.as_ref()?;
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(1),
            MouseEventKind::ScrollUp => self.scroll_by(-1),
            // A click is not a "send".
            MouseEventKind::Down(_) if self.reviewing() => {
                *self = Self::default();
                Some(Action::Cancelled)
            }
            MouseEventKind::Down(_) => self.close(),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        match action {
            Action::Synced { report, review, .. } if !report.is_empty() => {
                *self = Self {
                    sections: Some(report.clone()),
                    ..Self::default()
                };
                if !review.dry_run {
                    self.send = review.sendable();
                    self.proposals = review.proposals().map(|c| (c.clone(), true)).collect();
                }
            }
            Action::Pushed { report, .. } if !report.is_empty() => {
                *self = Self {
                    sections: Some(report.clone()),
                    ..Self::default()
                };
            }
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if self.sections.is_none() {
            return;
        }
        let (lines, rows) = self.lines();
        let width = area.width.min(110);
        // Rows after wrapping (the text is 6 narrower: borders and padding).
        let wrapped = |line: &Line<'static>| {
            Paragraph::new(line.clone())
                .wrap(Wrap { trim: false })
                .line_count(width.saturating_sub(6))
                .max(1) as u16
        };
        let starts: Vec<u16> = lines
            .iter()
            .scan(0, |at, line| {
                let start = *at;
                *at += wrapped(line);
                Some(start)
            })
            .collect();
        let total = lines.iter().map(wrapped).sum::<u16>();
        let height = (total + 4).min(area.height.saturating_sub(2));
        let area = area.centered(Constraint::Length(width), Constraint::Length(height));
        let visible = height.saturating_sub(4);
        // Keep the proposal under the cursor on screen; don't scroll past the end.
        if let Some(&row) = rows.get(self.cursor).and_then(|&line| starts.get(line)) {
            self.scroll = self.scroll.min(row).max((row + 1).saturating_sub(visible));
        }
        self.scroll = self.scroll.min(total.saturating_sub(visible));
        let (title, hint) = if self.reviewing() {
            (
                " Sync review ",
                " j/k move · Space untick / tick for the trash · Enter send · Esc send nothing ",
            )
        } else {
            (" Sync report ", " j/k scroll · any other key closes ")
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(title, theme::accent_bold()))
            .title_bottom(Line::styled(hint, theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(block),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::ilias::{Id, Item};
    use crate::sync::Review;

    fn synced(report: Vec<(String, Vec<String>)>) -> Action {
        Action::Synced {
            summary: String::new(),
            report,
            saved: Vec::new(),
            review: Box::default(),
        }
    }

    fn key(r: &mut Report, code: KeyCode) -> Option<Action> {
        r.handle_key_event(KeyEvent::from(code))
    }

    fn screen(report: &mut Report) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| report.draw(f, f.area())).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn opens_only_with_something_to_report() {
        let mut report = Report::default();
        report.update(&synced(Vec::new()));
        assert!(
            key(&mut report, KeyCode::Char('j')).is_none(),
            "nothing to show"
        );

        let big: Vec<String> = (1..=40)
            .map(|i| format!("Lecture {i} (107.9 MB)"))
            .collect();
        report.update(&synced(vec![("Skipped: bigger than 50 MB".into(), big)]));
        assert!(screen(&mut report).contains("Skipped: bigger than 50 MB (40)"));

        // j scrolls (and is used up), any other key closes.
        assert!(matches!(
            key(&mut report, KeyCode::Char('j')),
            Some(Action::Handled)
        ));
        assert_eq!(report.scroll, 1);
        assert!(matches!(
            key(&mut report, KeyCode::Char('q')),
            Some(Action::Handled)
        ));
        assert!(key(&mut report, KeyCode::Char('j')).is_none(), "closed");
    }

    fn folder() -> Item {
        Item {
            kind: "fold".into(),
            id: Id::Ref(89),
            title: "Unterlagen".into(),
            ..Item::default()
        }
    }

    fn trash(n: u64) -> Change {
        Change::Trash {
            place: folder(),
            file: Item {
                kind: "file".into(),
                id: Id::Ref(n),
                ..Item::default()
            },
            path: PathBuf::from(format!("/dl/Unterlagen/Blatt {n}.pdf")),
            base: "v1".into(),
        }
    }

    /// A review of one upload and two proposed deletions, as `sync` reports it.
    fn review(dry_run: bool) -> Action {
        let upload = Change::Upload {
            place: folder(),
            path: PathBuf::from("/dl/Unterlagen/Notizen.md"),
            renamed_from: None,
        };
        Action::Synced {
            summary: String::new(),
            report: vec![
                (
                    "New on your disk: to upload".into(),
                    vec!["Unterlagen/Notizen.md".into()],
                ),
                (
                    PROPOSALS.into(),
                    vec![
                        "Unterlagen/Blatt 1.pdf".into(),
                        "Unterlagen/Blatt 2.pdf".into(),
                    ],
                ),
                (
                    "Gone on ILIAS, kept on your disk".into(),
                    vec!["Unterlagen/Alt.pdf".into()],
                ),
            ],
            saved: Vec::new(),
            review: Box::new(Review {
                changes: vec![upload, trash(1), trash(2)],
                dry_run,
                ..Review::default()
            }),
        }
    }

    #[test]
    fn the_review_sends_everything_but_the_unticked_deletions() {
        let mut report = Report::default();
        report.update(&review(false));
        let shown = screen(&mut report);
        assert!(shown.contains("Sync review"), "{shown}");
        assert!(shown.contains("[x] Unterlagen/Blatt 1.pdf"), "{shown}");
        assert!(shown.contains("[x] Unterlagen/Blatt 2.pdf"), "{shown}");
        assert!(shown.contains("Unterlagen/Alt.pdf"), "{shown}");
        // Every deletion ticked by default; the second unticked.
        key(&mut report, KeyCode::Char('j'));
        key(&mut report, KeyCode::Char(' '));
        assert!(screen(&mut report).contains("[ ] Unterlagen/Blatt 2.pdf"));
        // Other keys don't close it.
        assert!(matches!(
            key(&mut report, KeyCode::Char('x')),
            Some(Action::Handled)
        ));
        let Some(Action::AskPush(changes)) = key(&mut report, KeyCode::Enter) else {
            panic!("Enter goes on to the question");
        };
        assert_eq!(changes.len(), 2);
        assert!(matches!(changes[0], Change::Upload { .. }));
        assert_eq!(changes[1], trash(1));
        assert!(key(&mut report, KeyCode::Char('j')).is_none(), "closed");
    }

    #[test]
    fn closing_the_review_sends_nothing_and_enter_sends_the_deletions_too() {
        let mut report = Report::default();
        report.update(&review(false));
        assert!(matches!(
            key(&mut report, KeyCode::Esc),
            Some(Action::Cancelled)
        ));
        // Enter right away sends the deletions too.
        report.update(&review(false));
        let Some(Action::AskPush(changes)) = key(&mut report, KeyCode::Enter) else {
            panic!("Enter goes on to the question");
        };
        assert_eq!(
            changes
                .iter()
                .filter(|c| matches!(c, Change::Trash { .. }))
                .count(),
            2
        );
        // A click is not a yes.
        report.update(&review(false));
        let click = MouseEvent {
            kind: MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        assert!(matches!(
            report.handle_mouse_event(click),
            Some(Action::Cancelled)
        ));
    }

    #[test]
    fn a_dry_run_review_is_a_plain_report() {
        let mut report = Report::default();
        report.update(&review(true));
        let shown = screen(&mut report);
        assert!(shown.contains("Sync report"), "{shown}");
        assert!(!shown.contains("[ ]"), "{shown}");
        assert!(matches!(
            key(&mut report, KeyCode::Enter),
            Some(Action::Handled)
        ));
        assert!(
            key(&mut report, KeyCode::Enter).is_none(),
            "closed, nothing sent"
        );
    }
}
