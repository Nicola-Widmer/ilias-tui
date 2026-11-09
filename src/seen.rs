//! What's new since your last visit: the items of a course or folder that appeared or changed
//! since you last looked at it.
//!
//! [`diff`] compares a freshly fetched page with the cached copy. What a sync finds this way
//! is kept as *unseen* until you open the course or folder it is in; [`Seen`] stores that
//! next to the session as `<ilias host>.seen.json`:
//! `{ "<container>": { "course": <id>, "path": [<folder>, …], "items": [<id>, …] } }`
//! (`path`: the folders from the course down to the container). Every course and folder on
//! the way counts them, so the trail to a new file shows on each level. What a page you are
//! looking at shows new is seen right away.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::ilias::parse::parse_date;
use crate::ilias::{Item, Kind, Page};
use crate::session;

/// What is new on a page, by what it means to you.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct New {
    /// New or changed files, folders and everything else.
    pub items: Vec<u64>,
    /// Forums and threads with new posts.
    pub posts: Vec<u64>,
}

impl New {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.posts.is_empty()
    }

    /// Everything, to mark on screen (each row says itself which kind of new it is).
    pub fn all(&self) -> Vec<u64> {
        self.items.iter().chain(&self.posts).copied().collect()
    }
}

/// How much is new below a course or folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub items: usize,
    pub posts: usize,
}

/// What `new` has that `old` doesn't (by id), or has with other properties. Forums and
/// threads count as new posts only when one of their numbers went up: reading posts lowers
/// the unread count, which is no news. `old_day` and `new_day` are the days each copy was
/// fetched on, so that ILIAS' relative dates ("Today, 10:00" yesterday is "Yesterday, 10:00"
/// today) compare as the same date.
pub fn diff(old: &Page, old_day: Date, new: &Page, new_day: Date) -> New {
    let before: HashMap<u64, Vec<String>> = old
        .items()
        .map(|i| (i.key(), version(i, old_day)))
        .collect();
    let mut found = New::default();
    for item in new.items() {
        let now = version(item, new_day);
        match (before.get(&item.key()), &item.kind) {
            (Some(was), _) if *was == now => {}
            // A new thread is a new post; a new forum is new content.
            (None, Kind::Thread) => found.posts.push(item.key()),
            (None, _) => found.items.push(item.key()),
            (Some(was), Kind::Forum | Kind::Thread) => {
                if went_up(was, &now) {
                    found.posts.push(item.key());
                }
            }
            (Some(_), _) => found.items.push(item.key()),
        }
    }
    found
}

/// Whether any number in the properties grew (posts, unread posts, the last post's date).
fn went_up(was: &[String], now: &[String]) -> bool {
    let numbers = |properties: &[String]| -> Vec<u64> {
        properties
            .join(" ")
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|n| n.parse().ok())
            .collect()
    };
    let (was, now) = (numbers(was), numbers(now));
    was.len() != now.len() || now.iter().zip(&was).any(|(now, was)| now > was)
}

/// The item's properties, with dates made absolute and without countdowns.
fn version(item: &Item, day: Date) -> Vec<String> {
    item.properties
        .iter()
        .filter(|p| !is_countdown(p))
        .map(|p| match parse_date(p, day) {
            Some(date) => date.to_string(),
            None => p.clone(),
        })
        .collect()
}

/// Time left, which changes by itself: `Deadline: 5 Hours, 8 Minutes, 11 Seconds`,
/// `Abgabetermin: 2 Tage, 3 Stunden`.
fn is_countdown(property: &str) -> bool {
    const UNITS: &[&str] = &[
        "year", "month", "week", "day", "hour", "minute", "second", "jahr", "monat", "woche",
        "tag", "stunde", "sekunde",
    ];
    let value = property
        .rsplit_once(": ")
        .map_or(property, |(_, value)| value);
    value.split(", ").all(|part| {
        let mut words = part.split_whitespace();
        let number = words.next().is_some_and(|n| n.parse::<u32>().is_ok());
        let unit = words.next().map(str::to_lowercase);
        number
            && unit.is_some_and(|u| UNITS.iter().any(|unit| u.starts_with(unit)))
            && words.next().is_none()
    })
}

/// The items one container got since you last opened it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Unseen {
    /// The course (or the synced folder) it belongs to, where My courses counts it.
    course: u64,
    /// The folders between the course and the container, from the top.
    #[serde(default)]
    path: Vec<u64>,
    items: BTreeSet<u64>,
    /// Forums and threads with new posts.
    #[serde(default)]
    posts: BTreeSet<u64>,
}

