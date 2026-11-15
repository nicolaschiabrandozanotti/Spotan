//! Both existing cell UIs share one native window and the player's model.
use super::{layout::Layout, native_state::StateCache};
use crate::{
    application::UserData, command, model::playable::Playable, queue::Queue,
    spotify::{PlayerEvent, Spotify},
};
use api::ratatui_bridge::*;
use cursive::{
    Printer, Vec2,
    direction::Direction,
    event::{AnyCb, Event, EventResult, Key, MouseButton, MouseEvent},
    style::{Color, ColorStyle, Effect},
    view::{CannotFocus, Selector, View, ViewNotFound},
    views::NamedView,
};
use spoti_tools::{
    self as api, Record, Track,
    lyrics::{App, MouseResult},
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

// Synthetic window events; shared with the Wayland backend, inert in terminals.
pub fn close_event() -> Event {
    Event::Unknown(b"spoti-window-close".to_vec())
}
pub fn motion_event(position: Vec2) -> Event {
    let mut bytes = b"spoti-motion:".to_vec();
    bytes.extend_from_slice(&(position.x as u32).to_le_bytes());
    bytes.extend_from_slice(&(position.y as u32).to_le_bytes());
    Event::Unknown(bytes)
}
pub fn motion_position(event: &Event) -> Option<Vec2> {
    let Event::Unknown(bytes) = event else {
        return None;
    };
    let bytes = bytes.strip_prefix(b"spoti-motion:")?;
    if bytes.len() != 8 {
        return None;
    };
    Some(Vec2::new(
        u32::from_le_bytes(bytes[..4].try_into().ok()?) as usize,
        u32::from_le_bytes(bytes[4..].try_into().ok()?) as usize,
    ))
}
struct Lyrics {
    app: App,
    canvas: CellCanvas,
    state_cache: StateCache,
    last_theme: Instant,
    requests: mpsc::Sender<Track>,
    results: mpsc::Receiver<(String, Result<Option<Record>, String>)>,
}
impl Lyrics {
    fn new() -> Self {
        let (requests, rx) = mpsc::channel::<Track>();
        let (tx, results) = mpsc::channel();
        thread::Builder::new()
            .name("spoti-lyrics".into())
            .spawn(move || {
                while let Ok(mut track) = rx.recv() {
                    for newer in rx.try_iter() {
                        track = newer;
                    }
                    let key = track.key();
                    let result = api::lookup(&track).map_err(|error| error.to_string());
                    if tx.send((key, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("lyrics worker");
        let mut app = App::new();
        app.disconnected = false;
        app.message = "Elegí una canción en tu biblioteca".into();
        Self {
            app,
            canvas: CellCanvas::default(),
            state_cache: StateCache::default(),
            last_theme: Instant::now(),
            requests,
            results,
        }
    }
    fn update_state(&mut self, mode: PlayerEvent, playable: Option<&Playable>) -> bool {
        self.state_cache.update(&mut self.app.state, mode, playable)
    }
    fn draw(&mut self, metadata_changed: bool, size: Vec2) {
        self.app.disconnected = false;
        if metadata_changed {
            let key = self.app.state.playable.as_ref().map_or_else(String::new, Track::key);
            if key != self.app.active {
                self.app.active = key;
                self.app.accept_track();
                if let Some(track) = self.app.state.playable.as_ref().filter(|t| t.kind == "Track") {
                    let _ = self.requests.send(track.clone());
                }
            }
        }
        while let Ok((key, result)) = self.results.try_recv() {
            if key == self.app.active {
                self.app.accept_lyrics(result);
            }
        }
        if self.last_theme.elapsed() >= Duration::from_secs(1) {
            self.app.theme = api::theme::Theme::read();
            self.last_theme = Instant::now();
        }
        let width = size.x.max(1).min(u16::MAX as usize) as u16;
        let height = size.y.max(1).min(u16::MAX as usize) as u16;
        self.canvas.draw(&mut self.app, width, height);
    }
}

pub struct NativePanels {
    player: NamedView<Layout>,
    lyrics: Mutex<Lyrics>,
    queue: Arc<Queue>,
    spotify: Spotify,
    size: Vec2,
    show: bool,
    right_focus: bool,
    captured: bool,
}
impl NativePanels {
    pub fn new(player: NamedView<Layout>, queue: Arc<Queue>, spotify: Spotify) -> Self {
        Self {
            player,
            lyrics: Mutex::new(Lyrics::new()),
            queue,
            spotify,
            size: Vec2::zero(),
            show: true,
            right_focus: false,
            captured: false,
        }
    }
    fn split(&self) -> Option<usize> {
        (self.show && self.size.x >= 50).then_some(self.size.x * 45 / 100)
    }
    fn small(&self) -> bool {
        self.size.x < 20 || self.size.y < 7
    }
    fn draw_lyrics(&self, printer: &Printer, offset: usize, right: usize) {
        let mut lyrics = self.lyrics.lock().unwrap();
        let mode = self.spotify.get_current_status();
        let index = self.queue.get_current_index();
        let metadata_changed = {
            // Release the queue before drawing, looking up lyrics or sending requests.
            let queue = self.queue.queue.read().unwrap();
            lyrics.update_state(mode, index.and_then(|i| queue.get(i)))
        };
        lyrics.draw(metadata_changed, Vec2::new(right, self.size.y));
        let theme = &lyrics.app.theme;
        if offset > 0 {
            printer.with_color(
                ColorStyle::new(color(theme.highlight_bg), color(theme.background)),
                |printer| printer.print_vline((offset - 1, 0), self.size.y, "│"),
            );
        }
        let target = printer
            .offset((offset, 0))
            .cropped((right, self.size.y))
            .focused(self.right_focus);
        let buffer = lyrics.canvas.buffer();
        for y in 0..self.size.y {
            let mut x = 0;
            while x < right {
                let cell = &buffer[(x as u16, y as u16)];
                target.with_color(ColorStyle::new(color(cell.fg), color(cell.bg)), |printer| {
                    if cell.modifier.contains(Modifier::BOLD) {
                        printer.with_effect(Effect::Bold, |printer| {
                            printer.print((x, y), cell.symbol())
                        });
                    } else {
                        printer.print((x, y), cell.symbol());
                    }
                });
                x += cell.symbol().width().max(1);
            }
        }
    }
    pub fn hide_lyrics(&mut self) {
        self.show = false;
        self.right_focus = false;
        self.captured = false;
    }
    fn command(text: String) -> EventResult {
        EventResult::with_cb(move |s| {
            if let (Some(data), Ok(commands)) =
                (s.user_data::<UserData>().cloned(), command::parse(&text))
            {
                for command in commands {
                    data.cmd.handle(s, command);
                }
            }
        })
    }
    fn close_lyrics() -> EventResult {
        EventResult::with_cb(|s| {
            s.call_on_name("spoti-native-panels", |view: &mut NativePanels| {
                view.hide_lyrics()
            });
        })
    }
    fn right_event(&mut self, event: Event, offset: usize) -> EventResult {
        if matches!(
            event,
            Event::Char('q' | 'Q') | Event::Key(Key::Esc) | Event::CtrlChar('c')
        ) {
            self.hide_lyrics();
            return EventResult::consumed();
        }
        let mut lyrics = self.lyrics.lock().unwrap();
        if let Some(position) = motion_position(&event) {
            lyrics.app.hover = Some((position.x.saturating_sub(offset) as u16, position.y as u16));
            return EventResult::consumed();
        }
        if let Event::Mouse {
            position,
            offset: parent,
            event,
        } = event
        {
            let x = position.x.saturating_sub(parent.x + offset);
            let y = position.y.saturating_sub(parent.y);
            let kind = match event {
                MouseEvent::Press(MouseButton::Left) => {
                    Some(MouseEventKind::Down(CrossMouseButton::Left))
                }
                MouseEvent::Release(MouseButton::Left) => {
                    Some(MouseEventKind::Up(CrossMouseButton::Left))
                }
                MouseEvent::Hold(MouseButton::Left) => {
                    Some(MouseEventKind::Drag(CrossMouseButton::Left))
                }
                MouseEvent::WheelUp => Some(MouseEventKind::ScrollUp),
                MouseEvent::WheelDown => Some(MouseEventKind::ScrollDown),
                _ => None,
            };
            if let Some(kind) = kind {
                return match lyrics.app.mouse(CrossMouseEvent {
                    kind,
                    column: x.min(u16::MAX as usize) as u16,
                    row: y.min(u16::MAX as usize) as u16,
                    modifiers: KeyModifiers::empty(),
                }) {
                    MouseResult::Command(command) => Self::command(command),
                    MouseResult::Close => Self::close_lyrics(),
                    MouseResult::None => EventResult::consumed(),
                };
            }
            return EventResult::Ignored;
        }
        let code = match event {
            Event::Char(c) => KeyCode::Char(c),
            Event::Key(key) => match key {
                Key::Left => KeyCode::Left,
                Key::Right => KeyCode::Right,
                Key::Up => KeyCode::Up,
                Key::Down => KeyCode::Down,
                Key::PageUp => KeyCode::PageUp,
                Key::PageDown => KeyCode::PageDown,
                _ => return EventResult::consumed(),
            },
            Event::Unknown(_) | Event::Refresh | Event::FocusLost | Event::WindowResize => {
                return EventResult::Ignored;
            }
            _ => return EventResult::consumed(),
        };
        let page = self.size.y.saturating_sub(10).max(1);
        if let Some(command) = lyrics
            .app
            .key(CrossKeyEvent::new(code, KeyModifiers::empty()), page)
        {
            Self::command(command.to_owned())
        } else {
            EventResult::consumed()
        }
    }
}
fn color(color: CrossColor) -> Color {
    match color {
        CrossColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        CrossColor::Indexed(n) => Color::from_256colors(n),
        CrossColor::Reset => Color::TerminalDefault,
        _ => Color::TerminalDefault,
    }
}
impl View for NativePanels {
    fn draw(&self, printer: &Printer) {
        if self.small() {
            printer.print((0, 0), "Ampliá la ventana · q salir");
            return;
        }
        if let Some(left) = self.split() {
            self.player.draw(
                &printer
                    .cropped((left, self.size.y))
                    .focused(!self.right_focus),
            );
            self.draw_lyrics(printer, left + 1, self.size.x - left - 1);
        } else if self.show && self.right_focus {
            self.draw_lyrics(printer, 0, self.size.x);
        } else {
            self.player.draw(&printer.focused(true));
        }
    }
    fn layout(&mut self, size: Vec2) {
        self.size = size;
        if self.small() {
            return;
        }
        let width = self.split().unwrap_or(size.x);
        let area = Vec2::new(width, size.y);
        self.player.required_size(area);
        self.player.layout(area);
    }
    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        constraint
    }
    fn on_event(&mut self, event: Event) -> EventResult {
        if event == Event::Char('L') && !self.right_focus {
            // Text inputs get first refusal, just like Cursive's global bindings.
            let result = self.player.on_event(event.clone());
            if result.is_consumed() {
                return result;
            }
        }
        if event == Event::Char('L') {
            if self.size.x < 50 {
                self.show = true;
                self.right_focus = !self.right_focus;
            } else {
                self.show = !self.show;
                self.right_focus = false;
            }
            self.captured = false;
            return EventResult::consumed();
        }
        if event == Event::Alt(Key::Left) {
            self.right_focus = false;
            return EventResult::consumed();
        }
        if event == Event::Alt(Key::Right) {
            self.show = true;
            self.right_focus = true;
            return EventResult::consumed();
        }
        if self.small() {
            return EventResult::Ignored;
        }
        if let Some(left) = self.split() {
            let position =
                motion_position(&event).or_else(|| event.mouse_position());
            if let Some(position) = position {
                if !self.captured {
                    self.right_focus = position.x > left;
                }
                if matches!(
                    event,
                    Event::Mouse {
                        event: MouseEvent::Press(MouseButton::Left),
                        ..
                    }
                ) {
                    self.captured = true;
                }
            }
            let result = if self.right_focus {
                self.right_event(event.clone(), left + 1)
            } else if motion_position(&event).is_some() {
                EventResult::consumed()
            } else {
                self.player.on_event(event.clone())
            };
            if matches!(
                event,
                Event::Mouse {
                    event: MouseEvent::Release(_),
                    ..
                }
            ) {
                self.captured = false;
            }
            result
        } else if self.show && self.right_focus {
            self.right_event(event, 0)
        } else {
            self.right_focus = false;
            if motion_position(&event).is_some() {
                EventResult::consumed()
            } else {
                self.player.on_event(event)
            }
        }
    }
    fn call_on_any(&mut self, selector: &Selector, callback: AnyCb) {
        self.player.call_on_any(selector, callback);
    }
    fn focus_view(&mut self, selector: &Selector) -> Result<EventResult, ViewNotFound> {
        self.right_focus = false;
        self.player.focus_view(selector)
    }
    fn take_focus(&mut self, _: Direction) -> Result<EventResult, CannotFocus> {
        Ok(EventResult::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_and_command_inputs_keep_letters_instead_of_toggling_lyrics() {
        use crate::{config::Config, events::EventManager, library::Library};
        use cursive::{Cursive, traits::Nameable, views::{EditView, TextView}};
        use std::sync::atomic::AtomicUsize;
        for width in [32,132] {
            let mut s = Cursive::new();
            let cfg = Config::new_for_test();
            let ev = EventManager::new(s.cb_sink().clone());
            let spotify = Spotify::new_for_test(cfg.clone(), ev.clone());
            let library = Library::new_for_test(ev.clone(), spotify.clone(), cfg.clone());
            let queue = Arc::new(Queue::new(spotify.clone(), cfg.clone(), library.clone()));
            let mut layout = Layout::new(TextView::new("status"), &ev, cfg.build_theme(), cfg, Arc::new(AtomicUsize::new(0)));
            layout.add_screen("library", super::super::library::LibraryView::new(queue.clone(),library.clone()));
            layout.add_screen("search", super::super::search::SearchView::new(ev,queue.clone(),library));
            layout.set_screen("search");
            let view = NativePanels::new(layout.with_name("main"),queue,spotify);
            s.add_fullscreen_layer(view.with_name("spoti-native-panels"));
            s.screen_mut().layout(Vec2::new(width,31));
            s.add_global_callback('q', |s| s.quit());
            let query = "LITTLE love vnpbqFUP[]+/0123 ñÑ";
            for c in query.chars() { s.on_event(Event::Char(c)); }
            assert!(s.is_running());
            assert_eq!(s.call_on_name("search_edit", |v: &mut EditView| v.get_content().to_string()).unwrap(), query);
            assert!(s.call_on_name("spoti-native-panels", |v: &mut NativePanels| v.show && !v.right_focus).unwrap());
            s.call_on_name("main", |v: &mut Layout| { v.set_screen("library"); v.enable_jump(); });
            s.on_event(Event::Char('L'));
            assert!(s.call_on_name("spoti-native-panels", |v: &mut NativePanels| v.show && !v.right_focus).unwrap());
            s.call_on_name("main", |v: &mut Layout| v.clear_cmdline());
            s.on_event(Event::Char('L'));
            assert!(s.call_on_name("spoti-native-panels", |v: &mut NativePanels| if width<50 {v.right_focus} else {!v.show}).unwrap());
        }
    }
    #[test]
    fn integrated_panels_resize_and_capture_mouse_without_touching_playback() {
        use crate::{config::Config, events::EventManager, library::Library};
        use cursive::{Cursive, buffer::PrintBuffer, traits::Nameable, views::TextView};
        use std::sync::atomic::AtomicUsize;
        let mut cursive = Cursive::new();
        let cfg = Config::new_for_test();
        let ev = EventManager::new(cursive.cb_sink().clone());
        let spotify = Spotify::new_for_test(cfg.clone(), ev.clone());
        let library = Library::new_for_test(ev.clone(), spotify.clone(), cfg.clone());
        let queue = Arc::new(Queue::new(spotify.clone(), cfg.clone(), library));
        let theme = cfg.build_theme();
        let layout = Layout::new(
            TextView::new("status"),
            &ev,
            theme.clone(),
            cfg,
            Arc::new(AtomicUsize::new(0)),
        );
        let mut view = NativePanels::new(layout.with_name("main"), queue, spotify);
        for size in [(1, 1), (19, 6), (49, 12), (80, 30), (132, 31)] {
            let size = Vec2::from(size);
            view.layout(size);
            let mut buffer = PrintBuffer::new();
            buffer.resize(size);
            let buffer = parking_lot::RwLock::new(buffer);
            view.draw(&Printer::new(size, &theme, &buffer));
        }
        view.on_event(motion_event(Vec2::new(100, 10)));
        assert!(view.right_focus);
        view.on_event(Event::Mouse {
            offset: Vec2::zero(),
            position: Vec2::new(100, 10),
            event: MouseEvent::Press(MouseButton::Left),
        });
        assert!(view.captured);
        view.on_event(motion_event(Vec2::new(2, 10)));
        assert!(view.right_focus);
        view.on_event(Event::Mouse {
            offset: Vec2::zero(),
            position: Vec2::new(2, 10),
            event: MouseEvent::Release(MouseButton::Left),
        });
        view.on_event(motion_event(Vec2::new(2, 10)));
        assert!(!view.right_focus);
        view.on_event(Event::Char('L'));
        assert!(!view.show);
        view.layout(Vec2::new(32, 20));
        view.on_event(Event::Char('L'));
        assert!(view.right_focus);
        view.on_event(motion_event(Vec2::new(2, 10)));
        assert!(view.right_focus);
        view.on_event(Event::Char('q'));
        assert!(!view.right_focus);
        view.show = true;
        cursive.add_fullscreen_layer(view.with_name("spoti-native-panels"));
        cursive.screen_mut().layout(Vec2::new(132, 31));
        cursive.add_global_callback('q', Cursive::quit);
        cursive.on_event(Event::Char('L'));
        cursive.on_event(Event::Char('L'));
        cursive.on_event(motion_event(Vec2::new(100, 10)));
        assert!(
            cursive
                .call_on_name("spoti-native-panels", |view: &mut NativePanels| view
                    .right_focus)
                .unwrap()
        );
        cursive.on_event(Event::Char('q'));
        assert!(cursive.is_running());
    }
}
