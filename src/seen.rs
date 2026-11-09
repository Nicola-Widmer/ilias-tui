//! What's new since your last visit: the items of a course or folder that appeared or changed
//! since you last looked at it.
//!
//! [`diff`] compares a freshly fetched page with the cached copy. What a sync finds this way
//! is kept as *unseen* until you open the course (or folder) it belongs to; [`Seen`] stores
//! that next to the session as `<ilias host>.seen.json`:
//! `{ "<container ref_id>": { "course": <ref_id>, "items": [<ref_id>, …] } }`.
//! What a page you are looking at shows new is seen right away.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::ilias::parse::parse_date;
use crate::ilias::{Item, Page};
use crate::session;

/// The items of `new` that `old` doesn't have (by ref id), or has with other properties.
/// `old_day` and `new_day` are the days each copy was fetched on, so that ILIAS' relative
/// dates ("Today, 10:00" yesterday is "Yesterday, 10:00" today) compare as the same date.
pub fn diff(old: &Page, old_day: Date, new: &Page, new_day: Date) -> Vec<u64> {
    let before: HashMap<u64, Vec<String>> = old
        .items()
        .map(|i| (i.key(), version(i, old_day)))
        .collect();
    new.items()
        .filter(|i| before.get(&i.key()) != Some(&version(i, new_day)))
        .map(|i| i.key())
        .collect()
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
    items: BTreeSet<u64>,
}

#[derive(Debug, Default)]
pub struct Seen {
    file: Option<PathBuf>,
    /// By container ref id.
    unseen: BTreeMap<u64, Unseen>,
}

impl Seen {
    pub fn load(base: &Url) -> Self {
        let file = session::path(base)
            .ok()
            .map(|p| p.with_extension("seen.json"));
        Self::from_file(file)
    }

    fn from_file(file: Option<PathBuf>) -> Self {
        let unseen = file
            .as_ref()
            .and_then(|f| fs::read_to_string(f).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { file, unseen }
    }

    #[cfg(test)]
    pub fn in_file(file: PathBuf) -> Self {
        Self::from_file(Some(file))
    }

    /// Remembers `items` of `container` (in `course`) as not seen yet.
    pub fn record(&mut self, course: u64, container: u64, items: &[u64]) {
        if items.is_empty() {
            return;
        }
        let entry = self.unseen.entry(container).or_default();
        entry.course = course;
        entry.items.extend(items);
        self.save();
    }

    /// You opened `place` (a course, or any container): everything unseen in it, and in its
    /// folders if it is a course, is seen now. Returns those items, to mark them on screen.
    pub fn open(&mut self, place: u64) -> Vec<u64> {
        let mut items = Vec::new();
        self.unseen.retain(|container, unseen| {
            let opened = *container == place || unseen.course == place;
            if opened {
                items.extend(&unseen.items);
            }
            !opened
        });
        if !items.is_empty() {
            self.save();
        }
        items
    }

    /// Unseen items per course.
    pub fn counts(&self) -> HashMap<u64, usize> {
        let mut counts = HashMap::new();
        for unseen in self.unseen.values() {
            *counts.entry(unseen.course).or_default() += unseen.items.len();
        }
        counts
    }

    /// Best effort, like the page cache: a failed write only loses a "new" mark.
    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = session::private_dir(dir);
        }
        if let Ok(json) = serde_json::to_string(&self.unseen) {
            let _ = fs::write(file, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ilias::Id;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

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
        assert_eq!(diff(&old, day, &new, day), [2, 4]);
        assert_eq!(diff(&new, day, &new, day), Vec::<u64>::new());
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
        assert_eq!(diff(&old, monday, &newer, tuesday), [1]);
    }

    #[test]
    fn opening_a_course_marks_it_and_its_folders_seen() {
        let file = std::env::temp_dir().join(format!("ilias-tui-seen-{}.json", std::process::id()));
        let _ = fs::remove_file(&file);
        let mut seen = Seen::in_file(file.clone());
        // A sync of course 1 found new files in it (container 1) and in its folder 2;
        // course 5 got one too.
        seen.record(1, 1, &[10]);
        seen.record(1, 2, &[20, 21]);
        seen.record(5, 5, &[50]);
        seen.record(5, 5, &[]);
        assert_eq!(seen.counts(), HashMap::from([(1, 3), (5, 1)]));

        // Kept per host, across runs.
        let mut again = Seen::in_file(file.clone());
        assert_eq!(again.counts(), seen.counts());

        // Opening the folder takes only its own items.
        assert_eq!(again.open(2), [20, 21]);
        assert_eq!(again.counts(), HashMap::from([(1, 1), (5, 1)]));
        // Opening the course takes the rest of it.
        assert_eq!(again.open(1), [10]);
        assert!(again.open(1).is_empty(), "seen now");
        assert_eq!(
            Seen::in_file(file.clone()).counts(),
            HashMap::from([(5, 1)])
        );

        let _ = fs::remove_file(&file);
    }
}