#[derive(Debug, Default)]
pub struct Seen {
    file: Option<PathBuf>,
    /// By container ref id.
    unseen: BTreeMap<u64, Unseen>,
}

impl Seen {
    pub fn load(base: &Url) -> Self {
        Self::from_file(session::host_file(base, "seen"))
    }

    fn from_file(file: Option<PathBuf>) -> Self {
        let unseen = session::read_json(file.as_deref()).unwrap_or_default();
        Self { file, unseen }
    }

    #[cfg(test)]
    pub fn in_file(file: PathBuf) -> Self {
        Self::from_file(Some(file))
    }

    /// Remembers `items` of `container` as not seen yet; `path` leads from `course` down to
    /// `container` (the folders in between).
    pub fn record(&mut self, course: u64, path: &[u64], container: u64, new: &New) {
        if new.is_empty() {
            return;
        }
        let entry = self.unseen.entry(container).or_default();
        entry.course = course;
        entry.path = path.to_vec();
        entry.items.extend(&new.items);
        entry.posts.extend(&new.posts);
        self.save();
    }

    /// You opened `place`: what is unseen on its own page is seen now. Folders below keep
    /// theirs (and keep counting) until you open them. Returns the items, to mark them.
    pub fn open(&mut self, place: u64) -> Vec<u64> {
        let Some(unseen) = self.unseen.remove(&place) else {
            return Vec::new();
        };
        self.save();
        unseen.items.into_iter().chain(unseen.posts).collect()
    }

    /// Unseen items per course and folder: each counts what is below it, its own page
    /// included.
    pub fn counts(&self) -> HashMap<u64, Counts> {
        let mut counts: HashMap<u64, Counts> = HashMap::new();
        for (container, unseen) in &self.unseen {
            let mut places: BTreeSet<u64> = unseen.path.iter().copied().collect();
            places.extend([unseen.course, *container]);
            for place in places {
                let count = counts.entry(place).or_default();
                count.items += unseen.items.len();
                count.posts += unseen.posts.len();
            }
        }
        counts
    }

