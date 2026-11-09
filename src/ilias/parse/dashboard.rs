//! The dashboard: favourites and the To-Do block; and the course in a breadcrumb.

use scraper::Html;
use url::Url;

use super::*;

/// What ilias-tui uses from the dashboard.
#[derive(Debug, Default)]
pub struct Dashboard {
    /// The "Favoriten" block, as an items block titled like on ILIAS.
    pub favorites: Option<Block>,
    pub todo: Vec<Item>,
}

pub fn parse_dashboard(html: &str, page_url: &Url) -> Dashboard {
    let doc = Html::parse_document(html);
    Dashboard {
        favorites: parse_favorites(&doc, page_url),
        todo: parse_todo(&doc, page_url),
    }
}

/// The dashboard's favourites, shown as tiles (cards) or as a list.
pub(super) fn parse_favorites(doc: &Html, page_url: &Url) -> Option<Block> {
    let block = doc.select(&sel(r#"[id^="block_dash_fav"]"#)).next()?;
    let title = block.select(&sel("h2")).next().map(text_of)?;
    let mut walker = Walker::new(page_url);
    let items: Vec<Item> = block
        .select(&sel(".il-card, .il-std-item"))
        .filter_map(|container| {
            let mut item = walker.item(container)?;
            item.favorite = true;
            Some(item)
        })
        .collect();
    (!items.is_empty()).then_some(Block::Items { title, items })
}

/// The dashboard's "To-Do" block: assignments with an upcoming deadline, as exercise items
/// whose description is the deadline as ILIAS shows it (e.g. `5. Nov 2026, 00:00`).
pub(super) fn parse_todo(doc: &Html, page_url: &Url) -> Vec<Item> {
    let Some(block) = doc.select(&sel(r#"[id^="block_pdtasks"]"#)).next() else {
        return Vec::new();
    };
    let mut walker = Walker::new(page_url);
    block
        .select(&sel(".il-std-item"))
        .filter_map(|container| {
            // Several assignments can belong to the same exercise.
            walker.seen.clear();
            let mut item = walker.item(container)?;
            // `Abgabe zur Übungseinheit "Sheet 3"` → `Sheet 3`
            if let Some((_, rest)) = item.title.split_once('"')
                && let Some((name, _)) = rest.rsplit_once('"')
            {
                item.title = name.to_string();
            }
            Some(item)
        })
        .collect()
}

/// The course (or else group) in a page's breadcrumb.
pub fn course_title(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    ["crs", "grp"].iter().find_map(|kind| {
        doc.select(&sel(&format!(r#".breadcrumb a[href*="goto.php/{kind}/"]"#)))
            .next()
            .map(text_of)
    })
}

/// Parses an ILIAS date like `5. Nov 2026, 23:55` (German or English month names) or
/// `Heute, 23:55` / `Morgen, 23:55`.
pub fn parse_date(text: &str, today: jiff::civil::Date) -> Option<jiff::civil::DateTime> {
    let (day, time) = text.split_once(", ")?;
    let (hour, minute) = time.trim().split_once(':')?;
    let time = jiff::civil::time(hour.parse().ok()?, minute.get(..2)?.parse().ok()?, 0, 0);
    let date = match day.trim() {
        "Heute" | "Today" => today,
        "Morgen" | "Tomorrow" => today.tomorrow().ok()?,
        "Gestern" | "Yesterday" => today.yesterday().ok()?,
        day => {
            let mut parts = day.split_whitespace();
            let d: i8 = parts.next()?.trim_end_matches('.').parse().ok()?;
            let month = parts.next()?.trim_end_matches('.').to_lowercase();
            let year: i16 = parts.next()?.parse().ok()?;
            const MONTHS: [&[&str]; 12] = [
                &["jan"],
                &["feb"],
                &["mär", "mrz", "mar"],
                &["apr"],
                &["mai", "may"],
                &["jun"],
                &["jul"],
                &["aug"],
                &["sep"],
                &["okt", "oct"],
                &["nov"],
                &["dez", "dec"],
            ];
            let m = MONTHS
                .iter()
                .position(|names| names.iter().any(|n| month.starts_with(n)))?;
            jiff::civil::Date::new(year, m as i8 + 1, d).ok()?
        }
    };
    Some(date.to_datetime(time))
}
