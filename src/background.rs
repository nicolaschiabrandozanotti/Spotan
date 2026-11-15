//! A small, deterministic braille rasterizer for a slow animated backdrop.
//! Work scales with terminal width, rather than evaluating every subpixel.
use crate::theme::Theme;
use ratatui::{layout::Rect, style::Color, Frame};
use crate::braille::Raster;

pub struct Background {
    pub enabled: bool,
    raster: Raster,
}

impl Default for Background {
    fn default() -> Self {
        Self { enabled: true, raster: Raster::default() }
    }
}

impl Background {
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &Theme, seconds: f64) {
        if !self.enabled || area.is_empty() { return; }
        let width=area.width as usize;
        let height=area.height as usize;
        let dots=self.raster.render(width,height,seconds);

        let violet = blend(theme.background, theme.title, 0.32);
        let green = blend(theme.background, theme.playing, 0.27);
        let buffer = frame.buffer_mut();
        for y in 0..height {
            for x in 0..width {
                let bits = dots[y * width + x];
                if bits == 0 { continue; }
                let character = char::from_u32(0x2800 + bits as u32).unwrap();
                buffer[(area.x + x as u16, area.y + y as u16)]
                    .set_char(character)
                    .set_fg(if (x + y * 2) % 19 < 12 { violet } else { green })
                    .set_bg(theme.background);
            }
        }
    }
}

fn blend(background: Color, accent: Color, amount: f32) -> Color {
    let rgb = |color, fallback| match color { Color::Rgb(r,g,b) => (r,g,b), _ => fallback };
    let (r,g,b) = rgb(background, (30,30,46));
    let (ar,ag,ab) = rgb(accent, (203,166,247));
    let channel = |base: u8, target: u8| (base as f32 + (target as f32 - base as f32) * amount).round() as u8;
    Color::Rgb(channel(r,ar),channel(g,ag),channel(b,ab))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, style::Style, widgets::Block, Terminal};

    fn raster(width: u16, height: u16, seconds: f64, enabled: bool) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width,height)).unwrap();
        let mut background = Background { enabled, ..Default::default() };
        terminal.draw(|frame| {
            frame.render_widget(Block::default().style(Style::default().bg(Theme::default().background)),frame.area());
            background.draw(frame,frame.area(),&Theme::default(),seconds);
        }).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn frames_animate_and_only_contain_braille_on_the_original_background() {
        let first = raster(80,25,0.0,true);
        let second = raster(80,25,1.0,true);
        assert_ne!(first,second);
        assert_eq!(first,raster(80,25,0.0,true));
        assert_eq!(first,raster(80,25,0.10,true));
        let mut dots = 0;
        for cell in &first.content {
            assert_eq!(cell.bg,Theme::default().background);
            if cell.symbol() != " " {
                assert!(cell.symbol().chars().all(|c| ('\u{2800}'..='\u{28ff}').contains(&c)));
                dots += 1;
            }
        }
        assert!(dots > 20 && dots < first.content.len() / 2);
    }

    #[test]
    fn toggle_removes_art_and_small_windows_remain_safe() {
        for (width,height) in [(0,0),(1,1),(20,5),(150,45)] {
            let off = raster(width,height,10.0,false);
            assert!(off.content.iter().all(|cell| cell.symbol()==" "));
            let _ = raster(width,height,10.0,true);
        }

    }
}
