//! One-line footer: a spinner while something runs, results and errors, and key hints.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
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
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            started: Instant::now(),
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
        self.status = match action {
            Action::Load { deep: true, .. } => Status::Busy("Searching…".into()),
            Action::Load { refresh: true, .. } => Status::Busy("Refreshing…".into()),
            Action::Load { .. } => Status::Busy("Loading…".into()),
            Action::Loaded { .. } => Status::Idle,
            Action::Progress(text) => Status::Busy(text.clone()),
            Action::Download { item, .. } => Status::Busy(format!("Downloading {}…", item.title)),
            Action::Downloaded { path, .. } => Status::Done(format!("Saved {}", path.display())),
            Action::OpenFile(path) if open::is_risky(path) => Status::Warning(format!(
                "Not opened: {} could run code. Shown in your file manager instead.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
            Action::OpenFile(path) => Status::Done(format!("Opened {}", path.display())),
            Action::OpenInBrowser(item) => {
                Status::Info(format!("Opened {} in your browser", item.title))
            }
            Action::Login => Status::Busy("Complete the login in the Chrome window…".into()),
            Action::LoggedIn => Status::Done("Logged in".into()),
            Action::Error(e) => Status::Error(e.clone()),
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
            Action::Ask { .. } | Action::Quit | Action::ToggleHelp | Action::Filter(_) => {
                return None;
            }
        };
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let hints: Vec<Span> = HINTS
            .iter()
            .flat_map(|(key, label)| {
                [
                    Span::styled(format!(" {key} "), theme::accent_bold()),
                    Span::styled(format!("{label} "), theme::muted()),
                ]
            })
            .collect();
        let hints = Line::from(hints);
        let [left, right] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(hints.width() as u16 + 1),
        ])
        .areas(area);

        let (icon, text, color) = match &self.status {
            Status::Idle => ("", String::new(), theme::FG),
            Status::Busy(text) => {
                let frame_index = (self.started.elapsed().as_millis() / 80) as usize;
                (
                    theme::SPINNER[frame_index % theme::SPINNER.len()],
                    text.clone(),
                    theme::ACCENT,
                )
            }
            Status::Done(text) => (theme::check(), text.clone(), theme::GREEN),
            Status::Info(text) => ("", text.clone(), theme::FG),
            Status::Warning(text) => (theme::warning(), text.clone(), theme::YELLOW),
            Status::Error(text) => (theme::warning(), text.clone(), theme::RED),
        };
        let message = Line::from(vec![
            Span::styled(format!(" {icon} "), Style::new().fg(color)),
            Span::styled(text, Style::new().fg(color)),
        ]);
        let bar = Style::new().bg(theme::BAR_BG).fg(Color::Reset);
        frame.render_widget(Paragraph::new(message).style(bar), left);
        frame.render_widget(Paragraph::new(hints).style(bar).right_aligned(), right);
    }
}
