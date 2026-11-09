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
    pub(super) seen: HashSet<u64>,
    /// Ids of the text links so far, to list each one once.
    pub(super) seen_links: HashSet<u64>,
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

    pub(super) fn item(&mut self, container: ElementRef) -> Option<Item> {
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
        if !matches!(url.scheme(), "http" | "https") || crate::ilias::write::is_write(&url) {
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

    pub(super) fn flush(&mut self) {
        let text = clean(&std::mem::take(&mut self.line));
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
