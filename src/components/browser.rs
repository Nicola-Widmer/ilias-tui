//! The main view: "My courses" and, below it, a stack of opened containers. Each level shows
//! the ILIAS page like the web UI does: tabs, page text, section headings and items.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Margin;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block as Border, BorderType, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph,
    Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::Component;
use super::filter::matches;
use crate::action::Action;
use crate::icons::icon;
use crate::ilias::{Block, Item, Page, Post};
use crate::theme;

struct Level {
    /// `None` for the "My courses" root.
    item: Option<Item>,
    /// Deep search results (everything below `item`) instead of its page.
    deep: bool,
    page: Page,
    /// All items of `page`, in display order.
    items: Vec<Item>,
    /// The `/` filter on this list; kept when you go deeper and come back.
    query: String,
    /// Indices into `items` that match `query`.
    visible: Vec<usize>,
    /// Selection: position within `visible`.
    cursor: Option<usize>,
    /// Scroll state of the rendered rows.
    list: ListState,
    /// Something (a favourites change) made the page wrong while it was hidden under
    /// other levels; it reloads once it is on screen again.
    out_of_date: bool,
}

/// One rendered row; only `Item` rows can be selected.
enum Row {
    Blank,
    Heading(String),
    Text(String),
    /// Already styled, e.g. a line of a forum post.
    Styled(Line<'static>),
    Item(usize),
}

impl Level {
    fn new(item: Option<Item>, deep: bool, page: Page) -> Self {
        let mut level = Self {
            item,
            deep,
            items: Vec::new(),
            page: Page::default(),
            query: String::new(),
            visible: Vec::new(),
            cursor: None,
            list: ListState::default(),
            out_of_date: false,
        };
        level.set_page(page);
        level
    }

    /// Shows a newer version of the page, keeping the selection where it was. Nothing
    /// changes if the page is the same.
    fn refresh(&mut self, page: &Page) {
        if self.page == *page {
            return;
        }
        let cursor = self.cursor;
        self.set_page(page.clone());
        if cursor.is_some_and(|c| c < self.visible.len()) {
            self.cursor = cursor;
        }
    }

    /// Fetches the page again (the tab on screen), showing the current one meanwhile.
    fn reload(&self) -> Action {
        Action::Load {
            item: self.item.clone(),
            url: self
                .page
                .tabs
                .iter()
                .find(|t| t.active)
                .map(|t| t.url.clone()),
            refresh: true,
            deep: self.deep,
        }
    }

    fn set_page(&mut self, page: Page) {
        self.items = page.items().cloned().collect();
        self.page = page;
        self.filter(&self.query.clone());
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
        self.query = query.to_string();
        self.visible = (0..self.items.len())
            .filter(|&i| {
                let item = &self.items[i];
                matches(&format!("{} {}", item.title, item.description), query)
            })
            .collect();
        self.cursor = (!self.visible.is_empty()).then_some(0);
    }

    /// Moves the selection; on pages without items (forum threads) scrolls instead.
    fn move_cursor(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        match self.cursor {
            Some(cursor) => self.cursor = Some(to(cursor, self.visible.len() - 1)),
            // `draw` clamps the offset to the page length.
            None => *self.list.offset_mut() = to(self.list.offset(), usize::MAX / 2),
        }
    }

