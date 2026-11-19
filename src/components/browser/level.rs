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
    /// What can be selected, in page order: the items that match `query` (outside closed
    /// panes, unless filtering) and, while not filtering, the pane captions.
    pub(super) visible: Vec<Entry>,
    /// Whether each accordion pane of `page` is open, in page order.
    pub(super) open: Vec<bool>,
    /// Selection: position within `visible`.
    pub(super) cursor: Option<usize>,
    /// Scroll state of the rendered rows.
    pub(super) list: ListState,
    /// Something (a favourites change) made the page wrong while it was hidden under
    /// other levels; it reloads once it is on screen again.
    pub(super) out_of_date: bool,
    /// In a forum thread, the post at the top of the view (an index into its posts), as
    /// last drawn: `R` replies to it.
    pub(super) top_post: Option<usize>,
}

/// Columns a pane's borders take on each side together: `│ ` and ` │`.
pub(super) const PANE_BORDERS: usize = 4;

/// Something that can be selected: an item (index into `Level::items`) or an accordion
/// pane's caption (index into `Level::open`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Entry {
    Item(usize),
    Pane(usize),
}

/// Where a block stands on the page.
struct Place {
    /// Every pane around it is open.
    shown: bool,
    /// Index into `Level::items` of its first item.
    first_item: usize,
    /// Index into `Level::open` of the next pane.
    pane: usize,
    /// How many panes it is in.
    depth: usize,
}

/// One rendered row; only `Item` and `Pane` rows can be selected.
pub(super) enum Row {
    Blank,
    Heading(String),
    Text(String),
    /// Already styled, e.g. a line of a forum post.
    Styled(Line<'static>),
    /// The first line of the page's `n`-th post.
    PostHeader(usize, Line<'static>),
    Item(usize),
    /// An accordion pane's caption: its index into `Level::open`, title, and whether it is open.
    Pane(usize, String, bool),
    /// A pane's top border, with its label (e.g. a date) in it.
    PaneTop(String),
    /// The line between an open pane's caption and its content.
    PaneRule,
    /// A pane's bottom border; `closes` if the pane was open (its content is above).
    PaneBottom {
        closes: bool,
    },
}

impl Row {
    pub(super) fn entry(&self) -> Option<Entry> {
        match self {
            Row::Item(i) => Some(Entry::Item(*i)),
            Row::Pane(p, ..) => Some(Entry::Pane(*p)),
            _ => None,
        }
    }
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
            open: Vec::new(),
            cursor: None,
            list: ListState::default(),
            out_of_date: false,
            top_post: None,
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
        let panes = |page: &Page| -> Vec<(String, bool)> {
            page.blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Pane { title, open, .. } => Some((title.clone(), *open)),
                    _ => None,
                })
                .collect()
        };
        // A newer version of the same page keeps the panes you opened or closed.
        let (old, new) = (panes(&self.page), panes(&page));
        let same = old.len() == new.len() && old.iter().zip(&new).all(|(a, b)| a.0 == b.0);
        if !same || self.open.len() != new.len() {
            self.open = new.into_iter().map(|(_, open)| open).collect();
        }
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

    /// The posts of a forum thread, in order.
    pub(super) fn posts(&self) -> impl Iterator<Item = &crate::ilias::Post> {
        self.page.blocks.iter().filter_map(|b| match b {
            Block::Post(post) => Some(post),
            _ => None,
        })
    }

    /// The selected item; `None` on a pane caption.
    pub(super) fn selected(&self) -> Option<&Item> {
        match self.visible.get(self.cursor?)? {
            Entry::Item(i) => self.items.get(*i),
            Entry::Pane(_) => None,
        }
    }

    /// Calls `f` with each block of the page and its place.
    fn each_block<'a>(&'a self, mut f: impl FnMut(&'a Block, &Place)) {
        let mut place = Place {
            shown: true,
            first_item: 0,
            pane: 0,
            depth: 0,
        };
        // Whether each pane around the block is open.
        let mut around: Vec<bool> = Vec::new();
        for block in &self.page.blocks {
            place.shown = around.iter().all(|&open| open);
            place.depth = around.len();
            f(block, &place);
            match block {
                Block::Items { items, .. } => place.first_item += items.len(),
                Block::Pane { .. } => {
                    around.push(self.open.get(place.pane).copied().unwrap_or(true));
                    place.pane += 1;
                }
                Block::PaneEnd => {
                    around.pop();
                }
                _ => {}
            }
        }
    }

