//! One level of the stack: an opened page, its items, filter and selection, laid out as rows.

use ratatui::text::Line;
use ratatui::widgets::ListState;

use crate::action::Action;
use crate::components::filter::matches;
use crate::ilias::{Block, Item, Page};

use super::*;

pub(super) struct Level {
    /// `None` for the "My courses" root.
    pub(super) item: Option<Item>,
    /// Deep search results (everything below `item`) instead of its page.
    pub(super) deep: bool,
    pub(super) page: Page,
    /// All items of `page`, in display order.
    pub(super) items: Vec<Item>,
    /// The `/` filter on this list; kept when you go deeper and come back.
    pub(super) query: String,
    /// Indices into `items` that match `query`.
    pub(super) visible: Vec<usize>,
    /// Selection: position within `visible`.
    pub(super) cursor: Option<usize>,
    /// Scroll state of the rendered rows.
    pub(super) list: ListState,
    /// Something (a favourites change) made the page wrong while it was hidden under
    /// other levels; it reloads once it is on screen again.
    pub(super) out_of_date: bool,
}

/// One rendered row; only `Item` rows can be selected.
pub(super) enum Row {
    Blank,
    Heading(String),
    Text(String),
    /// Already styled, e.g. a line of a forum post.
    Styled(Line<'static>),
    Item(usize),
}

impl Level {
    pub(super) fn new(item: Option<Item>, deep: bool, page: Page) -> Self {
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
    pub(super) fn refresh(&mut self, page: &Page) {
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
    pub(super) fn reload(&self) -> Action {
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

    pub(super) fn set_page(&mut self, page: Page) {
        self.items = page.items().cloned().collect();
        self.page = page;
        self.filter(&self.query.clone());
    }

    pub(super) fn title(&self) -> String {
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

    pub(super) fn selected(&self) -> Option<&Item> {
        self.items.get(*self.visible.get(self.cursor?)?)
    }

    pub(super) fn filter(&mut self, query: &str) {
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
    pub(super) fn move_cursor(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        match self.cursor {
            Some(cursor) => self.cursor = Some(to(cursor, self.visible.len() - 1)),
            // `draw` clamps the offset to the page length.
            None => *self.list.offset_mut() = to(self.list.offset(), usize::MAX / 2),
        }
    }

    /// Lays the page out as rows. While filtering, only matching items (with their section
    /// headings) are shown.
    pub(super) fn rows(&self, filtering: bool, width: usize) -> Vec<Row> {
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
