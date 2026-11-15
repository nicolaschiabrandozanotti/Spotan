use ratatui::style::{Color, Style};
use std::{fs, path::Path, sync::Mutex, time::SystemTime};

type Cache = Mutex<Option<((SystemTime, u64), Theme)>>;
static CACHE: Cache = Mutex::new(None);

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
    pub fn read() -> Self { Self::read_cached(&crate::config_home().join("ncspot/config.toml"), &CACHE) }
    // Re-parses only when mtime/len changed; a missing file is never cached.
    fn read_cached(path: &Path, cache: &Cache) -> Self {
        let stamp = fs::metadata(path).and_then(|m| Ok((m.modified()?, m.len()))).ok();
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(stamp), Some((old, theme))) = (stamp, cache.as_ref()) {
            if *old == stamp { return theme.clone(); }
        }
        let theme = Self::parse(path);
        *cache = stamp.map(|s| (s, theme.clone()));
        theme
    }
    fn parse(path: &Path) -> Self {
        let mut theme = Self::default();
        let data = fs::read_to_string(path).unwrap_or_default();
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
    #[test]
    fn read_cached_reparses_only_on_change() {
        let path = std::env::temp_dir().join(format!("spoti-theme-{}.toml", std::process::id()));
        let cache: Cache = Mutex::new(None);
        fs::write(&path, "[theme]\nbackground = \"red\"\n").unwrap();
        assert_eq!(Theme::read_cached(&path, &cache).background, Color::Red);
        // Same len, forged cached theme: a hit must return it without re-reading.
        let m = fs::metadata(&path).unwrap();
        *cache.lock().unwrap() = Some(((m.modified().unwrap(), m.len()), Theme { background: Color::Blue, ..Theme::default() }));
        assert_eq!(Theme::read_cached(&path, &cache).background, Color::Blue);
        fs::write(&path, "[theme]\nbackground = \"green\"\n").unwrap();
        assert_eq!(Theme::read_cached(&path, &cache).background, Color::Green);
        fs::remove_file(&path).unwrap();
    }
}
