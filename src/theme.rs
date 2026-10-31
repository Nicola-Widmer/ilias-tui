//! Colours and UI glyphs, in one place. The palette is a muted "One Dark"-like set that works
//! on dark terminals; the header uses the ILIAS navy.

use ratatui::style::{Color, Modifier, Style};

use crate::icons::nerd_font;

pub const FG: Color = Color::Rgb(220, 223, 228);
pub const MUTED: Color = Color::Rgb(127, 132, 142);
pub const BORDER: Color = Color::Rgb(62, 68, 81);
pub const SELECTED_BG: Color = Color::Rgb(44, 49, 60);
pub const BAR_BG: Color = Color::Rgb(33, 37, 43);
/// ILIAS' header navy.
pub const HEADER_BG: Color = Color::Rgb(46, 64, 94);

pub const BLUE: Color = Color::Rgb(97, 175, 239);
pub const CYAN: Color = Color::Rgb(86, 182, 194);
pub const GREEN: Color = Color::Rgb(152, 195, 121);
pub const YELLOW: Color = Color::Rgb(229, 192, 123);
pub const ORANGE: Color = Color::Rgb(209, 154, 102);
pub const RED: Color = Color::Rgb(224, 108, 117);
pub const PURPLE: Color = Color::Rgb(198, 120, 221);
pub const MAGENTA: Color = Color::Rgb(213, 129, 172);
pub const GRAY: Color = Color::Rgb(171, 178, 191);

pub const ACCENT: Color = BLUE;

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn accent_bold() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

/// A UI glyph: the Nerd Font one, or a plain Unicode fallback.
pub fn glyph(nerd: &'static str, plain: &'static str) -> &'static str {
    if nerd_font() { nerd } else { plain }
}

pub fn logo() -> &'static str {
    glyph("\u{f19d}", "◆") // graduation-cap
}

pub fn chevron() -> &'static str {
    glyph("\u{f054}", "›") // chevron-right
}

pub fn search() -> &'static str {
    glyph("\u{f002}", "/") // search
}

pub fn check() -> &'static str {
    glyph("\u{f00c}", "✓")
}

pub fn warning() -> &'static str {
    glyph("\u{f071}", "!") // exclamation-triangle
}

pub fn empty_folder() -> &'static str {
    glyph("\u{f115}", "∅") // folder-open-o
}

pub fn keyboard() -> &'static str {
    glyph("\u{f11c}", "⌨")
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
