//! Icons per object type and file extension.
//!
//! Uses Font Awesome glyphs (`nf-fa-*`), which every [Nerd Font](https://www.nerdfonts.com)
//! contains. Turn them off in the settings (`c`) or with `ILIAS_NERD_FONT=0` for plain Unicode
//! symbols if your terminal font has no Nerd Font glyphs.

use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::Color;

use crate::theme::colors;

use crate::ilias::Item;

static NERD_FONT: AtomicBool = AtomicBool::new(true);

pub fn nerd_font() -> bool {
    NERD_FONT.load(Ordering::Relaxed)
}

/// Switches between Nerd Font glyphs and plain symbols (from the settings).
pub fn set_nerd_font(on: bool) {
    NERD_FONT.store(on, Ordering::Relaxed);
}

/// Glyph and colour for an item.
pub fn icon(item: &Item) -> (&'static str, Color) {
    let c = colors();
    let (nerd, plain, color) = match item.kind.as_str() {
        "crs" => ("\u{f19d}", "◆", c.cyan),           // graduation-cap
        "grp" => ("\u{f0c0}", "◆", c.blue),           // users
        "cat" | "root" => ("\u{f0e8}", "▸", c.blue),  // sitemap
        "fold" => ("\u{f07b}", "▸", c.yellow),        // folder
        "sess" => ("\u{f073}", "◷", c.purple),        // calendar
        "exc" | "ass" => ("\u{f044}", "✎", c.orange), // edit
        "sub" => ("\u{f093}", "↑", c.orange),         // upload
        "tst" => ("\u{f046}", "✓", c.orange),         // check-square-o
        "svy" => ("\u{f080}", "▤", c.orange),         // bar-chart
        "iass" => ("\u{f0ae}", "✓", c.orange),        // tasks
        "poll" => ("\u{f080}", "▤", c.orange),
        "chtr" => ("\u{f086}", "✉", c.green),
        "frm" => ("\u{f086}", "✉", c.green),          // comments
        "thr" => ("\u{f0e5}", "›", c.green),          // comment-o
        "webr" | "link" => ("\u{f0c1}", "↗", c.blue), // link
        "wiki" | "lm" | "htlm" | "sahs" | "copa" => ("\u{f02d}", "▤", c.magenta), // book
        "blog" => ("\u{f09e}", "▤", c.magenta),       // rss
        "mcst" | "xvid" | "xoct" => ("\u{f008}", "▶", c.magenta), // film
        "book" => ("\u{f274}", "◷", c.purple),        // calendar-check-o
        "dcl" => ("\u{f1c0}", "▤", c.blue),           // database
        "file" => return file_icon(item.extension().as_deref().unwrap_or("")),
        _ => ("\u{f1b2}", "•", c.gray), // cube
    };
    (if nerd_font() { nerd } else { plain }, color)
}

fn file_icon(ext: &str) -> (&'static str, Color) {
    let c = colors();
    let (nerd, color) = match ext {
        "pdf" => ("\u{f1c1}", c.red),
        "doc" | "docx" | "odt" | "rtf" | "pages" => ("\u{f1c2}", c.blue),
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "numbers" => ("\u{f1c3}", c.green),
        "ppt" | "pptx" | "odp" | "key" => ("\u{f1c4}", c.orange),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "heic" | "bmp" | "tif" | "tiff" => {
            ("\u{f1c5}", c.purple)
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" => ("\u{f1c6}", c.yellow),
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac" => ("\u{f1c7}", c.magenta),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => ("\u{f1c8}", c.magenta),
        "py" | "java" | "rs" | "js" | "ts" | "c" | "h" | "cpp" | "hpp" | "cs" | "go" | "kt"
        | "swift" | "rb" | "php" | "html" | "css" | "json" | "xml" | "yml" | "yaml" | "sql"
        | "sh" | "ipynb" | "r" | "m" | "scala" => ("\u{f1c9}", c.green),
        "txt" | "md" | "log" | "tex" => ("\u{f0f6}", c.fg),
        _ => ("\u{f15b}", c.gray),
    };
    (if nerd_font() { nerd } else { "≡" }, color)
}
