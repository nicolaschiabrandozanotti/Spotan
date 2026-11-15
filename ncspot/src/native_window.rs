//! Cursive cell backend drawn directly into Wayland shared-memory buffers.
//! Protocol setup follows the MIT-licensed Smithay client toolkit examples.
use cairo::{Context, FontSlant, FontWeight, Format, Glyph, ImageSurface};
use cursive::{
    Vec2,
    backend::Backend,
    event::{Event, Key, MouseButton, MouseEvent},
    style::{Color, ColorPair, Effect},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{
            CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
        },
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

const CELL_W: usize = 10;
const CELL_H: usize = 22;
pub use crate::ui::native_panels::{close_event, motion_event};
#[derive(PartialEq)]
struct Cell {
    text: String,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
    underline: bool,
}
impl Clone for Cell {
    fn clone(&self) -> Self {
        Self { text: self.text.clone(), fg: self.fg, bg: self.bg,
            bold: self.bold, underline: self.underline }
    }
    fn clone_from(&mut self, source: &Self) {
        self.text.clone_from(&source.text);
        self.fg = source.fg;
        self.bg = source.bg;
        self.bold = source.bold;
        self.underline = source.underline;
    }
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            fg: (232, 223, 223),
            bg: (24, 24, 31),
            bold: false,
            underline: false,
        }
    }
}
struct FrameBuffer {
    buffer: Buffer,
    cells: Vec<Cell>,
}
struct CachedGlyph {
    family: &'static str,
    glyphs: Vec<Glyph>,
}
struct State {
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    compositor: CompositorState,
    shm: Shm,
    window: Window,
    pool: SlotPool,
    buffers: Vec<FrameBuffer>,
    width: u32,
    height: u32,
    scale: i32,
    configured: bool,
    frame_ready: Rc<std::cell::Cell<bool>>,
    frame_submitted: Instant,
    dirty: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    loop_handle: LoopHandle<'static, State>,
    mods: Modifiers,
    held: Option<MouseButton>,
    events: VecDeque<Event>,
    cells: Vec<Cell>,
    cols: usize,
    rows: usize,
    cursor: Vec2,
    colors: ColorPair,
    bold: bool,
    reverse: bool,
    underline: bool,
    glyphs: HashMap<(bool, i32), HashMap<String, CachedGlyph>>,
    changed: Vec<usize>,
    glyph_buffer: Vec<Glyph>,
}
pub struct NativeBackend {
    state: RefCell<State>,
    event_loop: EventLoop<'static, State>,
    qh: QueueHandle<State>,
}
impl NativeBackend {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_app_id("ncspot")
    }
    pub fn draw_ready(&self) -> Rc<std::cell::Cell<bool>> {
        self.state.borrow().frame_ready.clone()
    }
    fn with_app_id(app_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init(&conn)?;
        let qh = queue.handle();
        let event_loop = EventLoop::try_new()?;
        WaylandSource::new(conn, queue).insert(event_loop.handle())?;
        let compositor = CompositorState::bind(&globals, &qh)?;
        let shell = XdgShell::bind(&globals, &qh)?;
        let shm = Shm::bind(&globals, &qh)?;
        let window = shell.create_window(
            compositor.create_surface(&qh),
            WindowDecorations::RequestServer,
            &qh,
        );
        window.set_title("Spoti");
        window.set_app_id(app_id);
        window.set_min_size(Some((280, 220)));
        window.commit();
        let pool = SlotPool::new(1024 * 660 * 4, &shm)?;
        let state = State {
            registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &qh),
            outputs: OutputState::new(&globals, &qh),
            compositor,
            shm,
            window,
            pool,
            buffers: vec![],
            width: 1024,
            height: 660,
            scale: 1,
            configured: false,
            frame_ready: Rc::new(std::cell::Cell::new(true)),
            frame_submitted: Instant::now(),
            dirty: true,
            keyboard: None,
            pointer: None,
            loop_handle: event_loop.handle(),
            mods: Modifiers::default(),
            held: None,
            events: VecDeque::new(),
            cells: vec![Cell::default(); 102 * 30],
            cols: 102,
            rows: 30,
            cursor: Vec2::zero(),
            colors: ColorPair::terminal_default(),
            bold: false,
            reverse: false,
            underline: false,
            glyphs: HashMap::new(),
            changed: Vec::new(),
            glyph_buffer: Vec::new(),
        };
        Ok(Self {
            state: RefCell::new(state),
            event_loop,
            qh,
        })
    }
}
fn rgb(color: Color, background: bool) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::RgbLowRes(r, g, b) => {
            let c = |n| if n == 0 { 0 } else { 55 + 40 * n };
            (c(r), c(g), c(b))
        }
        Color::TerminalDefault => {
            if background {
                (24, 24, 31)
            } else {
                (232, 223, 223)
            }
        }
        Color::Dark(base) | Color::Light(base) => {
            let bright = matches!(color, Color::Light(_));
            let n = base as usize;
            let colors = if bright {
                [
                    (116, 81, 81),
                    (218, 180, 180),
                    (174, 194, 165),
                    (224, 206, 166),
                    (167, 185, 215),
                    (210, 180, 190),
                    (151, 194, 194),
                    (244, 233, 233),
                ]
            } else {
                [
                    (31, 24, 24),
                    (180, 110, 110),
                    (140, 166, 130),
                    (195, 175, 132),
                    (132, 154, 188),
                    (176, 140, 158),
                    (118, 168, 168),
                    (232, 223, 223),
                ]
            };
            colors[n]
        }
    }
}
impl Backend for NativeBackend {
    fn poll_event(&mut self) -> Option<Event> {
        if let Some(event) = self.state.borrow_mut().events.pop_front() {
            return Some(event);
        }
        if let Err(error) = self
            .event_loop
            .dispatch(Duration::ZERO, &mut *self.state.borrow_mut())
        {
            log::error!("Wayland: {error}");
            return Some(close_event());
        }
        self.state.borrow_mut().release_hidden();
        self.state.borrow_mut().events.pop_front()
    }
    fn set_title(&mut self, _title: String) {}
    fn refresh(&mut self) {
        let mut state = self.state.borrow_mut();
        if let Err(error) = state.draw(&self.qh) {
            log::error!("Spoti renderer: {error}");
            state.events.push_back(close_event());
        }
    }
    fn has_colors(&self) -> bool {
        true
    }
    fn screen_size(&self) -> Vec2 {
        let s = self.state.borrow();
        Vec2::new(s.cols, s.rows)
    }
    fn move_to(&self, pos: Vec2) {
        self.state.borrow_mut().cursor = pos;
    }
    fn print(&self, text: &str) {
        let mut s = self.state.borrow_mut();
        let pair = if s.reverse {
            s.colors.invert()
        } else {
            s.colors
        };
        for symbol in text.graphemes(true) {
            let width = symbol.width();
            let x = s.cursor.x;
            let y = s.cursor.y;
            if width == 0 {
                if x > 0 && y < s.rows {
                    let i = y * s.cols + x - 1;
                    if i < s.cells.len() {
                        s.cells[i].text.push_str(symbol);
                        s.dirty = true;
                    }
                }
                continue;
            }
            if x < s.cols && y < s.rows {
                let index = y * s.cols + x;
                let cell = Cell {
                    text: symbol.to_owned(),
                    fg: rgb(pair.front, false),
                    bg: rgb(pair.back, true),
                    bold: s.bold,
                    underline: s.underline,
                };
                if s.cells[index] != cell {
                    s.cells[index] = cell;
                    s.dirty = true;
                }
                if width > 1 && x + 1 < s.cols {
                    let next = Cell {
                        text: String::new(),
                        fg: rgb(pair.front, false),
                        bg: rgb(pair.back, true),
                        bold: s.bold,
                        underline: s.underline,
                    };
                    if s.cells[index + 1] != next {
                        s.cells[index + 1] = next;
                        s.dirty = true;
                    }
                }
            }
            s.cursor.x = s.cursor.x.saturating_add(width);
        }
    }
    fn clear(&self, color: Color) {
        let mut s = self.state.borrow_mut();
        let cell = Cell {
            bg: rgb(color, true),
            ..Cell::default()
        };
        s.cells.fill(cell);
        s.dirty = true;
    }
    fn set_color(&self, colors: ColorPair) -> ColorPair {
        let mut s = self.state.borrow_mut();
        std::mem::replace(&mut s.colors, colors)
    }
    fn set_effect(&self, effect: Effect) {
        let mut s = self.state.borrow_mut();
        match effect {
            Effect::Bold => s.bold = true,
            Effect::Reverse => s.reverse = true,
            Effect::Underline => s.underline = true,
            _ => (),
        }
    }
    fn unset_effect(&self, effect: Effect) {
        let mut s = self.state.borrow_mut();
        match effect {
            Effect::Bold => s.bold = false,
            Effect::Reverse => s.reverse = false,
            Effect::Underline => s.underline = false,
            _ => (),
        }
    }
    fn is_persistent(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "spoti-wayland"
    }
}
impl State {
    fn resize(&mut self) {
        self.cols = (self.width as usize / CELL_W).max(1);
        self.rows = (self.height as usize / CELL_H).max(1);
        self.cells = vec![Cell::default(); self.cols * self.rows];
        self.buffers.clear();
        if let Ok(pool) = SlotPool::new(
            (self.width as usize
                * self.height as usize
                * 4
                * self.scale.max(1) as usize
                * self.scale.max(1) as usize)
                .max(4096),
            &self.shm,
        ) {
            self.pool = pool;
        }
        self.glyphs.clear();
        self.dirty = true;
        self.events.push_back(Event::WindowResize);
    }
    /// No frame callback for a while means the compositor is not showing the window
    /// (minimized / other workspace). Free the SHM pool once the compositor released every
    /// buffer (it keeps the last contents); `draw` recreates it lazily, repainting in full.
    fn release_hidden(&mut self) {
        if self.frame_ready.get()
            || self.buffers.is_empty()
            || self.frame_submitted.elapsed() < Duration::from_secs(3)
        {
            return;
        }
        let pool = &mut self.pool;
        if self.buffers.iter().all(|frame| pool.canvas(&frame.buffer).is_some()) {
            if let Ok(small) = SlotPool::new(4096, &self.shm) {
                self.buffers.clear();
                self.pool = small;
            }
        }
    }
    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn std::error::Error>> {
        if !self.configured || !self.frame_ready.get() || !self.dirty {
            return Ok(());
        }
        let scale = self.scale.max(1);
        let width = self.width as i32 * scale;
        let height = self.height as i32 * scale;
        let stride = width * 4;
        let index = self
            .buffers
            .iter()
            .position(|frame| self.pool.canvas(&frame.buffer).is_some());
        let index = if let Some(index) = index {
            index
        } else if self.buffers.len() < 2 {
            let (buffer, _) =
                self.pool
                    .create_buffer(width, height, stride, wl_shm::Format::Argb8888)?;
            self.buffers.push(FrameBuffer {
                buffer,
                cells: vec![],
            });
            self.buffers.len() - 1
        } else {
            return Ok(());
        };
        let frame = &mut self.buffers[index];
        let fresh = frame.cells.len() != self.cells.len();
        let Some(canvas) = self.pool.canvas(&frame.buffer) else {
            return Ok(());
        };
        // The Cairo surface/context are dropped before releasing the mutable SHM
        // canvas borrow or attaching it to the compositor. No dangling pixel owner.
        {
            let surface = unsafe {
                ImageSurface::create_for_data_unsafe(
                    canvas.as_mut_ptr(),
                    Format::ARgb32,
                    width,
                    height,
                    stride,
                )?
            };
            let ctx = Context::new(&surface)?;
            if fresh {
                let (r, g, b) = self.cells.first().map(|c| c.bg).unwrap_or((24, 24, 31));
                ctx.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                ctx.paint()?;
            }
            self.changed.clear();
            self.changed.extend(self
                .cells
                .iter()
                .enumerate()
                .filter_map(|(i, cell)| (fresh || frame.cells[i] != *cell).then_some(i)));
            for &index in &self.changed {
                let cell = &self.cells[index];
                let (r, g, b) = cell.bg;
                ctx.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                ctx.rectangle(
                    (index % self.cols * CELL_W) as f64 * scale as f64,
                    (index / self.cols * CELL_H) as f64 * scale as f64,
                    (CELL_W * scale as usize) as f64,
                    (CELL_H * scale as usize) as f64,
                );
                ctx.fill()?;
            }
            let mut last_font = None;
            let mut last_color = None;
            for &index in &self.changed {
                let cell = &self.cells[index];
                if cell.text.trim().is_empty() {
                    continue;
                }
                if last_color != Some(cell.fg) {
                    let (r, g, b) = cell.fg;
                    ctx.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                    last_color = Some(cell.fg);
                }
                let x = (index % self.cols * CELL_W) as f64 * scale as f64;
                let y = (index / self.cols * CELL_H + 17) as f64 * scale as f64;
                let cache = self.glyphs.entry((cell.bold, scale)).or_default();
                if !cache.contains_key(cell.text.as_str()) {
                    let mut selected = None;
                    for family in [
                        "JetBrainsMono Nerd Font Mono",
                        "DejaVu Sans",
                        "Noto Sans Symbols 2",
                        "Noto Sans",
                        "Noto Color Emoji",
                    ] {
                        ctx.select_font_face(
                            family,
                            FontSlant::Normal,
                            if cell.bold {
                                FontWeight::Bold
                            } else {
                                FontWeight::Normal
                            },
                        );
                        ctx.set_font_size(16.0 * scale as f64);
                        let (glyphs, _) = ctx.scaled_font().text_to_glyphs(0.0, 0.0, &cell.text)?;
                        let complete = glyphs.iter().all(|glyph| glyph.index() != 0);
                        if selected.is_none() || complete {
                            selected = Some(CachedGlyph { family, glyphs });
                        }
                        if complete {
                            break;
                        }
                    }
                    cache.insert(cell.text.clone(), selected.expect("font candidates"));
                    last_font = None;
                }
                let cached = &cache[&cell.text];
                if last_font != Some((cached.family, cell.bold)) {
                    ctx.select_font_face(
                        cached.family,
                        FontSlant::Normal,
                        if cell.bold {
                            FontWeight::Bold
                        } else {
                            FontWeight::Normal
                        },
                    );
                    ctx.set_font_size(16.0 * scale as f64);
                    last_font = Some((cached.family, cell.bold));
                }
                self.glyph_buffer.clear();
                self.glyph_buffer.extend(cached
                    .glyphs
                    .iter()
                    .map(|g| Glyph::new(g.index(), g.x() + x, g.y() + y)));
                ctx.show_glyphs(&self.glyph_buffer)?;
                if cell.underline {
                    ctx.rectangle(
                        x,
                        y + scale as f64,
                        (CELL_W * scale as usize) as f64,
                        scale as f64,
                    );
                    ctx.fill()?;
                }
            }
            surface.flush();
        }
        frame.cells.clone_from(&self.cells);
        self.window.wl_surface().set_buffer_scale(scale);
        self.window.wl_surface().damage_buffer(0, 0, width, height);
        self.window
            .wl_surface()
            .frame(qh, self.window.wl_surface().clone());
        frame.buffer.attach_to(self.window.wl_surface())?;
        self.window.commit();
        self.frame_ready.set(false);
        self.frame_submitted = Instant::now();
        self.dirty = false;
        Ok(())
    }
    fn push_key(&mut self, event: KeyEvent) {
        #[cfg(feature = "share_clipboard")]
        if self.mods.ctrl && self.mods.shift && matches!(event.keysym, Keysym::v | Keysym::V)
            || self.mods.shift && event.keysym == Keysym::Insert
        {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    for c in text.chars() {
                        self.events
                            .push_back(Event::Char(if c.is_control() { ' ' } else { c }));
                    }
                }
            }
            return;
        }
        if !self.mods.ctrl && !self.mods.alt && !self.mods.logo {
            if let Some(text) = event
                .utf8
                .as_deref()
                .filter(|text| text.chars().count() > 1 && text.chars().all(|c| !c.is_control()))
            {
                self.events.extend(text.chars().map(Event::Char));
                return;
            }
        }
        if let Some(event) = key_event(event.keysym, event.utf8.as_deref(), self.mods) {
            self.events.push_back(event)
        }
    }
}
fn key_event(sym: Keysym, text: Option<&str>, mods: Modifiers) -> Option<Event> {
    if mods.logo {
        return None;
    }
    let key = match sym {
        Keysym::Return | Keysym::KP_Enter => Some(Key::Enter),
        Keysym::Tab | Keysym::ISO_Left_Tab => Some(Key::Tab),
        Keysym::BackSpace => Some(Key::Backspace),
        Keysym::Escape => Some(Key::Esc),
        Keysym::Left => Some(Key::Left),
        Keysym::Right => Some(Key::Right),
        Keysym::Up => Some(Key::Up),
        Keysym::Down => Some(Key::Down),
        Keysym::Home => Some(Key::Home),
        Keysym::End => Some(Key::End),
        Keysym::Page_Up => Some(Key::PageUp),
        Keysym::Page_Down => Some(Key::PageDown),
        Keysym::Delete => Some(Key::Del),
        Keysym::Insert => Some(Key::Ins),
        _ if (0xffbe..=0xffc9).contains(&sym.raw()) => {
            Some(Key::from_f((sym.raw() - 0xffbd) as u8))
        }
        _ => None,
    };
    if let Some(key) = key {
        return Some(match (mods.ctrl, mods.alt, mods.shift) {
            (true, true, _) => Event::CtrlAlt(key),
            (true, false, true) => Event::CtrlShift(key),
            (true, false, false) => Event::Ctrl(key),
            (false, true, true) => Event::AltShift(key),
            (false, true, false) => Event::Alt(key),
            (false, false, true) => Event::Shift(key),
            _ => Event::Key(key),
        });
    }
    if mods.ctrl {
        if mods.alt {
            return None;
        }
        return sym.key_char().map(|c| {
            Event::CtrlChar(if mods.shift {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            })
        });
    }
    let text = text.filter(|text| !text.is_empty())?;
    let c = text.chars().next()?;
    (!c.is_control()).then_some(if mods.alt {
        Event::AltChar(c)
    } else {
        Event::Char(c)
    })
}
impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if surface == self.window.wl_surface() {
            self.scale = factor.max(1);
            self.resize();
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_ready.set(true);
        if self.frame_submitted.elapsed() >= Duration::from_millis(250) {
            self.events.push_back(Event::Refresh);
        }
        if let Err(error) = self.draw(qh) {
            log::error!("Wayland frame: {error}");
            self.events.push_back(close_event());
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}
impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}
impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.events.push_back(close_event());
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let w = configure.new_size.0.map(|n| n.get()).unwrap_or(self.width);
        let h = configure.new_size.1.map(|n| n.get()).unwrap_or(self.height);
        if !self.configured || w != self.width || h != self.height {
            self.width = w.max(1);
            self.height = h.max(1);
            self.resize();
        }
        self.configured = true;
        self.frame_ready.set(true);
    }
}
impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self
                .seats
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|state, _, event| state.push_key(event)),
                )
                .ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self
                .seats
                .get_pointer_with_theme(
                    qh,
                    &seat,
                    self.shm.wl_shm(),
                    self.compositor.create_surface(qh),
                    ThemeSpec::default(),
                )
                .ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if cap == Capability::Pointer {
            self.pointer.take();
            self.held = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}
impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.mods = Modifiers::default();
        self.events.push_back(Event::FocusLost);
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.push_key(event);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        mods: Modifiers,
        _: u32,
    ) {
        self.mods = mods;
    }
}
impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.window.wl_surface() {
                continue;
            }
            let position = Vec2::new(
                (event.position.0.max(0.0) / CELL_W as f64) as usize,
                (event.position.1.max(0.0) / CELL_H as f64) as usize,
            );
            let mouse = match event.kind {
                PointerEventKind::Enter { .. } => {
                    if let Some(pointer) = &self.pointer {
                        let _ = pointer.set_cursor(conn, CursorIcon::Default);
                    }
                    self.events.push_back(motion_event(position));
                    None
                }
                PointerEventKind::Motion { .. } => {
                    if let Some(button) = self.held {
                        Some(MouseEvent::Hold(button))
                    } else {
                        self.events.push_back(motion_event(position));
                        None
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    let button = match button {
                        272 => MouseButton::Left,
                        273 => MouseButton::Right,
                        274 => MouseButton::Middle,
                        _ => continue,
                    };
                    self.held = Some(button);
                    Some(MouseEvent::Press(button))
                }
                PointerEventKind::Release { button, .. } => {
                    let button = match button {
                        272 => MouseButton::Left,
                        273 => MouseButton::Right,
                        274 => MouseButton::Middle,
                        _ => continue,
                    };
                    self.held = None;
                    Some(MouseEvent::Release(button))
                }
                PointerEventKind::Axis { vertical, .. } => {
                    if vertical.absolute < 0.0 {
                        Some(MouseEvent::WheelUp)
                    } else if vertical.absolute > 0.0 {
                        Some(MouseEvent::WheelDown)
                    } else {
                        None
                    }
                }
                PointerEventKind::Leave { .. } => None,
            };
            if let Some(event) = mouse {
                self.events.push_back(Event::Mouse {
                    offset: Vec2::zero(),
                    position,
                    event,
                });
            }
        }
    }
}
impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_registry!(State);
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState,];
}

