//! The main view: "My courses" and, below it, a stack of opened containers. Each level shows
//! the ILIAS page like the web UI does: tabs, page text, section headings and items.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Margin;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block as Border, BorderType, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph,
    Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::Component;
use super::filter::matches;
use crate::action::Action;
use crate::icons::icon;
use crate::ilias::{Block, Item, Page};
use crate::theme;

struct Level {
    /// `None` for the "My courses" root.
    item: Option<Item>,
    /// Deep search results (everything below `item`) instead of its page.
    deep: bool,
    page: Page,
    /// All items of `page`, in display order.
    items: Vec<Item>,
    /// Indices into `items` that match the current filter.
    visible: Vec<usize>,
    /// Selection: position within `visible`.
    cursor: Option<usize>,
    /// Scroll state of the rendered rows.
    list: ListState,
}

/// One rendered row; only `Item` rows can be selected.
enum Row {
    Blank,
    Heading(String),
    Text(String),
    Item(usize),
}

impl Level {
    fn new(item: Option<Item>, deep: bool, page: Page) -> Self {
        let mut level = Self {
            item,
            deep,
            items: Vec::new(),
            page: Page::default(),
            visible: Vec::new(),
            cursor: None,
            list: ListState::default(),
        };
        level.set_page(page, "");
        level
    }

    fn set_page(&mut self, page: Page, query: &str) {
        self.items = page.items().cloned().collect();
        self.page = page;
        self.filter(query);
    }

    fn title(&self) -> String {
        let name = self
            .item
            .as_ref()
            .map_or("My courses", |i| i.title.as_str());
        if self.deep {
            format!("Search in {name}")
        } else {
            name.to_string()
        }
    }

    fn selected(&self) -> Option<&Item> {
        self.items.get(*self.visible.get(self.cursor?)?)
    }

    fn filter(&mut self, query: &str) {
        self.visible = (0..self.items.len())
            .filter(|&i| {
                let item = &self.items[i];
                matches(&format!("{} {}", item.title, item.description), query)
            })
            .collect();
        self.cursor = (!self.visible.is_empty()).then_some(0);
    }

    fn move_cursor(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        if let Some(cursor) = self.cursor {
            self.cursor = Some(to(cursor, self.visible.len() - 1));
        }
    }

    /// Lays the page out as rows. While filtering, only matching items (with their section
    /// headings) are shown.
    fn rows(&self, filtering: bool, width: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut index = 0;
        for block in &self.page.blocks {
            match block {
                Block::Heading(text) if !filtering => {
                    blank_line(&mut rows);
                    rows.push(Row::Heading(text.clone()));
                }
                Block::Text(text) if !filtering => {
                    rows.extend(wrap(text, width).into_iter().map(Row::Text));
                }
                Block::Items { title, items } => {
                    let range = index..index + items.len();
                    index += items.len();
                    let shown: Vec<usize> = self
                        .visible
                        .iter()
                        .copied()
                        .filter(|i| range.contains(i))
                        .collect();
                    if shown.is_empty() {
                        continue;
                    }
                    if !title.is_empty() {
                        blank_line(&mut rows);
                        rows.push(Row::Heading(title.clone()));
                    }
                    // A gap after each item keeps its detail line apart from the next title;
                    // as its own row, it stays outside the selection highlight.
                    for i in shown {
                        rows.push(Row::Item(i));
                        rows.push(Row::Blank);
                    }
                }
                _ => {}
            }
        }
        rows
    }
}

/// Separates a new section from what came before, without doubling up empty lines.
fn blank_line(rows: &mut Vec<Row>) {
    if !rows.is_empty() && !matches!(rows.last(), Some(Row::Blank)) {
        rows.push(Row::Blank);
    }
}

/// Greedy word wrap.
fn wrap(text: &str, width: usize) -> Vec<String> {
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

fn item_row(item: &Item, query: &str, saved: bool) -> ListItem<'static> {
    let (glyph, color) = icon(item);
    let title_style = if item.is_container() {
        Style::new().fg(theme::FG).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme::FG)
    };
    let mut title = vec![Span::styled(format!("{glyph}  "), Style::new().fg(color))];
    title.extend(highlight(&item.title, query, title_style));
    if saved {
        title.push(Span::styled(
            format!("  {} saved", theme::check()),
            Style::new().fg(theme::GREEN),
        ));
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

fn heading_row(text: String) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled("▍", Style::new().fg(theme::ACCENT)),
        Span::styled(
            text,
            Style::new().fg(theme::YELLOW).add_modifier(Modifier::BOLD),
        ),
    ]))
}

