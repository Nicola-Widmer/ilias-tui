//! A one-line question for a bit of text: a new folder's name, or a forum post's title and
//! text when there is no `$EDITOR`. Like the file dialog,
//! it only leads to the usual question ([`Action::Ask`]), so nothing is sent before a clear
//! "yes".

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::Component;
use crate::action::{Action, Draft};
use crate::ilias::Item;
use crate::theme;

/// What the text is for.
enum Purpose {
    /// The name of a new folder in `container`, which is at `place`.
    FolderName { container: Item, place: String },
    /// The title of a new thread.
    PostTitle { draft: Draft },
    /// The text of a post (`title` is empty for a reply).
    PostText { draft: Draft, title: String },
}

struct Open {
    purpose: Purpose,
    input: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct Prompt {
    open: Option<Open>,
}

/// ILIAS takes titles up to this many characters.
const MAX_TITLE: usize = 255;

impl Open {
    /// The question that follows the text, or what is wrong with it.
    fn ask(&self) -> Result<Action, String> {
        let text = self.input.trim();
        match &self.purpose {
            Purpose::FolderName { container, place } => {
                if text.is_empty() {
                    return Err("Type a name for the folder".into());
                }
                if text.chars().count() > MAX_TITLE {
                    return Err(format!("At most {MAX_TITLE} characters"));
                }
                Ok(Action::Ask {
                    question: format!(
                        "Create the folder “{text}” in {place}? Everyone who can open it sees it."
                    ),
                    then: Box::new(Action::NewFolder {
                        container: container.clone(),
                        name: text.to_string(),
                    }),
                })
            }
            _ if text.is_empty() => Err("Type something, or Esc to cancel".into()),
            Purpose::PostTitle { draft } => Ok(Action::AskPostText {
                draft: draft.clone(),
                title: Some(text.to_string()),
            }),
            Purpose::PostText { draft, title } => Ok(crate::compose::question(
                draft.clone(),
                title.clone(),
                text.to_string(),
            )),
        }
    }

    /// Title and label of the dialog.
    fn labels(&self) -> (String, &'static str) {
        match &self.purpose {
            Purpose::FolderName { place, .. } => (format!(" New folder in {place} "), "Name:"),
            Purpose::PostTitle {
                draft: Draft::Thread { place, .. },
            } => (
                format!(" New thread in {place} "),
                "Title (set $EDITOR to write in your editor):",
            ),
            Purpose::PostTitle { .. } => (" New post ".into(), "Title:"),
            Purpose::PostText {
                draft: Draft::Reply { author, .. },
                ..
            } => (
                format!(" Reply to {author} "),
                "Text, one line (set $EDITOR to write more):",
            ),
            Purpose::PostText { title, .. } => (
                format!(" {title} "),
                "Text, one line (set $EDITOR to write more):",
            ),
        }
    }
}

impl Component for Prompt {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        let open = self.open.as_mut()?;
        open.error = None;
        match key.code {
            KeyCode::Esc => {
                self.open = None;
                return Some(Action::Cancelled);
            }
            KeyCode::Enter => match open.ask() {
                Ok(ask) => {
                    self.open = None;
                    return Some(ask);
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
        let purpose = match action {
            Action::AskFolderName { container, place } => Purpose::FolderName {
                container: container.clone(),
                place: place.clone(),
            },
            // A reply has no title of its own; a new thread asks for it first.
            Action::AskPostText { draft, title } => match (draft, title) {
                (Draft::Thread { .. }, None) => Purpose::PostTitle {
                    draft: draft.clone(),
                },
                (_, title) => Purpose::PostText {
                    draft: draft.clone(),
                    title: title.clone().unwrap_or_default(),
                },
            },
            _ => return None,
        };
        self.open = Some(Open {
            purpose,
            input: String::new(),
            error: None,
        });
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let Some(open) = &self.open else {
            return;
        };
        let (title, label) = open.labels();
        let mut text = vec![
            Line::styled(label, theme::muted()),
            Line::from(vec![
                Span::styled(open.input.clone(), Style::new().fg(theme::colors().fg)),
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
            .title(Line::styled(title, theme::accent_bold()))
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

    fn typed(prompt: &mut Prompt, text: &str) -> Option<Action> {
        for c in text.chars() {
            prompt.handle_key_event(KeyCode::Char(c).into());
        }
        prompt.handle_key_event(KeyCode::Enter.into())
    }

    fn asking() -> Prompt {
        let mut prompt = Prompt::default();
        prompt.update(&Action::AskFolderName {
            container: Item {
                title: "Unterlagen".into(),
                ..Item::default()
            },
            place: "DEMO › Unterlagen".into(),
        });
        prompt
    }

    #[test]
    fn a_folder_name_leads_to_the_question() {
        let mut prompt = asking();
        let Some(Action::Ask { question, then }) = typed(&mut prompt, "  Woche 5 ") else {
            panic!("expected the question");
        };
        assert_eq!(
            question,
            "Create the folder “Woche 5” in DEMO › Unterlagen? Everyone who can open it sees it."
        );
        assert!(
            matches!(*then, Action::NewFolder { ref name, ref container } if name == "Woche 5" && container.title == "Unterlagen")
        );
        assert!(prompt.open.is_none());
    }

    #[test]
    fn an_empty_name_stays_open_and_esc_cancels() {
        let mut prompt = asking();
        assert!(matches!(typed(&mut prompt, "  "), Some(Action::Handled)));
        assert!(prompt.open.as_ref().unwrap().error.is_some());
        assert!(matches!(
            prompt.handle_key_event(KeyCode::Esc.into()),
            Some(Action::Cancelled)
        ));
        assert!(prompt.handle_key_event(KeyCode::Esc.into()).is_none());
    }

    #[test]
    fn without_an_editor_a_thread_asks_for_title_then_text() {
        use crate::action::Draft;
        let draft = Draft::Thread {
            forum: Item::default(),
            place: "DEMO › Fragen".into(),
        };
        let mut prompt = Prompt::default();
        prompt.update(&Action::AskPostText {
            draft: draft.clone(),
            title: None,
        });
        let next = typed(&mut prompt, "Termin").unwrap();
        assert!(matches!(next, Action::AskPostText { title: Some(ref t), .. } if t == "Termin"));
        prompt.update(&next);
        let Some(Action::Ask { question, then }) = typed(&mut prompt, "Wann?") else {
            panic!("expected the question");
        };
        assert!(
            question.starts_with("Start the thread “Termin” in DEMO › Fragen: “Wann?”"),
            "{question}"
        );
        assert!(
            matches!(*then, Action::SendPost { ref title, ref text, .. } if title == "Termin" && text == "Wann?")
        );
    }
}
