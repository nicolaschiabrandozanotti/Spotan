//! UI types shared with the ncspot native adapter without a second dependency version.
pub use crossterm::event::{
    KeyCode, KeyEvent as CrossKeyEvent, KeyModifiers, MouseButton as CrossMouseButton,
    MouseEvent as CrossMouseEvent, MouseEventKind,
};
pub use ratatui::{
    backend::TestBackend,
    layout::Rect,
    style::{Color as CrossColor, Modifier},
    Terminal,
};

/// Paint cells for another UI without a terminal diff or a third full-sized grid.
pub struct CellCanvas {
    terminal: Terminal<TestBackend>,
}

impl Default for CellCanvas {
    fn default() -> Self {
        Self { terminal: Terminal::new(TestBackend::new(1, 1)).expect("cell canvas") }
    }
}

impl CellCanvas {
    pub fn draw(&mut self, app: &mut crate::lyrics::App, width: u16, height: u16) {
        let area = Rect::new(0, 0, width, height);
        if self.terminal.current_buffer_mut().area != area {
            self.terminal.resize(area).expect("resize cell canvas");
        }
        self.terminal.current_buffer_mut().reset();
        app.draw(&mut self.terminal.get_frame());
    }

    pub fn buffer(&mut self) -> &ratatui::buffer::Buffer {
        self.terminal.current_buffer_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lyrics::App, theme::Theme};

    fn app(text: &str) -> App {
        let mut app = App::new();
        app.theme = Theme::default();
        app.background.enabled = false;
        app.disconnected = false;
        app.message.clear();
        app.rows = vec![text.into()];
        app
    }

    #[test]
    fn canvas_matches_terminal_after_text_changes_and_resizes() {
        let mut canvas = CellCanvas::default();
        for (width, height, text) in [(80,32,"Mucho texto 漢字 e\u{301} ♪"), (80,32,"Corto"), (24,18,"Otra línea muy larga para envolver"), (132,40,"♪"), (1,1,"texto")] {
            let mut direct = app(text);
            canvas.draw(&mut direct, width, height);
            let mut terminal = Terminal::new(TestBackend::new(width,height)).unwrap();
            let mut reference = app(text);
            terminal.draw(|frame| reference.draw(frame)).unwrap();
            let direct = canvas.buffer();
            let reference = terminal.backend().buffer();
            for y in 0..height {
                let mut x = 0;
                while x < width {
                    assert_eq!(direct[(x,y)], reference[(x,y)], "visible cell {x},{y}");
                    // The terminal diff omits cells covered by a wide glyph;
                    // the native adapter also advances by that glyph's width.
                    x += unicode_width::UnicodeWidthStr::width(direct[(x,y)].symbol()).max(1) as u16;
                }
            }
        }
    }
}
