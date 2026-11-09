//! What a parsed page is made of: items, tabs, posts, blocks and the page itself.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

/// What an item is. Stored as ILIAS' short type name (`crs`, `fold`, …), so cached pages
/// keep working; types ilias-tui has no use for stay [`Kind::Other`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Kind {
    Root,
    Category,
    Course,
    Group,
    Folder,
    Session,
    File,
    Exercise,
    /// One task of an exercise; its id is the assignment's own number.
    Assignment,
    /// An assignment's "Handed-in files" entry (its Submission tab).
    Submission,
    Test,
    Survey,
    Forum,
    /// A forum thread; its id is the thread's number.
    Thread,
    /// The ILIAS weblink object.
    Weblink,
    /// A web link in page text.
    Link,
    /// Any other ILIAS type, by its short name (`wiki`, `lm`, …).
    Other(String),
    #[default]
    Unknown,
}

impl Kind {
    const CODES: &[(&str, Kind)] = &[
        ("root", Kind::Root),
        ("cat", Kind::Category),
        ("crs", Kind::Course),
        ("grp", Kind::Group),
        ("fold", Kind::Folder),
        ("sess", Kind::Session),
        ("file", Kind::File),
        ("exc", Kind::Exercise),
        ("ass", Kind::Assignment),
        ("sub", Kind::Submission),
        ("tst", Kind::Test),
        ("svy", Kind::Survey),
        ("frm", Kind::Forum),
        ("thr", Kind::Thread),
        ("webr", Kind::Weblink),
        ("link", Kind::Link),
        ("", Kind::Unknown),
    ];

    /// ILIAS' short type name, as in `goto.php/<code>/<ref_id>`.
    pub fn code(&self) -> &str {
        match self {
            Kind::Other(code) => code,
            kind => Self::CODES
                .iter()
                .find(|(_, k)| k == kind)
                .map_or("", |(code, _)| code),
        }
    }

    /// Courses and groups, the places a breadcrumb path starts.
    pub fn is_course(&self) -> bool {
        matches!(self, Kind::Course | Kind::Group)
    }
}

impl From<&str> for Kind {
    fn from(code: &str) -> Self {
        Self::CODES
            .iter()
            .find(|(c, _)| *c == code)
            .map_or_else(|| Kind::Other(code.to_string()), |(_, kind)| kind.clone())
    }
}

impl From<String> for Kind {
    fn from(code: String) -> Self {
        Kind::from(code.as_str())
    }
}

impl From<Kind> for String {
    fn from(kind: Kind) -> Self {
        kind.code().to_string()
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// Which object an item is. Only [`Id::Ref`] is an ILIAS reference id (what permalinks,
/// favourites and the crawl use); the others are numbers of their own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Id {
    Ref(u64),
    Assignment(u64),
    Thread(u64),
    /// Made up by ilias-tui from a link (attachments, handed-in files, text links); the top
    /// bit keeps it apart from reference ids.
    Local(u64),
    #[default]
    None,
}

impl std::fmt::Display for Id {
    /// The number, as ILIAS shows it in links.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())
    }
}

impl Id {
    /// A number for indexes kept on disk (saved files, what's new). It is the same as the
    /// one stored before ids had types.
    pub fn key(self) -> u64 {
        match self {
            Id::Ref(n) | Id::Assignment(n) | Id::Thread(n) | Id::Local(n) => n,
            Id::None => 0,
        }
    }

    /// An id that ilias-tui makes from a link (FNV-1a, like the cache's file names).
    pub fn local(link: &str) -> Self {
        let hash = link.bytes().fold(0xcbf29ce484222325_u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        });
        Id::Local(hash | 1 << 63)
    }

    /// The id stored as `key` for an item of `kind`.
    fn from_key(kind: &Kind, key: u64) -> Self {
        match kind {
            _ if key == 0 => Id::None,
            Kind::Assignment => Id::Assignment(key),
            Kind::Thread => Id::Thread(key),
            _ if key & 1 << 63 != 0 => Id::Local(key),
            _ => Id::Ref(key),
        }
    }
}

/// An object in the ILIAS repository (course, folder, file, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stored", into = "Stored")]
pub struct Item {
    pub kind: Kind,
    pub id: Id,
    pub title: String,
    pub url: String,
    pub description: String,
    /// What ILIAS lists under the title, e.g. `["pdf", "1.2 MB", "1. Mar 2026, 10:00"]`.
    pub properties: Vec<String>,
    /// Whether it is one of the user's ILIAS favourites (as far as the page says).
    pub favorite: bool,
    /// The "add to / remove from favourites" link from the item's action menu.
    pub favorite_toggle: Option<String>,
    /// Files: the folders it is in, from the course down, e.g. `["DEMO.101", "Unterlagen"]`.
    pub path: Vec<String>,
}

