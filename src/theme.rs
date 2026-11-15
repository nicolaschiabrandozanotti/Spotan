use ratatui::style::{Color, Style};
use std::fs;

#[derive(Clone, Debug)]
pub struct Theme {
    pub background: Color,
    pub primary: Color,
    pub secondary: Color,
    pub title: Color,
    pub playing: Color,
    pub highlight_bg: Color,
    pub statusbar: Color,
    pub statusbar_bg: Color,
    pub progress: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self { background: Color::Rgb(30,30,46), primary: Color::Rgb(205,214,244),
            secondary: Color::Rgb(166,173,200), title: Color::Rgb(203,166,247),
            playing: Color::Rgb(166,227,161), highlight_bg: Color::Rgb(69,71,90),
            statusbar: Color::Rgb(30,30,46), statusbar_bg: Color::Rgb(203,166,247),
            progress: Color::Rgb(166,227,161) }
    }
}

fn color(value: &str) -> Option<Color> {
    if let Some(hex) = value.strip_prefix('#').filter(|v| v.len() == 6) {
        let number = u32::from_str_radix(hex, 16).ok()?;
        return Some(Color::Rgb((number >> 16) as u8, (number >> 8) as u8, number as u8));
    }
    Some(match value {
        "default" => Color::Reset, "black" => Color::Black, "red" => Color::Red,
        "green" => Color::Green, "yellow" => Color::Yellow, "blue" => Color::Blue,
        "magenta" => Color::Magenta, "cyan" => Color::Cyan, "white" => Color::Gray,
        "light black" => Color::DarkGray, "light red" => Color::LightRed,
        "light green" => Color::LightGreen, "light yellow" => Color::LightYellow,
        "light blue" => Color::LightBlue, "light magenta" => Color::LightMagenta,
        "light cyan" => Color::LightCyan, "light white" => Color::White,
        _ => return None,
    })
}

impl Theme {
    pub fn read() -> Self {
        let mut theme = Self::default();
        let data = fs::read_to_string(crate::config_home().join("ncspot/config.toml")).unwrap_or_default();
        if let Ok(config) = data.parse::<toml::Value>() {
            for (key, target) in [
                ("background", &mut theme.background), ("primary", &mut theme.primary),
                ("secondary", &mut theme.secondary), ("title", &mut theme.title),
                ("playing_selected", &mut theme.playing), ("highlight_bg", &mut theme.highlight_bg),
                ("statusbar", &mut theme.statusbar), ("statusbar_bg", &mut theme.statusbar_bg),
                ("statusbar_progress", &mut theme.progress),
            ] {
                if let Some(value) = config.get("theme").and_then(|v| v.get(key)).and_then(|v| v.as_str()).and_then(color) {
                    *target = value;
                }
            }
        }
        theme
    }
    pub fn base(&self) -> Style { Style::default().fg(self.primary).bg(self.background) }
    pub fn selected(&self) -> Style { self.base().fg(self.playing).bg(self.highlight_bg) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ncspot_colors() {
        assert_eq!(color("#1e1e2e"), Some(Color::Rgb(30,30,46)));
        assert_eq!(color("light green"), Some(Color::LightGreen));
        assert_eq!(color("#broken"), None);
    }
}
