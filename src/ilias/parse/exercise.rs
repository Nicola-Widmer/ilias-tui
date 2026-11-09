//! Exercises: the Submission tab as an item, handed-in files, upload and delete forms.

use scraper::Html;
use url::Url;

use super::*;

/// An assignment's overview gets its Submission tab as an item too, so the list keys reach
/// it: under the heading of the section of the same name if there is one, else at the end.
pub(super) fn add_submission(tabs: &[Tab], blocks: &mut Vec<Block>) {
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

/// A stable id for a file attached to an assignment, which has no reference id of its own.
/// The top bit keeps it apart from real reference ids (FNV-1a, like the cache's file names).
pub(super) fn attachment_id(url: &str) -> u64 {
    let hash = url.bytes().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    hash | 1 << 63
}