/// How an item is kept in the page cache (unchanged since before ids had types).
#[derive(Serialize, Deserialize)]
struct Stored {
    kind: Kind,
    ref_id: u64,
    title: String,
    url: String,
    description: String,
    properties: Vec<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    favorite_toggle: Option<String>,
    #[serde(default)]
    path: Vec<String>,
}

impl From<Stored> for Item {
    fn from(s: Stored) -> Self {
        Item {
            id: Id::from_key(&s.kind, s.ref_id),
            kind: s.kind,
            title: s.title,
            url: s.url,
            description: s.description,
            properties: s.properties,
            favorite: s.favorite,
            favorite_toggle: s.favorite_toggle,
            path: s.path,
        }
    }
}

impl From<Item> for Stored {
    fn from(i: Item) -> Self {
        Stored {
            ref_id: i.id.key(),
            kind: i.kind,
            title: i.title,
            url: i.url,
            description: i.description,
            properties: i.properties,
            favorite: i.favorite,
            favorite_toggle: i.favorite_toggle,
            path: i.path,
        }
    }
}

impl Item {
    /// The number indexes on disk use for it (see [`Id::key`]).
    pub fn key(&self) -> u64 {
        self.id.key()
    }

    pub fn is_container(&self) -> bool {
        matches!(
            self.kind,
            Kind::Root
                | Kind::Category
                | Kind::Course
                | Kind::Group
                | Kind::Folder
                | Kind::Session
                | Kind::Forum
                | Kind::Thread
                | Kind::Exercise
                | Kind::Assignment
                | Kind::Submission
        )
    }

    pub fn kind_label(&self) -> &str {
        match &self.kind {
            Kind::Root | Kind::Category => "Category",
            Kind::Course => "Course",
            Kind::Group => "Group",
            Kind::Folder => "Folder",
            Kind::Session => "Session",
            Kind::File => "File",
            Kind::Exercise => "Exercise",
            Kind::Assignment => "Assignment",
            Kind::Submission => "Submission",
            Kind::Test => "Test",
            Kind::Survey => "Survey",
            Kind::Forum => "Forum",
            Kind::Thread => "Thread",
            Kind::Weblink => "Weblink",
            Kind::Link => "Link",
            Kind::Unknown => "",
            Kind::Other(code) => match code.as_str() {
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
                other => other,
            },
        }
    }

    /// The address to share: the ILIAS permalink (`goto.php/<kind>/<ref_id>`), or the item's
    /// own link for what has no reference id (assignments, threads, text links, …).
    pub fn permalink(&self, base: &Url) -> String {
        match self.id {
            Id::Ref(ref_id) => format!("{base}goto.php/{}/{ref_id}", self.kind),
            _ => self.url.clone(),
        }
    }

    /// File extension, lowercase. ILIAS lists the file type as the first property;
    /// the title is the fallback.
    pub fn extension(&self) -> Option<String> {
        if self.kind != Kind::File {
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
    #[serde(default, deserialize_with = "skip_unreadable")]
    pub todo: Vec<Todo>,
}

/// An assignment still to hand in (My courses' To-Do).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    /// What Enter opens: the exercise, titled with the assignment's name.
    pub assignment: Item,
    pub course: Option<String>,
    /// As ILIAS shows it, e.g. `5. Nov 2026, 00:00`.
    pub deadline: String,
    /// How long was left when the list was made, e.g. `due in 3 days`; empty if unknown.
    pub due: String,
    pub urgency: Urgency,
}

/// How soon a [`Todo`] is due, for its colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Urgency {
    /// Overdue, or due within a day.
    Now,
    /// Within 3 days.
    Soon,
    #[default]
    Later,
}

/// To-Do entries cached by older versions (in another shape) are dropped; the refresh that
/// follows every cached page brings them back.
fn skip_unreadable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Todo>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect())
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
    pub fn insert_after(&mut self, after: Id, more: Vec<Item>) -> usize {
        let known: HashSet<Id> = self.items().map(|i| i.id).collect();
        for block in &mut self.blocks {
            let Block::Items { items, .. } = block else {
                continue;
            };
            let Some(at) = items.iter().position(|i| i.id == after) else {
                continue;
            };
            let path = items[at].path.clone();
            let mut seen = known;
            let more: Vec<Item> = more
                .into_iter()
                .filter(|i| seen.insert(i.id))
                .map(|mut i| {
                    if i.kind == Kind::File {
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
    pub fn remove(&mut self, id: Id) {
        for block in &mut self.blocks {
            if let Block::Items { items, .. } = block {
                items.retain(|i| i.id != id);
            }
        }
        self.blocks
            .retain(|b| !matches!(b, Block::Items { items, .. } if items.is_empty()));
    }
}
