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
}

impl Item {
    pub fn is_container(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "root" | "cat" | "crs" | "grp" | "fold" | "sess"
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
            "tst" => "Test",
            "svy" => "Survey",
            "frm" => "Forum",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    Heading(String),
    Text(String),
    /// An item group / content block; `title` is its header (may be empty).
    Items {
        title: String,
        items: Vec<Item>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub tabs: Vec<Tab>,
    pub blocks: Vec<Block>,
}

impl Page {
    pub fn from_items(items: Vec<Item>) -> Self {
        let blocks = vec![Block::Items {
            title: String::new(),
            items,
        }];
        Self {
            tabs: Vec::new(),
            blocks,
        }
    }

    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.blocks.iter().flat_map(|b| match b {
            Block::Items { items, .. } => items.as_slice(),
            _ => &[],
        })
    }

    /// Drops an item everywhere (e.g. the container itself, linked from its own page).
    pub fn remove(&mut self, ref_id: u64) {
        for block in &mut self.blocks {
            if let Block::Items { items, .. } = block {
                items.retain(|i| i.ref_id != ref_id);
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

/// Parses a whole ILIAS page.
pub fn parse_page(html: &str, page_url: &Url) -> Page {
    // Relative links are relative to the ILIAS root, also on `goto.php/crs/123` permalinks.
    let mut link_base = page_url.clone();
    if let Some(i) = page_url.path().find("goto.php/") {
        link_base.set_path(&page_url.path()[..i]);
    }
    let page_url = &link_base;
    let doc = Html::parse_document(html);
    let tabs = doc
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

    let root = doc
        .select(&sel("#il_center_col"))
        .next()
        .or_else(|| doc.select(&sel("main")).next())
        .unwrap_or_else(|| doc.root_element());
    let mut walker = Walker {
        page_url,
        blocks: Vec::new(),
        line: String::new(),
        seen: HashSet::new(),
    };
    walker.walk(root);
    walker.flush();
    Page {
        tabs,
        blocks: walker.blocks,
    }
}

/// Walks the content column in document order and splits it into [`Block`]s.
struct Walker<'a> {
    page_url: &'a Url,
    blocks: Vec<Block>,
    /// Text of the paragraph currently being collected.
    line: String,
    seen: HashSet<u64>,
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

impl Walker<'_> {
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
        if hidden || SKIPPED_TAGS.contains(&name) || SKIPPED_CLASSES.iter().any(|c| has_class(c)) {
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
                match self.blocks.last_mut() {
                    Some(Block::Items { items, .. }) => items.push(item),
                    _ => self.blocks.push(Block::Items {
                        title: String::new(),
                        items: vec![item],
                    }),
                }
            }
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

    fn item(&mut self, container: ElementRef) -> Option<Item> {
        let a = container
            .select(&sel("a.il_ContainerItemTitle, .il-item-title a"))
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
        let description = container
            .select(&sel(".il_Description, .il-item-description"))
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
        Some(Item {
            kind,
            ref_id,
            title: text_of(a),
            url: url.into(),
            description,
            properties,
        })
    }

    fn flush(&mut self) {
        let text = clean(&std::mem::take(&mut self.line));
        if !text.is_empty() && text != "•" {
            self.blocks.push(Block::Text(text));
        }
    }
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
}
