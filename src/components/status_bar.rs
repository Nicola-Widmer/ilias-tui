//! One-line footer: a spinner while something runs, results and errors, and key hints.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::Component;
use crate::action::Action;
use crate::{open, theme};

enum Status {
    Idle,
    Busy(String),
    Done(String),
    Info(String),
    Warning(String),
    Error(String),
}

pub struct StatusBar {
    status: Status,
    started: Instant,
    /// Shown as a badge until the app is online again.
    offline: bool,
    /// The status is a change on ILIAS whose page reloads next; keep it through the reload.
    keep: bool,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            started: Instant::now(),
            offline: false,
            keep: false,
        }
    }
}

const HINTS: &[(&str, &str)] = &[
    ("/", "filter"),
    ("s", "search"),
    ("?", "help"),
    ("q", "quit"),
];

impl Component for StatusBar {
    fn update(&mut self, action: &Action) -> Option<Action> {
        if self.keep {
            match action {
                Action::Load { refresh: true, .. } => return None,
                Action::Loaded { .. } => {
                    self.keep = false;
                    return None;
                }
                _ => {}
            }
        }
        self.status = match action {
            Action::Load { deep: true, .. } => Status::Busy("Searching…".into()),
            Action::Load { refresh: true, .. } => Status::Busy("Refreshing…".into()),
            Action::Load { .. } => Status::Busy("Loading…".into()),
            Action::Loaded { .. } => Status::Idle,
            Action::Progress(text) => Status::Busy(text.clone()),
            Action::Download { item, .. } => Status::Busy(format!("Downloading {}…", item.title)),
            Action::Downloaded { path, .. } => Status::Done(format!("Saved {}", path.display())),
            Action::DownloadDirSet(dir) => {
                Status::Done(format!("Files now go to {}", crate::config::display(dir)))
            }
            Action::OpenFile(path) if open::is_risky(path) => Status::Warning(format!(
                "Not opened: {} could run code. Shown in your file manager instead.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
            Action::OpenFile(path) => Status::Done(format!("Opened {}", path.display())),
            Action::OpenInBrowser(item) => {
                Status::Info(format!("Opened {} in your browser", item.title))
            }
            Action::Login => Status::Busy("Complete the login in the Chrome window…".into()),
            Action::LoggedIn => {
                self.offline = false;
                Status::Done("Logged in".into())
            }
            Action::Offline => {
                self.offline = true;
                Status::Warning(
                    "Offline: showing cached pages and saved files. L goes online.".into(),
                )
            }
            Action::Error(e) => Status::Error(e.clone()),
            Action::Sync(roots) => {
                Status::Busy(format!("Sync: starting ({} selected)…", roots.len()))
            }
            Action::Synced { summary, .. } => Status::Done(summary.clone()),
            Action::Cancelled => Status::Info("Cancelled; nothing was sent to ILIAS".into()),
            Action::ToggleFavorite(_) => Status::Busy("Updating your favourites…".into()),
            Action::FavoriteToggled { item, dry_run } => {
                let change = if item.favorite {
                    format!("removed {} from", item.title)
                } else {
                    format!("added {} to", item.title)
                };
                if *dry_run {
                    Status::Info(format!("Dry run: would have {change} your favourites"))
                } else {
                    let mut text = format!("{change} your favourites");
                    text[..1].make_ascii_uppercase();
                    Status::Done(text)
                }
            }
            Action::HandIn { assignment, .. } => {
                Status::Busy(format!("Handing in for {}…", assignment.title))
            }
            Action::HandedIn { assignment, file } => match file {
                Some(file) => Status::Done(format!("Handed in {file} for {}", assignment.title)),
                None => Status::Info(format!(
                    "Dry run: would have handed in for {}",
                    assignment.title
                )),
            },
            Action::DeleteSubmission { file, .. } => {
                Status::Busy(format!("Deleting {} from your submission…", file.title))
            }
            Action::SubmissionDeleted {
                assignment,
                file,
                deleted,
            } => match deleted {
                true => Status::Done(format!(
                    "Deleted {} from your submission for {}",
                    file.title, assignment.title
                )),
                false => Status::Info(format!("Dry run: would have deleted {}", file.title)),
            },
            Action::Ask { .. }
            | Action::AskHandIn { .. }
            | Action::Handled
            | Action::Quit
            | Action::ToggleHelp
            | Action::ToggleSettings
            | Action::SetConfig(_)
            | Action::Filter(_) => {
                return None;
            }
        };
        self.keep = matches!(
            action,
            Action::HandedIn { file: Some(_), .. }
                | Action::SubmissionDeleted { deleted: true, .. }
        );
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let mut hints: Vec<Span> = HINTS
            .iter()
            .flat_map(|(key, label)| {
                [
                    Span::styled(format!(" {key} "), theme::accent_bold()),
                    Span::styled(format!("{label} "), theme::muted()),
                ]
            })
            .collect();
        if self.offline {
            let badge = Style::new()
                .fg(theme::colors().bar_bg)
                .bg(theme::colors().yellow)
                .add_modifier(Modifier::BOLD);
            hints.insert(0, Span::styled(" OFFLINE ", badge));
            hints.insert(1, Span::raw(" "));
        }
        let hints = Line::from(hints);
        let [left, right] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(hints.width() as u16 + 1),
        ])
        .areas(area);

        let (icon, text, color) = match &self.status {
            Status::Idle => ("", String::new(), theme::colors().fg),
            Status::Busy(text) => {
                let frame_index = (self.started.elapsed().as_millis() / 80) as usize;
                (
                    theme::SPINNER[frame_index % theme::SPINNER.len()],
                    text.clone(),
                    theme::colors().accent,
                )
            }
            Status::Done(text) => (theme::check(), text.clone(), theme::colors().green),
            Status::Info(text) => ("", text.clone(), theme::colors().fg),
            Status::Warning(text) => (theme::warning(), text.clone(), theme::colors().yellow),
            Status::Error(text) => (theme::warning(), text.clone(), theme::colors().red),
        };
        let message = Line::from(vec![
            Span::styled(format!(" {icon} "), Style::new().fg(color)),
            Span::styled(text, Style::new().fg(color)),
        ]);
        let bar = Style::new()
            .bg(theme::colors().bar_bg)
            .fg(theme::colors().fg);
        frame.render_widget(Paragraph::new(message).style(bar), left);
        frame.render_widget(Paragraph::new(hints).style(bar).right_aligned(), right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Item;

    #[test]
    fn a_change_on_ilias_stays_shown_through_the_reload_after_it() {
        let mut bar = StatusBar::default();
        let file = Item {
            title: "Bericht.pdf".into(),
            ..Item::default()
        };
        bar.update(&Action::SubmissionDeleted {
            assignment: Item::default(),
            file,
            deleted: true,
        });
        for action in [
            Action::Load {
                item: None,
                url: None,
                refresh: true,
                deep: false,
            },
            Action::Handled,
            Action::Loaded {
                item: None,
                page: Default::default(),
                refresh: true,
                deep: false,
            },
        ] {
            bar.update(&action);
            assert!(matches!(bar.status, Status::Done(_)));
        }
        // Later loads show as usual.
        bar.update(&Action::Load {
            item: None,
            url: None,
            refresh: true,
            deep: false,
        });
        assert!(matches!(bar.status, Status::Busy(_)));
    }
}
