//! Writing forum posts: the text comes from `$VISUAL` / `$EDITOR` (the TUI steps aside while
//! it runs), or, without one, from one-line prompts. Either way the post is only sent after
//! the question [`question`] builds.

use std::io::Write as _;
use std::process::Command;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::action::{Action, Draft};

/// The editor to write posts in, from `$VISUAL` or `$EDITOR` (e.g. `vim`, `code -w`).
pub fn editor() -> Option<String> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|e| !e.trim().is_empty())
}

/// What the editor starts with: room for the text, then how it works, as comments.
fn template(draft: &Draft) -> String {
    let help = match draft {
        Draft::Thread { place, .. } => format!(
            "# New thread in {place}.\n# The first line is the thread's title, the lines after it are the post.\n"
        ),
        Draft::Reply { thread, author, .. } => {
            format!("# Reply to {author} in “{}”.\n", thread.title)
        }
    };
    format!(
        "\n\n{help}# Lines starting with # are left out. Save and close to go on (you are asked\n# before anything is sent); leave it empty to cancel.\n"
    )
}

/// Title and text from what was written; `None` if nothing was.
pub fn read(draft: &Draft, written: &str) -> Result<Option<(String, String)>> {
    let lines: Vec<&str> = written
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    let text = lines.join("\n").trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    match draft {
        Draft::Thread { .. } => {
            let (title, body) = text.split_once('\n').unwrap_or((&text, ""));
            if body.trim().is_empty() {
                bail!("A new thread needs a title (the first line) and a text below it");
            }
            Ok(Some((title.trim().to_string(), body.trim().to_string())))
        }
        Draft::Reply { .. } => Ok(Some((String::new(), text))),
    }
}

/// Lets the user write the post in their editor, with the TUI out of the way meanwhile.
/// `None` if they wrote nothing.
pub fn in_editor(
    terminal: &mut DefaultTerminal,
    editor: &str,
    draft: &Draft,
) -> Result<Option<(String, String)>> {
    let path = std::env::temp_dir().join(format!("ilias-tui-post-{}.txt", std::process::id()));
    std::fs::File::create(&path)?.write_all(template(draft).as_bytes())?;
    let mut words = editor.split_whitespace();
    let program = words.next().ok_or_else(|| eyre!("no editor set"))?;
    let mouse = std::env::var("ILIAS_MOUSE").as_deref() != Ok("0");

    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    let status = Command::new(program).args(words).arg(&path).status();
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }
    // A full redraw. (`clear` would ask the terminal where its cursor is, which not every
    // terminal answers in time.)
    let size = terminal.size()?;
    terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))?;

    let written = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    let status = status.map_err(|e| eyre!("could not start {program}: {e}"))?;
    if !status.success() {
        bail!("{program} ended with an error; nothing was sent");
    }
    read(draft, &written)
}

/// The start of a text for a question: one line, at most `n` characters.
fn start(text: &str, n: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(n) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// The question before a post is sent: what, where, and how it starts.
pub fn question(draft: Draft, title: String, text: String) -> Action {
    let question = match &draft {
        Draft::Thread { place, .. } => format!(
            "Start the thread “{title}” in {place}: “{}”? Everyone who can read the forum sees it.",
            start(&text, 60)
        ),
        Draft::Reply { thread, author, .. } => format!(
            "Reply to {author} in “{}”: “{}”? Everyone who can read the forum sees it.",
            thread.title,
            start(&text, 60)
        ),
    };
    Action::Ask {
        question,
        then: Box::new(Action::SendPost { draft, title, text }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Item;

    fn thread() -> Draft {
        Draft::Thread {
            forum: Item::default(),
            place: "DEMO › Forum".into(),
        }
    }

    fn reply() -> Draft {
        Draft::Reply {
            thread: Item {
                title: "Blatt 1".into(),
                ..Item::default()
            },
            post: 2,
            author: "Ada".into(),
        }
    }

    #[test]
    fn reads_title_and_text_without_the_comments() {
        let written = format!(
            "Frage zu 2\nWie ist\n\ndas gemeint?\n{}",
            template(&thread())
        );
        assert_eq!(
            read(&thread(), &written).unwrap(),
            Some(("Frage zu 2".into(), "Wie ist\n\ndas gemeint?".into()))
        );
        assert_eq!(
            read(&thread(), &template(&thread())).unwrap(),
            None,
            "cancelled"
        );
        assert!(read(&thread(), "Only a title\n").is_err());
        assert_eq!(
            read(&reply(), "Danke!\n# x").unwrap(),
            Some((String::new(), "Danke!".into()))
        );
    }

    #[test]
    fn asks_with_place_and_the_start_of_the_text() {
        let Action::Ask { question, then } = question(thread(), "Frage".into(), "a ".repeat(50))
        else {
            panic!("expected a question");
        };
        assert!(
            question.starts_with("Start the thread “Frage” in DEMO › Forum: “a a a"),
            "{question}"
        );
        assert!(question.contains("…”?"), "{question}");
        assert!(matches!(*then, Action::SendPost { .. }));
        let Action::Ask { question, .. } = super::question(reply(), String::new(), "Danke!".into())
        else {
            panic!("expected a question");
        };
        assert_eq!(
            question,
            "Reply to Ada in “Blatt 1”: “Danke!”? Everyone who can read the forum sees it."
        );
    }
}
