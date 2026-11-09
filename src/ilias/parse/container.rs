//! The forms of courses and folders that change them: uploading files into one. Each one is
//! read for one container (by reference id) and only from that container's own form.

use scraper::Html;
use url::Url;

use super::*;

/// How files get into a course or folder, from its "Upload Files" form.
#[derive(Debug, PartialEq, Eq)]
pub struct FilesForm {
    /// Takes one file at a time and keeps it until the form is sent; answers with its id.
    pub upload: Url,
    /// Takes, per file, its title, description and id ([`Self::fields`]), and creates them.
    pub finish: Url,
    /// The names of the title, description and file id fields, in this order.
    pub fields: [String; 3],
    /// The biggest file ILIAS takes here, in bytes, if the form says.
    pub max_size: Option<u64>,
    /// How many files one form takes, if it says.
    pub max_files: Option<usize>,
}

/// The upload form for files into container `ref_id`; `None` if the page has none for it.
pub fn files_form(html: &str, page_url: &Url, ref_id: u64) -> Option<FilesForm> {
    let base = link_base(page_url);
    let own = ref_id.to_string();
    let for_here = |url: &Url, class: &str, command: &str| {
        url.origin() == base.origin()
            && param(url, "ref_id").as_deref() == Some(own.as_str())
            && param(url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case(class))
            && (param(url, "cmd").as_deref() == Some(command)
                || param(url, "fallbackCmd").as_deref() == Some(command))
    };
    let doc = Html::parse_document(html);
    let (form, finish) = doc
        .select(&sel(r#"form[action*="fallbackCmd=uploadFiles"]"#))
        .find_map(|form| {
            let url = base.join(form.attr("action")?).ok()?;
            for_here(&url, "ilObjFileGUI", "uploadFiles").then_some((form, url))
        })?;
    let name = |css: &str| {
        form.select(&sel(css))
            .next()
            .and_then(|input| input.attr("name"))
            .map(String::from)
    };
    let fields = [
        name(r#"input[type="text"]"#)?,
        name("textarea")?,
        name(r#"input[type="hidden"]"#)?,
    ];
    // The address for the files only exists in a script:
    // `File.init('<id>', '<upload>', '<remove>', 'file_id', <count>, <max files>, <max size>, …`.
    let (upload, numbers) = html
        .split("il.UI.Input.File.init(")
        .skip(1)
        .find_map(|call| {
            let args: Vec<&str> = call.splitn(8, ',').map(str::trim).collect();
            let url = base
                .join(&args.get(1)?.trim_matches('\'').replace("&amp;", "&"))
                .ok()?;
            for_here(&url, "ilObjFileUploadHandlerGUI", "upload").then_some((url, args))
        })?;
    let number = |i: usize| numbers.get(i)?.parse().ok();
    Some(FilesForm {
        upload,
        finish,
        fields,
        max_size: number(6),
        max_files: number(5).map(|n: u64| n as usize),
    })
}

/// The "Add New Item" link for a new `kind` in container `ref_id`, if the page offers it.
pub fn create_link(html: &str, page_url: &Url, ref_id: u64, kind: &Kind) -> Option<Url> {
    let base = link_base(page_url);
    let own = ref_id.to_string();
    Html::parse_document(html)
        .select(&sel(r#"a[href*="cmd=create"]"#))
        .filter_map(|a| base.join(a.attr("href")?).ok())
        .find(|url| {
            url.origin() == base.origin()
                && param(url, "cmd").as_deref() == Some("create")
                && param(url, "ref_id").as_deref() == Some(own.as_str())
                && param(url, "new_type").as_deref() == Some(kind.code())
        })
}

/// The form ILIAS shows for a new item: where it posts and its title and description fields.
#[derive(Debug, PartialEq, Eq)]
pub struct CreateForm {
    pub action: Url,
    pub title: String,
    pub description: String,
}

/// The form for a new `kind` (made by `class`, e.g. `ilObjFolderGUI`) in container `ref_id`.
pub fn create_form(
    html: &str,
    page_url: &Url,
    ref_id: u64,
    kind: &Kind,
    class: &str,
) -> Option<CreateForm> {
    let base = link_base(page_url);
    let own = ref_id.to_string();
    let doc = Html::parse_document(html);
    let (form, action) = doc.select(&sel("form[action]")).find_map(|form| {
        let url = base.join(form.attr("action")?).ok()?;
        let expected = url.origin() == base.origin()
            && param(&url, "cmdClass").is_some_and(|c| c.eq_ignore_ascii_case(class))
            && param(&url, "fallbackCmd").as_deref() == Some("save")
            && param(&url, "ref_id").as_deref() == Some(own.as_str())
            && param(&url, "new_type").as_deref() == Some(kind.code());
        expected.then_some((form, url))
    })?;
    let name = |css: &str| {
        form.select(&sel(css))
            .next()
            .and_then(|input| input.attr("name"))
            .map(String::from)
    };
    Some(CreateForm {
        action,
        title: name(r#"input[type="text"]"#)?,
        description: name("textarea")?,
    })
}
