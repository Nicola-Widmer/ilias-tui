//! How each kind of row looks: items, text, headings, forum posts, To-Do entries.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::ListItem;

use crate::icons::icon;
use crate::ilias::{Item, Kind, Post, Todo, Urgency};
use crate::seen::{Counts, Group};
use crate::theme;

use super::level::PANE_BORDERS;
use super::*;

/// A forum post: author line, then title and text behind a bar, indented by reply depth.
pub(super) fn post_rows(
    rows: &mut Vec<Row>,
    index: usize,
    post: &Post,
    show_title: bool,
    width: usize,
) {
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
    rows.push(Row::PostHeader(index, Line::from(header)));
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
pub(super) fn todo_row(todo: &Todo, last: bool) -> ListItem<'static> {
    let item = &todo.assignment;
    let (glyph, color) = icon(item);
    let due_color = match todo.urgency {
        Urgency::Now => theme::colors().red,
        Urgency::Soon => theme::colors().yellow,
        Urgency::Later => theme::colors().green,
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
                format!("   {}", todo.due),
                Style::new().fg(due_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", todo.deadline), theme::muted()),
        ]),
        Line::styled(
            format!("   {}", todo.course.as_deref().unwrap_or("")),
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
    if !rows.is_empty() && !matches!(rows.last(), Some(Row::Blank | Row::PaneRule)) {
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
    /// New or changed since your last visit (a forum or thread: new posts).
    pub(super) new: bool,
    /// A course or folder: how much below it is new since you last opened it.
    pub(super) new_inside: Counts,
}

pub(super) fn item_row(item: &Item, query: &str, marks: Marks) -> Text<'static> {
    // A link to an ILIAS file or folder looks like one.
    let target = item.link_target();
    let item = target.as_ref().unwrap_or(item);
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
    // New posts in a forum read differently from new files and folders.
    let posts = Style::new()
        .fg(theme::colors().cyan)
        .add_modifier(Modifier::BOLD);
    if marks.new {
        title.push(match item.kind {
            Kind::Forum | Kind::Thread => Span::styled("  ● new posts", posts),
            _ => Span::styled("  ● new", new),
        });
    }
    let counts = &marks.new_inside;
    let items = counts.get(Group::Files) + counts.get(Group::Folders) + counts.get(Group::Other);
    let post_count = counts.get(Group::Posts);
    if items > 0 {
        title.push(Span::styled(format!("  ● {items} new"), new));
    }
    // It counts forums and threads, not posts.
    match post_count {
        0 => {}
        1 => title.push(Span::styled("  ● new posts in 1 forum", posts)),
        n => title.push(Span::styled(format!("  ● new posts in {n} forums"), posts)),
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
    let kind = if item.is_container() || matches!(item.kind, Kind::File | Kind::Link) {
        ""
    } else {
        item.kind_label()
    };
    // A web link's address; the icon already says it is one.
    let description = match item.kind {
        Kind::Link => web_address(&item.description),
        _ => &item.description,
    };
    let details: Vec<&str> = [kind, description]
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
    Text::from(lines)
}

/// `https://example.org/a` → `example.org/a`.
fn web_address(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
}

/// Whether `item_row` shows a line of details below the title.
pub(super) fn has_details(item: &Item) -> bool {
    let target = item.link_target();
    let item = target.as_ref().unwrap_or(item);
    let kind = !(item.is_container() || matches!(item.kind, Kind::File | Kind::Link));
    kind || !item.description.is_empty() || !item.properties.is_empty()
}

pub(super) fn heading_row(text: String) -> Text<'static> {
    Text::from(Line::from(vec![
        Span::styled("▍", Style::new().fg(theme::colors().accent)),
        Span::styled(
            text,
            Style::new()
                .fg(theme::colors().yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
}

/// An accordion pane's caption, `▸` closed or `▾` open (Enter switches).
pub(super) fn pane_row(title: String, open: bool) -> Text<'static> {
    let arrow = match open {
        true => theme::glyph("\u{f078}", "▾"),  // chevron-down
        false => theme::glyph("\u{f054}", "▸"), // chevron-right
    };
    Text::from(Line::from(vec![
        Span::styled(
            format!("{arrow}  "),
            Style::new().fg(theme::colors().accent),
        ),
        Span::styled(
            title,
            Style::new()
                .fg(theme::colors().fg)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
}

/// A horizontal border of a pane `width` columns wide: `╭─ 17.09.2026 ───╮`.
pub(super) fn pane_border(left: &str, label: &str, right: &str, width: usize) -> Text<'static> {
    let border = Style::new().fg(theme::colors().border);
    let mut spans = vec![Span::styled(format!("{left}─"), border)];
    if !label.is_empty() {
        spans.push(Span::styled(format!(" {label} "), theme::accent_bold()));
    }
    let used: usize = spans.iter().map(Span::width).sum::<usize>() + right.chars().count();
    spans.push(Span::styled(
        format!("{}{right}", "─".repeat(width.saturating_sub(used))),
        border,
    ));
    Text::from(Line::from(spans))
}

/// Puts each line of `text` into `depth` pane boxes, `width` columns wide in all: `│ `,
/// the line (cut with `…` if too long), padding, ` │`.
pub(super) fn framed(text: Text<'static>, depth: usize, width: usize) -> Text<'static> {
    let border = Style::new().fg(theme::colors().border);
    let mut lines = text.lines;
    // Innermost box first.
    for level in (0..depth).rev() {
        let inner = width.saturating_sub(PANE_BORDERS * (level + 1));
        lines = lines
            .into_iter()
            .map(|line| {
                let line = fit(line, inner);
                let pad = inner.saturating_sub(line.width());
                let mut spans = vec![Span::styled("│ ", border)];
                spans.extend(line.spans);
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(" │", border));
                Line::from(spans)
            })
            .collect();
    }
    Text::from(lines)
}

/// `line` with its own style on every span, cut to `width` columns.
fn fit(line: Line<'static>, width: usize) -> Line<'static> {
    let (style, fits) = (line.style, line.width() <= width);
    let spans = line.spans.into_iter().map(|s| {
        let patched = style.patch(s.style);
        s.style(patched)
    });
    if fits {
        return Line::from(spans.collect::<Vec<_>>());
    }
    let mut out = Vec::new();
    let mut left = width.saturating_sub(1);
    for span in spans {
        let mut content = String::new();
        for c in span.content.chars() {
            let w = Span::raw(c.to_string()).width();
            if w > left {
                break;
            }
            left -= w;
            content.push(c);
        }
        let full = content.chars().count() == span.content.chars().count();
        out.push(Span::styled(content, span.style));
        if !full {
            break;
        }
    }
    out.push(Span::styled("…", theme::muted()));
    Line::from(out)
}

/// Page text; URLs are highlighted like links.
pub(super) fn text_row(text: String) -> Text<'static> {
    let mut spans = vec![Span::raw(" ")];
    // A short label such as `Inhalt:` or `Input 1:` in bold, as ILIAS pages tend to have.
    let label_end = text
        .find(": ")
        .map(|at| at + 1)
        .or_else(|| text.ends_with(':').then_some(text.len()))
        .filter(|&end| text[..end].chars().count() <= 30 && !text[end..].starts_with("//"));
    let text = match label_end {
        Some(end) => {
            spans.push(Span::styled(
                text[..end].to_string(),
                Style::new()
                    .fg(theme::colors().fg)
                    .add_modifier(Modifier::BOLD),
            ));
            text[end..].to_string()
        }
        None => text,
    };
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
    Text::from(Line::from(spans))
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
