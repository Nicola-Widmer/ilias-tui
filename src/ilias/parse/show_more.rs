//! Lists ILIAS cuts short behind a "Show more" button.

use scraper::{ElementRef, Html, Selector};
use url::Url;

use super::*;

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
    /// The last item shown before the button.
    pub fn last_shown(&self) -> Option<Id> {
        // ILIAS reads the ref id the same way: the third part of the list id.
        Some(Id::Ref(self.shown.last()?.split('_').nth(2)?.parse().ok()?))
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
