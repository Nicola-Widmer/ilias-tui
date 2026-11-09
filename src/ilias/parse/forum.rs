//! Forum threads: one post per `.ilFrmPostRow`.

use scraper::ElementRef;
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
    Post {
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
