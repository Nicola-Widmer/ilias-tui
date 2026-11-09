//! The dialog for handing in a file: asks which one, then leaves the rest to the usual
//! question ([`Action::Ask`]), so nothing is sent before a clear "yes".

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::Action;
use crate::config;
use crate::ilias::Item;
use crate::theme;

struct Open {
    assignment: Item,
    url: String,
    input: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct HandIn {
    open: Option<Open>,
}

/// A path as typed, or as a terminal pastes a file dropped on it: quoted, or with
/// backslashes in front of spaces.
fn parse_path(input: &str) -> Result<PathBuf, String> {
    let input = input.trim();
    let unquoted = ['\'', '"']
        .iter()
        .find_map(|q| input.strip_prefix(*q)?.strip_suffix(*q));
    let path = match unquoted {
        Some(path) => path.to_string(),
        None => input.replace("\\ ", " "),
    };
    let path = config::parse_dir(&path).map_err(|_| "Use a full path, e.g. ~/Uni/sheet-3.pdf")?;
    let meta = std::fs::metadata(&path).map_err(|_| format!("No file at {}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() == 0 {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(path)
}

/// `Hand in "sheet-3.pdf" (1.2 MB) for Blatt 3? …`.
fn question(assignment: &Item, path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let size = std::fs::metadata(path).map_or(0, |m| m.len()) as f64 / 1_000_000.0;
    format!(
        "Hand in \"{name}\" ({size:.1} MB) for {}? Your lecturer can see it.",
        assignment.title
    )
}

impl Component for HandIn {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        let open = self.open.as_mut()?;
        open.error = None;
        match key.code {
            KeyCode::Esc => {
                self.open = None;
                return Some(Action::Cancelled);
            }
            KeyCode::Enter => match parse_path(&open.input) {
                Ok(path) => {
                    let question = question(&open.assignment, &path);
                    let then = Box::new(Action::HandIn {
                        assignment: open.assignment.clone(),
                        url: open.url.clone(),
                        path,
                    });
                    self.open = None;
                    return Some(Action::Ask { question, then });
                }
                Err(error) => open.error = Some(error),
            },
            KeyCode::Backspace => {
                open.input.pop();
            }
            KeyCode::Char(c) => open.input.push(c),
            _ => {}
        }
        Some(Action::Handled)
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        if let Action::AskHandIn { assignment, url } = action {
            self.open = Some(Open {
                assignment: assignment.clone(),
                url: url.clone(),
                input: String::new(),
                error: None,
            });
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let Some(open) = &self.open else {
            return;
        };
        let fg = Style::new().fg(theme::colors().fg);
        let mut text = vec![
            Line::styled("File to hand in (or drop it here):", theme::muted()),
            Line::from(vec![
                Span::styled(open.input.clone(), fg),
                Span::styled("▏", Style::new().fg(theme::colors().accent)),
            ]),
        ];
        if let Some(error) = &open.error {
            text.push(Line::default());
            text.push(Line::styled(
                error.clone(),
                Style::new().fg(theme::colors().red),
            ));
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().accent))
            .title(Line::styled(
                format!(" Hand in: {} ", open.assignment.title),
                theme::accent_bold(),
            ))
            .title_bottom(Line::styled(" ↵ continue  esc cancel ", theme::muted()).centered())
            .padding(Padding::uniform(1))
            .style(
                Style::new()
                    .bg(theme::colors().bar_bg)
                    .fg(theme::colors().fg),
            );
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false }).block(block);
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

    fn dialog() -> HandIn {
        let mut dialog = HandIn::default();
        dialog.update(&Action::AskHandIn {
            assignment: Item {
                title: "Blatt 1".into(),
                ..Item::default()
            },
            url: "u".into(),
        });
        dialog
    }

    fn typed(dialog: &mut HandIn, text: &str) -> Option<Action> {
        for c in text.chars() {
            dialog.handle_key_event(KeyCode::Char(c).into());
        }
        dialog.handle_key_event(KeyCode::Enter.into())
    }

    #[test]
    fn reads_typed_quoted_and_dropped_paths() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-handin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("my sheet.pdf");
        std::fs::write(&file, "x").unwrap();
        let shown = file.display().to_string();
        assert_eq!(parse_path(&shown), Ok(file.clone()));
        assert_eq!(parse_path(&format!(" '{shown}' ")), Ok(file.clone()));
        assert_eq!(parse_path(&shown.replace(' ', "\\ ")), Ok(file.clone()));

        assert!(parse_path("sheet.pdf").is_err(), "relative");
        assert!(parse_path(&dir.display().to_string()).is_err(), "a folder");
        assert!(parse_path(&dir.join("nope").display().to_string()).is_err());
        let empty = dir.join("empty.pdf");
        std::fs::write(&empty, "").unwrap();
        assert!(parse_path(&empty.display().to_string()).is_err());

        // A good path leads to the question, not to the upload.
        let mut dialog = dialog();
        let action = typed(&mut dialog, &shown);
        let Some(Action::Ask { question, then }) = action else {
            panic!("expected the question");
        };
        assert!(question.contains("my sheet.pdf"), "{question}");
        assert!(question.contains("Blatt 1"), "{question}");
        assert!(matches!(*then, Action::HandIn { ref path, .. } if *path == file));
        assert!(dialog.open.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_path_stays_open_and_esc_cancels() {
        let mut dialog = dialog();
        assert!(matches!(
            typed(&mut dialog, "/no/such/file.pdf"),
            Some(Action::Handled)
        ));
        assert!(dialog.open.as_ref().unwrap().error.is_some());
        assert!(matches!(
            dialog.handle_key_event(KeyCode::Esc.into()),
            Some(Action::Cancelled)
        ));
        assert!(
            dialog.handle_key_event(KeyCode::Esc.into()).is_none(),
            "closed"
        );
    }
}
