//! Shared lyrics model and cell rendering for the terminal and native window.
use crate::{
    self as api,
    background::Background,
    controls::{self, Action, Button},
    theme::Theme,
    Record, State,
};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
struct WrappedLyrics {
    width: usize,
    source: Vec<String>,
    lines: Vec<(usize, String)>,
    starts: Vec<usize>,
}

impl WrappedLyrics {
    fn update(&mut self, rows: &[String], width: usize) {
        if self.width == width && self.source == rows {
            return;
        }
        self.width = width;
        self.source.resize_with(rows.len(), String::new);
        for (cached, row) in self.source.iter_mut().zip(rows) {
            cached.clone_from(row);
        }
        self.lines.clear();
        self.starts.clear();
        for (index, text) in rows.iter().enumerate() {
            self.starts.push(self.lines.len());
            self.lines.extend(api::wrap_line(text, width).into_iter().map(|part| (index, part)));
            self.lines.push((index, String::new()));
        }
    }
}

pub struct App {
    pub state: State,
    pub disconnected: bool,
    pub active: String,
    pub rows: Vec<String>,
    pub times: Vec<f64>,
    pub message: String,
    pub follow: bool,
    pub scroll: usize,
    pub offset: f64,
    pub theme: Theme,
    pub background: Background,
    pub buttons: Vec<Button>,
    pub progress: Rect,
    pub hover: Option<(u16, u16)>,
    pub seek_drag: Option<u16>,
    wrapped: WrappedLyrics,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: State::default(),
            disconnected: true,
            active: String::new(),
            rows: vec![],
            times: vec![],
            message: "Conectando con ncspot…".into(),
            follow: true,
            scroll: 0,
            offset: 0.0,
            theme: Theme::read(),
            background: Background::default(),
            buttons: Vec::new(),
            progress: Rect::default(),
            hover: None,
            seek_drag: None,
            wrapped: WrappedLyrics::default(),
        }
    }

    pub fn accept_track(&mut self) {
        // Do not retain the previous song's layout or strings across tracks.
        self.wrapped = WrappedLyrics::default();
        self.rows.clear();
        self.times.clear();
        self.scroll = 0;
        self.follow = true;
        self.offset = 0.0;
        self.message = if self
            .state
            .playable
            .as_ref()
            .is_some_and(|t| t.kind == "Track")
        {
            "Buscando letra en LRCLIB…"
        } else {
            "Poné una canción en ncspot"
        }
        .into();
    }

    pub fn accept_lyrics(&mut self, result: std::result::Result<Option<Record>, String>) {
        match result {
            Ok(record) => {
                let timeline = api::parse_lrc(
                    record
                        .as_ref()
                        .and_then(|r| r.synced_lyrics.as_deref())
                        .unwrap_or(""),
                );
                self.message.clear();
                if timeline.is_empty() {
                    self.rows = api::no_lyrics(record.as_ref())
                        .lines()
                        .map(String::from)
                        .collect();
                    if record.as_ref().is_some_and(|r| r.plain_lyrics.is_some()) {
                        self.message = "LETRA SIN TIEMPOS · ↑/↓ para leer".into();
                    }
                } else {
                    (self.times, self.rows) = timeline.into_iter().unzip();
                }
            }
            Err(error) => {
                self.message = "No pude consultar LRCLIB".into();
                self.rows = vec![error];
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent, page: usize) -> Option<&'static str> {
        match key.code {
            KeyCode::Char(' ') | KeyCode::Char('p') => Some("playpause"),
            KeyCode::Char('n') => Some("next"),
            KeyCode::Char('b') => Some("previous"),
            KeyCode::Right => Some("seek +5s"),
            KeyCode::Left => Some("seek -5s"),
            KeyCode::Char('+') | KeyCode::Char('=') => Some("volup 5"),
            KeyCode::Char('-') => Some("voldown 5"),
            KeyCode::Down | KeyCode::Char('j') => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(1);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(1);
                None
            }
            KeyCode::PageDown => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(page);
                None
            }
            KeyCode::PageUp => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(page);
                None
            }
            KeyCode::Char('f' | 'F') => {
                self.follow = true;
                None
            }
            KeyCode::Char('v' | 'V') => {
                self.background.enabled = !self.background.enabled;
                None
            }
            KeyCode::Char(']') => {
                self.offset += 0.25;
                None
            }
            KeyCode::Char('[') => {
                self.offset -= 0.25;
                None
            }
            _ => None,
        }
    }

    pub fn seek_command(&self, column: u16) -> Option<String> {
        let duration = self.state.playable.as_ref()?.duration;
        if duration == 0 || self.progress.width == 0 {
            return None;
        }
        let distance = column
            .saturating_sub(self.progress.x)
            .min(self.progress.width.saturating_sub(1));
        let fraction = distance as f64 / self.progress.width.saturating_sub(1).max(1) as f64;
        Some(format!(
            "seek {}ms",
            (duration as f64 * fraction).round() as u64
        ))
    }

    pub fn mouse(&mut self, event: MouseEvent) -> MouseResult {
        self.hover = Some((event.column, event.row));
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(3);
            }
            MouseEventKind::ScrollUp => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if inside(self.progress, event.column, event.row) {
                    self.seek_drag = Some(event.column);
                    if let Some(command) = self.seek_command(event.column) {
                        return MouseResult::Command(command);
                    }
                } else if let Some(action) = self
                    .buttons
                    .iter()
                    .find(|button| inside(button.area, event.column, event.row))
                    .map(|button| button.action)
                {
                    match action {
                        Action::Player(command) => return MouseResult::Command(command.to_owned()),
                        Action::Follow => self.follow = true,
                        Action::Background => self.background.enabled = !self.background.enabled,
                        Action::OffsetDown => self.offset -= 0.25,
                        Action::OffsetUp => self.offset += 0.25,
                        Action::ScrollUp => {
                            self.follow = false;
                            self.scroll = self.scroll.saturating_sub(3);
                        }
                        Action::ScrollDown => {
                            self.follow = false;
                            self.scroll = self.scroll.saturating_add(3);
                        }
                        Action::Close => return MouseResult::Close,
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(start) = self.seek_drag.take() {
                    if start != event.column {
                        if let Some(command) = self.seek_command(event.column) {
                            return MouseResult::Command(command);
                        }
                    }
                }
            }
            _ => (),
        }
        MouseResult::None
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let margin = (area.width / 24).clamp(1, 4);
        let text_width = area.width.saturating_sub(2 * margin).saturating_sub(2) as usize;
        self.buttons.clear();
        self.progress = Rect::default();
        let theme = &self.theme;
        // Paint every cell with the exact ncspot background, including empty space.
        frame.render_widget(Block::default().style(theme.base()), area);
        if area.height < 11 || area.width < 20 {
            line(
                frame,
                Rect::new(0, 0, area.width, 1),
                "Ampliá la ventana · q salir",
                theme.base(),
                false,
            );
            return;
        }
        let inner = Rect::new(
            margin,
            0,
            area.width.saturating_sub(2 * margin),
            area.height,
        );
        let track = self.state.playable.as_ref();
        let position = api::elapsed(&self.state, api::now());
        let duration = track.map_or(0.0, |t| t.duration as f64 / 1000.0);
        let playing = self.state.mode.get("Playing").is_some();
        let footer = controls::layout(
            area.width,
            area.height,
            margin,
            playing,
            self.follow,
            self.background.enabled,
        );
        if footer.top < 7 {
            line(
                frame,
                Rect::new(0, 0, area.width, 1),
                "Ampliá la ventana · q salir",
                theme.base(),
                false,
            );
            return;
        }
        self.background.draw(
            frame,
            Rect::new(area.x, 5, area.width, footer.top.saturating_sub(6)),
            theme,
            api::now(),
        );
        line(
            frame,
            row(inner, 0),
            "LETRAS",
            theme.base().fg(theme.title),
            true,
        );
        line(
            frame,
            row(inner, 1),
            &api::clean(track.map_or("Elegí una canción en ncspot", |t| t.title.as_str())),
            theme.base().add_modifier(Modifier::BOLD),
            true,
        );
        line(
            frame,
            row(inner, 2),
            &api::clean(&track.map_or(String::new(), |t| t.artists.join(", "))),
            theme.base().fg(theme.secondary),
            true,
        );
        let status = if self.disconnected {
            "NCSPOT DESCONECTADO · esperando reconexión".into()
        } else if !self.message.is_empty() {
            self.message.clone()
        } else {
            format!(
                "{} · {}",
                if playing { "REPRODUCIENDO" } else { "EN PAUSA" },
                if self.times.is_empty() {
                    "SIN TIEMPOS"
                } else {
                    "SINCRONIZADO"
                }
            )
        };
        line(
            frame,
            row(inner, 3),
            &status,
            theme.base().fg(theme.title),
            true,
        );

        let current = api::active_line(&self.times, position + self.offset);
        let visible = footer.top.saturating_sub(6) as usize;
        self.wrapped.update(&self.rows, text_width);
        let rendered = &self.wrapped.lines;
        let starts = &self.wrapped.starts;
        if self.follow && !self.times.is_empty() {
            let anchor = starts.get(current.unwrap_or(0)).copied().unwrap_or(0);
            self.scroll = anchor.saturating_sub(visible / 2);
        }
        self.scroll = self.scroll.min(rendered.len().saturating_sub(visible));
        for (y, (index, text)) in rendered.iter().skip(self.scroll).take(visible).enumerate() {
            let selected = current == Some(*index) && !text.is_empty();
            let style = if selected {
                theme.selected().add_modifier(Modifier::BOLD)
            } else if current.is_some_and(|current| *index < current) {
                theme.base().fg(theme.secondary)
            } else {
                theme.base()
            };
            if selected {
                line(frame, row(inner, 5 + y as u16), text, style, true);
            } else {
                lyric_line(frame, row(inner, 5 + y as u16), text, style);
            }
        }
        self.progress = footer.progress;
        let bar_width = footer.progress.width as usize;
        let filled = if duration > 0.0 {
            (bar_width as f64 * (position / duration).clamp(0.0, 1.0)).round() as usize
        } else {
            0
        };
        line(
            frame,
            Rect::new(inner.x, footer.top, 6, 1),
            &clock(position),
            theme.base().fg(theme.title),
            false,
        );
        line(
            frame,
            footer.progress,
            &format!("{}{}", "━".repeat(filled), "─".repeat(bar_width - filled)),
            theme.base().fg(theme.title),
            false,
        );
        line(
            frame,
            Rect::new(inner.right().saturating_sub(6), footer.top, 6, 1),
            &clock(duration),
            theme.base().fg(theme.title),
            true,
        );
        for button in &footer.buttons {
            let hovered = self.hover.is_some_and(|(x, y)| inside(button.area, x, y));
            let style = if hovered || button.active {
                theme.selected().add_modifier(Modifier::BOLD)
            } else {
                theme.base().fg(theme.secondary)
            };
            line(frame, button.area, &button.label, style, true);
        }
        let mut label = format!(
            "LRCLIB · {}",
            if self.follow && !self.times.is_empty() {
                "AUTO"
            } else {
                "MANUAL"
            }
        );
        if self.offset.abs() > 0.001 {
            label.push_str(&format!(" · {:+.2}s", self.offset));
        }
        if let Some((x, y)) = self.hover {
            if inside(footer.progress, x, y) {
                label = "Clic o arrastrá para buscar en el tema".into();
            }
        }
        line(
            frame,
            row(inner, footer.status),
            &label,
            Style::default().fg(theme.statusbar).bg(theme.statusbar_bg),
            true,
        );
        self.buttons = footer.buttons;
    }
}