/// Page text; URLs are highlighted like links.
fn text_row(text: String) -> ListItem<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if word.starts_with("http://") || word.starts_with("https://") {
            Style::new()
                .fg(theme::BLUE)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new().fg(theme::FG)
        };
        spans.push(Span::styled(word.to_string(), style));
    }
    ListItem::new(Line::from(spans))
}

#[derive(Default)]
pub struct Browser {
    stack: Vec<Level>,
    /// What is being loaded: ref id (`None` = "My courses") and whether it is a deep search.
    /// Responses for anything else are stale (the user navigated away) and get dropped.
    pending: Option<(Option<u64>, bool)>,
    query: String,
    /// Ref ids of files saved locally; Enter opens these without downloading.
    saved: HashSet<u64>,
    /// Clickable areas from the last draw.
    hits: Vec<(Rect, Hit)>,
    /// Item position and time of the last click, to detect double-clicks.
    last_click: Option<(usize, Instant)>,
}

/// What a mouse click at some screen area means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// An item row; the position within `Level::visible`.
    Item(usize),
    Tab(usize),
    /// A breadcrumb; the index into `Browser::stack`.
    Crumb(usize),
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Browser {
    pub fn new(saved: HashSet<u64>) -> Self {
        Self {
            saved,
            ..Self::default()
        }
    }

    fn current(&self) -> Option<&Level> {
        self.stack.last()
    }

    fn open(&self, item: &Item) -> Action {
        if item.is_container() {
            Action::Load {
                item: Some(item.clone()),
                url: None,
                refresh: false,
                deep: false,
            }
        } else if item.kind == "file" {
            Action::Download {
                item: item.clone(),
                open: true,
            }
        } else {
            Action::OpenInBrowser(item.clone())
        }
    }

    fn click(&mut self, hit: Hit) -> Option<Action> {
        match hit {
            Hit::Item(position) => {
                let double = self
                    .last_click
                    .is_some_and(|(p, at)| p == position && at.elapsed() < DOUBLE_CLICK);
                self.stack.last_mut()?.cursor = Some(position);
                if double {
                    self.last_click = None;
                    return self.current()?.selected().map(|i| self.open(i));
                }
                self.last_click = Some((position, Instant::now()));
                None
            }
            Hit::Tab(index) => {
                let level = self.current()?;
                let tab = level.page.tabs.get(index).filter(|t| !t.active)?;
                Some(Action::Load {
                    item: level.item.clone(),
                    url: Some(tab.url.clone()),
                    refresh: true,
                    deep: false,
                })
            }
            Hit::Crumb(index) if index + 1 < self.stack.len() => {
                self.stack.truncate(index + 1);
                self.pending = None;
                Some(Action::Filter(String::new()))
            }
            Hit::Crumb(_) => None,
        }
    }

    /// Loads the next (`+1`) or previous (`-1`) tab of the current page.
    fn switch_tab(&self, step: isize) -> Option<Action> {
        let level = self.current()?;
        let tabs = &level.page.tabs;
        let active = tabs.iter().position(|t| t.active).unwrap_or(0) as isize;
        let next = (active + step).rem_euclid(tabs.len().max(1) as isize) as usize;
        Some(Action::Load {
            item: level.item.clone(),
            url: Some(tabs.get(next)?.url.clone()),
            refresh: true,
            deep: false,
        })
    }
}

