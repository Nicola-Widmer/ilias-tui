//! The sync report: after a sync that skipped big files, saved new versions beside your
//! edited copies or had failures, it lists them. Scrolls with j/k; any other key closes it.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::theme;

#[derive(Default)]
pub struct Report {
    /// The report's lines while it is open.
    lines: Option<Vec<Line<'static>>>,
    scroll: u16,
}

impl Report {
    fn scroll_by(&mut self, delta: i32) -> Option<Action> {
        self.scroll = (i32::from(self.scroll) + delta).clamp(0, u16::MAX.into()) as u16;
        Some(Action::Handled)
    }

    fn close(&mut self) -> Option<Action> {
        self.lines = None;
        Some(Action::Handled)
    }
}

impl Component for Report {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        self.lines.as_ref()?;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(10),
            KeyCode::PageUp => self.scroll_by(-10),
            _ => self.close(),
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        self.lines.as_ref()?;
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(1),
            MouseEventKind::ScrollUp => self.scroll_by(-1),
            MouseEventKind::Down(_) => self.close(),
            _ => None,
        }
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        if let Action::Synced { report, .. } | Action::Pushed { report, .. } = action
            && !report.is_empty()
        {
            let mut lines = Vec::new();
            for (title, items) in report {
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::styled(
                    format!("{title} ({})", items.len()),
                    Style::new()
                        .fg(theme::colors().yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                lines.extend(items.iter().map(|item| {
                    Line::styled(format!("  {item}"), Style::new().fg(theme::colors().fg))
                }));
            }
            self.lines = Some(lines);
            self.scroll = 0;
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let Some(lines) = &self.lines else {
            return;
        };
        let height = (lines.len() as u16 + 4).min(area.height.saturating_sub(2));
        let area = area.centered(Constraint::Max(110), Constraint::Length(height));
        // Don't scroll past the end.
        let visible = height.saturating_sub(4);
        self.scroll = self
            .scroll
            .min((lines.len() as u16).saturating_sub(visible));
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(" Sync report ", theme::accent_bold()))
            .title_bottom(
                Line::styled(" j/k scroll · any other key closes ", theme::muted()).centered(),
            )
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines.clone())
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(block),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(report: Vec<(String, Vec<String>)>) -> Action {
        Action::Synced {
            summary: String::new(),
            report,
            saved: Vec::new(),
            review: Box::default(),
        }
    }

    #[test]
    fn opens_only_with_something_to_report() {
        let key = |r: &mut Report, code| r.handle_key_event(KeyEvent::from(code));
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
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
        terminal.draw(|f| report.draw(f, f.area())).unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(
            screen.contains("Skipped: bigger than 50 MB (40)"),
            "{screen}"
        );

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
}
