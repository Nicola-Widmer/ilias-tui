//! The keys of the main view, in one place: which key means which [`Command`], and how the
//! help overlay describes it. The README's keys table is checked against it (see the
//! tests). Overlays have their own keys while they are open: the settings, the sync report,
//! the filter while you type, and questions. The sync review's keys are here too
//! ([`REVIEW`]), with a README table of their own.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// What a key asks for in the main view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Down,
    Up,
    First,
    Last,
    /// Open the selected item (folder, file, link, …).
    Open,
    /// Clear the filter, then go back.
    Back,
    NextTab,
    PreviousTab,
    Filter,
    Search,
    Download,
    OpenInBrowser,
    CopyLink,
    Favorite,
    Reload,
    HandIn,
    DeleteSubmission,
    Upload,
    NewFolder,
    Delete,
    Reply,
    DownloadAll,
    MarkForSync,
    Sync,
    SyncAll,
    Login,
    Settings,
    Help,
    Quit,
    /// Sync review: tick or untick the proposal under the cursor.
    Tick,
    /// Sync review: on to the question before sending.
    Send,
}

pub struct Binding {
    pub command: Command,
    /// The keys, as the help and the README name them.
    pub keys: &'static [KeyCode],
    /// More keys for the same, not listed (e.g. Home for `g`).
    pub also: &'static [KeyCode],
    pub help: &'static str,
}

const fn bind(command: Command, keys: &'static [KeyCode], help: &'static str) -> Binding {
    Binding {
        command,
        keys,
        also: &[],
        help,
    }
}

use Command::*;
use KeyCode::{BackTab, Backspace, Char, End, Enter, Esc, Home, Left, Right, Tab};

/// Every key of the main view, by section, in the order the help shows them.
pub const SECTIONS: &[(&str, &[Binding])] = &[
    (
        "Navigate",
        &[
            bind(Down, &[KeyCode::Down, Char('j')], "Move selection down"),
            bind(Up, &[KeyCode::Up, Char('k')], "Move selection up"),
            Binding {
                also: &[Home],
                ..bind(First, &[Char('g')], "First item")
            },
            Binding {
                also: &[End],
                ..bind(Last, &[Char('G')], "Last item")
            },
            bind(
                Open,
                &[Enter, Right, Char('l')],
                "Open folder, file (downloads it first) or link",
            ),
            Binding {
                also: &[Backspace],
                ..bind(Back, &[Esc, Left, Char('h')], "Clear filter, then go back")
            },
            bind(
                NextTab,
                &[Tab],
                "Next tab (Inhalt, Info, …); My courses: into the To-Do panel",
            ),
            bind(PreviousTab, &[BackTab], "Previous tab"),
        ],
    ),
    (
        "Search",
        &[
            bind(
                Filter,
                &[Char('/')],
                "Filter this list, kept when you come back (Esc clears)",
            ),
            bind(Search, &[Char('s')], "Search everything below here"),
        ],
    ),
    (
        "Items",
        &[
            bind(Download, &[Char('d')], "Download a fresh copy (no open)"),
            bind(OpenInBrowser, &[Char('o')], "Open in your web browser"),
            bind(CopyLink, &[Char('y')], "Copy the permalink"),
            bind(
                Favorite,
                &[Char('f')],
                "Add to / remove from favourites (asks first)",
            ),
            bind(Reload, &[Char('r')], "Reload"),
        ],
    ),
    (
        "Sync",
        &[
            bind(MarkForSync, &[Char(' ')], "Mark a course / folder for sync"),
            bind(
                Sync,
                &[Char('S')],
                "Sync the marked, else the selected (on a course page: the course)",
            ),
            bind(SyncAll, &[Char('A')], "Sync all courses (asks first)"),
        ],
    ),
    (
        "Exercises",
        &[
            bind(
                HandIn,
                &[Char('H')],
                "Assignment: hand in a file (asks first)",
            ),
            bind(
                DeleteSubmission,
                &[Char('X')],
                "Handed-in files: delete one (asks first)",
            ),
            bind(
                DownloadAll,
                &[Char('a')],
                "Submissions page: download everyone's",
            ),
        ],
    ),
    (
        "Edit · where ILIAS lets you, asks first",
        &[
            bind(Upload, &[Char('u')], "Upload files here"),
            bind(
                NewFolder,
                &[Char('n')],
                "New folder; in a forum, new thread",
            ),
            bind(Delete, &[Char('D')], "Move the selected item to the trash"),
            bind(Reply, &[Char('R')], "Thread: reply to the post marked ↩"),
        ],
    ),
    (
        "App",
        &[
            bind(
                Login,
                &[Char('L')],
                "Log in again / go online (opens Chrome if needed)",
            ),
            bind(Settings, &[Char('c')], "Settings (folder, sync, icons, …)"),
            bind(Help, &[Char('?')], "Toggle this help"),
            bind(Quit, &[Char('q')], "Quit (also Ctrl-c)"),
        ],
    ),
];