impl Component for Browser {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        let level = self.stack.last_mut()?;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => level.move_cursor(|c, last| (c + 1).min(last)),
            KeyCode::Up | KeyCode::Char('k') => level.move_cursor(|c, _| c.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => level.move_cursor(|_, _| 0),
            KeyCode::End | KeyCode::Char('G') => level.move_cursor(|_, last| last),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                return self.current()?.selected().map(|i| self.open(i));
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Esc | KeyCode::Char('h') => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
                self.pending = None;
                // Lists start unfiltered when you come back to them.
                return Some(Action::Filter(String::new()));
            }
            KeyCode::Tab => return self.switch_tab(1),
            KeyCode::BackTab => return self.switch_tab(-1),
            KeyCode::Char('d') => {
                let item = level.selected().filter(|i| i.kind == "file")?;
                return Some(Action::Download {
                    item: item.clone(),
                    open: false,
                });
            }
            KeyCode::Char('o') => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::OpenInBrowser(item.clone()));
            }
            KeyCode::Char('r') => {
                let url = level
                    .page
                    .tabs
                    .iter()
                    .find(|t| t.active)
                    .map(|t| t.url.clone());
                return Some(Action::Load {
                    item: level.item.clone(),
                    url,
                    refresh: true,
                    deep: level.deep,
                });
            }
            KeyCode::Char('s') if !level.deep => {
                return Some(Action::Load {
                    item: level.item.clone(),
                    url: None,
                    refresh: false,
                    deep: true,
                });
            }
            KeyCode::Char('L') => return Some(Action::Login),
            _ => return None,
        }
        None
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        let level = self.stack.last_mut()?;
        match mouse.kind {
            MouseEventKind::ScrollDown => level.move_cursor(|c, last| (c + 1).min(last)),
            MouseEventKind::ScrollUp => level.move_cursor(|c, _| c.saturating_sub(1)),
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                let (_, hit) = *self.hits.iter().find(|(rect, _)| rect.contains(at))?;
                return self.click(hit);
            }
            _ => {}
        }
        None
    }

    fn update(&mut self, action: &Action) -> Option<Action> {
        match action {
            Action::Load { item, deep, .. } => {
                self.pending = Some((item.as_ref().map(|i| i.ref_id), *deep));
            }
            Action::Loaded {
                item,
                page,
                refresh,
                deep,
            } => {
                if self.pending != Some((item.as_ref().map(|i| i.ref_id), *deep)) {
                    return None;
                }
                self.pending = None;
                match self.stack.last_mut() {
                    // A background refresh that found nothing new: keep the view as it is.
                    Some(level) if *refresh && level.page == *page => {}
                    Some(level) if *refresh => {
                        let cursor = level.cursor;
                        level.set_page(page.clone(), &self.query);
                        if cursor.is_some_and(|c| c < level.visible.len()) {
                            level.cursor = cursor;
                        }
                    }
                    _ => {
                        self.query.clear();
                        self.stack
                            .push(Level::new(item.clone(), *deep, page.clone()));
                        // A new list starts unfiltered; tell the search bar.
                        return Some(Action::Filter(String::new()));
                    }
                }
            }
            Action::Filter(query) => {
                self.query.clone_from(query);
                if let Some(level) = self.stack.last_mut() {
                    level.filter(query);
                }
            }
            Action::Downloaded { item, .. } => {
                self.saved.insert(item.ref_id);
            }
            Action::Error(_) => self.pending = None,
            // Reload whatever is on screen with the fresh session.
            Action::LoggedIn => {
                let level = self.current();
                return Some(Action::Load {
                    item: level.and_then(|l| l.item.clone()),
                    url: None,
                    refresh: level.is_some(),
                    deep: level.is_some_and(|l| l.deep),
                });
            }
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
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

        let mut border = Border::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(theme::BORDER))
            .padding(Padding::horizontal(1));
        let Some(level) = self.stack.last_mut() else {
            let loading = Paragraph::new("Loading your courses…")
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
        let filtering = !self.query.is_empty();
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
                Row::Item(i) => {
                    let item = &level.items[i];
                    item_row(item, &self.query, self.saved.contains(&item.ref_id))
                }
            })
            .collect();
        let heights: Vec<u16> = list_items.iter().map(|i| i.height() as u16).collect();
        level.list.select(selected_row);
        let list = List::new(list_items)
            .highlight_style(Style::new().bg(theme::SELECTED_BG))
            .highlight_symbol(Line::styled("▌ ", Style::new().fg(theme::ACCENT)))
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
                .track_style(Style::new().fg(theme::BORDER))
                .thumb_style(Style::new().fg(theme::MUTED));
            frame.render_stateful_widget(scrollbar, body.inner(Margin::new(0, 1)), &mut state);
        }
    }
}

impl Browser {
    /// ILIAS-style top bar: logo, breadcrumb and the full name of the current object.
    fn draw_header(&mut self, frame: &mut Frame, area: Rect) {
        let logo = format!(" {}  ILIAS ", theme::logo());
        let mut spans = vec![Span::styled(
            logo.clone(),
            Style::new()
                .fg(theme::HEADER_BG)
                .bg(theme::ACCENT)
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
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::GRAY)
            };
            let separator = if i == 0 {
                "  ".to_string()
            } else {
                format!(" {} ", theme::chevron())
            };
            let separator = Span::styled(separator, Style::new().fg(theme::MUTED));
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
                Style::new().fg(theme::GRAY),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::HEADER_BG)),
            area,
        );
    }
}