/// Renderer-only preview: no Spotify session, credentials, or playback changes.
pub fn demo() -> Result<(), Box<dyn std::error::Error>> {
    use cursive::{
        Cursive, CursiveRunner, style::Style, utils::markup::StyledString, views::TextView,
    };
    use cursive::traits::Nameable;
    let backend = NativeBackend::with_app_id("spoti-native-test")?;
    let mut pacing = crate::ui::DrawPacing::native(backend.draw_ready());
    let mut runner = CursiveRunner::new(Cursive::new(), Box::new(backend));
    runner.set_fps(8);
    let mut theme = cursive::theme::Theme::default();
    theme.palette[cursive::style::PaletteColor::Background] = Color::Rgb(24, 24, 31);
    theme.palette[cursive::style::PaletteColor::Primary] = Color::Rgb(232, 223, 223);
    theme.palette[cursive::style::PaletteColor::View] = Color::Rgb(24, 24, 31);
    runner.set_theme(theme);
    let mut text =
        StyledString::plain("\n  SPOTI · Ventana nativa\n\n  Biblioteca    Buscar    Cola\n\n");
    text.append_styled(
        "  Letras · música · áéíóú ñ ♪    \n\n",
        Style::from(Color::Rgb(210, 180, 190)),
    );
    text.append_styled(
        "  Línea activa sincronizada\n\n",
        Style::from(ColorPair {
            front: Color::Rgb(244, 233, 233),
            back: Color::Rgb(116, 81, 81),
        }),
    );
    text.append_styled(
        "  ⠀⠁⠃⠉⠛⠿⣿  ────────  │  ↑ ↓ ← →\n",
        Color::Rgb(135, 95, 105),
    );
    runner.add_fullscreen_layer(TextView::new(text).with_name("native-demo"));
    runner.add_global_callback('t', |s| {
        s.call_on_name("native-demo", |view: &mut TextView| {
            view.set_content("\n  SPOTI · cuadro actualizado después de ocultar la ventana");
        });
    });
    runner.add_global_callback('q', Cursive::quit);
    runner.add_global_callback(close_event(), Cursive::quit);
    runner.refresh();
    while runner.is_running() { pacing.step(&mut runner); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::native_panels::motion_position;
    #[test]
    fn cell_history_copy_reuses_text_storage_and_updates_styles() {
        let mut cell = Cell::default();
        cell.text = String::with_capacity(64);
        cell.text.push_str("old");
        let pointer = cell.text.as_ptr();
        let source = Cell { text: "漢字".into(), fg: (1,2,3), bg: (4,5,6), bold: true, underline: true };
        cell.clone_from(&source);
        assert!(cell == source);
        assert_eq!(cell.text.as_ptr(), pointer);
    }
    #[test]
    fn native_keys_keep_text_modifiers_and_navigation() {
        assert_eq!(
            key_event(
                Keysym::L,
                Some("L"),
                Modifiers {
                    shift: true,
                    ..Default::default()
                }
            ),
            Some(Event::Char('L'))
        );
        assert_eq!(
            key_event(
                Keysym::Left,
                None,
                Modifiers {
                    alt: true,
                    ..Default::default()
                }
            ),
            Some(Event::Alt(Key::Left))
        );
        assert_eq!(
            key_event(
                Keysym::c,
                Some("\u{3}"),
                Modifiers {
                    ctrl: true,
                    ..Default::default()
                }
            ),
            Some(Event::CtrlChar('c'))
        );
        assert_eq!(
            key_event(
                Keysym::C,
                Some("\u{3}"),
                Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                }
            ),
            Some(Event::CtrlChar('C'))
        );
        assert_eq!(
            key_event(Keysym::eacute, Some("é"), Modifiers::default()),
            Some(Event::Char('é'))
        );
    }
    #[test]
    fn pointer_motion_round_trips_without_becoming_a_click() {
        let p = Vec2::new(72, 19);
        assert_eq!(motion_position(&motion_event(p)), Some(p));
        assert_eq!(motion_position(&close_event()), None);
    }
}
