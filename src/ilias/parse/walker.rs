//! The walk through the content column that turns it into blocks.

use std::collections::HashSet;

use scraper::ElementRef;
use scraper::node::Node;
use url::Url;

use super::*;

/// Walks the content column in document order and splits it into [`Block`]s.
pub(super) struct Walker<'a> {
    pub(super) page_url: &'a Url,
    pub(super) blocks: Vec<Block>,
    /// Text of the paragraph currently being collected.
    pub(super) line: String,
    /// Links in that paragraph; they follow it as `link` items.
    pub(super) links: Vec<Item>,
    pub(super) seen: HashSet<Id>,
    /// Ids of the text links so far, to list each one once.
    pub(super) seen_links: HashSet<Id>,
}

pub(super) const SKIPPED_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "dialog", "button", "select", "img", "svg", "iframe",
];
pub(super) const SKIPPED_CLASSES: &[&str] = &[
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
pub(super) const BLOCK_TAGS: &[&str] = &[
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
pub(super) const ITEM_CONTAINERS: &str = ".ilContainerListItemOuter, .il-std-item";

impl<'a> Walker<'a> {
    pub(super) fn new(page_url: &'a Url) -> Self {
        Self {
            page_url,
            blocks: Vec::new(),
            line: String::new(),
            links: Vec::new(),
            seen: HashSet::new(),
            seen_links: HashSet::new(),
        }
    }

    pub(super) fn walk(&mut self, el: ElementRef) {
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
        if self.accordion_pane(el) {
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
                    kind: Kind::File,
                    id: Id::local(url.as_str()),
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
        // A link around paragraphs (`<a href="goto.php/fold/7"><p>Unterlagen</p></a>`, as in
        // page-editor navigation) is one line with its link.
        let blocks_inside = || {
            el.children()
                .filter_map(ElementRef::wrap)
                .any(|c| BLOCK_TAGS.contains(&c.value().name()))
        };
        if name == "a" && blocks_inside() {
            self.flush();
            self.line = visible_text(el);
            let link = self.text_link(el);
            self.links.extend(link);
            self.flush();
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

    /// One pane of an accordion (page editor accordions and `ilAccordionGUI` alike):
    /// `<div class="ilc_va_icntr_AccordICntr"><div class="il_VAccordionToggleDef">caption</div>
    /// <div class="il_VAccordionContentDef ilAccHideContent">content</div></div>`. ILIAS hides
    /// the content with `ilAccHideContent` unless the accordion is "all open"; with "first
    /// open", its script opens the first pane. A pane without content is left out.
    fn accordion_pane(&mut self, el: ElementRef) -> bool {
        let child = |suffix: &str| {
            el.children().filter_map(ElementRef::wrap).find(|c| {
                c.value().classes().any(|x| {
                    x == format!("il_VAccordion{suffix}") || x == format!("il_HAccordion{suffix}")
                })
            })
        };
        let (Some(toggle), Some(content)) = (child("ToggleDef"), child("ContentDef")) else {
            return false;
        };
        self.flush();
        let first_open = el
            .parent()
            .and_then(ElementRef::wrap)
            .is_some_and(|accordion| {
                accordion.value().attr("data-copg-tabs-behaviour") == Some("FirstOpen")
                    && accordion.children().filter_map(ElementRef::wrap).next() == Some(el)
            });
        let hidden = content.value().classes().any(|c| c == "ilAccHideContent");
        // The label was walked already, as the last text.
        let label = row_label(el)
            .filter(|label| self.blocks.last() == Some(&Block::Text(label.clone())))
            .unwrap_or_default();
        if !label.is_empty() {
            self.blocks.pop();
        }
        let start = self.blocks.len();
        self.blocks.push(Block::Pane {
            title: visible_text(toggle),
            open: !hidden || first_open,
            label,
        });
        self.walk(content);
        self.flush();
        if self.blocks.len() == start + 1 {
            self.blocks.pop();
        } else {
            self.blocks.push(Block::PaneEnd);
        }
        true
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
            kind: Kind::File,
            id: Id::local(url.as_str()),
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

    pub(super) fn item(&mut self, container: ElementRef) -> Option<Item> {
        let a = container
            .select(&sel(
                "a.il_ContainerItemTitle, .il-item-title a, .card-title a",
            ))
            .next()?;
        let url = self.page_url.join(a.attr("href")?).ok()?;
        // Some links (e.g. weblinks: `baseClass=ilLinkResourceHandlerGUI&ref_id=…`) carry no
        // type; the item's icon still tells it.
        let (kind, id) = parse_target(&url).or_else(|| {
            let ref_id = url
                .query_pairs()
                .find(|(k, _)| k == "ref_id")?
                .1
                .parse()
                .ok()?;
            Some((kind_from_icon(container)?, Id::Ref(ref_id)))
        })?;
        if !self.seen.insert(id) {
            return None;
        }
        // Assignments show the time left in front of the title instead of a description.
        let description = container
            .select(&sel(if kind == Kind::Assignment {
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
                    .any(|(k, v)| k == "item_ref_id" && v == id.key().to_string())
            });
        let favorite = favorite_toggle.as_ref().is_some_and(|link| {
            link.query()
                .is_some_and(|q| q.contains("cmd=removeFromDesk"))
        });
        Some(Item {
            kind,
            id,
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
        if !matches!(url.scheme(), "http" | "https") || crate::ilias::write::is_write(&url) {
            return None;
        }
        let id = Id::local(url.as_str());
        if !self.seen_links.insert(id) {
            return None;
        }
        let text = visible_text(a);
        Some(Item {
            kind: Kind::Link,
            id,
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

    pub(super) fn flush(&mut self) {
        let mut text = clean(&std::mem::take(&mut self.line));
        // `Input 1: Skript` with a link on "Skript": the link follows as an entry, so the
        // line keeps only its label.
        if let Some(label) = label_of_links(&text, &self.links) {
            text = label;
        }
        if !text.is_empty() && text != "•" {
            self.blocks.push(Block::Text(text));
        }
        for link in std::mem::take(&mut self.links) {
            self.push_item(link);
        }
    }
}

pub(super) fn is_item_container(el: ElementRef) -> bool {
    let e = el.value();
    e.classes()
        .any(|c| c == "ilContainerListItemOuter" || c == "il-std-item")
}

/// The short text beside a lone accordion pane in a table-like row, such as the date
/// cell of an agenda: `<div class="…AgendaRow"><div><p>17.09.2026</p></div><div><div
/// class="ilc_va_cntr_…">one pane</div><p>…</p></div></div>`. Going up from the accordion,
/// the first ancestor with something visible before it must have exactly one such thing:
/// a short text without links.
fn row_label(pane: ElementRef) -> Option<String> {
    let accordion = pane.parent().and_then(ElementRef::wrap)?;
    let panes = accordion
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|c| c.value().classes().any(|x| x.contains("AccordICntr")))
        .count();
    if panes != 1 {
        return None;
    }
    let shows = |el: &ElementRef| {
        let e = el.value();
        !SKIPPED_TAGS.contains(&e.name())
            && !e
                .attr("style")
                .is_some_and(|s| s.replace(' ', "").contains("display:none"))
            && !visible_text(*el).is_empty()
    };
    let mut node = accordion;
    for _ in 0..4 {
        if node.value().id() == Some("il_center_col") {
            return None;
        }
        let before: Vec<ElementRef> = node
            .prev_siblings()
            .filter_map(ElementRef::wrap)
            .filter(shows)
            .collect();
        match before.as_slice() {
            [] => node = node.parent().and_then(ElementRef::wrap)?,
            [cell] => {
                let text = visible_text(*cell);
                let plain = cell.select(&sel("a")).next().is_none();
                return (plain && text.chars().count() <= 40).then_some(text);
            }
            _ => return None,
        }
    }
    None
}

/// What is left of a line of text without its links' titles, if that is no more than a
/// label: `Material: CBL26 | EBNF` with links on both → `Material:`. `None` if the line
/// says more than that, or has no links.
fn label_of_links(text: &str, links: &[Item]) -> Option<String> {
    if links.is_empty() {
        return None;
    }
    let mut rest = text.to_string();
    for link in links {
        let at = rest.find(link.title.as_str())?;
        rest.replace_range(at..at + link.title.len(), " ");
    }
    let separators: &[char] = &[' ', '|', ',', ';', '/', '-', '–', '·', '(', ')'];
    let label = clean(rest.trim_matches(separators));
    let separated = rest
        .trim_matches(separators)
        .split(&label)
        .all(|part| part.chars().all(|c| separators.contains(&c)));
    (label.chars().count() <= 30 && separated).then_some(label)
}
