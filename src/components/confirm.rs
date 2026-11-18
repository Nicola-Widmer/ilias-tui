//! A yes/no dialog in front of everything else. Actions that change data on ILIAS go
//! through it ([`Action::Ask`]).

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::theme;

#[derive(Default)]
pub struct Confirm {
    /// The question and what to do on "yes".
    pending: Option<(String, Action)>,
}

impl Component for Confirm {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        // While open, the dialog swallows every key.
        let (_, then) = self.pending.as_ref()?;
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                let then = then.clone();
                self.pending = None;
                Some(then)
            }
            KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => {
                self.pending = None;
                Some(Action::Cancelled)
            }
            // Other keys keep the dialog open but must not reach the list behind it.
            _ => Some(Action::Progress("Press y or n".into())),
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        self.pending.as_ref()?;
        // A click is not a "yes".
        matches!(mouse.kind, MouseEventKind::Down(_)).then(|| {
            self.pending = None;
            Action::Cancelled
        })
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        if let Action::Ask { question, then } = action {
            self.pending = Some((question.clone(), (**then).clone()));
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let Some((question, then)) = &self.pending else {
            return;
        };
        // What kind of question it is: its icon, title and colour.
        let colors = theme::colors();
        let (icon, title, color) = match then {
            Action::Sync(_) => (theme::sync(), "Sync", colors.accent),
            Action::Push(_) => (theme::send(), "Send to ILIAS", colors.yellow),
            Action::TrashLocal(_) => (theme::trash(), "Your Trash", colors.orange),
            Action::OpenLink(_) => (theme::external(), "Another site", colors.accent),
            _ => (theme::warning(), "Changes ILIAS", colors.yellow),
        };
        let bold = Style::new().fg(color).add_modifier(Modifier::BOLD);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(color))
            .title(Line::styled(format!(" {icon}  {title} "), bold))
            .padding(Padding::horizontal(2))
            .style(Style::new().bg(colors.bar_bg).fg(colors.fg));
        // The first line is the question; any further lines are what it is about.
        let mut parts = question.lines();
        let mut text = vec![
            Line::raw(""),
            Line::styled(
                parts.next().unwrap_or_default().to_string(),
                Style::new().fg(colors.fg).add_modifier(Modifier::BOLD),
            ),
        ];
        let items: Vec<Line> = parts
            .map(|item| {
                Line::from(vec![
                    Span::styled(format!("  {} ", theme::bullet()), Style::new().fg(color)),
                    Span::styled(item.to_string(), Style::new().fg(colors.fg)),
                ])
            })
            .collect();
        if !items.is_empty() {
            text.push(Line::raw(""));
            text.extend(items);
        }
        let button = |key: &str, label: &str, bg| {
            Span::styled(
                format!(" {key}  {label} "),
                Style::new()
                    .fg(colors.bar_bg)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )
        };
        text.extend([
            Line::raw(""),
            Line::from(vec![
                button("y", "Yes", color),
                Span::raw("   "),
                button("n", "No", colors.muted),
            ])
            .centered(),
        ]);
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: true }).block(block);
        // Long questions wrap: make the box tall enough that the y/n line stays visible
        // (the width inside the borders and padding is 6 less).
        let width = area.width.min(76);
        let height = paragraph.line_count(width.saturating_sub(6)) as u16;
        let area = area.centered(Constraint::Length(width), Constraint::Length(height));
        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Item;

    fn ask(confirm: &mut Confirm) {
        confirm.update(&Action::Ask {
            question: "Add “DEMO” to your ILIAS favourites?".into(),
            then: Box::new(Action::ToggleFavorite(Item::default())),
        });
    }

    #[test]
    fn sync_questions_are_not_titled_as_changes() {
        let mut confirm = Confirm::default();
        confirm.update(&Action::Ask {
            question: "Sync all 2 courses?".into(),
            then: Box::new(Action::Sync(vec![])),
        });
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| confirm.draw(f, f.area())).unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(
            screen.contains("Sync") && !screen.contains("Changes ILIAS"),
            "{screen}"
        );
    }

    #[test]
    fn long_questions_still_show_the_answers() {
        let mut confirm = Confirm::default();
        let rules = crate::sync::Rules {
            max_mb: 50.0,
            missing_only: true,
            two_way: false,
        };
        confirm.update(&Action::Ask {
            question: format!("Sync all 17 courses in My courses? {}", rules.describe()),
            then: Box::new(Action::Sync(vec![])),
        });
        for width in [100, 50] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 20)).unwrap();
            terminal.draw(|f| confirm.draw(f, f.area())).unwrap();
            let screen = format!("{:?}", terminal.backend().buffer());
            // The whole question (its last word) and the answers below it.
            assert!(screen.contains("are."), "{screen}");
            assert!(
                screen.contains("y  Yes") && screen.contains("n  No"),
                "{screen}"
            );
        }
    }

    #[test]
    fn runs_the_action_only_on_yes() {
        let key = |c: &mut Confirm, code| c.handle_key_event(KeyEvent::from(code));
        let mut confirm = Confirm::default();
        assert!(
            key(&mut confirm, KeyCode::Char('y')).is_none(),
            "nothing asked yet"
        );

        ask(&mut confirm);
        assert!(matches!(
            key(&mut confirm, KeyCode::Char('j')),
            Some(Action::Progress(_))
        ));
        assert!(matches!(
            key(&mut confirm, KeyCode::Char('n')),
            Some(Action::Cancelled)
        ));
        assert!(confirm.pending.is_none());

        ask(&mut confirm);
        assert!(matches!(
            key(&mut confirm, KeyCode::Char('y')),
            Some(Action::ToggleFavorite(_))
        ));
        assert!(confirm.pending.is_none());
    }
}