#[derive(Debug, PartialEq)]
pub enum MouseResult {
    None,
    Command(String),
    Close,
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
    use serde_json::json;

    fn fixture(rows: Vec<String>) -> App {
        let mut app = App::new();
        app.theme = Theme::default();
        app.background.enabled = false;
        app.disconnected = false;
        app.message.clear();
        app.state.mode = json!({"Paused":{"secs":12,"nanos":0}});
        app.rows = rows;
        app.times = vec![0.0, 10.0, 20.0];
        app
    }

    fn draw(app: &mut App, width: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn cached_view_matches_fresh_layout_after_edits_resizes_and_scrolling() {
        let rows = vec!["abcdefghijklmnop q".into(), "漢字 e\u{301} 🙂 palabras muy largas".into(), "".into()];
        let mut cached = fixture(rows);
        let _ = draw(&mut cached, 32);
        // Same byte length, different word boundaries; a size-only key is insufficient.
        cached.rows[0] = "abcd efghijklmnopq".into();
        for width in [32, 20, 80, 32] {
            let mut fresh = fixture(cached.rows.clone());
            assert_eq!(draw(&mut cached, width), draw(&mut fresh, width));
        }
        cached.follow = false;
        cached.scroll = 2;
        let mut fresh = fixture(cached.rows.clone());
        fresh.follow = false;
        fresh.scroll = 2;
        assert_eq!(draw(&mut cached, 32), draw(&mut fresh, 32));
        cached.accept_track();
        cached.accept_lyrics(Ok(Some(Record { plain_lyrics: Some("Otro texto de prueba".into()), ..Default::default() })));
        let mut fresh = fixture(Vec::new());
        fresh.accept_track();
        fresh.accept_lyrics(Ok(Some(Record { plain_lyrics: Some("Otro texto de prueba".into()), ..Default::default() })));
        assert_eq!(draw(&mut cached, 32), draw(&mut fresh, 32));
    }
}
fn inside(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x && x < area.right() && y >= area.y && y < area.bottom()
}

fn row(area: Rect, y: u16) -> Rect {
    Rect::new(area.x, y, area.width, 1)
}
fn line(frame: &mut Frame, area: Rect, text: &str, style: Style, centered: bool) {
    // Remove background glyphs as well as changing their colors.
    frame.render_widget(Clear, area);
    let paragraph = Paragraph::new(text).style(style).alignment(if centered {
        Alignment::Center
    } else {
        Alignment::Left
    });
    frame.render_widget(paragraph, area);
}
fn lyric_line(frame: &mut Frame, area: Rect, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    // Keep a small opaque halo around the text; art survives beside each line.
    let width = (UnicodeWidthStr::width(text).min(area.width as usize) as u16)
        .saturating_add(4)
        .min(area.width);
    let text_area = Rect::new(area.x + (area.width - width) / 2, area.y, width, 1);
    line(frame, text_area, text, style, true);
}
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
