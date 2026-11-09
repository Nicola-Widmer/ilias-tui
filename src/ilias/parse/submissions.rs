//! An assignment's submissions page ("Submissions and Grades", `ilExerciseManagementGUI`
//! `cmd=members`), which only people who grade see: one entry per participant, with their
//! handed-in files to download, and the exercise's assignments as tabs.

use scraper::{ElementRef, Html};
use url::Url;

use super::*;

/// Whether the page is a submissions page: its table lists participants.
pub(super) fn is_submissions(doc: &Html) -> bool {
    doc.select(&sel(r#"input[name="listed_part_ids[]"]"#))
        .next()
        .is_some()
}

/// The page as tabs (the other tabs, then one per assignment) and the participants.
pub(super) fn submissions_page(doc: &Html, page_url: &Url, mut tabs: Vec<Tab>) -> Page {
    let base = link_base(page_url);
    // The assignments: ILIAS' own list on the page. Each is this page with its `ass_id`.
    let assignments: Vec<Tab> = doc
        .select(&sel(r#"select[name="ass_id"] option[value]"#))
        .filter_map(|option| {
            let id = option.attr("value").filter(|v| v.parse::<u64>().is_ok())?;
            let mut url = page_url.clone();
            let pairs: Vec<(String, String)> = page_url
                .query_pairs()
                .filter(|(k, _)| k != "ass_id")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(pairs)
                .append_pair("ass_id", id);
            Some(Tab {
                title: text_of(option),
                url: url.into(),
                active: option.attr("selected").is_some(),
            })
        })
        .collect();
    let assignment = assignments
        .iter()
        .find(|t| t.active)
        .map(|t| t.title.clone())
        .unwrap_or_default();
    if !assignments.is_empty() {
        tabs.retain(|t| !t.active);
        tabs.splice(0..0, assignments);
    }
    let mut folder = breadcrumb_path(doc, &base);
    if !assignment.is_empty() {
        folder.push(assignment.clone());
    }
    let rows = sel("tr");
    let items = doc
        .select(&rows)
        .filter(|row| {
            row.select(&sel(r#"input[name="listed_part_ids[]"]"#))
                .next()
                .is_some()
        })
        .filter_map(|row| participant(row, &base, &folder))
        .collect();
    Page {
        tabs,
        blocks: vec![Block::Items {
            title: match assignment.is_empty() {
                true => "Submissions".into(),
                false => format!("Submissions: {assignment}"),
            },
            items,
        }],
        ..Page::default()
    }
}

/// One participant's row: name, login, when they handed in, and their download.
fn participant(row: ElementRef, base: &Url, folder: &[String]) -> Option<Item> {
    let cells: Vec<String> = row.select(&sel("td")).map(text_of).collect();
    let (name, login) = (
        cells.get(1)?.clone(),
        cells.get(2).cloned().unwrap_or_default(),
    );
    if name.is_empty() {
        return None;
    }
    let download = row
        .select(&sel(r#"button[data-action*="cmd=downloadReturned"]"#))
        .find_map(|b| {
            let url = base.join(b.attr("data-action")?).ok()?;
            (url.origin() == base.origin()).then(|| (url, text_of(b)))
        });
    // "Download Submissions (2)"
    let count = download.as_ref().and_then(|(_, label)| {
        let (_, n) = label.rsplit_once('(')?;
        n.trim_end_matches(')').parse::<usize>().ok()
    });
    let handed_in = cells.get(3).filter(|c| !c.is_empty());
    let mut properties = match (count, handed_in) {
        (Some(1), _) => vec!["1 file".to_string()],
        (Some(n), _) => vec![format!("{n} files")],
        (None, _) if download.is_some() => vec!["handed in".to_string()],
        (None, _) => vec!["nothing handed in".to_string()],
    };
    if let Some(date) = handed_in.filter(|_| download.is_some()) {
        properties.push(format!("handed in {}", absolute(date)));
    }
    let status = row
        .select(&sel(r#"select[name^="status["] option[selected]"#))
        .next()
        .map(text_of);
    properties.extend(status);
    let mut path = folder.to_vec();
    path.push(match login.is_empty() {
        true => name.clone(),
        false => format!("{name} ({login})"),
    });
    let url = download.map(|(url, _)| url.to_string()).unwrap_or_default();
    Some(Item {
        kind: Kind::Participant,
        id: Id::local(&format!("{url}|{}", path.join("/"))),
        title: name,
        url,
        description: login,
        properties,
        path,
        ..Item::default()
    })
}

/// `Today, 19:38:51` as the date it is now (`9. Nov 2026, 19:38`), so it reads the same
/// tomorrow; ILIAS' own absolute dates stay as they are.
fn absolute(date: &str) -> String {
    let today = jiff::Zoned::now().date();
    match parse_date(date, today) {
        Some(at) if !date.starts_with(|c: char| c.is_ascii_digit()) => {
            at.strftime("%-d. %b %Y, %H:%M").to_string()
        }
        _ => date.to_string(),
    }
}
