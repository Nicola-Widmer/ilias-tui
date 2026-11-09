//! Small helpers for reading HTML: selectors, visible text, link bases.

use scraper::node::Node;
use scraper::{ElementRef, Selector};
use url::Url;

pub(super) fn sel(css: &str) -> Selector {
    Selector::parse(css).unwrap()
}

pub(super) fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn text_of(el: ElementRef) -> String {
    clean(&el.text().collect::<Vec<_>>().join(" "))
}

/// Text without screen-reader-only parts such as "(Ausgewählte)".
pub(super) fn visible_text(el: ElementRef) -> String {
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
pub(super) fn kind_from_icon(container: ElementRef) -> Option<String> {
    let src = container
        .select(&sel("img"))
        .find_map(|img| img.attr("src"))?;
    let name = src.rsplit('/').next()?.strip_prefix("icon_")?;
    let kind = name.split(['.', '_']).next()?;
    (!kind.is_empty() && kind != "custom").then(|| kind.to_string())
}

/// What relative links on a page are relative to: the ILIAS root, also on `goto.php/crs/123`
/// permalinks.
pub(super) fn link_base(page_url: &Url) -> Url {
    let mut base = page_url.clone();
    if let Some(i) = page_url.path().find("goto.php/") {
        base.set_path(&page_url.path()[..i]);
    }
    base
}
