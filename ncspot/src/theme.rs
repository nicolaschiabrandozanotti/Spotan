#![allow(clippy::enum_glob_use)]

use cursive::theme::Color::*;
use cursive::theme::PaletteColor::*;
use cursive::theme::*;
use log::warn;

use crate::config::ConfigTheme;

/// Get the given color from the given [ConfigTheme]. The first argument is the [ConfigTheme] to get
/// the color out of. The second argument is the name of the color to get and is an identifier. The
/// third argument is a [Color] that is used as the default when no color can be parsed from the
/// provided [ConfigTheme].
///
/// # Examples
///
/// ```rust
/// load_color!(config_theme, background, TerminalDefault)
/// load_color!(config_theme, primary, TerminalDefault)
/// ```
macro_rules! load_color {
    ( $theme: expr_2021, $member: ident, $default: expr_2021 ) => {
        $theme
            .as_ref()
            .and_then(|t| t.$member.clone())
            .and_then(|c| Color::parse(c.as_ref()))
            .unwrap_or_else(|| {
                warn!(
                    "Failed to parse color in \"{}\", falling back to default",
                    stringify!($member)
                );
                $default
            })
    };
}

fn spoti(r: u8, g: u8, b: u8) -> Color {
    Rgb(r, g, b)
}

/// Create a [cursive::theme::Theme] from `theme_cfg`.
pub fn load(theme_cfg: &Option<ConfigTheme>) -> Theme {
    let mut palette = Palette::default();
    let borders = BorderStyle::Simple;

    palette[Background] = load_color!(theme_cfg, background, spoti(0x18, 0x18, 0x1F));
    palette[View] = load_color!(theme_cfg, background, spoti(0x18, 0x18, 0x1F));
    palette[Primary] = load_color!(theme_cfg, primary, spoti(0xE8, 0xDF, 0xDF));
    palette[Secondary] = load_color!(theme_cfg, secondary, spoti(0xB4, 0xA3, 0xA8));
    palette[TitlePrimary] = load_color!(theme_cfg, title, spoti(0xD2, 0xB4, 0xBE));
    palette[HighlightText] = load_color!(theme_cfg, highlight, spoti(0xF4, 0xE9, 0xE9));
    palette[Highlight] = load_color!(theme_cfg, highlight_bg, spoti(0x74, 0x51, 0x51));
    palette[HighlightInactive] = load_color!(theme_cfg, highlight_inactive_bg, spoti(0x48, 0x36, 0x3B));
    palette.set_color("playing", load_color!(theme_cfg, playing, spoti(0xDA, 0xB4, 0xB4)));
    palette.set_color(
        "playing_selected",
        load_color!(theme_cfg, playing_selected, spoti(0xE8, 0xDF, 0xDF)),
    );
    palette.set_color(
        "playing_bg",
        load_color!(theme_cfg, playing_bg, spoti(0x1F, 0x18, 0x18)),
    );
    palette.set_color("error", load_color!(theme_cfg, error, spoti(0xF4, 0xE9, 0xE9)));
    palette.set_color("error_bg", load_color!(theme_cfg, error_bg, spoti(0x74, 0x42, 0x4C)));
    palette.set_color(
        "statusbar_progress",
        load_color!(theme_cfg, statusbar_progress, spoti(0xDA, 0xB4, 0xB4)),
    );
    palette.set_color(
        "statusbar_progress_bg",
        load_color!(theme_cfg, statusbar_progress_bg, spoti(0x48, 0x36, 0x3B)),
    );
    palette.set_color("statusbar", load_color!(theme_cfg, statusbar, spoti(0xE8, 0xDF, 0xDF)));
    palette.set_color(
        "statusbar_bg",
        load_color!(theme_cfg, statusbar_bg, spoti(0x74, 0x51, 0x51)),
    );
    palette.set_color("cmdline", load_color!(theme_cfg, cmdline, spoti(0xE8, 0xDF, 0xDF)));
    palette.set_color(
        "cmdline_bg",
        load_color!(theme_cfg, cmdline_bg, spoti(0x1F, 0x18, 0x18)),
    );
    palette.set_color(
        "search_match",
        load_color!(theme_cfg, search_match, spoti(0xE6, 0xBD, 0xCB)),
    );

    Theme {
        shadow: false,
        palette,
        borders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_defaults_are_spoti_palette() {
        let theme = load(&None);
        assert_eq!(theme.palette[Background], Rgb(0x18, 0x18, 0x1F));
        assert_eq!(theme.palette[Highlight], Rgb(0x74, 0x51, 0x51));
    }
}
