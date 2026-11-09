//! Rights: what ILIAS lets you change on a page, as its menus and forms offer it ("Add New
//! Item", the upload form, each item's Delete). Nothing here is guessed: a right is only
//! there if the page has the link or form for it, for this very page.

use scraper::Html;
use serde::{Deserialize, Serialize};
use url::Url;

use super::*;

/// What you may change on a page besides reading it. Students usually have none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rights {
    /// The page has an upload form for files into itself.
    #[serde(default)]
    pub upload: bool,
    /// What "Add New Item" offers to create here.
    #[serde(default)]
    pub create: Vec<Kind>,
    /// Items (by reference id) whose action menu offers Delete.
    #[serde(default)]
    pub delete: Vec<u64>,
}

impl Rights {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn may_create(&self, kind: &Kind) -> bool {
        self.create.contains(kind)
    }

    pub fn may_delete(&self, item: &Item) -> bool {
        matches!(item.id, Id::Ref(n) if self.delete.contains(&n))
    }
}

/// For `ilias-tui show`: `upload · new: fold, file · delete: 3 items`.
impl std::fmt::Display for Rights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.upload {
            parts.push("upload".to_string());
        }
        if !self.create.is_empty() {
            let kinds: Vec<&str> = self.create.iter().map(Kind::code).collect();
            parts.push(format!("new: {}", kinds.join(", ")));
        }
        if !self.delete.is_empty() {
            parts.push(format!("delete: {} items", self.delete.len()));
        }
        f.write_str(&parts.join(" · "))
    }
}

/// The reference id a page is about: `goto.php/<kind>/<ref_id>`, else its `ref_id`.
pub fn own_ref_id(page_url: &Url) -> Option<u64> {
    let segments: Vec<&str> = page_url.path_segments()?.collect();
    if let Some(pos) = segments.iter().position(|s| *s == "goto.php") {
        return segments.get(pos + 2)?.parse().ok();
    }
    param(page_url, "ref_id")?.parse().ok()
}

/// The value of `key` in a link's query.
pub fn param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// The rights the page at `page_url` offers for itself and its items.
pub(super) fn rights(doc: &Html, page_url: &Url) -> Rights {
    let Some(own) = own_ref_id(page_url).map(|n| n.to_string()) else {
        return Rights::default();
    };
    let base = link_base(page_url);
    let links = |css: &str, attr: &str| -> Vec<Url> {
        doc.select(&sel(css))
            .filter_map(|el| base.join(el.attr(attr)?).ok())
            .filter(|url| url.origin() == base.origin())
            .filter(|url| param(url, "ref_id").as_deref() == Some(own.as_str()))
            .collect()
    };
    let mut rights = Rights {
        upload: links(r#"form[action*="fallbackCmd=uploadFiles"]"#, "action")
            .iter()
            .any(|url| {
                param(url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case("ilObjFileGUI"))
                    && param(url, "new_type").as_deref() == Some("file")
            }),
        ..Rights::default()
    };
    for url in links(r#"a[href*="cmd=create"]"#, "href") {
        if param(&url, "cmd").as_deref() == Some("create")
            && let Some(kind) = param(&url, "new_type").filter(|t| !t.is_empty())
            && !rights.may_create(&kind.as_str().into())
        {
            rights.create.push(kind.as_str().into());
        }
    }
    // Action menus hold buttons that go to their `data-action`.
    let menu = links(r#"button[data-action*="cmd=delete"]"#, "data-action");
    for url in menu
        .into_iter()
        .chain(links(r#"a[href*="cmd=delete"]"#, "href"))
    {
        if param(&url, "cmd").as_deref() == Some("delete")
            && let Some(item) = param(&url, "item_ref_id").and_then(|n| n.parse().ok())
            && !rights.delete.contains(&item)
        {
            rights.delete.push(item);
        }
    }
    rights
}
