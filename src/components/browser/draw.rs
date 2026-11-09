//! Drawing: header with breadcrumb, tab bar, the page, the To-Do panel; records click areas.

use ratatui::Frame;
use ratatui::layout::Margin;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block as Border, BorderType, HighlightSpacing, List, ListItem, Padding, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};

use crate::ilias::Page;
use crate::theme;

use super::*;
use crate::seen::Group;

impl Browser {
    /// The whole view: header, tabs, the page and the To-Do panel.
    pub(super) fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.hits.clear();
        let has_tabs = u16::from(self.current().is_some_and(|l| !l.page.tabs.is_empty()));
        // Header, a gap, the tabs (if any), then the content frame.
        let [header, _, tabs_area, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(has_tabs),
            Constraint::Length(has_tabs),
            Constraint::Fill(1),
        ])
        .areas(area);
        self.draw_header(frame, header);

        // "My courses": the To-Do list beside the page, or below it on narrow terminals.
        let todo_count = self.todo_items().len() as u16;
        let (body, todo_area) = if todo_count == 0 {
            (body, None)
        } else if body.width >= 100 {
            let [main, side] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(44)])
                .spacing(1)
                .areas(body);
            (main, Some(side))
        } else {
            let height = (todo_count * 4 + 1).min(body.height / 2);
            let [main, bottom] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(height)]).areas(body);
            (main, Some(bottom))
        };
        if let Some(area) = todo_area {
            self.draw_todo(frame, area);
        }

        let mut border = Border::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::colors().border))
            .padding(Padding::horizontal(1));
        let Some(level) = self.stack.last_mut() else {
            // Nothing pending any more means it failed (e.g. offline without a cached copy);
            // the status bar says why.
            let message = match self.pending {
                Some(_) => "Loading your courses…",
                None => "My courses is not available",
            };
            let loading = Paragraph::new(message)
                .style(theme::muted())
                .centered()
                .block(border);
            frame.render_widget(loading, body);
            return;
        };
        if !level.page.tabs.is_empty() {
            frame.render_widget(tab_bar(&level.page), tabs_area);
            // Same layout as `tab_bar`: indent, then " title " pills separated by a space.
            let mut x = tabs_area.x + TABS_INDENT;
            for (i, tab) in level.page.tabs.iter().enumerate() {
                let width = Span::raw(format!(" {} ", tab.title)).width() as u16;
                self.hits
                    .push((Rect::new(x, tabs_area.y, width, 1), Hit::Tab(i)));
                x += width + 1;
            }
        }
        let query = level.query.clone();
        let filtering = !query.is_empty();
        if filtering {
            let count = Line::from(vec![
                Span::styled(format!(" {} ", level.visible.len()), theme::accent_bold()),
                Span::styled(format!("of {} ", level.items.len()), theme::muted()),
            ]);
            border = border.title_bottom(count.right_aligned());
        }
        let inner = border.inner(body);
        frame.render_widget(border, body);

        let rows = level.rows(filtering, inner.width.saturating_sub(4) as usize);
        if rows.is_empty() {
            let message = if level.items.is_empty() {
                format!("{}  Nothing here", theme::empty_folder())
            } else {
                format!("{}  No matches", theme::search())
            };
            let [_, middle, _] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .areas(inner);
            frame.render_widget(
                Paragraph::new(message).style(theme::muted()).centered(),
                middle,
            );
            return;
        }
        let selected = level.cursor.and_then(|c| level.visible.get(c));
        let selected_row = rows
            .iter()
            .position(|r| matches!(r, Row::Item(i) if Some(i) == selected));
        let row_count = rows.len();
        // Visible position of each item row, for mouse hit-testing.
        let positions: Vec<Option<usize>> = rows
            .iter()
            .map(|r| match r {
                Row::Item(i) => level.visible.iter().position(|v| v == i),
                _ => None,
            })
            .collect();
        let list_items: Vec<ListItem> = rows
            .into_iter()
            .map(|row| match row {
                Row::Blank => ListItem::new(""),
                Row::Heading(text) => heading_row(text),
                Row::Text(text) => text_row(text),
                Row::Styled(line) => ListItem::new(line),
                Row::Item(i) => {
                    let item = &level.items[i];
                    // Kinds of news switched off in the settings are neither marked nor counted.
                    let shown = |group| !self.hide_new.contains(&group);
                    let mut new_inside = self
                        .new_counts
                        .get(&item.key())
                        .cloned()
                        .unwrap_or_default();
                    new_inside.0.retain(|group, _| shown(*group));
                    let marks = Marks {
                        saved: self.saved.contains(&item.key()),
                        marked: self.marked.iter().any(|m| m.id == item.id),
                        new: self.new.contains(&item.key()) && shown(Group::of(&item.kind)),
                        new_inside,
                    };
                    item_row(item, &query, marks)
                }
            })
            .collect();
        let heights: Vec<u16> = list_items.iter().map(|i| i.height() as u16).collect();
        level.list.select(selected_row);
        if selected_row.is_none() {
            // Scrolled with j/k on a page without items: stop at the end of the page.
            let last = row_count.saturating_sub(inner.height as usize);
            *level.list.offset_mut() = level.list.offset().min(last);
        }
        let list = List::new(list_items)
            .highlight_style(Style::new().bg(theme::colors().selected_bg))
            .highlight_symbol(Line::styled("▌ ", Style::new().fg(theme::colors().accent)))
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, inner, &mut level.list);

        let mut y = inner.y;
        for (row, height) in heights.iter().enumerate().skip(level.list.offset()) {
            if y >= inner.bottom() {
                break;
            }
            let height = (*height).min(inner.bottom() - y);
            if let Some(position) = positions[row] {
                let rect = Rect::new(inner.x, y, inner.width, height);
                self.hits.push((rect, Hit::Item(position)));
            }
            y += height;
        }

        // Rows can be two lines high, so this is approximate; good enough as an indicator.
        if row_count > inner.height as usize {
            let mut state = ScrollbarState::new(row_count).position(selected_row.unwrap_or(0));
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::new().fg(theme::colors().border))
                .thumb_style(Style::new().fg(theme::colors().muted));
            frame.render_stateful_widget(scrollbar, body.inner(Margin::new(0, 1)), &mut state);
        }
    }
}

