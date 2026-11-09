//! The dialog for picking files to send: one to hand in for an assignment, or several to
//! upload into a course or folder. It asks which, then leaves the rest to the usual question
//! ([`Action::Ask`]), so nothing is sent before a clear "yes".

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
use crate::ilias::write::megabytes;
use crate::theme;

/// What the files are for.
enum Purpose {
    /// One file for `assignment`, whose submission page is `url`.
    HandIn { assignment: Item, url: String },
    /// Files into `container`, which is at `place` (`DEMO.101 › Unterlagen`).
    Upload { container: Item, place: String },
}

struct Open {
    purpose: Purpose,
    input: String,
    error: Option<String>,
    /// What the path could be after the last `Tab`, when it fits several files.
    choices: Vec<String>,
}

/// At most this many of the choices are listed.
const CHOICES: usize = 8;

#[derive(Default)]
pub struct PickFiles {
    open: Option<Open>,
}

/// Files bigger than this (in all) get a word of warning in the question.
const BIG: u64 = 50_000_000;

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

/// Several paths, as typed (quoted, or with backslashes in front of spaces) or as a terminal
/// pastes files dropped on it. A single path with spaces in it works unquoted too.
fn parse_paths(input: &str) -> Result<Vec<PathBuf>, String> {
    let whole = parse_path(input);
    if whole.is_ok() {
        return whole.map(|path| vec![path]);
    }
    let words = split_paths(input);
    if words.len() < 2 {
        return whole.map(|path| vec![path]);
    }
    words.iter().map(|word| parse_path(word)).collect()
}

/// Splits at spaces outside quotes; a backslash keeps the next character (not on Windows,
/// where it separates folders).
fn split_paths(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') if !cfg!(windows) => word.extend(chars.next()),
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// `Upload 2 files (3.4 MB) to DEMO.101 › Unterlagen: a.pdf (1.2 MB), b.pdf (2.2 MB)? …`.
fn upload_question(place: &str, paths: &[PathBuf]) -> String {
    let total: u64 = paths.iter().map(|p| size(p)).sum();
    let files: Vec<String> = paths
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            format!("{name} ({})", megabytes(size(p)))
        })
        .collect();
    let count = match paths.len() {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    };
    let big = if total > BIG {
        " That is big: it takes a while."
    } else {
        ""
    };
    format!(
        "Upload {count} ({}) to {place}: {}? Everyone who can open it can see them.{big}",
        megabytes(total),
        files.join(", ")
    )
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

impl Component for PickFiles {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        let open = self.open.as_mut()?;
        open.error = None;
        open.choices.clear();
        match key.code {
            KeyCode::Tab => {
                let several = matches!(open.purpose, Purpose::Upload { .. });
                let done = super::complete::complete(&open.input, several);
                open.input = done.input;
                open.choices = done.choices;
            }
            KeyCode::Esc => {
                self.open = None;
                return Some(Action::Cancelled);
            }
            KeyCode::Enter => {
                let ask = match &open.purpose {
                    Purpose::HandIn { assignment, url } => {
                        parse_path(&open.input).map(|path| Action::Ask {
                            question: question(assignment, &path),
                            then: Box::new(Action::HandIn {
                                assignment: assignment.clone(),
                                url: url.clone(),
                                path,
                            }),
                        })
                    }
                    Purpose::Upload { container, place } => {
                        parse_paths(&open.input).map(|paths| Action::Ask {
                            question: upload_question(place, &paths),
                            then: Box::new(Action::Upload {
                                container: container.clone(),
                                paths,
                            }),
                        })
                    }
                };
                match ask {
                    Ok(ask) => {
                        self.open = None;
                        return Some(ask);
                    }
                    Err(error) => open.error = Some(error),
                }
            }
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
            Action::AskHandIn { assignment, url } => Purpose::HandIn {
                assignment: assignment.clone(),
                url: url.clone(),
            },
            Action::AskUpload { container, place } => Purpose::Upload {
                container: container.clone(),
                place: place.clone(),
            },
            _ => return None,
        };
        self.open = Some(Open {
            purpose,
            input: String::new(),
            error: None,
            choices: Vec::new(),
        });
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let Some(open) = &self.open else {
            return;
        };
        let fg = Style::new().fg(theme::colors().fg);
        let (title, prompt) = match &open.purpose {
            Purpose::HandIn { assignment, .. } => (
                format!(" Hand in: {} ", assignment.title),
                "File to hand in (or drop it here):",
            ),
            Purpose::Upload { place, .. } => (
                format!(" Upload to {place} "),
                "Files to upload (or drop them here; quote paths with spaces):",
            ),
        };
        let mut text = vec![
            Line::styled(prompt, theme::muted()),
            Line::from(vec![
                Span::styled(open.input.clone(), fg),
                Span::styled("▏", Style::new().fg(theme::colors().accent)),
            ]),
        ];
        if !open.choices.is_empty() {
            let mut shown = open.choices[..open.choices.len().min(CHOICES)].join("   ");
            if open.choices.len() > CHOICES {
                shown.push_str(&format!("   … {} more", open.choices.len() - CHOICES));
            }
            text.push(Line::styled(shown, theme::muted()));
        }
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
            .title_bottom(
                Line::styled(" ⇥ complete  ↵ continue  esc cancel ", theme::muted()).centered(),
            )
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

