//! Turns ILIAS 10 HTML pages into a [`Page`]: tabs plus the content column as a sequence of
//! headings, text paragraphs and titled item sections, in the order ILIAS shows them.

use std::collections::HashSet;

use scraper::node::Node;
use scraper::{ElementRef, Html, Selector};
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

fn sel(css: &str) -> Selector {
    Selector::parse(css).unwrap()
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_of(el: ElementRef) -> String {
    clean(&el.text().collect::<Vec<_>>().join(" "))
}

/// Text without screen-reader-only parts such as "(Ausgewählte)".
fn visible_text(el: ElementRef) -> String {
    fn collect(el: ElementRef, out: &mut String) {
        for child in el.children() {
            match child.value() {
                Node::Text(text) => out.push_str(text),
                Node::Element(e) if !e.classes().any(|c| c == "ilAccHidden" || c == "sr-only") => {
                    if let Some(child) = ElementRef::wrap(child) {
                        collect(child, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = String::new();
    collect(el, &mut out);
    clean(&out)
}

/// Object type from a list item's icon, e.g. `…/icon_webr.svg` → `webr`.
fn kind_from_icon(container: ElementRef) -> Option<String> {
    let src = container
        .select(&sel("img"))
        .find_map(|img| img.attr("src"))?;
    let name = src.rsplit('/').next()?.strip_prefix("icon_")?;
    let kind = name.split(['.', '_']).next()?;
    (!kind.is_empty() && kind != "custom").then(|| kind.to_string())
}

/// What relative links on a page are relative to: the ILIAS root, also on `goto.php/crs/123`
/// permalinks.
fn link_base(page_url: &Url) -> Url {
    let mut base = page_url.clone();
    if let Some(i) = page_url.path().find("goto.php/") {
        base.set_path(&page_url.path()[..i]);
    }
    base
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

/// An assignment's overview gets its Submission tab as an item too, so the list keys reach
/// it: under the heading of the section of the same name if there is one, else at the end.
fn add_submission(tabs: &[Tab], blocks: &mut Vec<Block>) {
    let Some(tab) = tabs.iter().find(|t| {
        !t.active
            && t.url
                .to_ascii_lowercase()
                .contains("cmdclass=ilexsubmissionfilegui")
    }) else {
        return;
    };
    let item = Item {
        kind: "sub".into(),
        ref_id: attachment_id(&tab.url),
        title: "Handed-in files".into(),
        url: tab.url.clone(),
        description: "H hands one in, X deletes one".into(),
        ..Item::default()
    };
    let block = Block::Items {
        title: tab.title.clone(),
        items: vec![item],
    };
    match blocks
        .iter()
        .position(|b| matches!(b, Block::Heading(h) if *h == tab.title))
    {
        Some(heading) => blocks[heading] = block,
        None => blocks.push(block),
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

fn post(row: ElementRef, page_url: &Url) -> Post {
    // "Ada Lovelace | alovelace (Moderator) | 19. Feb 2024, 07:12"
    let header = row
        .select(&sel(".ilFrmPostHeader .small"))
        .next()
        .map(text_of)
        .unwrap_or_default();
    let mut parts: Vec<&str> = header.split(" | ").collect();
    let date = match parts.len() {
        0 | 1 => "",
        _ => parts.pop().unwrap_or_default(),
    };
    let title = row.select(&sel(".ilFrmPostTitle")).next();
    let body = row
        .select(&sel(".ilFrmPostContent"))
        .next()
        .map(|content| {
            let mut walker = Walker::new(page_url);
            walker.walk(content);
            walker.flush();
            walker
                .blocks
                .into_iter()
                .filter_map(|b| match b {
                    Block::Heading(text) | Block::Text(text) => Some(text),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Post {
        author: parts.join(" · "),
        date: date.to_string(),
        title: title.map(text_of).unwrap_or_default(),
        body,
        depth: row
            .value()
            .classes()
            .find_map(|c| c.strip_prefix("ilFrmPost-level-")?.parse::<usize>().ok())
            .map_or(0, |level| level.saturating_sub(1)),
        // Unread posts have their title linked to "mark as read" (never followed here).
        unread: title.is_some_and(|t| {
            t.select(&sel(r#"a[href*="cmd=markPostRead"]"#))
                .next()
                .is_some()
        }),
    }
}

/// What it takes to hand in a file, from an assignment's upload form.
#[derive(Debug, PartialEq, Eq)]
pub struct UploadForm {
    /// Takes the file; this stores it in the submission.
    pub upload: Url,
    /// Takes the name of the uploaded file, under [`Self::field`], and finishes the form.
    pub finish: Url,
    pub field: String,
}

/// `None` if the page has no upload form, as when the deadline has passed.
pub fn parse_upload_form(html: &str, page_url: &Url) -> Option<UploadForm> {
    let doc = Html::parse_document(html);
    let form = doc
        .select(&sel(r#"form[action*="cmd=addUpload"]"#))
        .next()?;
    // The address for the file only exists in a script: `File.init('id', 'upload url', …`.
    let (_, script) = html.split_once("il.UI.Input.File.init(")?;
    let upload = script.split('\'').nth(3)?.replace("&amp;", "&");
    let field = form.select(&sel(r#"input[type="hidden"]"#)).next()?;
    Some(UploadForm {
        upload: page_url.join(&upload).ok()?,
        finish: page_url.join(form.attr("action")?).ok()?,
        field: field.attr("name")?.to_string(),
    })
}

/// Where a form on the page that holds handed-in file `delivered` (as a checkbox, or hidden
/// once you are asked to confirm) and the button `cmd[command]` posts to. `None` if there is
/// no such form, as when the deadline has passed and ILIAS offers no Delete button.
pub fn delete_form(html: &str, page_url: &Url, delivered: &str, command: &str) -> Option<Url> {
    let doc = Html::parse_document(html);
    let button = sel(&format!(r#"input[type="submit"][name="cmd[{command}]"]"#));
    let file = sel(r#"input[name="delivered[]"]"#);
    doc.select(&sel("form[action]"))
        .find(|form| {
            form.select(&button).next().is_some()
                && form
                    .select(&file)
                    .any(|input| input.attr("value") == Some(delivered))
        })
        .and_then(|form| page_url.join(form.attr("action")?).ok())
}

/// Whether the page still lists handed-in file `delivered`.
pub fn lists_delivered(html: &str, delivered: &str) -> bool {
    Html::parse_document(html)
        .select(&sel(r#"input[name="delivered[]"]"#))
        .any(|input| input.attr("value") == Some(delivered))
}

/// Whether the page's text mentions `text`, as a person would read it.
pub fn shows(html: &str, text: &str) -> bool {
    Html::parse_document(html)
        .root_element()
        .text()
        .any(|t| t.contains(text))
}

/// A list that ILIAS cut short behind a "Show more" button (`ilContainerShowMore`, shown
/// when a container has a block limit).
#[derive(Debug, PartialEq, Eq)]
pub struct ShowMore {
    /// Where the button asks for the rest (`cmd=renderBlockAsynch&type=<block>`). ILIAS
    /// expects a POST of the items already shown, as `ids[]`, and answers with the next ones
    /// (and another button if there are still more).
    pub url: Url,
    /// The shown items' list ids (`lg_div_<ref_id>_pref_<parent>`), in order.
    pub shown: Vec<String>,
}

impl ShowMore {
    /// The reference id of the last item shown before the button.
    pub fn last_ref_id(&self) -> Option<u64> {
        // ILIAS reads the ref id the same way: the third part of the list id.
        self.shown.last()?.split('_').nth(2)?.parse().ok()
    }
}

/// The page's "Show more" buttons. The button's address is only in its script:
/// `il.Container.initShowMore('<button id>', '<block>', '<url>')`.
pub fn show_more(html: &str, page_url: &Url) -> Vec<ShowMore> {
    let doc = Html::parse_document(html);
    let mut found = Vec::new();
    for script in html.split("initShowMore(").skip(1) {
        // Inside JSON the quotes are escaped (`\'`).
        let args: Vec<&str> = script
            .split('\'')
            .map(|a| a.trim_end_matches('\\'))
            .collect();
        let (Some(id), Some(link)) = (args.get(1), args.get(5)) else {
            continue;
        };
        let Ok(url) = link_base(page_url).join(&link.replace("&amp;", "&")) else {
            continue;
        };
        if found.iter().any(|m: &ShowMore| m.url == url) {
            continue;
        }
        let Ok(button) = Selector::parse(&format!(r#"[id="{id}"]"#)) else {
            continue;
        };
        let Some(list) = doc.select(&button).next().and_then(|b| {
            b.ancestors().filter_map(ElementRef::wrap).find(|a| {
                a.value()
                    .classes()
                    .any(|c| c == "ilContainerItemsContainer")
            })
        }) else {
            continue;
        };
        let shown = list
            .select(&sel("[data-list-item-id]"))
            .filter_map(|e| e.attr("data-list-item-id"))
            .map(String::from)
            .collect();
        found.push(ShowMore { url, shown });
    }
    found
}

/// Whether the page has a "Show more" button at all (also one [`show_more`] can't read).
pub fn has_show_more(html: &str) -> bool {
    html.contains("ilContainerShowMore")
}

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
fn parse_favorites(doc: &Html, page_url: &Url) -> Option<Block> {
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
fn parse_todo(doc: &Html, page_url: &Url) -> Vec<Item> {
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

/// Walks the content column in document order and splits it into [`Block`]s.
struct Walker<'a> {
    page_url: &'a Url,
    blocks: Vec<Block>,
    /// Text of the paragraph currently being collected.
    line: String,
    /// Links in that paragraph; they follow it as `link` items.
    links: Vec<Item>,
    seen: HashSet<u64>,
    /// Ids of the text links so far, to list each one once.
    seen_links: HashSet<u64>,
}

const SKIPPED_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "dialog", "button", "select", "img", "svg", "iframe",
];
const SKIPPED_CLASSES: &[&str] = &[
    "dropdown",
    "ilAccHidden",
    "ilAccHeadingHidden",
    "sr-only",
    // The page title; it is already in the breadcrumb.
    "il-page-content-header",
    "modal",
    "il-copg-mob-fullscreen-modal",
    "ilContainerShowMore",
    "il-viewcontrol-section",
    "il-viewcontrol-sortation",
];
const BLOCK_TAGS: &[&str] = &[
    "div",
    "p",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "section",
    "article",
    "br",
    "dl",
    "dt",
    "dd",
    "h5",
    "h6",
    "form",
    "fieldset",
    "hr",
    "blockquote",
    "pre",
    "header",
    "footer",
    "nav",
];
const ITEM_CONTAINERS: &str = ".ilContainerListItemOuter, .il-std-item";

impl<'a> Walker<'a> {
    fn new(page_url: &'a Url) -> Self {
        Self {
            page_url,
            blocks: Vec::new(),
            line: String::new(),
            links: Vec::new(),
            seen: HashSet::new(),
            seen_links: HashSet::new(),
        }
    }

    fn walk(&mut self, el: ElementRef) {
        for child in el.children() {
            match child.value() {
                Node::Text(text) => self.line.push_str(text),
                Node::Element(_) => {
                    if let Some(child) = ElementRef::wrap(child) {
                        self.element(child);
                    }
                }
                _ => {}
            }
        }
    }

    fn element(&mut self, el: ElementRef) {
        let e = el.value();
        let name = e.name();
        let has_class = |c: &str| e.classes().any(|x| x == c);
        let hidden = e
            .attr("style")
            .is_some_and(|s| s.replace(' ', "").contains("display:none"));
        // The tabs are read on their own (`parse_page`).
        let tabs = e.id() == Some("ilTab");
        if hidden
            || tabs
            || SKIPPED_TAGS.contains(&name)
            || SKIPPED_CLASSES.iter().any(|c| has_class(c))
        {
            return;
        }
        if has_class("ilContainerBlock") || has_class("il-item-group") {
            self.flush();
            let title = el
                .select(&sel(
                    ".ilContainerBlockHeader h2, .ilContainerBlockHeader h3",
                ))
                .next()
                .or_else(|| {
                    el.children()
                        .filter_map(ElementRef::wrap)
                        .find(|c| matches!(c.value().name(), "h2" | "h3"))
                })
                .map(text_of)
                .unwrap_or_default();
            let items: Vec<Item> = el
                .select(&sel(ITEM_CONTAINERS))
                .filter_map(|c| self.item(c))
                .collect();
            if !items.is_empty() {
                self.blocks.push(Block::Items { title, items });
            }
            return;
        }
        if is_item_container(el) {
            self.flush();
            if let Some(item) = self.item(el) {
                self.push_item(item);
            }
            return;
        }
        // Submission pages: a table row per handed-in file, with a checkbox for deleting it.
        if name == "tr"
            && let Some(item) = self.submitted_file(el)
        {
            self.flush();
            self.push_item(item);
            return;
        }
        // Assignment pages: `<div class="row"><div class="control-label">Last Submission</div>
        // <div>Today, 17:58</div></div>`, or a file with a download link as the value. Rows
        // without a label hold only buttons and links to other tabs.
        if has_class("row")
            && let Some(label) = el
                .children()
                .filter_map(ElementRef::wrap)
                .find(|c| c.value().classes().any(|x| x == "control-label"))
        {
            self.flush();
            let label_text = visible_text(label);
            if label_text.is_empty() {
                return;
            }
            let download = el.select(&sel(r#"a[href*="cmd=downloadFile"]"#)).next();
            if let Some(url) = download
                .and_then(|a| a.attr("href"))
                .and_then(|href| self.page_url.join(href).ok())
            {
                self.push_item(Item {
                    kind: "file".into(),
                    ref_id: attachment_id(url.as_str()),
                    title: label_text,
                    url: url.into(),
                    ..Item::default()
                });
                return;
            }
            let value: Vec<String> = el
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| c.id() != label.id())
                .map(visible_text)
                .filter(|t| !t.is_empty())
                .collect();
            self.blocks.push(Block::Text(match value.is_empty() {
                true => label_text,
                false => format!("{label_text}: {}", value.join(" ")),
            }));
            return;
        }
        // Info screens and forms: `<div class="form-group"><div class="control-label">Sprache</div>
        // <div>Deutsch</div></div>` reads best as one line.
        if has_class("form-group")
            && let Some(label) = el.select(&sel(".control-label")).next()
        {
            self.flush();
            let value: Vec<String> = el
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| c.id() != label.id())
                .map(visible_text)
                .filter(|t| !t.is_empty())
                .collect();
            let label = visible_text(label);
            self.blocks.push(Block::Text(match value.is_empty() {
                true => label,
                false => format!("{label}: {}", value.join(" ")),
            }));
            return;
        }
        let heading = matches!(name, "h1" | "h2" | "h3" | "h4")
            || e.classes().any(|c| c.starts_with("ilc_heading"));
        if heading {
            self.flush();
            let text = text_of(el);
            if !text.is_empty() {
                self.blocks.push(Block::Heading(text));
            }
            return;
        }
        if name == "a"
            && let Some(link) = self.text_link(el)
        {
            self.links.push(link);
        }
        if BLOCK_TAGS.contains(&name) {
            self.flush();
            if name == "li" {
                self.line.push_str("• ");
            }
            self.walk(el);
            self.flush();
        } else {
            if matches!(name, "td" | "th") {
                self.line.push_str("  ");
            }
            self.walk(el);
        }
    }

    /// `<tr><td><input name="delivered[]" value="2"></td><td><label>Bericht.pdf</label></td>
    /// <td>6. Nov 2026, 18:05</td><td><a href="…cmd=download…&delivered=2">Download</a></td>`
    fn submitted_file(&self, row: ElementRef) -> Option<Item> {
        row.select(&sel(r#"input[name="delivered[]"]"#)).next()?;
        let link = row.select(&sel(r#"a[href*="delivered="]"#)).next()?;
        let url = self.page_url.join(link.attr("href")?).ok()?;
        let cells: Vec<String> = row
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|c| c.value().name() == "td")
            .map(visible_text)
            .filter(|t| !t.is_empty() && *t != visible_text(link))
            .collect();
        let (title, properties) = cells.split_first()?;
        Some(Item {
            kind: "file".into(),
            ref_id: attachment_id(url.as_str()),
            title: title.clone(),
            url: url.into(),
            properties: properties.to_vec(),
            ..Item::default()
        })
    }

    fn push_item(&mut self, item: Item) {
        match self.blocks.last_mut() {
            Some(Block::Items { items, .. }) => items.push(item),
            _ => self.blocks.push(Block::Items {
                title: String::new(),
                items: vec![item],
            }),
        }
    }

    fn item(&mut self, container: ElementRef) -> Option<Item> {
        let a = container
            .select(&sel(
                "a.il_ContainerItemTitle, .il-item-title a, .card-title a",
            ))
            .next()?;
        let url = self.page_url.join(a.attr("href")?).ok()?;
        // Some links (e.g. weblinks: `baseClass=ilLinkResourceHandlerGUI&ref_id=…`) carry no
        // type; the item's icon still tells it.
        let (kind, ref_id) = parse_target(&url).or_else(|| {
            let ref_id = url
                .query_pairs()
                .find(|(k, _)| k == "ref_id")?
                .1
                .parse()
                .ok()?;
            Some((kind_from_icon(container)?, ref_id))
        })?;
        if !self.seen.insert(ref_id) {
            return None;
        }
        // Assignments show the time left in front of the title instead of a description.
        let description = container
            .select(&sel(if kind == "ass" {
                ".col-sm-3"
            } else {
                ".il_Description, .il-item-description"
            }))
            .next()
            .map(text_of)
            .unwrap_or_default();
        let mut properties: Vec<String> = container
            .select(&sel(".il_ItemProperty"))
            .map(text_of)
            // "Lernfortschritt:" with only an icon as value says nothing in a terminal.
            .filter(|p| !p.is_empty() && !p.ends_with(':'))
            .collect();
        for value in container.select(&sel(".il-item-property-value")) {
            // Rating stars (forum threads) are buttons; their text says nothing.
            if value.select(&sel(".ilRatingOverlay")).next().is_some() {
                continue;
            }
            let name = value
                .prev_siblings()
                .filter_map(ElementRef::wrap)
                .next()
                .map(text_of)
                .unwrap_or_default();
            let value = text_of(value);
            if !value.is_empty() {
                properties.push(if name.is_empty() {
                    value
                } else {
                    format!("{name}: {value}")
                });
            }
        }
        // The action menu offers either "add to" or "remove from" favourites. Only links
        // for this very item count (menus can be nested in containers).
        let favorite_toggle = container
            .select(&sel(
                r#"[href*="cmd=addToDesk"], [data-action*="cmd=addToDesk"], [href*="cmd=removeFromDesk"], [data-action*="cmd=removeFromDesk"]"#,
            ))
            .filter_map(|e| e.attr("href").or(e.attr("data-action")))
            .filter_map(|link| self.page_url.join(link).ok())
            .find(|link| {
                link.query_pairs()
                    .any(|(k, v)| k == "item_ref_id" && v == ref_id.to_string())
            });
        let favorite = favorite_toggle.as_ref().is_some_and(|link| {
            link.query()
                .is_some_and(|q| q.contains("cmd=removeFromDesk"))
        });
        Some(Item {
            kind,
            ref_id,
            title: text_of(a),
            url: url.into(),
            description,
            properties,
            favorite,
            favorite_toggle: favorite_toggle.map(Into::into),
            path: Vec::new(),
        })
    }

    /// A link in page text (e.g. a Zoom link on a course page), as a `link` item. Only web
    /// links, and none that would change something on ILIAS.
    fn text_link(&mut self, a: ElementRef) -> Option<Item> {
        let href = a.attr("href").filter(|h| !h.starts_with('#'))?;
        let url = self.page_url.join(href).ok()?;
        if !matches!(url.scheme(), "http" | "https") || super::write::is_write(&url) {
            return None;
        }
        let ref_id = attachment_id(url.as_str());
        if !self.seen_links.insert(ref_id) {
            return None;
        }
        let text = visible_text(a);
        Some(Item {
            kind: "link".into(),
            ref_id,
            title: if text.is_empty() {
                url.to_string()
            } else {
                text
            },
            description: url.to_string(),
            url: url.into(),
            ..Item::default()
        })
    }

    fn flush(&mut self) {
        let text = clean(&std::mem::take(&mut self.line));
        if !text.is_empty() && text != "•" {
            self.blocks.push(Block::Text(text));
        }
        for link in std::mem::take(&mut self.links) {
            self.push_item(link);
        }
    }
}

/// A stable id for a file attached to an assignment, which has no reference id of its own.
/// The top bit keeps it apart from real reference ids (FNV-1a, like the cache's file names).
fn attachment_id(url: &str) -> u64 {
    let hash = url.bytes().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    hash | 1 << 63
}

fn is_item_container(el: ElementRef) -> bool {
    let e = el.value();
    e.classes()
        .any(|c| c == "ilContainerListItemOuter" || c == "il-std-item")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn finds_show_more_buttons() {
        // As on the local ILIAS 10: the script twice, once escaped inside JSON.
        let html = r#"<div class="ilContainerBlock"><ul class="ilContainerItemsContainer">
            <li><div class="ilContainerListItemOuter" data-list-item-id="lg_div_110_pref_109"></div></li>
            <li><div class="ilContainerListItemOuter" data-list-item-id="lg_div_111_pref_109"></div></li>
            <li class="ilContainerShowMore"><button class="btn btn-default" data-action="" id="il_ui_fw_1">Show More</button></li>
            </ul></div>
            <script>il.Container.initShowMore('il_ui_fw_1', '_other', 'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch');</script>
            <script>var x = {"code": "il.Container.initShowMore(\'il_ui_fw_1\', \'_other\', \'ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch\')"};</script>"#;
        let found = show_more(html, &url("https://ilias.example/ilias/goto.php/fold/109"));
        assert_eq!(
            found,
            [ShowMore {
                url: url(
                    "https://ilias.example/ilias/ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjFolderGUI&cmd=renderBlockAsynch&ref_id=109&type=_other&cmdMode=asynch"
                ),
                shown: vec!["lg_div_110_pref_109".into(), "lg_div_111_pref_109".into()],
            }]
        );
        assert_eq!(found[0].last_ref_id(), Some(111));
        assert!(has_show_more(html));
        assert!(show_more("<p>no button</p>", &url("https://ilias.example/ilias/")).is_empty());
    }

    #[test]
    fn parses_permalinks() {
        let base = "https://elearning.hslu.ch/ilias/";
        assert_eq!(
            parse_target(&url(&format!("{base}goto.php/crs/3035013"))),
            Some(("crs".into(), 3035013))
        );
        assert_eq!(
            parse_target(&url(&format!("{base}goto.php?target=file_42_download"))),
            Some(("file".into(), 42))
        );
        assert_eq!(
            parse_target(&url(&format!(
                "{base}ilias.php?baseClass=ilrepositorygui&cmdNode=yb:o1&cmdClass=ilObjFileGUI&cmd=sendfile&ref_id=123456"
            ))),
            Some(("file".into(), 123456))
        );
        assert_eq!(
            parse_target(&url(&format!(
                "{base}ilias.php?baseClass=ilrepositorygui&cmdClass=ilObjForumGUI&cmd=viewThread&ref_id=123456&thr_pk=42&page=0"
            ))),
            Some(("thr".into(), 42))
        );
        assert_eq!(
            parse_target(&url(&format!("{base}ilias.php?ref_id=1"))),
            None
        );
    }

    #[test]
    fn finds_the_logout_link() {
        let html = r#"<a href="https://ilias.example/ilias/ilias.php?baseClass=ilstartupgui&amp;cmd=doLogout&amp;rtoken=abc&amp;lang=de">Abmelden</a>"#;
        let link = logout_link(html, &url("https://ilias.example/ilias/ilias.php")).unwrap();
        assert!(link.as_str().contains("cmd=doLogout&rtoken=abc"));
        assert_eq!(
            logout_link("<a href='x'>y</a>", &url("https://ilias.example/")),
            None
        );
    }

    #[test]
    fn parses_root_page_without_breadcrumbs() {
        let page_url = url("https://elearning.hslu.ch/ilias/goto.php/root/1");
        let page = parse_page(include_str!("../../tests/fixtures/root.html"), &page_url);
        let items: Vec<_> = page
            .items()
            .map(|i| (i.kind.as_str(), i.ref_id, i.title.as_str()))
            .collect();
        assert_eq!(items, vec![("cat", 5066130, "Öffentlicher Bereich")]);
    }

    #[test]
    fn parses_category_contents() {
        let page_url = url("https://elearning.hslu.ch/ilias/goto.php/cat/5066132");
        let page = parse_page(
            include_str!("../../tests/fixtures/category.html"),
            &page_url,
        );
        let items: Vec<_> = page.items().collect();
        assert!(items.len() > 3, "found {items:?}");
        assert!(items.iter().all(|i| i.kind == "grp" && !i.title.is_empty()));
        assert!(
            items
                .iter()
                .all(|i| i.ref_id != 5066130 && i.ref_id != 5066132)
        );
    }

    #[test]
    fn lists_the_assignments_of_an_exercise_with_their_modes_as_tabs() {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let html = r##"<ul id="ilTab">
              <li id="tab_content" class="active"><a href="ilias.php?cmd=showOverview&amp;ref_id=7">Assignments</a></li>
              <li id="tab_info"><a href="ilias.php?cmd=showSummary&amp;ref_id=7">Info</a></li></ul>
            <div id="il_center_col">
              <div class="il-viewcontrol-mode">
                <button class="btn engaged" aria-pressed="true" data-action="ilias.php?baseClass=ilexercisehandlergui&amp;mode=ongoing&amp;ref_id=7">Ongoing</button>
                <button class="btn" aria-pressed="false" data-action="ilias.php?baseClass=ilexercisehandlergui&amp;mode=past&amp;ref_id=7">Past</button>
              </div>
              <div class="il-item-group"><h3>Ongoing</h3><ul><li class="il-std-item-container"><div class="il-item il-std-item">
                <div class="row"><div class="col-sm-3">2 Days to Submit</div><div class="col-sm-9">
                  <h4 class="il-item-title"><a href="ilias.php?baseClass=ilexercisehandlergui&amp;cmdClass=ilAssignmentPresentationGUI&amp;ref_id=7&amp;ass_id=7">Blatt 1</a></h4>
                  <div class="il-item-actions"><button data-action="ilias.php?cmd=submissionScreen&amp;ref_id=7&amp;ass_id=7">Hand In</button></div>
                  <span class="il-item-property-name">Edit Until</span><span class="il-item-property-value">9. Nov 2026, 17:47</span>
                  <span class="il-item-property-name">Last Submission</span><span class="il-item-property-value">No submission yet.</span>
                </div></div></div></li></ul></div>
            </div>"##;
        let mut page = parse_page(html, &base);
        let titles: Vec<(&str, bool)> = page
            .tabs
            .iter()
            .map(|t| (t.title.as_str(), t.active))
            .collect();
        assert_eq!(
            titles,
            [("Ongoing", true), ("Past", false), ("Info", false)]
        );
        assert!(page.tabs[1].url.contains("mode=past"));

        // The assignment has the same number as the exercise's reference id; it stays.
        page.remove(7);
        let assignment = page.items().next().unwrap();
        assert_eq!(assignment.kind, "ass");
        assert_eq!(assignment.ref_id, 7);
        assert_eq!(assignment.title, "Blatt 1");
        assert_eq!(assignment.description, "2 Days to Submit");
        assert_eq!(
            assignment.properties,
            [
                "Edit Until: 9. Nov 2026, 17:47",
                "Last Submission: No submission yet."
            ]
        );
        assert!(assignment.is_container());
    }

    #[test]
    fn lists_handed_in_files_and_finds_their_delete_form() {
        let base =
            Url::parse("https://ilias.example/ilias/ilias.php?cmd=submissionScreen").unwrap();
        let html = r#"<h3>Submitted Files - Laborbericht</h3>
            <form action="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=post&amp;ref_id=7&amp;ass_id=3">
            <table><tr><th></th><th>File Name</th><th>Date</th><th>Action</th></tr>
            <tr class="tblrow1">
              <td class="std"><input type="checkbox" id="deli_2" name="delivered[]" value="2" /></td>
              <td class="std"><label for="deli_2">Bericht.pdf</label></td>
              <td class="std">6. Nov 2026, 18:05</td>
              <td class="std"><a href="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=download&amp;ref_id=7&amp;ass_id=3&amp;delivered=2">Download</a></td>
            </tr></table>
            <input class="btn btn-default" type="submit" name="cmd[confirmDeleteDelivered]" value="Delete" />
            </form>"#;
        let page = parse_page(html, &base);
        let file = page.items().next().unwrap();
        assert_eq!(file.kind, "file");
        assert_eq!(file.title, "Bericht.pdf");
        assert_eq!(file.properties, ["6. Nov 2026, 18:05"]);
        assert!(
            file.url
                .ends_with("cmd=download&ref_id=7&ass_id=3&delivered=2")
        );

        let form = delete_form(html, &base, "2", "confirmDeleteDelivered").unwrap();
        assert!(form.as_str().ends_with("cmd=post&ref_id=7&ass_id=3"));
        assert!(delete_form(html, &base, "3", "confirmDeleteDelivered").is_none());
        assert!(delete_form(html, &base, "2", "deleteDelivered").is_none());
        assert!(lists_delivered(html, "2"));
        assert!(!lists_delivered(html, "3"));
    }

    #[test]
    fn finds_the_upload_form_of_an_assignment() {
        let base = Url::parse("https://ilias.example/ilias/ilias.php?cmd=uploadForm").unwrap();
        let html = r##"<form class="c-form" action="ilias.php?cmd=addUpload&amp;ref_id=7&amp;ass_id=1" method="post">
              <template><input id="x" type="hidden" name="form/input_0/input_1[input_0][]" value="" /></template>
            </form>
            <script>il.UI.Input.File.init(
              'il_ui_fw_1',
              'ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1',
              'ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=remove&ref_id=7&ass_id=1',
              'filename', 0, 20);</script>"##;
        let form = parse_upload_form(html, &base).unwrap();
        assert_eq!(
            form.upload.as_str(),
            "https://ilias.example/ilias/ilias.php?cmdClass=ilRepoStandardUploadHandlerGUI&cmd=upload&ref_id=7&ass_id=1"
        );
        assert!(
            form.finish
                .as_str()
                .ends_with("cmd=addUpload&ref_id=7&ass_id=1")
        );
        assert_eq!(form.field, "form/input_0/input_1[input_0][]");

        assert_eq!(parse_upload_form("<p>Time is up.</p>", &base), None);
    }

    #[test]
    fn reads_an_assignment_with_its_files_and_submission() {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let html = r##"<div id="il_center_col">
              <h3>Work Instructions</h3><p>Solve the first sheet.</p>
              <h3>Files</h3>
              <div class="row"><div class="control-label col-sm-4"><p>Blatt 1.pdf</p></div>
                <div class="col-sm-8"><p><a href="ilias.php?cmd=downloadFile&amp;ref_id=7&amp;ass_id=1&amp;file=Blatt+1.pdf">Download</a></p></div></div>
              <h3>Submission</h3>
              <div class="row"><div class="control-label col-sm-4"><p>Last Submission</p></div>
                <div class="col-sm-8"><p>Today, 17:58</p></div></div>
              <div class="row"><div class="control-label col-sm-4"></div>
                <div class="col-sm-8"><button data-action="ilias.php?cmd=submissionScreen">Hand In</button></div></div>
            </div>"##;
        let page = parse_page(html, &base);
        let file = page.items().next().unwrap();
        assert_eq!(file.kind, "file");
        assert_eq!(file.title, "Blatt 1.pdf");
        assert_eq!(file.extension().as_deref(), Some("pdf"));
        assert!(file.url.contains("cmd=downloadFile"));
        assert!(file.ref_id > 1 << 62, "no real reference id");

        let text: Vec<&str> = page
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text(text) | Block::Heading(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text,
            [
                "Work Instructions",
                "Solve the first sheet.",
                "Files",
                "Submission",
                "Last Submission: Today, 17:58"
            ]
        );
    }

    #[test]
    fn an_assignment_lists_its_submission_below_the_section_of_that_name() {
        let base = Url::parse("https://ilias.example/ilias/").unwrap();
        let tabs = r#"<ul id="ilTab">
              <li id="tab_overview" class="active"><a href="ilias.php?cmdClass=ilAssignmentPresentationGUI&amp;ass_id=1">Overview</a></li>
              <li id="tab_submission"><a href="ilias.php?cmdClass=ilExSubmissionFileGUI&amp;cmd=submissionScreen&amp;ass_id=1">Abgabe</a></li>
            </ul>"#;
        let html = format!(
            r#"{tabs}<div id="il_center_col"><h3>Files</h3><h3>Abgabe</h3><p>Nothing yet.</p></div>"#
        );
        let page = parse_page(&html, &base);
        let submission = page.items().next().unwrap();
        assert_eq!(submission.kind, "sub");
        assert_eq!(submission.title, "Handed-in files");
        assert!(submission.url.ends_with("cmd=submissionScreen&ass_id=1"));
        assert!(submission.is_container());
        // It takes the place of the section's heading.
        assert!(matches!(&page.blocks[1], Block::Items { title, .. } if title == "Abgabe"));
        assert!(matches!(&page.blocks[2], Block::Text(t) if t == "Nothing yet."));

        // On the Submission tab itself, no such item.
        let active = tabs
            .replace(r#"tab_overview" class="active""#, r#"tab_overview""#)
            .replace(
                r#"id="tab_submission""#,
                r#"id="tab_submission" class="active""#,
            );
        let page = parse_page(&format!("{active}<p>Files</p>"), &base);
        assert_eq!(page.items().count(), 0);
    }

    /// Markup modelled on a real ILIAS 10 course: tabs, page-editor text, item groups,
    /// and files whose links are `cmd=sendfile` GUI links.
    const COURSE: &str = r##"
        <ul id="ilTab">
          <li><a href="ilias.php?baseClass=ildashboardgui&amp;cmd=show">Zu Favoriten</a></li>
          <li id="tab_view_content" class="active"><a href="ilias.php?cmd=view&amp;ref_id=7">Inhalt <span class="ilAccHidden">(Ausgewählte)</span></a></li>
          <li id="tab_info_short"><a href="ilias.php?cmd=showSummary&amp;ref_id=7">Info</a></li>
          <li id="ilLastTab"><a href="#">... </a></li>
        </ul>
        <div id="il_center_col">
          <a id="ilPageShowAdvContent" style="display:none" href="#">Vertiefungswissen anzeigen</a>
          <h1 class="il-page-content-header media-heading ilHeader">DEMO.101</h1>
          <h1 class="ilc_Paragraph ilc_heading1_Headline1">Willkommen</h1>
          <p class="ilc_Paragraph">Willkommen im Modul <strong>DEMO</strong></p>
          <p class="ilc_Paragraph">Ort: Raum B-042</p>
          <div class="ilContainerBlock">
            <div class="ilContainerBlockHeader"><h2 class="ilHeader">Unterlagen</h2></div>
            <ul class="ilContainerItemsContainer"><li class="ilCLI">
              <div class="ilContainerListItemOuter">
                <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle"
                  href="https://elearning.hslu.ch/ilias/ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=9">Lecture_01</a></h3>
                <div class="dropdown"><button>Aktionen</button><ul><li>Download</li></ul></div>
                <div class="il_Description">Folien Woche 1</div>
                <div class="il_ItemProperties"><span class="il_ItemProperty">pdf&nbsp;&nbsp;</span>
                  <span class="il_ItemProperty">1.2 MB&nbsp;&nbsp;</span></div>
              </div>
            </li><li class="ilCLI">
              <div class="ilContainerListItemOuter">
                <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle"
                  href="https://elearning.hslu.ch/ilias/goto.php/fold/8">Übungen</a></h3>
              </div>
            </li><li class="ilCLI">
              <div class="ilContainerListItemOuter">
                <img src="./Customizing/skin/hslu/hslu/images/standard/icon_webr.svg" class="ilListItemIcon"/>
                <h3 class="il_ContainerItemTitle"><a class="il_ContainerItemTitle" target="_blank"
                  href="ilias.php?baseClass=ilLinkResourceHandlerGUI&amp;ref_id=10&amp;cmd=calldirectlink">Zoom-Link</a></h3>
                <div class="il_ItemProperties"><span class="il_ItemProperty">Lernfortschritt:</span></div>
              </div>
            </li></ul>
          </div>
        </div>"##;

    #[test]
    fn parses_course_layout() {
        let page = parse_page(
            COURSE,
            &url("https://elearning.hslu.ch/ilias/goto.php/crs/7"),
        );
        assert_eq!(
            page.tabs,
            vec![
                Tab {
                    title: "Inhalt".into(),
                    url: "https://elearning.hslu.ch/ilias/ilias.php?cmd=view&ref_id=7".into(),
                    active: true,
                },
                Tab {
                    title: "Info".into(),
                    url: "https://elearning.hslu.ch/ilias/ilias.php?cmd=showSummary&ref_id=7"
                        .into(),
                    active: false,
                },
            ]
        );
        let [
            Block::Heading(heading),
            Block::Text(welcome),
            Block::Text(place),
            Block::Items { title, items },
        ] = page.blocks.as_slice()
        else {
            panic!("unexpected blocks: {:#?}", page.blocks);
        };
        assert_eq!(heading, "Willkommen");
        assert_eq!(welcome, "Willkommen im Modul DEMO");
        assert_eq!(place, "Ort: Raum B-042");
        assert_eq!(title, "Unterlagen");
        assert_eq!(items.len(), 3);
        let file = &items[0];
        assert_eq!((file.kind.as_str(), file.ref_id), ("file", 9));
        assert_eq!(file.description, "Folien Woche 1");
        assert_eq!(file.properties, vec!["pdf", "1.2 MB"]);
        assert_eq!(file.extension().as_deref(), Some("pdf"));
        assert_eq!(items[1].kind, "fold");
        let link = &items[2];
        assert_eq!((link.kind.as_str(), link.ref_id), ("webr", 10));
        assert!(link.properties.is_empty());
    }

    #[test]
    fn links_in_page_text_follow_their_paragraph() {
        // As the local ILIAS 10 renders a page-editor paragraph with links.
        let html = r##"<ul id="ilTab"><li id="tab_view_content"><a href="ilias.php?cmd=view">Content</a></li></ul>
            <div id="il_center_col">
              <div class="ilc_Paragraph ilc_text_block_Standard">Lecture on Mondays. Online:
                <a class="ilc_link_ExtLink" href="https://zoom.us/j/123456789" target="_blank">Zoom-Meeting</a>
                and the <a class="ilc_link_ExtLink" href="goto.php/fold/89">Unterlagen</a>
                (<a href="#top">top</a>, <a href="mailto:a@b.ch">mail</a>,
                <a href="ilias.php?cmd=markPostRead&amp;pos_pk=1">read</a>,
                <a href="https://zoom.us/j/123456789">again</a>).</div>
              <div class="ilContainerBlock"><h2>Inhalt</h2>
                <div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/file/7">Skript</a></div></div>
            </div>"##;
        let page = parse_page(html, &url("https://ilias.example/ilias/goto.php/crs/1"));
        assert!(
            matches!(&page.blocks[0], Block::Text(t) if t.starts_with("Lecture on Mondays. Online: Zoom-Meeting and the Unterlagen"))
        );
        let Block::Items { title, items } = &page.blocks[1] else {
            panic!("links after the paragraph: {:?}", page.blocks);
        };
        assert!(title.is_empty());
        let links: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|i| (i.kind.as_str(), i.title.as_str(), i.url.as_str()))
            .collect();
        // Only web links, each once, and none that would change something on ILIAS.
        assert_eq!(
            links,
            [
                ("link", "Zoom-Meeting", "https://zoom.us/j/123456789"),
                (
                    "link",
                    "Unterlagen",
                    "https://ilias.example/ilias/goto.php/fold/89"
                ),
            ]
        );
        assert_eq!(items[0].description, "https://zoom.us/j/123456789");
        assert!(matches!(&page.blocks[2], Block::Items { title, .. } if title == "Inhalt"));
    }

    #[test]
    fn joins_info_screen_label_and_value() {
        let html = r#"<div id="il_center_col"><div class="ilInfoScreenSec">
            <div class="ilFormHeader"><h2 class="ilHeader">Allgemein</h2></div>
            <div class="form-group row"><div class="il_InfoScreenProperty control-label">Sprache</div>
              <div class="il_InfoScreenPropertyValue"> Deutsch </div></div></div></div>"#;
        let page = parse_page(html, &url("https://elearning.hslu.ch/ilias/ilias.php"));
        assert_eq!(
            page.blocks,
            vec![
                Block::Heading("Allgemein".into()),
                Block::Text("Sprache: Deutsch".into())
            ]
        );
    }

    #[test]
    fn parses_ui_framework_item_groups() {
        let html = r#"<main><div class="il-item-group"><h3>2026</h3><div class="il-item-group-items"><ul>
            <li><div class="il-item il-std-item"><h4 class="il-item-title">
              <a href="https://elearning.hslu.ch/ilias/goto.php/crs/654321">DEMO.102</a></h4>
              <div class="il-item-description">Examples &amp; Exercises</div>
              <span class="il-item-property-name">Anmeldung</span><span class="il-item-property-value">Keine</span>
              <span class="il-item-property-name"></span><span class="il-item-property-value"></span>
            </div></li></ul></div></div></main>"#;
        let page = parse_page(html, &url("https://elearning.hslu.ch/ilias/ilias.php"));
        let [Block::Items { title, items }] = page.blocks.as_slice() else {
            panic!("unexpected blocks: {:#?}", page.blocks);
        };
        assert_eq!(title, "2026");
        assert_eq!(items[0].description, "Examples & Exercises");
        assert_eq!(items[0].properties, vec!["Anmeldung: Keine"]);
    }

    #[test]
    fn parses_forum_threads_without_rating_buttons() {
        let html = r#"<div id="il_center_col"><div class="il-item-group"><div class="il-item-group-items"><ul>
            <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=123456&amp;thr_pk=1&amp;page=0">Frage zu Blatt 1</a></h4>
              <span class="il-item-property-name">Beiträge</span><span class="il-item-property-value">2 (1 Ungelesen)</span>
              <span class="il-item-property-name">Bewertung</span><span class="il-item-property-value"><div class="il-standard-popover-content" style="display:none;"><div class="ilRatingOverlay"><p>Ihre Bewertung</p></div></div><button>Bewertungsskala aufklappen</button></span>
            </div></li>
            <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="ilias.php?baseClass=ilrepositorygui&amp;cmdClass=ilObjForumGUI&amp;cmd=viewThread&amp;ref_id=123456&amp;thr_pk=2&amp;page=0">Termine</a></h4>
            </div></li></ul></div></div></div>"#;
        let page = parse_page(
            html,
            &url("https://ilias.example/ilias/goto.php/frm/123456"),
        );
        let threads: Vec<_> = page.items().collect();
        assert_eq!(
            threads.len(),
            2,
            "both threads, although they share the forum's ref_id"
        );
        assert_eq!((threads[0].kind.as_str(), threads[0].ref_id), ("thr", 1));
        assert!(threads[0].is_container());
        assert_eq!(threads[0].properties, vec!["Beiträge: 2 (1 Ungelesen)"]);
    }

    #[test]
    fn parses_forum_posts() {
        let html = r##"<div id="il_center_col"><ul>
            <li class="ilFrmPostRow ilModeratorPosting ilFrmPost-level-1"><div class="ilFrmPostContentContainer">
              <div class="ilFrmPostHeader"><span class="small">Ada Lovelace | <a href="#">alovelace</a> (Moderator) | 19. Feb 2026, 07:12</span>
                <div class="ilFrmPostTitle">Willkommen</div></div>
              <div class="ilFrmPostContent"><p>Hallo zusammen!</p><p>Fragen bitte hier.</p></div></div>
              <div class="il-item-actions"><button>Antworten</button></div></li>
            <li class="ilFrmPostRow tblrow2 ilFrmPost-level-2"><div class="ilFrmPostContentContainer">
              <div class="ilFrmPostHeader"><span class="small">Alan Turing | <a href="#">aturing</a> | 20. Feb 2026, 10:00</span>
                <div class="ilFrmPostTitle"><a href="ilias.php?cmd=markPostRead&amp;pos_pk=2"><b>Re: Willkommen</b></a></div></div>
              <div class="ilFrmPostContent"><span>Danke!<br />Bis bald</span></div></div></li>
            </ul></div>"##;
        let page = parse_page(html, &url("https://ilias.example/ilias/ilias.php"));
        let posts: Vec<&Post> = page
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Post(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(page.blocks.len(), 2, "only the posts: {:#?}", page.blocks);
        assert_eq!(
            *posts[0],
            Post {
                author: "Ada Lovelace · alovelace (Moderator)".into(),
                date: "19. Feb 2026, 07:12".into(),
                title: "Willkommen".into(),
                body: vec!["Hallo zusammen!".into(), "Fragen bitte hier.".into()],
                depth: 0,
                unread: false,
            }
        );
        assert_eq!(posts[1].title, "Re: Willkommen");
        assert_eq!(posts[1].body, vec!["Danke!", "Bis bald"]);
        assert_eq!(posts[1].depth, 1);
        assert!(posts[1].unread);
    }

    #[test]
    fn parses_the_todo_block() {
        let html = r#"<div id="il_center_col"></div><aside id="il_right_col">
            <div id="block_pdtasks_0"><div class="il-item-group"><ul>
              <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/123456/1">Abgabe zur Übungseinheit "Blatt 1"</a></h4>
                <div class="il-item-description">17. Jan 2027, 23:55</div>
                <span class="il-item-property-name">Übung</span><span class="il-item-property-value">Abgabeordner</span></div></li>
              <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/exc/123456/2">Abgabe zur Übungseinheit "Blatt 2"</a></h4>
                <div class="il-item-description">5. Nov 2026, 00:00</div></div></li>
            </ul></div></div>
            <div id="block_pdnews_0"><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/file/9">Neue Datei</a></h4></div></div>
            </aside>"#;
        let todo = parse_dashboard(html, &url("https://ilias.example/ilias/ilias.php")).todo;
        let found: Vec<_> = todo
            .iter()
            .map(|i| {
                (
                    i.kind.as_str(),
                    i.ref_id,
                    i.title.as_str(),
                    i.description.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            vec![
                ("exc", 123456, "Blatt 1", "17. Jan 2027, 23:55"),
                ("exc", 123456, "Blatt 2", "5. Nov 2026, 00:00"),
            ]
        );
        assert_eq!(todo[0].properties, vec!["Übung: Abgabeordner"]);
    }

    #[test]
    fn finds_the_course_in_the_breadcrumb() {
        let html = r#"<nav class="breadcrumb"><span class="breadcrumb-crumb"><a href="ilias.php?ref_id=3">Abgabeordner</a></span>
            <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/crs/2">DEMO.101</a></span>
            <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/cat/1">2026</a></span></nav>"#;
        assert_eq!(course_title(html).as_deref(), Some("DEMO.101"));
        assert_eq!(course_title("<p>nothing</p>"), None);
    }

    #[test]
    fn parses_ilias_dates() {
        use jiff::civil::{date, datetime};
        let today = date(2026, 11, 1);
        assert_eq!(
            parse_date("5. Nov 2026, 00:00", today),
            Some(datetime(2026, 11, 5, 0, 0, 0, 0))
        );
        assert_eq!(
            parse_date("3. Mär 2027, 23:55", today),
            Some(datetime(2027, 3, 3, 23, 55, 0, 0))
        );
        assert_eq!(
            parse_date("1. Oct 2026, 8:05", today),
            Some(datetime(2026, 10, 1, 8, 5, 0, 0))
        );
        assert_eq!(
            parse_date("Morgen, 12:00", today),
            Some(datetime(2026, 11, 2, 12, 0, 0, 0))
        );
        assert_eq!(parse_date("bald", today), None);
    }

    #[test]
    fn reads_favourites_and_their_menu_links() {
        let html = r#"<div id="il_center_col">
            <div id="block_dash_fav_0"><h2>Favoriten</h2><div class="il-deck">
              <div class="il-card thumbnail">
                <ul class="dropdown-menu"><li><button data-action="ilias.php?baseClass=ildashboardgui&amp;cmd=removeFromDesk&amp;type=crs&amp;item_ref_id=101">Von Favoriten entfernen</button></li></ul>
                <div class="caption card-title"><a href="https://ilias.example/ilias/goto.php/crs/101">DEMO.101</a></div>
              </div></div></div>
            <div class="il-item-group"><h3>2026</h3><div class="il-item-group-items"><ul>
              <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/102">DEMO.102</a></h4>
                <ul class="dropdown-menu"><li><button data-action="ilias.php?cmd=addToDesk&amp;type=crs&amp;item_ref_id=102">Zu Favoriten hinzufügen</button></li></ul></div></li>
              <li><div class="il-item il-std-item"><h4 class="il-item-title"><a href="https://ilias.example/ilias/goto.php/crs/103">DEMO.103</a></h4>
                <ul class="dropdown-menu"><li><button data-action="ilias.php?cmd=addToDesk&amp;type=crs&amp;item_ref_id=999">Zu Favoriten hinzufügen</button></li></ul></div></li>
            </ul></div></div></div>"#;
        let base = url("https://ilias.example/ilias/ilias.php");
        let Some(Block::Items { title, items }) = parse_dashboard(html, &base).favorites else {
            panic!("no favourites");
        };
        assert_eq!(title, "Favoriten");
        assert_eq!((items[0].ref_id, items[0].favorite), (101, true));
        assert!(
            items[0]
                .favorite_toggle
                .as_deref()
                .unwrap()
                .contains("cmd=removeFromDesk")
        );

        let page = parse_page(html, &base);
        let course = |id| page.items().find(|i| i.ref_id == id).unwrap();
        assert!(!course(102).favorite);
        assert_eq!(
            course(102).favorite_toggle.as_deref(),
            Some("https://ilias.example/ilias/ilias.php?cmd=addToDesk&type=crs&item_ref_id=102")
        );
        // A menu link for another object is not this item's.
        assert_eq!(course(103).favorite_toggle, None);
    }

    #[test]
    fn files_know_their_folders() {
        let html = r#"<nav><div class="breadcrumb" dir="rtl">
              <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/fold/3">Woche 1</a></span>
              <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/fold/2">Unterlagen</a></span>
              <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/crs/1">DEMO.101</a></span>
              <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/cat/9">2026</a></span>
              <span class="breadcrumb-crumb"><a href="https://ilias.example/ilias/goto.php/root/1">Magazin</a></span>
            </div></nav>
            <div id="il_center_col"><div class="ilContainerListItemOuter">
              <a class="il_ContainerItemTitle" href="ilias.php?cmdClass=ilObjFileGUI&amp;cmd=sendfile&amp;ref_id=4">Lecture_01</a></div>
              <div class="ilContainerListItemOuter"><a class="il_ContainerItemTitle" href="goto.php/fold/5">Extras</a></div></div>"#;
        let page = parse_page(html, &url("https://ilias.example/ilias/goto.php/fold/3"));
        let items: Vec<_> = page.items().collect();
        assert_eq!(items[0].path, vec!["DEMO.101", "Unterlagen", "Woche 1"]);
        assert!(items[1].path.is_empty(), "only files need it");
    }
}