/// The keys of the sync review (two-way sync), which has them while it is open.
pub const REVIEW: &[Binding] = &[
    bind(
        Down,
        &[KeyCode::Down, Char('j')],
        "Next proposal (else scroll)",
    ),
    bind(
        Up,
        &[KeyCode::Up, Char('k')],
        "Previous proposal (else scroll)",
    ),
    bind(
        Tick,
        &[Char(' ')],
        "Tick / untick: move this file to the ILIAS trash",
    ),
    bind(
        Send,
        &[Enter],
        "Send what is listed and ticked (asks again only for a lot)",
    ),
    bind(Back, &[Esc, Char('q')], "Close, send nothing"),
];

/// The command `key` stands for in the sync review.
pub fn review_command(key: KeyEvent) -> Option<Command> {
    REVIEW
        .iter()
        .find(|b| b.keys.contains(&key.code))
        .map(|b| b.command)
}

/// The command `key` stands for in the main view.
pub fn command(key: KeyEvent) -> Option<Command> {
    SECTIONS
        .iter()
        .flat_map(|(_, bindings)| bindings.iter())
        .find(|b| b.keys.contains(&key.code) || b.also.contains(&key.code))
        .map(|b| b.command)
}

/// How the help and the README name a key.
pub fn key_name(key: KeyCode) -> String {
    match key {
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        Left => "←".into(),
        Right => "→".into(),
        Enter => "Enter".into(),
        Esc => "Esc".into(),
        Tab => "Tab".into(),
        BackTab => "Shift-Tab".into(),
        Char(' ') => "Space".into(),
        Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

/// A binding's keys for the help, e.g. `↓  j`.
pub fn label(binding: &Binding) -> String {
    binding
        .keys
        .iter()
        .map(|k| key_name(*k))
        .collect::<Vec<_>>()
        .join("  ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_key_means_one_thing() {
        let mut seen = BTreeSet::new();
        for (_, bindings) in SECTIONS {
            for binding in *bindings {
                for key in binding.keys.iter().chain(binding.also) {
                    assert!(seen.insert(format!("{key:?}")), "{key:?} is bound twice");
                }
            }
        }
        assert_eq!(command(KeyCode::Home.into()), Some(First));
        assert_eq!(command(Char('x').into()), None);
    }

    /// The README's keys table names exactly the keys bound here.
    #[test]
    fn the_readme_lists_every_key() {
        let readme = include_str!("../README.md");
        let table = readme
            .split("### Keys")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").nth(1))
            .expect("a keys table under ### Keys");
        let mut listed = BTreeSet::new();
        for row in table.lines().skip(2) {
            let keys = row.split('|').nth(1).unwrap_or_default();
            for code in keys.split('`').skip(1).step_by(2) {
                listed.extend(code.split_whitespace().map(String::from));
            }
        }
        listed.remove("Ctrl-c");
        let bound: BTreeSet<String> = SECTIONS
            .iter()
            .flat_map(|(_, bindings)| bindings.iter())
            .flat_map(|b| b.keys.iter().map(|k| key_name(*k)))
            .collect();
        assert_eq!(listed, bound);
    }

    /// The README's table of the sync review names exactly its keys.
    #[test]
    fn the_readme_lists_every_review_key() {
        let readme = include_str!("../README.md");
        let table = readme
            .split("In the sync review:")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").nth(1))
            .expect("a table after \"In the sync review:\"");
        let mut listed = BTreeSet::new();
        for row in table.lines().skip(2) {
            let keys = row.split('|').nth(1).unwrap_or_default();
            for code in keys.split('`').skip(1).step_by(2) {
                listed.extend(code.split_whitespace().map(String::from));
            }
        }
        let bound: BTreeSet<String> = REVIEW
            .iter()
            .flat_map(|b| b.keys.iter().map(|k| key_name(*k)))
            .collect();
        assert_eq!(listed, bound);
        let mut seen = BTreeSet::new();
        for key in REVIEW.iter().flat_map(|b| b.keys) {
            assert!(seen.insert(format!("{key:?}")), "{key:?} is bound twice");
        }
    }
}