    fn dialog() -> PickFiles {
        let mut dialog = PickFiles::default();
        dialog.update(&Action::AskHandIn {
            assignment: Item {
                title: "Blatt 1".into(),
                ..Item::default()
            },
            url: "u".into(),
        });
        dialog
    }

    fn typed(dialog: &mut PickFiles, text: &str) -> Option<Action> {
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

    #[test]
    fn uploads_several_files_and_names_place_and_sizes_when_asking() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-pick-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("week 1.pdf");
        let b = dir.join("b.txt");
        std::fs::write(&a, vec![b'x'; 1_500_000]).unwrap();
        std::fs::write(&b, "x").unwrap();
        let typed_paths = format!(
            "'{}' {}",
            a.display(),
            b.display().to_string().replace(' ', "\\ ")
        );
        assert_eq!(parse_paths(&typed_paths), Ok(vec![a.clone(), b.clone()]));
        // One path with a space needs no quotes.
        assert_eq!(parse_paths(&a.display().to_string()), Ok(vec![a.clone()]));
        assert!(parse_paths(&format!("{} /no/such.pdf", b.display())).is_err());

        let mut dialog = PickFiles::default();
        dialog.update(&Action::AskUpload {
            container: Item {
                title: "Unterlagen".into(),
                ..Item::default()
            },
            place: "DEMO › Unterlagen".into(),
        });
        let Some(Action::Ask { question, then }) = typed(&mut dialog, &typed_paths) else {
            panic!("expected the question");
        };
        assert!(
            question.starts_with(
                "Upload 2 files (1.5 MB) to DEMO › Unterlagen: week 1.pdf (1.5 MB), b.txt (0.0 MB)?"
            ),
            "{question}"
        );
        assert!(
            matches!(*then, Action::Upload { ref paths, .. } if *paths == [a.clone(), b.clone()])
        );

        // Big files get a warning.
        assert!(upload_question("x", &vec![a.clone(); 40]).contains("big"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tab_completes_the_path_and_lists_what_it_could_be() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for file in ["sheet-1.pdf", "sheet-2.pdf"] {
            std::fs::write(dir.join(file), "x").unwrap();
        }
        let mut dialog = dialog();
        for c in format!("{}/sh", dir.display()).chars() {
            dialog.handle_key_event(KeyCode::Char(c).into());
        }
        assert!(matches!(
            dialog.handle_key_event(KeyCode::Tab.into()),
            Some(Action::Handled)
        ));
        let open = dialog.open.as_ref().unwrap();
        assert_eq!(open.input, format!("{}/sheet-", dir.display()));
        assert_eq!(open.choices, ["sheet-1.pdf", "sheet-2.pdf"]);
        dialog.handle_key_event(KeyCode::Char('2').into());
        dialog.handle_key_event(KeyCode::Tab.into());
        let open = dialog.open.as_ref().unwrap();
        assert_eq!(open.input, format!("{}/sheet-2.pdf", dir.display()));
        assert!(open.choices.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
