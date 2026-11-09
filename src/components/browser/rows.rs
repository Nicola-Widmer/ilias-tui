//! How each kind of row looks: items, text, headings, forum posts, To-Do entries.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::ListItem;

use crate::icons::icon;
use crate::ilias::{Item, Post};
use crate::theme;

use super::*;

/// A forum post: author line, then title and text behind a bar, indented by reply depth.
pub(super) fn post_rows(rows: &mut Vec<Row>, post: &Post, show_title: bool, width: usize) {
    let indent = "  ".repeat(post.depth.min(6));
    let bar = || {
        Span::styled(
            format!("{indent}{} ", theme::glyph("▎", "│")),
            Style::new().fg(theme::colors().border),
        )
    };
    let mut header = vec![
        Span::raw(indent.clone()),
        Span::styled(post.author.clone(), theme::accent_bold()),
        Span::styled(format!("  {}", post.date), theme::muted()),
    ];
    if post.unread {
        header.push(Span::styled(
            "  ● new",
            Style::new()
                .fg(theme::colors().green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    rows.push(Row::Styled(Line::from(header)));
    if show_title && !post.title.is_empty() {
        rows.push(Row::Styled(Line::from(vec![
            bar(),
            Span::styled(
                post.title.clone(),
                Style::new()
                    .fg(theme::colors().fg)
                    .add_modifier(Modifier::BOLD),
            ),
        ])));
    }
    let width = width.saturating_sub(indent.len() + 2).max(20);
    for paragraph in &post.body {
        for line in wrap(paragraph, width) {
            rows.push(Row::Styled(Line::from(vec![
                bar(),
                Span::styled(line, Style::new().fg(theme::colors().fg)),
            ])));
        }
    }
}

/// A To-Do entry: assignment, how long is left (red when it's close), course.
pub(super) fn todo_row(item: &Item, last: bool) -> ListItem<'static> {
    let (glyph, color) = icon(item);
    let (due, date) = item
        .description
        .split_once(" · ")
        .unwrap_or((item.description.as_str(), ""));
    let days = due
        .strip_prefix("due in ")
        .and_then(|rest| rest.strip_suffix(" days").or(rest.strip_suffix(" day")))
        .and_then(|n| n.parse::<u32>().ok());
    let due_color = match days {
        _ if due == "overdue" || due.contains("hour") => theme::colors().red,
        Some(days) if days <= 3 => theme::colors().yellow,
        _ => theme::colors().green,
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{glyph}  "), Style::new().fg(color)),
            Span::styled(
                item.title.clone(),
                Style::new()
                    .fg(theme::colors().fg)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("   {due}"),
                Style::new().fg(due_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {date}"), theme::muted()),
        ]),
        Line::styled(
            format!("   {}", item.properties.first().map_or("", String::as_str)),
            theme::muted().add_modifier(Modifier::ITALIC),
        ),
    ];
    if !last {
        lines.push(Line::raw(""));
    }
    ListItem::new(Text::from(lines))
}

/// Separates a new section from what came before, without doubling up empty lines.
pub(super) fn blank_line(rows: &mut Vec<Row>) {
    if !rows.is_empty() && !matches!(rows.last(), Some(Row::Blank)) {
        rows.push(Row::Blank);
    }
}

/// Greedy word wrap.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// What is marked on an item row besides the item itself.
#[derive(Default)]
pub(super) struct Marks {
    pub(super) saved: bool,
    /// Marked for sync.
    pub(super) marked: bool,
    /// New or changed since your last visit.
    pub(super) new: bool,
    /// A course: how many items in it are new since you last opened it.
    pub(super) new_inside: usize,
}

pub(super) fn item_row(item: &Item, query: &str, marks: Marks) -> ListItem<'static> {
    let (glyph, color) = icon(item);
    let title_style = if item.is_container() {
        Style::new()
            .fg(theme::colors().fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme::colors().fg)
    };
    let mut title = vec![Span::styled(format!("{glyph}  "), Style::new().fg(color))];
    title.extend(highlight(&item.title, query, title_style));
    if item.favorite {
        title.push(Span::styled("  ★", Style::new().fg(theme::colors().yellow)));
    }
    let new = Style::new()
        .fg(theme::colors().green)
        .add_modifier(Modifier::BOLD);
    if marks.new {
        title.push(Span::styled("  ● new", new));
    }
    if marks.new_inside > 0 {
        title.push(Span::styled(format!("  ● {} new", marks.new_inside), new));
    }
    if marks.saved {
        title.push(Span::styled(
            format!("  {} saved", theme::check()),
            Style::new().fg(theme::colors().green),
        ));
    }
    if marks.marked {
        title.push(Span::styled("  ◉ sync", theme::accent_bold()));
    }
    let mut lines = vec![Line::from(title)];
    // The icon already says "course" or "folder", and files show their type as a property;
    // only less obvious objects (exercise, forum, …) get their kind spelled out.
    let kind = if item.is_container() || item.kind == "file" {
        ""
    } else {
        item.kind_label()
    };
    let details: Vec<&str> = [kind, item.description.as_str()]
        .into_iter()
        .chain(item.properties.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    if !details.is_empty() {
        lines.push(Line::styled(
            format!("   {}", details.join("  ·  ")),
            theme::muted().add_modifier(Modifier::ITALIC),
        ));
    }
    ListItem::new(Text::from(lines))
}

pub(super) fn heading_row(text: String) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled("▍", Style::new().fg(theme::colors().accent)),
        Span::styled(
            text,
            Style::new()
                .fg(theme::colors().yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
}

/// Page text; URLs are highlighted like links.
pub(super) fn text_row(text: String) -> ListItem<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if word.starts_with("http://") || word.starts_with("https://") {
            Style::new()
                .fg(theme::colors().blue)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new().fg(theme::colors().fg)
        };
        spans.push(Span::styled(word.to_string(), style));
    }
    ListItem::new(Line::from(spans))
}

/// Splits `text` into spans, emphasising every case-insensitive occurrence of a query word.
pub(super) fn highlight(text: &str, query: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut hit = vec![false; chars.len()];
    for word in query.split_whitespace() {
        let word: Vec<char> = word
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        if word.is_empty() || word.len() > lower.len() {
            continue;
        }
        for start in 0..=lower.len() - word.len() {
            if lower[start..start + word.len()] == word[..] {
                hit[start..start + word.len()]
                    .iter_mut()
                    .for_each(|h| *h = true);
            }
        }
    }
    let matched = base
        .fg(theme::colors().yellow)
        .add_modifier(Modifier::UNDERLINED);
    let mut spans = Vec::new();
    let mut start = 0;
    for i in 1..=chars.len() {
        if i == chars.len() || hit[i] != hit[start] {
            let part: String = chars[start..i].iter().collect();
            spans.push(Span::styled(part, if hit[start] { matched } else { base }));
            start = i;
        }
    }
    spans
}
