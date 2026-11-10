//! Colours and UI glyphs, in one place. The dark palette is a muted "One Dark"-like set on the
//! terminal's own background, the light one is its daytime sibling, and hub is orange on black.
//! Any of them can be adjusted colour by colour with `colors` in `config.json`. The header uses the ILIAS navy.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use ratatui::style::{Color, Modifier, Style};

use crate::icons::nerd_font;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The page behind everything; `Reset` is the terminal's own.
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub bar_bg: Color,
    pub header_bg: Color,
    pub header_fg: Color,
    pub accent: Color,
    pub blue: Color,
    pub cyan: Color,
    pub green: Color,
    pub yellow: Color,
    pub orange: Color,
    pub red: Color,
    pub purple: Color,
    pub magenta: Color,
    pub gray: Color,
}

const BLUE: Color = Color::Rgb(97, 175, 239);

const DARK: Palette = Palette {
    bg: Color::Reset,
    fg: Color::Rgb(220, 223, 228),
    muted: Color::Rgb(127, 132, 142),
    border: Color::Rgb(62, 68, 81),
    selected_bg: Color::Rgb(44, 49, 60),
    bar_bg: Color::Rgb(33, 37, 43),
    // ILIAS' header navy.
    header_bg: Color::Rgb(46, 64, 94),
    header_fg: Color::White,
    accent: BLUE,
    blue: BLUE,
    cyan: Color::Rgb(86, 182, 194),
    green: Color::Rgb(152, 195, 121),
    yellow: Color::Rgb(229, 192, 123),
    orange: Color::Rgb(209, 154, 102),
    red: Color::Rgb(224, 108, 117),
    purple: Color::Rgb(198, 120, 221),
    magenta: Color::Rgb(213, 129, 172),
    gray: Color::Rgb(171, 178, 191),
};

const LIGHT_BLUE: Color = Color::Rgb(44, 100, 210);

const LIGHT: Palette = Palette {
    bg: Color::Rgb(250, 250, 250),
    fg: Color::Rgb(56, 58, 66),
    muted: Color::Rgb(110, 115, 125),
    border: Color::Rgb(200, 204, 212),
    selected_bg: Color::Rgb(226, 231, 240),
    bar_bg: Color::Rgb(238, 240, 244),
    header_bg: DARK.header_bg,
    header_fg: Color::White,
    accent: LIGHT_BLUE,
    blue: LIGHT_BLUE,
    cyan: Color::Rgb(1, 120, 170),
    green: Color::Rgb(60, 130, 60),
    yellow: Color::Rgb(160, 110, 0),
    orange: Color::Rgb(186, 98, 0),
    red: Color::Rgb(202, 18, 67),
    purple: Color::Rgb(166, 38, 164),
    magenta: Color::Rgb(190, 50, 130),
    gray: Color::Rgb(100, 105, 115),
};

const HUB_ORANGE: Color = Color::Rgb(255, 144, 0);

const HUB: Palette = Palette {
    bg: Color::Rgb(0, 0, 0),
    fg: Color::Rgb(255, 255, 255),
    muted: Color::Rgb(150, 150, 150),
    border: Color::Rgb(60, 60, 60),
    selected_bg: Color::Rgb(45, 45, 45),
    bar_bg: Color::Rgb(27, 27, 27),
    header_bg: Color::Rgb(27, 27, 27),
    header_fg: Color::Rgb(255, 255, 255),
    accent: HUB_ORANGE,
    blue: HUB_ORANGE,
    cyan: HUB_ORANGE,
    green: Color::Rgb(120, 200, 90),
    yellow: Color::Rgb(255, 190, 70),
    orange: HUB_ORANGE,
    red: Color::Rgb(240, 80, 70),
    purple: Color::Rgb(200, 200, 200),
    magenta: Color::Rgb(255, 170, 90),
    gray: Color::Rgb(190, 190, 190),
};

/// The themes to choose from in the settings; the first is the default.
pub const THEMES: [&str; 3] = ["dark", "light", "hub"];

impl Palette {
    /// The named theme with `custom` colours on top (`"accent": "#ff8800"`). Also returns the
    /// entries it can't use: unknown names and values that aren't colours.
    pub fn build(theme: Option<&str>, custom: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut palette = match theme {
            Some("light") => LIGHT,
            Some("hub") => HUB,
            _ => DARK,
        };
        let mut ignored = Vec::new();
        for (name, value) in custom {
            match (palette.slot(name), value.parse()) {
                (Some(slot), Ok(color)) => *slot = color,
                _ => ignored.push(name.clone()),
            }
        }
        (palette, ignored)
    }