impl Browser {
    fn draw_todo(&mut self, frame: &mut Frame, area: Rect) {
        let Some(level) = self.stack.last() else {
            return;
        };
        let border_color = if self.todo_focus {
            theme::colors().accent
        } else {
            theme::colors().border
        };
        let mut border = Border::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color))
            .padding(Padding::horizontal(1))
            .title(Line::from(vec![
                Span::styled(
                    " To-Do ",
                    Style::new()
                        .fg(theme::colors().yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{} ", level.page.todo.len()), theme::muted()),
            ]));
        if !self.todo_focus {
            border = border.title_bottom(Line::styled(" ⇥ focus ", theme::muted()).right_aligned());
        }
        let inner = border.inner(area);
        frame.render_widget(border, area);
        let last = level.page.todo.len() - 1;
        let rows: Vec<ListItem> = level
            .page
            .todo
            .iter()
            .enumerate()
            .map(|(i, item)| todo_row(item, i == last))
            .collect();
        let heights: Vec<u16> = rows.iter().map(|r| r.height() as u16).collect();
        if !self.todo_focus {
            self.todo.select(None);
        }
        let list = List::new(rows)
            .highlight_style(Style::new().bg(theme::colors().selected_bg))
            .highlight_symbol(Line::styled("▌ ", Style::new().fg(theme::colors().accent)))
            .highlight_spacing(HighlightSpacing::Always);
        let selected = self.todo.selected();
        frame.render_stateful_widget(list, inner, &mut self.todo);
        if !self.todo_focus {
            // Remember the place for when the panel gets the focus again.
            self.todo.select(selected);
        }
        let mut y = inner.y;
        for (i, height) in heights.iter().enumerate().skip(self.todo.offset()) {
            if y >= inner.bottom() {
                break;
            }
            let height = (*height).min(inner.bottom() - y);
            self.hits
                .push((Rect::new(inner.x, y, inner.width, height), Hit::Todo(i)));
            y += height;
        }
    }

    /// ILIAS-style top bar: logo, breadcrumb and the full name of the current object.
    fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let logo = format!(" {}  ILIAS ", theme::logo());
        let mut spans = vec![Span::styled(
            logo.clone(),
            Style::new()
                .fg(theme::colors().header_bg)
                .bg(theme::colors().accent)
                .add_modifier(Modifier::BOLD),
        )];
        // (title, stack index) per crumb.
        let mut titles: Vec<(String, usize)> = self
            .stack
            .iter()
            .enumerate()
            .map(|(i, l)| (l.title(), i))
            .collect();
        // Collapse the middle of long breadcrumbs: "My courses › … › Woche 3".
        let width = |titles: &[(String, usize)]| {
            logo.chars().count()
                + titles
                    .iter()
                    .map(|(t, _)| t.chars().count() + 3)
                    .sum::<usize>()
        };
        for i in 1..titles.len().saturating_sub(1) {
            if width(&titles) <= area.width as usize {
                break;
            }
            titles[i].0 = "…".into();
        }
        titles.dedup_by(|a, b| a.0 == "…" && b.0 == "…");
        let description = self
            .current()
            .and_then(|l| l.item.as_ref())
            .map(|i| i.description.clone())
            .filter(|d| {
                !d.is_empty() && width(&titles) + d.chars().count() + 3 <= area.width as usize
            });
        let mut x = area.x + spans[0].width() as u16;
        for (i, (title, level)) in titles.iter().enumerate() {
            let last = i + 1 == titles.len();
            let style = if last {
                Style::new()
                    .fg(theme::colors().header_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::colors().gray)
            };
            let separator = if i == 0 {
                "  ".to_string()
            } else {
                format!(" {} ", theme::chevron())
            };
            let separator = Span::styled(separator, Style::new().fg(theme::colors().muted));
            let title = Span::styled(title.clone(), style);
            x += separator.width() as u16;
            let width = title.width() as u16;
            self.hits
                .push((Rect::new(x, area.y, width, 1), Hit::Crumb(*level)));
            x += width;
            spans.push(separator);
            spans.push(title);
        }
        if let Some(description) = description {
            spans.push(Span::styled(
                format!("  ·  {description}"),
                Style::new().fg(theme::colors().gray),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::colors().header_bg)),
            area,
        );
    }
}

/// Tabs as pills, the active one filled.
/// Left edge of the tab pills: lines them up with the section headings and icons below
/// (frame border + padding + selection marker).
pub(super) const TABS_INDENT: u16 = 4;

pub(super) fn tab_bar(page: &Page) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(TABS_INDENT as usize))];
    for tab in &page.tabs {
        let style = if tab.active {
            Style::new()
                .fg(theme::colors().header_bg)
                .bg(theme::colors().accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .fg(theme::colors().gray)
                .bg(theme::colors().bar_bg)
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(" ⇥ switch", theme::muted()));
    Line::from(spans)
}
