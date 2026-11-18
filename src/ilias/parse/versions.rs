//! A file's Versions tab (`ilFileVersionsGUI`): the versions ILIAS keeps of it, and its
//! "Replace File" form, which puts your copy in place of the file (ilias-tui never uses
//! "Create New Version"). Each one is read for one file (by reference id) and only from
//! that file's own page.

use scraper::Html;
use url::Url;

use super::*;

/// Whether `url` is on this ILIAS (`base`), of GUI class `class` and for file `own`.
fn for_file(url: &Url, base: &Url, class: &str, own: &str) -> bool {
    url.origin() == base.origin()
        && param(url, "ref_id").as_deref() == Some(own)
        && param(url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case(class))
}

/// The links of buttons (`data-action`) and anchors (`href`) on the page, resolved.
fn links(doc: &Html, base: &Url) -> Vec<Url> {
    let (button, anchor) = (sel("button[data-action]"), sel("a[href]"));
    let buttons = doc.select(&button).filter_map(|b| b.attr("data-action"));
    let anchors = doc.select(&anchor).filter_map(|a| a.attr("href"));
    buttons
        .chain(anchors)
        .filter_map(|link| base.join(link).ok())
        .collect()
}

/// The Versions entry of file `file`'s action menu on a container page.
pub fn versions_link(html: &str, page_url: &Url, file: u64) -> Option<Url> {
    let base = link_base(page_url);
    let own = file.to_string();
    links(&Html::parse_document(html), &base)
        .into_iter()
        .find(|url| for_file(url, &base, "ilFileVersionsGUI", &own) && param(url, "cmd").is_none())
}

/// The "Replace File" button of file `file`'s Versions tab.
pub fn replace_link(html: &str, page_url: &Url, file: u64) -> Option<Url> {
    let base = link_base(page_url);
    let own = file.to_string();
    links(&Html::parse_document(html), &base)
        .into_iter()
        .find(|url| {
            for_file(url, &base, "ilFileVersionsGUI", &own)
                && param(url, "cmd").as_deref() == Some("addReplacingVersion")
        })
}

/// The version numbers the Versions tab lists, e.g. `[2, 1]`.
pub fn versions(html: &str) -> Vec<u32> {
    let doc = Html::parse_document(html);
    doc.select(&sel("#ilFileVersionsTableGUI tbody tr"))
        .filter_map(|row| {
            let cells: Vec<String> = row.select(&sel("td")).map(text_of).collect();
            cells.get(1)?.trim().parse().ok()
        })
        .collect()
}

/// The "Replace File" form of a file.
#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceForm {
    /// Takes the file and keeps it until the form is sent; answers with its id. Always in
    /// `upload_mode=replace`.
    pub upload: Url,
    /// Takes title, description and the file's id, and replaces the file.
    pub save: Url,
    /// The title field and the title ILIAS filled in (kept, so the file keeps its name).
    pub title: (String, String),
    pub description: String,
    pub id: String,
    /// The biggest file ILIAS takes here, in bytes, if the form says.
    pub max_size: Option<u64>,
}

/// The form "Replace File" leads to, for file `file`; `None` if it isn't exactly that
/// form (another file, another site, another command, or an upload that adds a version).
pub fn replace_form(html: &str, page_url: &Url, file: u64) -> Option<ReplaceForm> {
    let base = link_base(page_url);
    let own = file.to_string();
    let doc = Html::parse_document(html);
    let (form, save) = doc
        .select(&sel(
            r#"form[action*="fallbackCmd=createReplacingVersion"]"#,
        ))
        .find_map(|form| {
            let url = base.join(form.attr("action")?).ok()?;
            let expected = for_file(&url, &base, "ilFileVersionsGUI", &own)
                && param(&url, "fallbackCmd").as_deref() == Some("createReplacingVersion");
            expected.then_some((form, url))
        })?;
    let field = |css: &str| form.select(&sel(css)).next();
    let title = field(r#"input[type="text"]"#)?;
    let title = (
        title.attr("name")?.to_string(),
        title.attr("value").unwrap_or_default().to_string(),
    );
    let description = field("textarea")?.attr("name")?.to_string();
    let id = field(r#"input[type="hidden"]"#)?.attr("name")?.to_string();
    // As for "Upload Files", the address for the file only exists in a script:
    // `File.init('<id>', '<upload>', '<remove>', 'file_id', <count>, <max files>, <max size>, …`.
    let (upload, args) = html
        .split("il.UI.Input.File.init(")
        .skip(1)
        .find_map(|call| {
            let args: Vec<&str> = call.splitn(8, ',').map(str::trim).collect();
            let url = base
                .join(&args.get(1)?.trim_matches('\'').replace("&amp;", "&"))
                .ok()?;
            let expected = for_file(&url, &base, "ilFileVersionsUploadHandlerGUI", &own)
                && param(&url, "cmd").as_deref() == Some("upload")
                && param(&url, "upload_mode").as_deref() == Some("replace");
            expected.then_some((url, args))
        })?;
    Some(ReplaceForm {
        upload,
        save,
        title,
        description,
        id,
        max_size: args.get(6).and_then(|n| n.parse().ok()),
    })
}