    fn slot(&mut self, name: &str) -> Option<&mut Color> {
        Some(match name {
            "bg" => &mut self.bg,
            "fg" => &mut self.fg,
            "muted" => &mut self.muted,
            "border" => &mut self.border,
            "selected_bg" => &mut self.selected_bg,
            "bar_bg" => &mut self.bar_bg,
            "header_bg" => &mut self.header_bg,
            "header_fg" => &mut self.header_fg,
            "accent" => &mut self.accent,
            "blue" => &mut self.blue,
            "cyan" => &mut self.cyan,
            "green" => &mut self.green,
            "yellow" => &mut self.yellow,
            "orange" => &mut self.orange,
            "red" => &mut self.red,
            "purple" => &mut self.purple,
            "magenta" => &mut self.magenta,
            "gray" => &mut self.gray,
            _ => return None,
        })
    }
}

static CURRENT: RwLock<Palette> = RwLock::new(DARK);

/// The colours in use.
pub fn colors() -> Palette {
    *CURRENT.read().unwrap_or_else(PoisonError::into_inner)
}

/// Switches the colours (at the start and from the settings).
pub fn apply(palette: Palette) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = palette;
}

pub fn muted() -> Style {
    Style::new().fg(colors().muted)
}

pub fn accent_bold() -> Style {
    Style::new()
        .fg(colors().accent)
        .add_modifier(Modifier::BOLD)
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

pub fn sync() -> &'static str {
    glyph("\u{f021}", "⟳") // refresh
}

pub fn gear() -> &'static str {
    glyph("\u{f013}", "⚙") // cog
}

pub fn keyboard() -> &'static str {
    glyph("\u{f11c}", "⌨")
}

// The sync review's sections and the dialogs (Font Awesome 4 glyphs, as above).
pub fn upload() -> &'static str {
    glyph("\u{f0ee}", "↑") // cloud-upload
}

pub fn edit() -> &'static str {
    glyph("\u{f040}", "✎") // pencil
}

pub fn rename() -> &'static str {
    glyph("\u{f044}", "✎") // pencil-square-o
}

pub fn trash() -> &'static str {
    glyph("\u{f1f8}", "✗") // trash
}

pub fn folder() -> &'static str {
    glyph("\u{f07b}", "+") // folder
}

pub fn unlink() -> &'static str {
    glyph("\u{f127}", "⊘") // chain-broken
}

pub fn lock() -> &'static str {
    glyph("\u{f023}", "⊘") // lock
}

pub fn question() -> &'static str {
    glyph("\u{f128}", "?")
}

pub fn ban() -> &'static str {
    glyph("\u{f05e}", "⊘")
}

pub fn failed() -> &'static str {
    glyph("\u{f057}", "✗") // times-circle
}

pub fn copy() -> &'static str {
    glyph("\u{f0c5}", "⧉")
}

pub fn send() -> &'static str {
    glyph("\u{f1d8}", "➤") // paper-plane
}

pub fn external() -> &'static str {
    glyph("\u{f08e}", "↗") // external-link
}

pub fn checked() -> &'static str {
    glyph("\u{f046}", "[x]") // check-square-o
}

pub fn unchecked() -> &'static str {
    glyph("\u{f096}", "[ ]") // square-o
}

pub fn bullet() -> &'static str {
    glyph("\u{f0da}", "›") // caret-right
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn custom_colours_go_on_top_of_the_theme() {
        assert_eq!(Palette::build(None, &BTreeMap::new()).0, DARK);
        assert_eq!(Palette::build(Some("light"), &BTreeMap::new()).0, LIGHT);
        assert_eq!(Palette::build(Some("hub"), &BTreeMap::new()).0, HUB);

        let (palette, ignored) = Palette::build(
            Some("light"),
            &custom(&[("accent", "#ff8800"), ("bg", "reset"), ("red", "blue")]),
        );
        assert_eq!(palette.accent, Color::Rgb(255, 136, 0));
        assert_eq!(palette.bg, Color::Reset);
        assert_eq!(palette.red, Color::Blue);
        assert_eq!(palette.fg, LIGHT.fg, "the rest stays");
        assert!(ignored.is_empty());
    }

    #[test]
    fn reports_what_it_cant_use() {
        let (palette, ignored) = Palette::build(
            None,
            &custom(&[("accent", "#nope"), ("sparkle", "#ff8800"), ("fg", "white")]),
        );
        assert_eq!(ignored, ["accent", "sparkle"]);
        assert_eq!(palette.accent, DARK.accent);
        assert_eq!(palette.fg, Color::White);
    }
}
