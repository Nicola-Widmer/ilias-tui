//! Turns ILIAS 10 HTML pages into a [`Page`]: tabs plus the content column as a sequence of
//! headings, text paragraphs and titled item sections, in the order ILIAS shows them.

use scraper::Html;
use url::Url;

mod dashboard;
mod exercise;
mod forum;
mod html;
mod model;
mod show_more;
mod walker;

pub use dashboard::{Dashboard, course_title, parse_dashboard, parse_date};
pub use exercise::{delete_form, lists_delivered, parse_upload_form};
pub use model::{Block, Item, Page, Post, Tab};
pub use show_more::{ShowMore, has_show_more, show_more};

use exercise::{add_submission, attachment_id};
use forum::post;
use html::*;
use walker::*;

/// Extracts `(kind, ref_id)` from an ILIAS link.
///
/// Understands ILIAS 10 permalinks (`goto.php/crs/123`), the legacy form
/// (`goto.php?target=crs_123`) and direct GUI links
/// (`ilias.php?cmdClass=ilObjFileGUI&cmd=sendfile&ref_id=123`, used for files in folders).
/// Forum threads (`cmd=viewThread&thr_pk=42`) become `("thr", 42)`.
pub fn parse_target(url: &Url) -> Option<(String, u64)> {
    let segments: Vec<&str> = url.path_segments()?.collect();
    if let Some(pos) = segments.iter().position(|s| *s == "goto.php")
        && let (Some(kind), Some(id)) = (segments.get(pos + 1), segments.get(pos + 2))
    {
        return Some((kind.to_string(), id.parse().ok()?));
    }
    let query: Vec<_> = url.query_pairs().collect();
    let param = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
    };
    if param("cmd") == Some("viewThread")
        && let Some(thread) = param("thr_pk")
    {
        return Some(("thr".into(), thread.parse().ok()?));
    }
    if param("cmdClass").is_some_and(|c| c.eq_ignore_ascii_case("ilAssignmentPresentationGUI"))
        && let Some(assignment) = param("ass_id")
    {
        return Some(("ass".into(), assignment.parse().ok()?));
    }
    if let Some(target) = param("target") {
        let mut parts = target.split('_');
        let kind = parts.next()?.to_string();
        let id = parts.next()?.parse().ok()?;
        return Some((kind, id));
    }
    let ref_id = param("ref_id")?.parse().ok()?;
    Some((kind_from_gui_class(param("cmdClass")?)?, ref_id))
}

/// Maps `ilObjFileGUI` → `file` etc.; unknown classes keep their middle part.
fn kind_from_gui_class(class: &str) -> Option<String> {
    let name = class
        .to_ascii_lowercase()
        .strip_prefix("ilobj")?
        .strip_suffix("gui")?
        .to_string();
    let kind = match name.as_str() {
        "file" => "file",
        "folder" => "fold",
        "course" => "crs",
        "group" => "grp",
        "category" => "cat",
        "session" => "sess",
        "exercise" => "exc",
        "forum" => "frm",
        "test" => "tst",
        "survey" => "svy",
        "linkresource" => "webr",
        "wiki" => "wiki",
        "blog" => "blog",
        "mediacast" => "mcst",
        _ => return Some(name),
    };
    Some(kind.into())
}

/// The "log out" link of a page; it carries a one-time token (`rtoken`).
pub fn logout_link(html: &str, page_url: &Url) -> Option<Url> {
    let doc = Html::parse_document(html);
    let href = doc
        .select(&sel(r#"a[href*="cmd=doLogout"]"#))
        .find_map(|a| a.attr("href"))?;
    page_url.join(href).ok()
}

/// Parses a whole ILIAS page.
pub fn parse_page(html: &str, page_url: &Url) -> Page {
    let link_base = link_base(page_url);
    let page_url = &link_base;
    let doc = Html::parse_document(html);
    let mut tabs: Vec<Tab> = doc
        .select(&sel("#ilTab > li"))
        // Real tabs have `tab_…` ids; the rest are the back link and the "…" overflow menu.
        .filter(|li| li.attr("id").is_some_and(|id| id.starts_with("tab_")))
        .filter_map(|li| {
            let a = li.select(&sel("a")).next()?;
            let href = a.attr("href").filter(|h| !h.starts_with('#'))?;
            Some(Tab {
                title: visible_text(a),
                url: page_url.join(href).ok()?.into(),
                active: li.attr("class").is_some_and(|c| c.contains("active")),
            })
        })
        .collect();
    // An exercise lists one kind of assignment at a time (ongoing, past, …): these switches
    // work like tabs, and take the place of the tab they belong to.
    let modes: Vec<Tab> = doc
        .select(&sel("#il_center_col .il-viewcontrol-mode button"))
        .filter_map(|button| {
            let link = button.attr("data-action")?;
            link.to_ascii_lowercase()
                .contains("ilexercisehandlergui")
                .then_some(())?;
            Some(Tab {
                title: visible_text(button),
                url: page_url.join(link).ok()?.into(),
                active: button.attr("aria-pressed") == Some("true"),
            })
        })
        .collect();
    if !modes.is_empty() {
        tabs.retain(|t| !t.active);
        tabs.splice(0..0, modes);
    }
    let path = breadcrumb_path(&doc, page_url);

    // A forum thread: its posts are all that matters.
    let posts: Vec<Block> = doc
        .select(&sel(".ilFrmPostRow"))
        .map(|row| Block::Post(post(row, page_url)))
        .collect();
    if !posts.is_empty() {
        return Page {
            tabs,
            blocks: posts,
            ..Page::default()
        };
    }

    let root = doc
        .select(&sel("#il_center_col"))
        .next()
        .or_else(|| doc.select(&sel("main")).next())
        .unwrap_or_else(|| doc.root_element());
    let mut walker = Walker::new(page_url);
    walker.walk(root);
    walker.flush();
    for block in &mut walker.blocks {
        if let Block::Items { items, .. } = block {
            for item in items.iter_mut().filter(|i| i.kind == "file") {
                item.path.clone_from(&path);
            }
        }
    }
    let mut blocks = walker.blocks;
    add_submission(&tabs, &mut blocks);
    Page {
        tabs,
        blocks,
        ..Page::default()
    }
}

/// The page's place from the course (or group) down, e.g. `["DEMO.101", "Unterlagen",
/// "Woche 3"]`; empty outside courses and groups.
fn breadcrumb_path(doc: &Html, page_url: &Url) -> Vec<String> {
    let mut crumbs: Vec<(String, String)> = doc
        .select(&sel(".breadcrumb a"))
        .filter_map(|a| {
            let url = page_url.join(a.attr("href")?).ok()?;
            Some((parse_target(&url)?.0, text_of(a)))
        })
        .collect();
    // ILIAS 10 writes the breadcrumb from the current object up to the root (and shows it
    // reversed); turn it around unless it already starts at the root.
    if crumbs.first().is_some_and(|(kind, _)| kind != "root") {
        crumbs.reverse();
    }
    let start = ["crs", "grp"]
        .iter()
        .find_map(|kind| crumbs.iter().position(|(k, _)| k == kind));
    match start {
        Some(start) => crumbs.drain(start..).map(|(_, title)| title).collect(),
        None => Vec::new(),
    }
}

/// Whether the page's text mentions `text`, as a person would read it.
pub fn shows(html: &str, text: &str) -> bool {
    Html::parse_document(html)
        .root_element()
        .text()
        .any(|t| t.contains(text))
}

#[cfg(test)]
mod tests;
