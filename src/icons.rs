//! Icons per object type and file extension.
//!
//! Uses Font Awesome glyphs (`nf-fa-*`), which every [Nerd Font](https://www.nerdfonts.com)
//! contains. Set `ILIAS_NERD_FONT=0` for plain Unicode symbols if your terminal font has no
//! Nerd Font glyphs.

use std::sync::OnceLock;

use ratatui::style::Color;

use crate::theme::*;

use crate::ilias::Item;

pub fn nerd_font() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("ILIAS_NERD_FONT").as_deref(),
            Ok("0" | "false" | "no" | "off")
        )
    })
}

/// Glyph and colour for an item.
pub fn icon(item: &Item) -> (&'static str, Color) {
    let (nerd, plain, color) = match item.kind.as_str() {
        "crs" => ("\u{f19d}", "◆", CYAN),          // graduation-cap
        "grp" => ("\u{f0c0}", "◆", BLUE),          // users
        "cat" | "root" => ("\u{f0e8}", "▸", BLUE), // sitemap
        "fold" => ("\u{f07b}", "▸", YELLOW),       // folder
        "sess" => ("\u{f073}", "◷", PURPLE),       // calendar
        "exc" => ("\u{f044}", "✎", ORANGE),        // edit
        "tst" => ("\u{f046}", "✓", ORANGE),        // check-square-o
        "svy" => ("\u{f080}", "▤", ORANGE),        // bar-chart
        "iass" => ("\u{f0ae}", "✓", ORANGE),       // tasks
        "poll" => ("\u{f080}", "▤", ORANGE),
        "chtr" => ("\u{f086}", "✉", GREEN),
        "frm" => ("\u{f086}", "✉", GREEN), // comments
        "thr" => ("\u{f0e5}", "›", GREEN), // comment-o
        "webr" => ("\u{f0c1}", "↗", BLUE), // link
        "wiki" | "lm" | "htlm" | "sahs" | "copa" => ("\u{f02d}", "▤", MAGENTA), // book
        "blog" => ("\u{f09e}", "▤", MAGENTA), // rss
        "mcst" | "xvid" | "xoct" => ("\u{f008}", "▶", MAGENTA), // film
        "book" => ("\u{f274}", "◷", PURPLE), // calendar-check-o
        "dcl" => ("\u{f1c0}", "▤", BLUE),  // database
        "file" => return file_icon(item.extension().as_deref().unwrap_or("")),
        _ => ("\u{f1b2}", "•", GRAY), // cube
    };
    (if nerd_font() { nerd } else { plain }, color)
}

fn file_icon(ext: &str) -> (&'static str, Color) {
    let (nerd, color) = match ext {
        "pdf" => ("\u{f1c1}", RED),
        "doc" | "docx" | "odt" | "rtf" | "pages" => ("\u{f1c2}", BLUE),
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "numbers" => ("\u{f1c3}", GREEN),
        "ppt" | "pptx" | "odp" | "key" => ("\u{f1c4}", ORANGE),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "heic" | "bmp" | "tif" | "tiff" => {
            ("\u{f1c5}", PURPLE)
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" => ("\u{f1c6}", YELLOW),
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac" => ("\u{f1c7}", MAGENTA),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => ("\u{f1c8}", MAGENTA),
        "py" | "java" | "rs" | "js" | "ts" | "c" | "h" | "cpp" | "hpp" | "cs" | "go" | "kt"
        | "swift" | "rb" | "php" | "html" | "css" | "json" | "xml" | "yml" | "yaml" | "sql"
        | "sh" | "ipynb" | "r" | "m" | "scala" => ("\u{f1c9}", GREEN),
        "txt" | "md" | "log" | "tex" => ("\u{f0f6}", FG),
        _ => ("\u{f15b}", GRAY),
    };
    (if nerd_font() { nerd } else { "≡" }, color)
}