    /// Lays the page out as rows. While filtering, only matching items (with their section
    /// headings) are shown.
    fn rows(&self, filtering: bool, width: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut index = 0;
        // Replies usually repeat the thread title; only show titles that say something new.
        let mut last_title = self
            .item
            .as_ref()
            .map(|i| i.title.clone())
            .unwrap_or_default();
        for block in &self.page.blocks {
            match block {
                Block::Heading(text) if !filtering => {
                    blank_line(&mut rows);
                    rows.push(Row::Heading(text.clone()));
                }
                Block::Text(text) if !filtering => {
                    rows.extend(wrap(text, width).into_iter().map(Row::Text));
                }
                Block::Post(post) if !filtering => {
                    blank_line(&mut rows);
                    let title = post.title.trim_start_matches("Re: ").to_string();
                    let new_title = title != last_title;
                    post_rows(&mut rows, post, new_title, width);
                    last_title = title;
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

/// A forum post: author line, then title and text behind a bar, indented by reply depth.
fn post_rows(rows: &mut Vec<Row>, post: &Post, show_title: bool, width: usize) {
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
fn todo_row(item: &Item, last: bool) -> ListItem<'static> {
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

/// What is marked on an item row besides the item itself.
#[derive(Default)]
struct Marks {
    saved: bool,
    /// Marked for sync.
    marked: bool,
    /// New or changed since your last visit.
    new: bool,
    /// A course: how many items in it are new since you last opened it.
    new_inside: usize,
}

fn item_row(item: &Item, query: &str, marks: Marks) -> ListItem<'static> {
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

fn heading_row(text: String) -> ListItem<'static> {
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
fn text_row(text: String) -> ListItem<'static> {
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

#[derive(Default)]
pub struct Browser {
    stack: Vec<Level>,
    /// What is being loaded: ref id (`None` = "My courses") and whether it is a deep search.
    /// Responses for anything else are stale (the user navigated away) and get dropped.
    pending: Option<(Option<u64>, bool)>,
    /// Ref ids of files saved locally; Enter opens these without downloading.
    saved: HashSet<u64>,
    /// Ref ids of items that are new since your last visit (marked for this run).
    new: HashSet<u64>,
    /// New items per course you haven't opened since (from `Seen`).
    new_counts: HashMap<u64, usize>,
    /// Clickable areas from the last draw.
    hits: Vec<(Rect, Hit)>,
    /// What was clicked last and when, to detect double-clicks.
    last_click: Option<(Hit, Instant)>,
    /// Selection in the To-Do panel ("My courses" only).
    todo: ListState,
    /// Keys go to the To-Do panel instead of the list.
    todo_focus: bool,
    /// Courses and folders marked with Space for the next sync.
    marked: Vec<Item>,
    /// How sync decides (from the settings), for the question before `A`.
    sync_rules: crate::sync::Rules,
}

/// What `S` can sync: places that hold files.
fn syncable(item: &Item) -> bool {
    matches!(item.kind.as_str(), "crs" | "grp" | "fold" | "sess")
}

/// What a mouse click at some screen area means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// An item row; the position within `Level::visible`.
    Item(usize),
    Tab(usize),
    /// A breadcrumb; the index into `Browser::stack`.
    Crumb(usize),
    /// An entry of the To-Do panel.
    Todo(usize),
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

impl Browser {
    pub fn new(saved: HashSet<u64>, sync_rules: crate::sync::Rules) -> Self {
        Self {
            saved,
            sync_rules,
            ..Self::default()
        }
    }

    fn current(&self) -> Option<&Level> {
        self.stack.last()
    }

    /// The filter on the list on screen.
    fn query(&self) -> &str {
        self.current().map_or("", |l| l.query.as_str())
    }

    /// `A`: asks before syncing every course in My courses.
    fn sync_all(&self) -> Action {
        // A course can be listed twice (Favourites and its semester): count and sync it once.
        let mut seen = HashSet::new();
        let courses: Vec<Item> = self
            .stack
            .iter()
            .find(|l| l.item.is_none() && !l.deep)
            .map(|l| {
                l.items
                    .iter()
                    .filter(|i| syncable(i) && seen.insert(i.ref_id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if courses.is_empty() {
            return Action::Error("No courses to sync: My courses is empty or not loaded".into());
        }
        let n = courses.len();
        Action::Ask {
            question: format!(
                "Sync all {n} course{} in My courses? {}",
                if n == 1 { "" } else { "s" },
                self.sync_rules.describe()
            ),
            then: Box::new(Action::Sync(courses)),
        }
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

    /// The To-Do entries of the page on screen.
    fn todo_items(&self) -> &[Item] {
        self.current().map_or(&[], |l| l.page.todo.as_slice())
    }

    fn click(&mut self, hit: Hit) -> Option<Action> {
        let double = self
            .last_click
            .is_some_and(|(h, at)| h == hit && at.elapsed() < DOUBLE_CLICK);
        self.last_click = (!double).then(|| (hit, Instant::now()));
        match hit {
            Hit::Item(position) => {
                self.todo_focus = false;
                self.stack.last_mut()?.cursor = Some(position);
                double
                    .then(|| self.current()?.selected().map(|i| self.open(i)))
                    .flatten()
            }
            Hit::Todo(index) => {
                self.todo_focus = true;
                self.todo.select(Some(index));
                double
                    .then(|| self.todo_items().get(index).map(|i| self.open(i)))
                    .flatten()
            }
            Hit::Tab(index) => {
                let level = self.current()?;
                let tab = level.page.tabs.get(index).filter(|t| !t.active)?;
                Some(Action::Load {
                    item: level.item.clone(),
                    url: Some(tab.url.clone()),
                    refresh: false,
                    deep: false,
                })
            }
            Hit::Crumb(index) if index + 1 < self.stack.len() => {
                self.stack.truncate(index + 1);
                self.pending = None;
                Some(Action::Filter(self.query().to_string()))
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
            refresh: false,
            deep: false,
        })
    }
}

impl Browser {
    /// Keys while the To-Do panel has the focus; `None` hands the key back to the list.
    fn todo_key(&mut self, key: KeyEvent) -> Option<Option<Action>> {
        let last = self.todo_items().len().checked_sub(1)?;
        let selected = self.todo.selected().unwrap_or(0).min(last);
        let to = match key.code {
            KeyCode::Down | KeyCode::Char('j') => (selected + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l' | 'o') => {
                return Some(self.todo_items().get(selected).map(|i| self.open(i)));
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                self.todo_focus = false;
                return Some(None);
            }
            // Anything else (filter, search, …) is about the list.
            _ => {
                self.todo_focus = false;
                return None;
            }
        };
        self.todo.select(Some(to));
        Some(None)
    }
}

impl Component for Browser {
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if self.todo_focus
            && let Some(action) = self.todo_key(key)
        {
            return action;
        }
        // On a page without tabs (My courses), Tab moves to the To-Do panel.
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            && let Some(level) = self.current()
            && level.page.tabs.is_empty()
            && !level.page.todo.is_empty()
        {
            self.todo_focus = true;
            if self.todo.selected().is_none() {
                self.todo.select(Some(0));
            }
            return None;
        }
        if key.code == KeyCode::Char('A') {
            return Some(self.sync_all());
        }
        // From inside an assignment's Submission, `H` and `X` are about the assignment.
        let assignment = self
            .stack
            .iter()
            .rev()
            .filter_map(|l| l.item.as_ref())
            .find(|i| i.kind != "sub")
            .cloned();
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
                // The list comes back with the filter you left it with; tell the search bar.
                return Some(Action::Filter(self.query().to_string()));
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
            KeyCode::Char('y') => {
                let item = level.selected().or(level.item.as_ref())?;
                return Some(Action::CopyLink(item.clone()));
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
            KeyCode::Char('f') => {
                let item = level.selected()?.clone();
                if item.favorite_toggle.is_none() {
                    return Some(Action::Error(format!(
                        "ILIAS offers no favourites option for {}",
                        item.title
                    )));
                }
                let question = if item.favorite {
                    format!("Remove “{}” from your ILIAS favourites?", item.title)
                } else {
                    format!("Add “{}” to your ILIAS favourites?", item.title)
                };
                return Some(Action::Ask {
                    question,
                    then: Box::new(Action::ToggleFavorite(item)),
                });
            }
            KeyCode::Char(' ') => {
                let item = level.selected()?.clone();
                if !syncable(&item) {
                    return Some(Action::Error(
                        "Only courses, groups and folders can be marked for sync".into(),
                    ));
                }
                match self.marked.iter().position(|m| m.ref_id == item.ref_id) {
                    Some(i) => {
                        self.marked.remove(i);
                    }
                    None => self.marked.push(item),
                }
            }
            KeyCode::Char('S') => {
                // The marked ones, else the selected course/folder, else the one you're in.
                let roots = if self.marked.is_empty() {
                    level
                        .selected()
                        .filter(|i| syncable(i))
                        .or(level.item.as_ref().filter(|i| syncable(i)))
                        .cloned()
                        .into_iter()
                        .collect()
                } else {
                    std::mem::take(&mut self.marked)
                };
                if roots.is_empty() {
                    return Some(Action::Error(
                        "Select a course or folder to sync (Space marks several)".into(),
                    ));
                }
                return Some(Action::Sync(roots));
            }
            KeyCode::Char('H') => {
                // Assignments that take files have a tab for them; other types don't.
                let submission = level.page.tabs.iter().find(|t| {
                    t.url
                        .to_ascii_lowercase()
                        .contains("cmdclass=ilexsubmissionfilegui")
                });
                return Some(match (assignment, submission) {
                    (Some(assignment), Some(tab)) => Action::AskHandIn {
                        assignment,
                        url: tab.url.clone(),
                    },
                    _ => Action::Error("Open an assignment that takes files to hand in".into()),
                });
            }
            KeyCode::Char('X') => {
                // Handed-in files link to their download with `delivered=<id>`.
                let file = level.selected().filter(|i| i.url.contains("&delivered="));
                return Some(match (assignment, file.cloned()) {
                    (Some(assignment), Some(file)) => Action::Ask {
                        question: format!(
                            "Delete {} from your submission for {}? It can't be restored.",
                            file.title, assignment.title
                        ),
                        then: Box::new(Action::DeleteSubmission { assignment, file }),
                    },
                    _ => Action::Error(
                        "Select a handed-in file (in an assignment's Submission) to delete it"
                            .into(),
                    ),
                });
            }
            KeyCode::Char('L') => return Some(Action::Login),
            _ => return None,
        }
        None
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Option<Action> {
        if self.todo_focus
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            )
        {
            let code = match mouse.kind {
                MouseEventKind::ScrollDown => KeyCode::Down,
                _ => KeyCode::Up,
            };
            return self.todo_key(KeyEvent::from(code)).flatten();
        }
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
            Action::SetConfig(config) => self.sync_rules = crate::sync::Rules::new(config),
            Action::Load { item, deep, .. } => {
                self.pending = Some((item.as_ref().map(|i| i.ref_id), *deep));
            }
            Action::Loaded {
                item,
                page,
                refresh,
                deep,
            } => {
                // Only the page asked for last counts; the rest is stale.
                if self.pending != Some((item.as_ref().map(|i| i.ref_id), *deep)) {
                    return None;
                }
                self.pending = None;
                match self.stack.last_mut() {
                    Some(level) if *refresh => level.refresh(page),
                    // Another tab of the page on screen (cached or fetched) takes its place.
                    Some(level)
                        if level.deep == *deep
                            && level.item.as_ref().map(|i| i.ref_id)
                                == item.as_ref().map(|i| i.ref_id) =>
                    {
                        *level = Level::new(item.clone(), *deep, page.clone());
                        return Some(Action::Filter(String::new()));
                    }
                    _ => {
                        self.todo_focus = false;
                        self.stack
                            .push(Level::new(item.clone(), *deep, page.clone()));
                        // A new list starts unfiltered; tell the search bar.
                        return Some(Action::Filter(String::new()));
                    }
                }
            }
            Action::Filter(query) => {
                let level = self.stack.last_mut()?;
                // Coming back with the same filter keeps the selection where it was.
                if level.query != *query {
                    level.filter(query);
                }
                // Going back (Esc, a breadcrumb) sends the level's filter: the moment a page
                // that went out of date while hidden is on screen again.
                if std::mem::take(&mut level.out_of_date) {
                    return Some(level.reload());
                }
            }
            Action::Downloaded { item, .. } => {
                self.saved.insert(item.ref_id);
            }
            Action::Synced { saved, .. } => self.saved.extend(saved),
            Action::NewItems(items) => self.new.extend(items),
            Action::NewCounts(counts) => self.new_counts.clone_from(counts),
            Action::Error(_) => self.pending = None,
            // The ★ marks (and My courses' favourites) are now wrong: reload the page on
            // screen, and mark the hidden ones that show the item, or the favourites.
            Action::FavoriteToggled {
                item,
                dry_run: false,
            } => {
                let (top, hidden) = self.stack.split_last_mut()?;
                for level in hidden {
                    level.out_of_date |=
                        level.item.is_none() || level.items.iter().any(|i| i.ref_id == item.ref_id);
                }
                return Some(top.reload());
            }
            // The submission shows on the page on screen and on the ones below it, and the
            // To-Do may change.
            Action::HandedIn { file: Some(_), .. }
            | Action::SubmissionDeleted { deleted: true, .. } => {
                let (top, hidden) = self.stack.split_last_mut()?;
                for level in hidden {
                    level.out_of_date = true;
                }
                return Some(top.reload());
            }
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
                    let marks = Marks {
                        saved: self.saved.contains(&item.ref_id),
                        marked: self.marked.iter().any(|m| m.ref_id == item.ref_id),
                        new: self.new.contains(&item.ref_id),
                        new_inside: self.new_counts.get(&item.ref_id).copied().unwrap_or(0),
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

/// Tabs as pills, the active one filled.
/// Left edge of the tab pills: lines them up with the section headings and icons below
/// (frame border + padding + selection marker).
const TABS_INDENT: u16 = 4;

fn tab_bar(page: &Page) -> Line<'static> {
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

    #[test]
    fn marks_what_is_new_since_the_last_visit() {
        let mut browser = Browser::new(HashSet::new(), Default::default());
        let courses = vec![item("crs", 1, "Algebra"), item("crs", 2, "Biologie")];
        load(&mut browser, None, Page::from_items(courses), false);
        browser.update(&Action::NewCounts(HashMap::from([(1, 3)])));
        let shown = screen(&mut browser);
        assert!(shown.contains("Algebra  ● 3 new"), "{shown}");
        assert!(!shown.contains("Biologie  ●"), "{shown}");

        let files = vec![item("file", 10, "Folien"), item("file", 11, "Skript")];
        load(
            &mut browser,
            Some(item("crs", 1, "Algebra")),
            Page::from_items(files),
            false,
        );
        browser.update(&Action::NewItems(vec![11]));
        let shown = screen(&mut browser);
        assert!(shown.contains("Skript  ● new"), "{shown}");
        assert!(!shown.contains("Folien  ●"), "{shown}");
    }

    #[test]
    fn a_asks_before_syncing_all_courses_from_anywhere() {
        let mut browser = Browser::new(HashSet::new(), Default::default());
        assert!(matches!(
            browser.handle_key_event(KeyCode::Char('A').into()),
            Some(Action::Error(_))
        ));

        let (a, b) = (item("crs", 1, "Algebra"), item("crs", 2, "Biologie"));
        // Algebra twice, as under Favourites and its semester.
        let courses = vec![a.clone(), item("webr", 3, "Link"), b, a.clone()];
        load(&mut browser, None, Page::from_items(courses), false);
        load(
            &mut browser,
            Some(a),
            Page::from_items(vec![item("fold", 4, "Woche 1")]),
            false,
        );

        let Some(Action::Ask { question, then }) =
            browser.handle_key_event(KeyCode::Char('A').into())
        else {
            panic!("expected a question");
        };
        assert!(question.starts_with("Sync all 2 courses"), "{question}");
        assert!(!question.contains("missing"), "{question}");
        let Action::Sync(roots) = *then else {
            panic!("expected a sync");
        };
        let ids: Vec<u64> = roots.iter().map(|r| r.ref_id).collect();
        assert_eq!(ids, [1, 2]);

        // The question follows the settings.
        browser.update(&Action::SetConfig(crate::config::Config {
            missing_only: true,
            ..Default::default()
        }));
        let Some(Action::Ask { question, .. }) =
            browser.handle_key_event(KeyCode::Char('A').into())
        else {
            panic!("expected a question");
        };
        assert!(question.contains("every missing file"), "{question}");
    }

    fn screen(browser: &mut Browser) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        terminal.draw(|f| browser.draw(f, f.area())).unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    #[test]
    fn links_in_page_text_can_be_selected_and_opened() {
        let mut browser = Browser::default();
        let zoom = Item {
            description: "https://zoom.us/j/1".into(),
            url: "https://zoom.us/j/1".into(),
            ..item("link", 99, "Zoom-Meeting")
        };
        let page = Page {
            blocks: vec![
                Block::Text("Online: Zoom-Meeting".into()),
                Block::Items {
                    title: String::new(),
                    items: vec![zoom.clone()],
                },
                Block::Items {
                    title: "Inhalt".into(),
                    items: vec![item("file", 2, "Skript")],
                },
            ],
            ..Page::default()
        };
        load(&mut browser, Some(item("crs", 1, "DEMO")), page, false);
        let shown = screen(&mut browser);
        assert!(shown.contains("Zoom-Meeting"), "{shown}");
        assert!(shown.contains("https://zoom.us/j/1"), "{shown}");

        // The link is the first entry; j moves on to the file and k back.
        let opens = |browser: &mut Browser, key: KeyCode| match browser.handle_key_event(key.into())
        {
            Some(Action::OpenInBrowser(item)) => item.url,
            other => panic!("expected the browser, got {other:?}"),
        };
        assert_eq!(opens(&mut browser, KeyCode::Enter), "https://zoom.us/j/1");
        browser.handle_key_event(KeyCode::Char('j').into());
        assert!(matches!(
            browser.handle_key_event(KeyCode::Enter.into()),
            Some(Action::Download { .. })
        ));
        browser.handle_key_event(KeyCode::Char('k').into());
        assert!(matches!(
            browser.handle_key_event(KeyCode::Char('y').into()),
            Some(Action::CopyLink(item)) if item.ref_id == 99
        ));
        assert_eq!(
            opens(&mut browser, KeyCode::Char('o')),
            "https://zoom.us/j/1"
        );
    }

    #[test]
    fn going_back_keeps_the_filter_of_that_list() {
        let mut browser = Browser::default();
        let mut filter = super::super::Filter::default();
        let courses = vec![
            item("crs", 1, "Algebra"),
            item("crs", 2, "Analysis"),
            item("crs", 3, "Biologie"),
        ];
        load(&mut browser, None, Page::from_items(courses), false);
        // `/an` finds Analysis.
        for key in [KeyCode::Char('/'), KeyCode::Char('a'), KeyCode::Char('n')] {
            if let Some(action) = filter.handle_key_event(key.into()) {
                browser.update(&action);
            }
        }
        filter.handle_key_event(KeyCode::Enter.into());
        assert_eq!(browser.current().unwrap().visible.len(), 1, "Analysis");
        let course = browser.current().unwrap().selected().unwrap().clone();
        assert_eq!(course.title, "Analysis");

        // Into the course: its list starts unfiltered.
        load(
            &mut browser,
            Some(course),
            Page::from_items(vec![item("fold", 4, "Woche 1")]),
            false,
        );
        assert!(browser.update(&Action::Filter(String::new())).is_none());
        filter.update(&Action::Filter(String::new()));
        assert!(!filter.is_visible());

        // Back: My courses comes back with "an", in the bar too, the selection unchanged.
        let back = browser.handle_key_event(KeyCode::Char('h').into()).unwrap();
        assert!(matches!(&back, Action::Filter(q) if q == "an"));
        browser.update(&back);
        filter.update(&back);
        assert!(filter.is_visible());
        let level = browser.current().unwrap();
        assert_eq!(level.selected().map(|i| i.title.as_str()), Some("Analysis"));
        assert_eq!(level.visible.len(), 1);
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
        let mut browser = Browser::new(HashSet::from([2]), Default::default());
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
            todo: Vec::new(),
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
            todo: Vec::new(),
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
            matches!(tab, Some(Action::Load { url: Some(ref u), refresh: false, .. }) if u == "u2")
        );
        // The other tab (here from the cache) replaces the page instead of opening on top.
        let info = Page::from_items(vec![item("file", 10, "Syllabus")]);
        load(
            &mut browser,
            Some(item("crs", 7, "DEMO")),
            info.clone(),
            false,
        );
        assert_eq!(browser.stack.len(), 1);
        assert_eq!(browser.current().unwrap().page, info);
    }

    #[test]
    fn h_hands_in_only_on_assignments_that_take_files() {
        let tab = |url: &str| crate::ilias::parse::Tab {
            title: "t".into(),
            url: url.into(),
            active: false,
        };
        let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('H').into());

        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![item("crs", 1, "DEMO")]),
            false,
        );
        assert!(matches!(press(&mut browser), Some(Action::Error(_))));

        let page = Page {
            tabs: vec![
                tab("overview"),
                tab("x?cmdClass=ilExSubmissionFileGUI&ass_id=1"),
            ],
            ..Page::default()
        };
        load(&mut browser, Some(item("exc", 2, "Blatt 1")), page, false);
        let action = press(&mut browser);
        assert!(
            matches!(action, Some(Action::AskHandIn { ref assignment, ref url })
                if assignment.title == "Blatt 1" && url.ends_with("ass_id=1"))
        );

        // A text assignment: no such tab.
        let page = Page {
            tabs: vec![tab("x?cmdClass=ilExSubmissionTextGUI&ass_id=2")],
            ..Page::default()
        };
        load(&mut browser, Some(item("ass", 3, "Essay")), page, false);
        assert!(matches!(press(&mut browser), Some(Action::Error(_))));

        // Handing in reloads the page on screen and marks the others.
        let handed_in = browser.update(&Action::HandedIn {
            assignment: item("ass", 3, "Essay"),
            file: Some("a.pdf".into()),
        });
        assert!(matches!(
            handed_in,
            Some(Action::Load { refresh: true, .. })
        ));
        assert!(browser.stack[0].out_of_date);
        let dry = browser.update(&Action::HandedIn {
            assignment: item("ass", 3, "Essay"),
            file: None,
        });
        assert!(dry.is_none(), "nothing changed");
    }

    #[test]
    fn x_asks_before_deleting_a_handed_in_file() {
        let press = |browser: &mut Browser| browser.handle_key_event(KeyCode::Char('X').into());
        let mut handed_in = item("file", 9, "Bericht.pdf");
        handed_in.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=download&ass_id=1&delivered=2".into();
        let mut browser = Browser::default();
        load(
            &mut browser,
            Some(item("ass", 2, "Laborbericht")),
            Page::from_items(vec![item("file", 8, "Vorlage.pdf"), handed_in]),
            false,
        );
        // The assignment's own file can't be deleted.
        assert!(matches!(press(&mut browser), Some(Action::Error(_))));

        browser.handle_key_event(KeyCode::Char('j').into());
        let Some(Action::Ask { question, then }) = press(&mut browser) else {
            panic!("asks first");
        };
        assert!(question.contains("Bericht.pdf") && question.contains("Laborbericht"));
        assert!(matches!(*then, Action::DeleteSubmission { ref file, .. } if file.ref_id == 9));

        // Inside the assignment's Submission, it is still about the assignment.
        let mut submission = item("sub", 3, "Submission");
        submission.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=submissionScreen&ass_id=1".into();
        let mut handed_in = item("file", 9, "Bericht.pdf");
        handed_in.url = "x?cmdClass=ilExSubmissionFileGUI&cmd=download&ass_id=1&delivered=2".into();
        let page = Page {
            tabs: vec![crate::ilias::parse::Tab {
                title: "Submission".into(),
                url: submission.url.clone(),
                active: true,
            }],
            ..Page::from_items(vec![handed_in])
        };
        load(&mut browser, Some(submission), page, false);
        let Some(Action::Ask { then, .. }) = press(&mut browser) else {
            panic!("asks first");
        };
        assert!(
            matches!(*then, Action::DeleteSubmission { ref assignment, .. } if assignment.title == "Laborbericht")
        );
        let hand_in = browser.handle_key_event(KeyCode::Char('H').into());
        assert!(
            matches!(hand_in, Some(Action::AskHandIn { ref assignment, .. }) if assignment.title == "Laborbericht")
        );

        let deleted = browser.update(&Action::SubmissionDeleted {
            assignment: item("ass", 2, "Laborbericht"),
            file: item("file", 9, "Bericht.pdf"),
            deleted: true,
        });
        assert!(matches!(deleted, Some(Action::Load { refresh: true, .. })));
    }

    #[test]
    fn todo_panel_gets_focus_with_tab_and_opens_assignments() {
        let mut browser = Browser::default();
        let mut assignment = item("exc", 5, "Blatt 1");
        assignment.description = "due in 2 days · 3. Nov 2026, 23:55".into();
        assignment.properties = vec!["DEMO.101".into()];
        let page = Page {
            todo: vec![assignment, item("exc", 6, "Blatt 2")],
            ..Page::from_items(vec![item("crs", 1, "DEMO.101")])
        };
        browser.update(&Action::Load {
            item: None,
            url: None,
            refresh: false,
            deep: false,
        });
        browser.update(&Action::Loaded {
            item: None,
            page,
            refresh: false,
            deep: false,
        });
        let key = |b: &mut Browser, code| b.handle_key_event(KeyEvent::from(code));

        // The panel sits to the right of the list on a wide terminal.
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|f| browser.draw(f, f.area())).unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(screen.contains("To-Do"), "{screen}");
        assert!(screen.contains("due in 2 days"), "{screen}");

        assert!(key(&mut browser, KeyCode::Tab).is_none());
        assert!(browser.todo_focus);
        assert!(key(&mut browser, KeyCode::Char('j')).is_none());
        let open = key(&mut browser, KeyCode::Enter);
        assert!(
            matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "Blatt 2")
        );

        // Esc gives the keys back to the course list.
        key(&mut browser, KeyCode::Esc);
        assert!(!browser.todo_focus);
        let open = key(&mut browser, KeyCode::Enter);
        assert!(
            matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "DEMO.101")
        );

        // Double-clicking an entry opens it.
        assert!(browser.click(Hit::Todo(0)).is_none());
        let open = browser.click(Hit::Todo(0));
        assert!(
            matches!(open, Some(Action::Load { item: Some(ref i), .. }) if i.title == "Blatt 1")
        );
    }

    #[test]
    fn f_asks_before_changing_favourites() {
        let mut course = item("crs", 1, "DEMO.101");
        course.favorite = true;
        course.favorite_toggle =
            Some("https://ilias.example/ilias.php?cmd=removeFromDesk&item_ref_id=1".into());
        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![course, item("crs", 2, "DEMO.102")]),
            false,
        );
        assert!(screen(&mut browser).contains("★"));

        let action = browser.handle_key_event(KeyEvent::from(KeyCode::Char('f')));
        let Some(Action::Ask { question, then }) = action else {
            panic!("expected a question, got {action:?}");
        };
        assert!(question.starts_with("Remove “DEMO.101”"), "{question}");
        assert!(matches!(*then, Action::ToggleFavorite(ref i) if i.ref_id == 1));

        // Without a menu link there is nothing to send.
        browser.handle_key_event(KeyEvent::from(KeyCode::Char('j')));
        let action = browser.handle_key_event(KeyEvent::from(KeyCode::Char('f')));
        assert!(matches!(action, Some(Action::Error(_))));
    }

    #[test]
    fn hidden_pages_reload_when_shown_again_after_a_favourites_change() {
        let mut browser = Browser::default();
        let course = item("crs", 1, "DEMO.101");
        load(
            &mut browser,
            None,
            Page::from_items(vec![course.clone()]),
            false,
        );
        let file = item("file", 5, "Lecture_01");
        load(
            &mut browser,
            Some(course.clone()),
            Page::from_items(vec![file.clone()]),
            false,
        );

        // The page on screen reloads right away …
        let now = browser.update(&Action::FavoriteToggled {
            item: file,
            dry_run: false,
        });
        assert!(
            matches!(now, Some(Action::Load { item: Some(ref i), refresh: true, .. }) if i.ref_id == 1)
        );
        // … while a refresh of hidden My courses is not applied behind the course.
        browser.update(&Action::Loaded {
            item: None,
            page: Page::default(),
            refresh: true,
            deep: false,
        });
        assert_eq!(browser.current().unwrap().items[0].ref_id, 5);

        // Going back shows My courses and reloads it, once.
        let back = browser
            .handle_key_event(KeyEvent::from(KeyCode::Esc))
            .unwrap();
        let reload = browser.update(&back);
        assert!(matches!(
            reload,
            Some(Action::Load {
                item: None,
                refresh: true,
                ..
            })
        ));
        assert!(browser.update(&Action::Filter(String::new())).is_none());
    }

    #[test]
    fn a_dry_run_marks_nothing_out_of_date() {
        let mut browser = Browser::default();
        load(
            &mut browser,
            None,
            Page::from_items(vec![item("crs", 1, "DEMO.101")]),
            false,
        );
        let change = Action::FavoriteToggled {
            item: item("crs", 1, "DEMO.101"),
            dry_run: true,
        };
        assert!(browser.update(&change).is_none());
    }
}