    /// Best effort, like the page cache: a failed write only loses a "new" mark.
    fn save(&self) {
        if let Some(file) = &self.file {
            let _ = session::write_json(file, &self.unseen);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::ilias::Id;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    fn items(keys: &[u64]) -> New {
        New {
            items: keys.to_vec(),
            posts: Vec::new(),
        }
    }

    /// The counts of new items (not posts) by place.
    fn counted(seen: &Seen) -> HashMap<u64, usize> {
        seen.counts()
            .into_iter()
            .map(|(k, c)| (k, c.items))
            .collect()
    }

    fn item(ref_id: u64, properties: &[&str]) -> Item {
        Item {
            kind: "file".into(),
            id: Id::Ref(ref_id),
            title: format!("File {ref_id}"),
            properties: properties.iter().map(|p| p.to_string()).collect(),
            ..Item::default()
        }
    }

    #[test]
    fn finds_new_and_changed_items() {
        let day = date(2026, 11, 9);
        let old = Page::from_items(vec![
            item(1, &["pdf", "1.2 MB", "1. Nov 2026, 10:00"]),
            item(2, &["pdf", "3 MB", "2. Nov 2026, 10:00"]),
            item(3, &["docx"]),
        ]);
        let new = Page::from_items(vec![
            item(1, &["pdf", "1.2 MB", "1. Nov 2026, 10:00"]),
            // Replaced by a new version.
            item(2, &["pdf", "3.1 MB", "9. Nov 2026, 08:00"]),
            item(4, &["xlsx"]),
            // 3 was deleted: nothing to mark.
        ]);
        assert_eq!(diff(&old, day, &new, day).items, [2, 4]);
        assert!(diff(&new, day, &new, day).is_empty());
    }

    #[test]
    fn forums_count_new_posts_not_reading() {
        let day = date(2026, 11, 9);
        let forum = |kind: &str, id, posts: &str| Item {
            kind: kind.into(),
            ..item(id, &[posts])
        };
        let old = Page::from_items(vec![
            forum("frm", 1, "Posts (Unread): 2 (14)"),
            forum("frm", 2, "Posts (Unread): 3 (9)"),
            forum("thr", 3, "Posts: 4"),
            item(4, &["pdf"]),
        ]);
        let new = Page::from_items(vec![
            // Someone posted.
            forum("frm", 1, "Posts (Unread): 3 (15)"),
            // You read three posts: nothing new.
            forum("frm", 2, "Posts (Unread): 0 (9)"),
            forum("thr", 3, "Posts: 4"),
            // A new thread is a new post; a new file and a new forum are new items.
            forum("thr", 5, "Posts: 1"),
            item(4, &["pdf"]),
            item(6, &["docx"]),
            forum("frm", 7, "Posts (Unread): 0 (0)"),
        ]);
        let found = diff(&old, day, &new, day);
        assert_eq!(found.posts, [1, 5]);
        assert_eq!(found.items, [6, 7]);
        assert_eq!(found.all(), [6, 7, 1, 5]);

        // Counted apart, on every level.
        let dir = std::env::temp_dir().join(format!("ilias-tui-posts-{}", std::process::id()));
        let mut seen = Seen::in_file(dir.join("host.seen.json"));
        seen.record(8, &[], 9, &found);
        let both = Counts { items: 2, posts: 2 };
        assert_eq!(seen.counts(), HashMap::from([(8, both), (9, both)]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn countdowns_are_no_change() {
        let day = date(2026, 11, 9);
        let old = Page::from_items(vec![item(1, &["Deadline: 5 Hours, 8 Minutes, 11 Seconds"])]);
        let new = Page::from_items(vec![item(1, &["Deadline: 4 Hours, 59 Minutes, 2 Seconds"])]);
        assert!(diff(&old, day, &new, day).is_empty());
        let german = Page::from_items(vec![item(1, &["Abgabetermin: 1 Tag, 3 Stunden"])]);
        assert!(diff(&old, day, &german, day).is_empty());
        assert!(!is_countdown("Version: 2"));
        assert!(!is_countdown("1.2 MB"));
    }

    #[test]
    fn relative_dates_are_the_same_date_a_day_later() {
        let old = Page::from_items(vec![item(1, &["pdf", "Today, 10:00"])]);
        let same = Page::from_items(vec![item(1, &["pdf", "Yesterday, 10:00"])]);
        let newer = Page::from_items(vec![item(1, &["pdf", "Today, 09:00"])]);
        let (monday, tuesday) = (date(2026, 11, 9), date(2026, 11, 10));
        assert!(diff(&old, monday, &same, tuesday).is_empty());
        assert_eq!(diff(&old, monday, &newer, tuesday).items, [1]);
    }

    #[test]
    fn every_level_counts_what_is_new_below_it() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-seen-{}", std::process::id()));
        let file = dir.join("host.seen.json");
        let _ = fs::remove_dir_all(&dir);
        let mut seen = Seen::in_file(file.clone());
        // A sync of course 1 found new files on its page, in its folder 2 and in folder 3
        // inside 2; course 5 got one too.
        seen.record(1, &[], 1, &items(&[10]));
        seen.record(1, &[], 2, &items(&[20, 21]));
        seen.record(1, &[2], 3, &items(&[30]));
        seen.record(5, &[], 5, &items(&[50]));
        seen.record(5, &[], 5, &items(&[]));
        let all = HashMap::from([(1, 4), (2, 3), (3, 1), (5, 1)]);
        assert_eq!(counted(&seen), all);

        // Kept per host, across runs.
        let mut again = Seen::in_file(file.clone());
        assert_eq!(counted(&again), all);

        // Opening the course takes only what is on its own page; its folders keep counting.
        assert_eq!(again.open(1), [10]);
        assert!(again.open(1).is_empty(), "seen now");
        assert_eq!(
            counted(&again),
            HashMap::from([(1, 3), (2, 3), (3, 1), (5, 1)])
        );
        // Down the trail, folder by folder.
        assert_eq!(again.open(2), [20, 21]);
        assert_eq!(
            counted(&again),
            HashMap::from([(1, 1), (2, 1), (3, 1), (5, 1)])
        );
        assert_eq!(again.open(3), [30]);
        assert_eq!(
            counted(&Seen::in_file(file.clone())),
            HashMap::from([(5, 1)])
        );

        // Files written before folders were counted (no `path`) still load.
        fs::write(&file, r#"{"7":{"course":7,"items":[70]}}"#).unwrap();
        assert_eq!(counted(&Seen::in_file(file)), HashMap::from([(7, 1)]));
        let _ = fs::remove_dir_all(&dir);
    }
}