    pub(super) fn filter(&mut self, query: &str) {
        self.query = query.to_string();
        let mut visible = Vec::new();
        // The filter looks into closed panes too.
        let filtering = !query.is_empty();
        self.each_block(|block, place| match block {
            Block::Pane { .. } if !filtering && place.shown => {
                visible.push(Entry::Pane(place.pane));
            }
            Block::Items { items, .. } if filtering || place.shown => {
                visible.extend(
                    items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| {
                            matches(&format!("{} {}", item.title, item.description), query)
                        })
                        .map(|(i, _)| Entry::Item(place.first_item + i)),
                );
            }
            _ => {}
        });
        self.visible = visible;
        self.cursor = (!self.visible.is_empty()).then_some(0);
    }

    /// Opens or closes the selected pane, keeping it selected. `false` if no pane is selected.
    pub(super) fn toggle_pane(&mut self) -> bool {
        let Some(&Entry::Pane(pane)) = self.cursor.and_then(|c| self.visible.get(c)) else {
            return false;
        };
        if let Some(open) = self.open.get_mut(pane) {
            *open = !*open;
        }
        self.filter(&self.query.clone());
        self.cursor = self.visible.iter().position(|e| *e == Entry::Pane(pane));
        true
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
        let mut posts = 0;
        // Replies usually repeat the thread title; only show titles that say something new.
        let mut last_title = self
            .item
            .as_ref()
            .map(|i| i.title.clone())
            .unwrap_or_default();
        self.each_block(|block, place| {
            if !place.shown && !filtering {
                return;
            }
            // Panes are boxes; inside one, text is narrower by its borders.
            let width = width.saturating_sub(PANE_BORDERS * place.depth).max(20);
            let pane_block = matches!(block, Block::Pane { .. } | Block::PaneEnd);
            if !filtering && !pane_block && matches!(rows.last(), Some(Row::PaneBottom { .. })) {
                rows.push(Row::Blank);
            }
            match block {
                Block::Heading(text) if !filtering => {
                    blank_line(&mut rows);
                    rows.push(Row::Heading(text.clone()));
                }
                Block::Text(text) if !filtering => {
                    rows.extend(wrap(text, width).into_iter().map(Row::Text));
                }
                Block::Pane { title, label, .. } if !filtering => {
                    // Panes one after another stack without a gap.
                    if !matches!(rows.last(), Some(Row::PaneBottom { .. })) {
                        blank_line(&mut rows);
                    }
                    let open = self.open.get(place.pane).copied().unwrap_or(true);
                    rows.push(Row::PaneTop(label.clone()));
                    rows.push(Row::Pane(place.pane, title.clone(), open));
                    rows.push(match open {
                        true => Row::PaneRule,
                        false => Row::PaneBottom { closes: false },
                    });
                }
                // Shown only if its pane is open.
                Block::PaneEnd if !filtering => {
                    if matches!(rows.last(), Some(Row::Blank)) {
                        rows.pop();
                    }
                    rows.push(Row::PaneBottom { closes: true });
                }
                Block::Post(post) if !filtering => {
                    blank_line(&mut rows);
                    let title = post.title.trim_start_matches("Re: ").to_string();
                    let new_title = title != last_title;
                    post_rows(&mut rows, posts, post, new_title, width);
                    posts += 1;
                    last_title = title;
                }
                Block::Items { title, items } => {
                    let range = place.first_item..place.first_item + items.len();
                    let shown: Vec<usize> = self
                        .visible
                        .iter()
                        .filter_map(|e| match e {
                            Entry::Item(i) if range.contains(i) => Some(*i),
                            _ => None,
                        })
                        .collect();
                    if shown.is_empty() {
                        return;
                    }
                    if !title.is_empty() {
                        blank_line(&mut rows);
                        rows.push(Row::Heading(title.clone()));
                    }
                    // A gap after each item keeps its detail line apart from the next title;
                    // as its own row, it stays outside the selection highlight.
                    for i in shown {
                        rows.push(Row::Item(i));
                        // Links in text without a detail line stay together.
                        let item = &self.items[i];
                        if item.kind != crate::ilias::Kind::Link || has_details(item) {
                            rows.push(Row::Blank);
                        }
                    }
                }
                _ => {}
            }
        });
        rows
    }
}