/// Splits `text` into spans, emphasising every case-insensitive occurrence of a query word.
fn highlight(text: &str, query: &str, base: Style) -> Vec<Span<'static>> {
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
    let matched = base.fg(theme::YELLOW).add_modifier(Modifier::UNDERLINED);
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

/// Tabs as pills, the active one filled.
/// Left edge of the tab pills: lines them up with the section headings and icons below
/// (frame border + padding + selection marker).
const TABS_INDENT: u16 = 4;

fn tab_bar(page: &Page) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(TABS_INDENT as usize))];
    for tab in &page.tabs {
        let style = if tab.active {
            Style::new()
                .fg(theme::HEADER_BG)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::GRAY).bg(theme::BAR_BG)
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(" ⇥ switch", theme::muted()));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn item(kind: &str, ref_id: u64, title: &str) -> Item {
        let url = format!("https://ilias.example/goto.php/{kind}/{ref_id}");
        Item {
            kind: kind.into(),
            ref_id,
            title: title.into(),
            url,
            ..Default::default()
        }
    }

    fn load(browser: &mut Browser, item: Option<Item>, page: Page, deep: bool) {
        browser.update(&Action::Load {
            item: item.clone(),
            url: None,
            refresh: false,
            deep,
        });
        browser.update(&Action::Loaded {
            item,
            page,
            refresh: false,
            deep,
        });
    }

    fn screen(browser: &mut Browser) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        terminal.draw(|f| browser.draw(f, f.area())).unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    #[test]
    fn navigates_into_containers_and_back() {
        let course = item("crs", 1, "Informatik 1");
        let file = item("file", 2, "Skript.pdf");
        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![course.clone()]),
            false,
        );

        let open = browser.handle_key_event(KeyCode::Enter.into());
        assert!(matches!(open, Some(Action::Load { item: Some(ref i), .. }) if *i == course));
        load(
            &mut browser,
            Some(course),
            Page::from_items(vec![file.clone()]),
            false,
        );
        assert!(
            matches!(browser.handle_key_event(KeyCode::Enter.into()), Some(Action::Download { item: ref i, open: true }) if *i == file)
        );

        let screen = screen(&mut browser);
        assert!(
            screen.contains("My courses") && screen.contains("Informatik 1"),
            "{screen}"
        );
        assert!(screen.contains("Skript.pdf"), "{screen}");

        browser.handle_key_event(KeyCode::Esc.into());
        assert_eq!(browser.current().unwrap().title(), "My courses");
    }

    #[test]
    fn marks_saved_files_and_d_saves_without_opening() {
        let file = item("file", 2, "Skript");
        let mut browser = Browser::new(HashSet::from([2]));
        load(
            &mut browser,
            None,
            Page::from_items(vec![file.clone()]),
            false,
        );
        assert!(screen(&mut browser).contains("saved"));

        let d = browser.handle_key_event(KeyCode::Char('d').into());
        assert!(matches!(d, Some(Action::Download { open: false, .. })));

        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![file.clone()]),
            false,
        );
        assert!(!screen(&mut browser).contains("saved"));
        browser.update(&Action::Downloaded {
            item: file,
            path: "/tmp/Skript.pdf".into(),
            open: true,
        });
        assert!(screen(&mut browser).contains("saved"));
    }

    fn click(browser: &mut Browser, hit: Hit) -> Option<Action> {
        let (rect, _) = *browser
            .hits
            .iter()
            .find(|(_, h)| *h == hit)
            .expect("hit drawn");
        browser.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        })
    }

    #[test]
    fn mouse_selects_opens_switches_tabs_and_goes_back() {
        let course = item("crs", 1, "DEMO");
        let mut file = item("file", 3, "Skript");
        file.properties = vec!["pdf".into()];
        let page = Page {
            tabs: vec![
                crate::ilias::parse::Tab {
                    title: "Inhalt".into(),
                    url: "u1".into(),
                    active: true,
                },
                crate::ilias::parse::Tab {
                    title: "Info".into(),
                    url: "u2".into(),
                    active: false,
                },
            ],
            blocks: vec![Block::Items {
                title: "Unterlagen".into(),
                items: vec![item("fold", 2, "Woche 1"), file],
            }],
        };
        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![course.clone()]),
            false,
        );
        load(&mut browser, Some(course), page, false);
        screen(&mut browser);

        // Single click selects, a second click on the same item opens it.
        assert!(click(&mut browser, Hit::Item(1)).is_none());
        assert_eq!(browser.current().unwrap().selected().unwrap().ref_id, 3);
        let open = click(&mut browser, Hit::Item(1));
        assert!(matches!(open, Some(Action::Download { open: true, .. })));

        // Wheel moves the selection.
        browser.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(browser.current().unwrap().selected().unwrap().ref_id, 2);

        let tab = click(&mut browser, Hit::Tab(1));
        assert!(matches!(tab, Some(Action::Load { url: Some(ref u), .. }) if u == "u2"));
        assert!(
            click(&mut browser, Hit::Tab(0)).is_none(),
            "active tab: nothing to do"
        );

        // Clicking "My courses" in the breadcrumb goes back.
        click(&mut browser, Hit::Crumb(0));
        assert_eq!(browser.stack.len(), 1);
    }

    #[test]
    fn drops_stale_responses() {
        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![item("crs", 1, "A")]),
            false,
        );
        browser.update(&Action::Load {
            item: Some(item("crs", 1, "A")),
            url: None,
            refresh: false,
            deep: false,
        });
        browser.handle_key_event(KeyCode::Esc.into()); // user gave up waiting
        browser.update(&Action::Loaded {
            item: Some(item("crs", 1, "A")),
            page: Page::default(),
            refresh: false,
            deep: false,
        });
        assert_eq!(browser.stack.len(), 1);
    }

    #[test]
    fn filters_and_selects_within_matches() {
        let mut browser = Browser::default();
        let mut described = item("file", 3, "Woche 2 › Lecture_02");
        described.description = "Folien".into();
        let items = vec![
            item("fold", 1, "Woche 1"),
            item("file", 2, "Woche 1 › Notizen"),
            described,
        ];
        load(&mut browser, None, Page::from_items(items), false);

        // Matches the description too.
        browser.update(&Action::Filter("folien 2".into()));
        let screen = screen(&mut browser);
        assert!(screen.contains("1 of 3"), "{screen}");
        assert!(!screen.contains("Woche 1"), "{screen}");
        assert!(
            matches!(browser.handle_key_event(KeyCode::Enter.into()), Some(Action::Download { item: ref i, .. }) if i.ref_id == 3)
        );
    }

    #[test]
    fn deep_search_opens_search_level() {
        let course = item("crs", 1, "DEMO");
        let mut browser = Browser::default();
        load(&mut browser, Some(course.clone()), Page::default(), false);
        let action = browser.handle_key_event(KeyCode::Char('s').into());
        assert!(matches!(action, Some(Action::Load { deep: true, .. })));
        load(
            &mut browser,
            Some(course),
            Page::from_items(vec![item("file", 2, "A › B.pdf")]),
            true,
        );
        assert!(screen(&mut browser).contains("Search in DEMO"));
    }

    #[test]
    fn renders_course_page_like_ilias() {
        let mut file = item("file", 9, "Lecture_01");
        file.properties = vec!["pdf".into(), "1.2 MB".into()];
        let page = Page {
            tabs: vec![
                crate::ilias::parse::Tab {
                    title: "Inhalt".into(),
                    url: "u1".into(),
                    active: true,
                },
                crate::ilias::parse::Tab {
                    title: "Info".into(),
                    url: "u2".into(),
                    active: false,
                },
            ],
            blocks: vec![
                Block::Text("Willkommen im Modul DEMO".into()),
                Block::Items {
                    title: "Unterlagen".into(),
                    items: vec![item("fold", 8, "Übungen"), file],
                },
            ],
        };
        let mut browser = Browser::default();
        load(&mut browser, Some(item("crs", 7, "DEMO")), page, false);

        let screen = screen(&mut browser);
        for expected in [
            "Inhalt",
            "Info",
            "Willkommen im Modul",
            "Unterlagen",
            "Übungen",
            "pdf  ·  1.2 MB",
        ] {
            assert!(
                screen.contains(expected),
                "missing {expected:?} in {screen}"
            );
        }
        // Text rows are skipped: the first selectable row is the folder.
        assert_eq!(browser.current().unwrap().selected().unwrap().ref_id, 8);
        browser.handle_key_event(KeyCode::Down.into());
        assert_eq!(browser.current().unwrap().selected().unwrap().ref_id, 9);

        let tab = browser.handle_key_event(KeyCode::Tab.into());
        assert!(
            matches!(tab, Some(Action::Load { url: Some(ref u), refresh: true, .. }) if u == "u2")
        );
    }
}
