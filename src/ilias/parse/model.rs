//! What a parsed page is made of: items, tabs, posts, blocks and the page itself.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

/// An object in the ILIAS repository (course, folder, file, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// ILIAS object type, e.g. `crs`, `grp`, `fold`, `file`.
    pub kind: String,
    pub ref_id: u64,
    pub title: String,
    pub url: String,
    pub description: String,
    /// What ILIAS lists under the title, e.g. `["pdf", "1.2 MB", "1. Mar 2026, 10:00"]`.
    pub properties: Vec<String>,
    /// Whether it is one of the user's ILIAS favourites (as far as the page says).
    #[serde(default)]
    pub favorite: bool,
    /// The "add to / remove from favourites" link from the item's action menu.
    #[serde(default)]
    pub favorite_toggle: Option<String>,
    /// Files: the folders it is in, from the course down, e.g. `["DEMO.101", "Unterlagen"]`.
    #[serde(default)]
    pub path: Vec<String>,
}

impl Item {
    pub fn is_container(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "root"
                | "cat"
                | "crs"
                | "grp"
                | "fold"
                | "sess"
                | "frm"
                | "thr"
                | "exc"
                | "ass"
                | "sub"
        )
    }

    pub fn kind_label(&self) -> &str {
        match self.kind.as_str() {
            "root" | "cat" => "Category",
            "crs" => "Course",
            "grp" => "Group",
            "fold" => "Folder",
            "sess" => "Session",
            "file" => "File",
            "exc" => "Exercise",
            "ass" => "Assignment",
            "sub" => "Submission",
            "tst" => "Test",
            "svy" => "Survey",
            "frm" => "Forum",
            "thr" => "Thread",
            "webr" => "Weblink",
            "wiki" => "Wiki",
            "blog" => "Blog",
            "mcst" => "Mediacast",
            "lm" | "htlm" | "sahs" | "copa" => "Learning module",
            "book" => "Booking",
            "dcl" => "Data collection",
            "xvid" | "xoct" => "Video",
            "iass" => "Assessment",
            "prtt" | "prtf" => "Portfolio",
            "chtr" => "Chat",
            "poll" => "Poll",
            "itgr" => "Item group",
            "link" => "Link",
            other => other,
        }
    }

    /// The address to share: the ILIAS permalink (`goto.php/<kind>/<ref_id>`), or the item's
    /// own link for what has no reference id (assignments, threads, text links, …).
    pub fn permalink(&self, base: &Url) -> String {
        let own_link = matches!(self.kind.as_str(), "ass" | "sub" | "thr" | "link")
            || self.ref_id == 0
            || self.ref_id & 1 << 63 != 0;
        match own_link {
            true => self.url.clone(),
            false => format!("{base}goto.php/{}/{}", self.kind, self.ref_id),
        }
    }

    /// File extension, lowercase. ILIAS lists the file type as the first property;
    /// the title is the fallback.
    pub fn extension(&self) -> Option<String> {
        if self.kind != "file" {
            return None;
        }
        let is_ext =
            |s: &str| (1..=5).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric());
        self.properties
            .first()
            .map(String::as_str)
            .filter(|p| is_ext(p))
            .or_else(|| {
                self.title
                    .rsplit_once('.')
                    .map(|(_, ext)| ext)
                    .filter(|e| is_ext(e))
            })
            .map(str::to_lowercase)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub title: String,
    pub url: String,
    pub active: bool,
}

/// A forum post.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    /// Name and login, e.g. `Ada Lovelace · alovelace (Moderator)`.
    pub author: String,
    pub date: String,
    pub title: String,
    /// Paragraphs of the post text.
    pub body: Vec<String>,
    /// Reply depth; 0 = the thread's first post.
    pub depth: usize,
    pub unread: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    Heading(String),
    Text(String),
    /// An item group / content block; `title` is its header (may be empty).
    Items {
        title: String,
        items: Vec<Item>,
    },
    Post(Post),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub tabs: Vec<Tab>,
    pub blocks: Vec<Block>,
    /// Assignments still to hand in; only on "My courses", shown beside the page.
    #[serde(default)]
    pub todo: Vec<Item>,
}

impl Page {
    pub fn from_items(items: Vec<Item>) -> Self {
        let blocks = vec![Block::Items {
            title: String::new(),
            items,
        }];
        Self {
            blocks,
            ..Self::default()
        }
    }

    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.blocks.iter().flat_map(|b| match b {
            Block::Items { items, .. } => items.as_slice(),
            _ => &[],
        })
    }

    /// Puts `more` right after the item `after` (in its block), leaving out items the page
    /// already has. Files get `after`'s folder path. Returns how many were added.
    pub fn insert_after(&mut self, after: u64, more: Vec<Item>) -> usize {
        let known: HashSet<u64> = self.items().map(|i| i.ref_id).collect();
        for block in &mut self.blocks {
            let Block::Items { items, .. } = block else {
                continue;
            };
            let Some(at) = items.iter().position(|i| i.ref_id == after) else {
                continue;
            };
            let path = items[at].path.clone();
            let mut seen = known;
            let more: Vec<Item> = more
                .into_iter()
                .filter(|i| seen.insert(i.ref_id))
                .map(|mut i| {
                    if i.kind == "file" {
                        i.path.clone_from(&path);
                    }
                    i
                })
                .collect();
            let added = more.len();
            items.splice(at + 1..at + 1, more);
            return added;
        }
        0
    }

    /// Drops an item everywhere (e.g. the container itself, linked from its own page).
    pub fn remove(&mut self, ref_id: u64) {
        for block in &mut self.blocks {
            if let Block::Items { items, .. } = block {
                // Assignments count separately: theirs are no reference ids.
                items.retain(|i| i.ref_id != ref_id || i.kind == "ass");
            }
        }
        self.blocks
            .retain(|b| !matches!(b, Block::Items { items, .. } if items.is_empty()));
    }
}
