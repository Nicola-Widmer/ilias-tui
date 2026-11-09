//! Forum threads: one post per `.ilFrmPostRow`; the forms for a new thread and a reply.

use scraper::{ElementRef, Html};
use url::Url;

use super::*;

pub(super) fn post(row: ElementRef, page_url: &Url) -> Post {
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
    // Each post starts with an anchor named by its number: `<a id="2">`.
    let id = row
        .select(&sel("a[id]"))
        .find_map(|a| a.attr("id")?.parse().ok())
        .unwrap_or_default();
    Post {
        id,
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

/// The first link or button on the page (`href` or `data-action`) that `wanted` accepts.
pub fn forum_link(html: &str, page_url: &Url, wanted: impl Fn(&Url) -> bool) -> Option<Url> {
    let base = link_base(page_url);
    let doc = Html::parse_document(html);
    let (buttons, anchors) = (sel("button[data-action]"), sel("a[href]"));
    let buttons = doc.select(&buttons).filter_map(|b| b.attr("data-action"));
    let anchors = doc.select(&anchors).filter_map(|a| a.attr("href"));
    buttons
        .chain(anchors)
        .filter_map(|link| base.join(link).ok())
        .find(|url| url.origin() == base.origin() && wanted(url))
}

/// ILIAS' form for a forum post (a new thread's first post, or a reply).
#[derive(Debug, PartialEq, Eq)]
pub struct PostForm {
    pub action: Url,
    /// Hidden fields, sent as they are (your name, the form's file hash, …).
    pub hidden: Vec<(String, String)>,
    /// The subject field and what ILIAS put in it (a reply's `Re: …`).
    pub subject: (String, String),
    pub message: String,
    /// The message is HTML (ILIAS' rich text editor is on).
    pub rich: bool,
    /// File inputs (attachments), sent empty as a browser does.
    pub files: Vec<String>,
    /// The button that sends it, `cmd[<command>]`, and its value.
    pub button: (String, String),
}

/// The form on the page that posts to an address `wanted` accepts, sent by `cmd[command]`.
pub fn post_form(
    html: &str,
    page_url: &Url,
    command: &str,
    wanted: impl Fn(&Url) -> bool,
) -> Option<PostForm> {
    let base = link_base(page_url);
    let doc = Html::parse_document(html);
    let button = sel(&format!(
        r#"input[type="submit"][name="cmd[{command}]"], input[type="SUBMIT"][name="cmd[{command}]"]"#
    ));
    let (form, action) = doc.select(&sel("form[action]")).find_map(|form| {
        let url = base.join(form.attr("action")?).ok()?;
        (url.origin() == base.origin() && wanted(&url) && form.select(&button).next().is_some())
            .then_some((form, url))
    })?;
    let submit = form.select(&button).next()?;
    let hidden = form
        .select(&sel(r#"input[type="hidden"][name]"#))
        .filter_map(|i| {
            Some((
                i.attr("name")?.to_string(),
                i.attr("value").unwrap_or_default().to_string(),
            ))
        })
        .collect();
    let subject = form.select(&sel(r#"input[name="subject"]"#)).next()?;
    let message = form.select(&sel(r#"textarea[name="message"]"#)).next()?;
    Some(PostForm {
        action,
        hidden,
        subject: (
            "subject".into(),
            subject.attr("value").unwrap_or_default().to_string(),
        ),
        message: "message".into(),
        rich: message.value().classes().any(|c| c == "RTEditor"),
        files: form
            .select(&sel(r#"input[type="file"][name]"#))
            .filter_map(|i| i.attr("name").map(String::from))
            .collect(),
        button: (
            submit.attr("name")?.to_string(),
            submit.attr("value").unwrap_or_default().to_string(),
        ),
    })
}

/// A message as ILIAS' rich text editor would send it: paragraphs, line breaks, escaped.
pub fn message_html(text: &str) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            format!(
                "<p>{}</p>",
                p.lines().map(escape).collect::<Vec<_>>().join("<br />")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
