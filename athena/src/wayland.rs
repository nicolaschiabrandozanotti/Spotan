//! Linux adapter: layer-shell + bounded SHM buffers, no GTK, GLib or D-Bus.
use crate::model::{self, Model, Rect, Target, CLOSED_HEIGHT, OPEN_HEIGHT, WIDTH};
use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{calloop::EventLoop, calloop_wayland_source::WaylandSource},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{
            CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
        },
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use spoti_control::{self as api, Action, Snapshot};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, QueueHandle,
};

enum Request {
    Refresh,
    Action(Action),
}
fn worker(
    socket: PathBuf,
    requests: mpsc::Receiver<Request>,
    updates: mpsc::SyncSender<Option<Snapshot>>,
) {
    while let Ok(first) = requests.recv() {
        let mut batch = vec![first];
        batch.extend(requests.try_iter());
        // Coalesce consecutive drags while preserving transport-button ordering.
        let mut actions: Vec<Action> = vec![];
        for r in batch {
            if let Request::Action(action) = r {
                let same = matches!(
                    (actions.last(), action),
                    (Some(Action::Volume(_)), Action::Volume(_))
                        | (Some(Action::Seek(_)), Action::Seek(_))
                );
                if same {
                    actions.pop();
                }
                actions.push(action);
            }
        }
        for a in actions {
            if let Err(e) = api::unix::command(&socket, &a.command()) {
                eprintln!("Spoti control: {e}");
            }
        }
        let _ = updates.try_send(api::unix::snapshot(&socket).ok());
    }
}
#[derive(Clone, PartialEq)]
struct Theme {
    surface: Color,
    bright: Color,
    text: Color,
    secondary: Color,
    primary: Color,
    on_primary: Color,
    outline: Color,
}
type Color = (f64, f64, f64);
impl Theme {
    fn load(path: &PathBuf) -> Self {
        let data = fs::read_to_string(path).unwrap_or_default();
        let color = |name: &str, fallback: &str| {
            let value = data
                .lines()
                .find_map(|line| {
                    let mut w = line.split_whitespace();
                    if w.next() != Some("@define-color") || w.next() != Some(name) {
                        return None;
                    }
                    w.next().map(|v| v.trim_end_matches(';'))
                })
                .unwrap_or(fallback);
            let hex = value
                .strip_prefix('#')
                .filter(|v| v.len() == 6)
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .unwrap_or_else(|| u32::from_str_radix(&fallback[1..], 16).unwrap());
            (
                ((hex >> 16) & 255) as f64 / 255.0,
                ((hex >> 8) & 255) as f64 / 255.0,
                (hex & 255) as f64 / 255.0,
            )
        };
        Self {
            surface: color("surface_container", "#1b2023"),
            bright: color("surface_container_highest", "#303638"),
            text: color("on_surface", "#dee3e6"),
            secondary: color("on_surface_variant", "#bfc8cc"),
            primary: color("primary", "#88d1ec"),
            on_primary: color("on_primary", "#003544"),
            outline: color("outline_variant", "#40484c"),
        }
    }
}
struct Options {
    socket: PathBuf,
    theme: PathBuf,
    top: i32,
    offset: i32,
    output: Option<String>,
    preview: bool,
}
impl Options {
    fn parse() -> api::Result<Self> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        let mut o = Self {
            socket: api::unix::default_socket(),
            theme: config.join("waybar/tokens/colors.css"),
            top: 34,
            offset: -24,
            output: None,
            preview: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => o.socket = args.next().ok_or("Missing socket path")?.into(),
                "--theme" => o.theme = args.next().ok_or("Missing theme path")?.into(),
                "--top" => o.top = args.next().ok_or("Missing top margin")?.parse()?,
                "--offset" => o.offset = args.next().ok_or("Missing horizontal offset")?.parse()?,
                "--output" => o.output = Some(args.next().ok_or("Missing output name")?),
                "--preview" => o.preview = true,
                "--help" | "-h" => {
                    println!("spoti-popover [--socket PATH] [--theme CSS] [--top 34] [--offset -24] [--output NAME]\nOptional Linux Wayland layer-shell controls. --preview starts open for testing.");
                    std::process::exit(0);
                }
                _ => return Err(format!("Unknown option: {arg}").into()),
            }
        }
        Ok(o)
    }
}
struct State {
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    compositor: CompositorState,
    shm: Shm,
    layer: LayerSurface,
    pointer: Option<ThemedPointer>,
    selected_output: Option<wl_output::WlOutput>,
    pool: SlotPool,
    buffers: Vec<Buffer>,
    dimensions: (u32, u32, i32),
    width: u32,
    height: u32,
    scale: i32,
    configured: bool,
    frame_ready: bool,
    dirty: bool,
    exit: bool,
    model: Model,
    close_at: Option<Instant>,
    animation: Option<Instant>,
    sender: mpsc::Sender<Request>,
    updates: mpsc::Receiver<Option<Snapshot>>,
    last_poll: Instant,
    last_action: Instant,
    last_theme: Instant,
    last_progress: Instant,
    theme: Theme,
    options: Options,
}
pub fn run() -> api::Result<()> {
    let options = Options::parse()?;
    memory_setup();
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let mut event_loop = EventLoop::try_new()?;
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let outputs = OutputState::new(&globals, &qh);
    let selected_output = None;
    let layer = shell.create_layer_surface(
        &qh,
        compositor.create_surface(&qh),
        Layer::Overlay,
        Some("spoti-player"),
        selected_output.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_margin(options.top, 0, 0, options.offset * 2);
    layer.set_size(
        WIDTH,
        if options.preview {
            OPEN_HEIGHT
        } else {
            CLOSED_HEIGHT
        },
    );
    let pool = SlotPool::new((WIDTH * CLOSED_HEIGHT * 4) as usize, &shm)?;
    let (sender, requests) = mpsc::channel();
    let (updates, receiver) = mpsc::sync_channel(1);
    let socket = options.socket.clone();
    thread::Builder::new()
        .name("spoti-athena-ipc".into())
        .stack_size(128 * 1024)
        .spawn(move || worker(socket, requests, updates))?;
    let mut model = Model::default();
    model.open = options.preview;
    let height = if model.open {
        OPEN_HEIGHT
    } else {
        CLOSED_HEIGHT
    };
    let theme = Theme::load(&options.theme);
    let mut state = State {
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        outputs,
        compositor,
        shm,
        layer,
        pointer: None,
        selected_output,
        pool,
        buffers: vec![],
        dimensions: (0, 0, 0),
        width: WIDTH,
        height,
        scale: 1,
        configured: false,
        frame_ready: true,
        dirty: true,
        exit: false,
        model,
        close_at: None,
        animation: None,
        sender,
        updates: receiver,
        last_poll: Instant::now() - Duration::from_secs(2),
        last_action: Instant::now() - Duration::from_secs(2),
        last_theme: Instant::now(),
        last_progress: Instant::now(),
        theme,
        options,
    };
    queue.roundtrip(&mut state)?;
    let output = state
        .outputs
        .outputs()
        .find(|out| {
            state.options.output.as_ref().is_none_or(|name| {
                state
                    .outputs
                    .info(out)
                    .is_some_and(|info| info.name.as_ref() == Some(name))
            })
        })
        .ok_or("Requested Wayland output was not found")?;
    state.layer = shell.create_layer_surface(
        &qh,
        state.compositor.create_surface(&qh),
        Layer::Overlay,
        Some("spoti-player"),
        Some(&output),
    );
    state.selected_output = Some(output);
    state.layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    state
        .layer
        .set_keyboard_interactivity(KeyboardInteractivity::None);
    state.layer.set_exclusive_zone(-1);
    state.layer.set_size(WIDTH, state.height);
    state.place();
    WaylandSource::new(conn, queue).insert(event_loop.handle())?;
    state.input_region(&qh)?;
    state.layer.commit();
    while !state.exit {
        // Closed panel sleeps in the Wayland event loop; it never polls the player.
        let busy = state.model.open || state.close_at.is_some();
        event_loop.dispatch(
            Some(Duration::from_millis(if busy { 32 } else { 2000 })),
            &mut state,
        )?;
        state.tick(&qh)?;
    }
    Ok(())
}
impl State {
    fn place(&mut self) {
        if let Some(info) = self
            .selected_output
            .as_ref()
            .and_then(|out| self.outputs.info(out))
        {
            let width = info.logical_size.map(|s| s.0).or_else(|| {
                info.modes
                    .iter()
                    .find(|m| m.current)
                    .map(|m| m.dimensions.0 / info.scale_factor.max(1))
            });
            if let Some(width) = width {
                self.layer.set_margin(
                    self.options.top,
                    0,
                    0,
                    ((width - WIDTH as i32) / 2 + self.options.offset).max(0),
                );
            }
        }
    }
    fn input_region(&self, _qh: &QueueHandle<Self>) -> api::Result<()> {
        let region = Region::new(&self.compositor)?;
        if self.model.open {
            region.add(0, 0, WIDTH as i32, OPEN_HEIGHT as i32);
        } else {
            region.add(model::ARROW.0 as i32, 0, model::ARROW.2 as i32, 18);
        }
        self.layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
        Ok(())
    }
    fn change_open(&mut self, previous: bool, qh: &QueueHandle<Self>) -> api::Result<()> {
        if previous == self.model.open {
            return Ok(());
        }
        self.height = if self.model.open {
            OPEN_HEIGHT
        } else {
            CLOSED_HEIGHT
        };
        self.layer.set_size(WIDTH, self.height);
        self.input_region(qh)?;
        if self.model.open {
            self.last_poll = Instant::now() - Duration::from_secs(2);
            self.animation = Some(Instant::now());
        } else {
            self.animation = None;
            self.buffers.clear();
            self.pool = SlotPool::new((WIDTH * CLOSED_HEIGHT * 4) as usize, &self.shm)?;
            memory_trim();
        }
        self.dirty = true;
        self.layer.commit();
        Ok(())
    }
    fn send(&mut self, action: Option<Action>) {
        if let Some(action) = action {
            let _ = self.sender.send(Request::Action(action));
            self.last_action = Instant::now();
            self.last_poll = Instant::now();
        }
    }
    fn tick(&mut self, qh: &QueueHandle<Self>) -> api::Result<()> {
        if self.close_at.is_some_and(|t| Instant::now() >= t) && self.model.drag.is_none() {
            self.close_at = None;
            let old = self.model.open;
            self.model.close();
            self.change_open(old, qh)?;
        }
        while let Ok(snapshot) = self.updates.try_recv() {
            if self.last_action.elapsed() > Duration::from_millis(200) {
                self.model.apply(snapshot);
                self.dirty = true;
            }
        }
        if self.model.open && self.last_poll.elapsed() >= Duration::from_millis(500) {
            let _ = self.sender.send(Request::Refresh);
            self.last_poll = Instant::now();
        }
        if self.last_theme.elapsed() >= Duration::from_secs(2) {
            let theme = Theme::load(&self.options.theme);
            if theme != self.theme {
                self.theme = theme;
                self.dirty = true;
            }
            self.last_theme = Instant::now();
        }
        let interval = if self.animation.is_some() {
            Duration::from_millis(32)
        } else {
            Duration::from_millis(125)
        };
        if self.model.open
            && (self.model.playing() || self.animation.is_some())
            && self.last_progress.elapsed() >= interval
        {
            self.dirty = true;
            self.last_progress = Instant::now();
        }
        if self
            .animation
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(160))
        {
            self.animation = None;
        }
        self.draw(qh)
    }
    fn draw(&mut self, qh: &QueueHandle<Self>) -> api::Result<()> {
        if !self.configured || !self.frame_ready || !self.dirty {
            return Ok(());
        }
        let dimensions = (self.width, self.height, self.scale);
        let w = self.width as i32 * self.scale;
        let h = self.height as i32 * self.scale;
        if self.dimensions != dimensions {
            self.buffers.clear();
            self.pool = SlotPool::new((w * h * 4) as usize, &self.shm)?;
            self.dimensions = dimensions;
        }
        let index = if let Some(i) = self
            .buffers
            .iter()
            .position(|b| self.pool.canvas(b).is_some())
        {
            i
        } else if self.buffers.len() < 2 {
            let (b, _) = self
                .pool
                .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)?;
            self.buffers.push(b);
            self.buffers.len() - 1
        } else {
            return Ok(());
        };
        if let Some(canvas) = self.pool.canvas(&self.buffers[index]) {
            // SAFETY: Cairo and its context are dropped before the SHM canvas is attached.
            let surface = unsafe {
                ImageSurface::create_for_data_unsafe(
                    canvas.as_mut_ptr(),
                    Format::ARgb32,
                    w,
                    h,
                    w * 4,
                )?
            };
            let ctx = Context::new(&surface)?;
            ctx.scale(self.scale as f64, self.scale as f64);
            let reveal = self
                .animation
                .map_or(1.0, |t| (t.elapsed().as_secs_f64() / 0.16).clamp(0.0, 1.0));
            render(&ctx, &self.model, &self.theme, reveal)?;
        }
        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, w, h);
        surface.frame(qh, surface.clone());
        self.buffers[index].attach_to(surface)?;
        self.layer.commit();
        self.frame_ready = false;
        self.dirty = false;
        Ok(())
    }
}
fn source(ctx: &Context, c: Color) {
    ctx.set_source_rgb(c.0, c.1, c.2);
}
fn rounded(ctx: &Context, r: Rect, radius: f64) {
    let Rect(x, y, w, h) = r;
    let r = radius.min(w / 2.0).min(h / 2.0);
    let pi = std::f64::consts::PI;
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    ctx.arc(x + r, y + h - r, r, pi / 2.0, pi);
    ctx.arc(x + r, y + r, r, pi, pi * 1.5);
    ctx.close_path();
}
fn text(
    ctx: &Context,
    text: &str,
    x: f64,
    baseline: f64,
    width: f64,
    size: f64,
    bold: bool,
    color: Color,
) -> api::Result<()> {
    ctx.select_font_face(
        "JetBrainsMono Nerd Font",
        FontSlant::Normal,
        if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        },
    );
    ctx.set_font_size(size);
    let mut text: String = text.chars().filter(|c| !c.is_control()).take(256).collect();
    while ctx.text_extents(&text)?.x_advance() > width {
        text.pop();
        if text.is_empty() {
            break;
        }
    }
    // Clip long labels to their own row; metadata never paints on controls.
    source(ctx, color);
    let ext = ctx.text_extents(&text)?;
    ctx.move_to(x + (width - ext.x_advance()) / 2.0, baseline);
    ctx.show_text(&text)?;
    Ok(())
}
fn line(ctx: &Context, points: &[(f64, f64)]) -> api::Result<()> {
    if let Some(&(x, y)) = points.first() {
        ctx.move_to(x, y);
        for &(x, y) in &points[1..] {
            ctx.line_to(x, y);
        }
        ctx.stroke()?;
    }
    Ok(())
}
fn slider(ctx: &Context, rect: Rect, value: f64, t: &Theme) -> api::Result<()> {
    let Rect(x, y, w, h) = rect;
    let cy = y + h / 2.0;
    rounded(ctx, Rect(x, cy - 2.0, w, 4.0), 2.0);
    source(ctx, t.bright);
    ctx.fill()?;
    rounded(ctx, Rect(x, cy - 2.0, w * value.clamp(0.0, 1.0), 4.0), 2.0);
    source(ctx, t.primary);
    ctx.fill()?;
    ctx.arc(
        x + w * value.clamp(0.0, 1.0),
        cy,
        4.0,
        0.0,
        std::f64::consts::TAU,
    );
    ctx.fill()?;
    Ok(())
}
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
fn render(ctx: &Context, m: &Model, t: &Theme, reveal: f64) -> api::Result<()> {
    ctx.set_operator(cairo::Operator::Clear);
    ctx.paint()?;
    ctx.set_operator(cairo::Operator::Over);
    rounded(ctx, Rect(194.0, -8.0, 48.0, 24.0), 12.0);
    source(
        ctx,
        if m.hover == Some(Target::Arrow) {
            t.bright
        } else {
            t.surface
        },
    );
    ctx.fill()?;
    source(ctx, t.primary);
    ctx.set_line_width(1.5);
    ctx.set_line_cap(cairo::LineCap::Round);
    if m.open {
        line(ctx, &[(214.0, 10.0), (218.0, 6.0), (222.0, 10.0)])?;
    } else {
        line(ctx, &[(214.0, 6.0), (218.0, 10.0), (222.0, 6.0)])?;
    }
    if !m.open {
        return Ok(());
    }
    ctx.save()?;
    ctx.rectangle(0.0, 20.0, WIDTH as f64, 128.0 * reveal);
    ctx.clip();
    rounded(ctx, Rect(4.5, 21.5, 427.0, 121.0), 18.0);
    source(ctx, t.surface);
    ctx.fill_preserve()?;
    source(ctx, t.outline);
    ctx.set_line_width(1.0);
    ctx.stroke()?;
    let track = m.snapshot.as_ref().and_then(|s| s.state.playable.as_ref());
    let title = track.map_or(
        if m.snapshot.is_some() {
            "Spoti sin canción"
        } else {
            "Spoti no está abierto"
        },
        |t| t.title.as_str(),
    );
    let artist = track.map_or_else(
        || "Elegí un tema en tu biblioteca".into(),
        |t| t.artists.join(", "),
    );
    text(ctx, title, 20.0, 47.0, 396.0, 12.0, true, t.text)?;
    text(ctx, &artist, 20.0, 65.0, 396.0, 11.0, false, t.secondary)?;
    let position = m.position(api::now());
    let duration = m.duration();
    text(
        ctx,
        &clock(position),
        18.0,
        91.0,
        39.0,
        11.0,
        false,
        t.secondary,
    )?;
    text(
        ctx,
        &clock(duration),
        380.0,
        91.0,
        38.0,
        11.0,
        false,
        t.secondary,
    )?;
    slider(
        ctx,
        model::SEEK,
        if duration > 0.0 {
            position / duration
        } else {
            0.0
        },
        t,
    )?;
    for (x, w, target) in [
        (22.0, 40.0, Target::Previous),
        (66.0, 41.0, Target::Backward),
        (111.0, 50.0, Target::Play),
        (165.0, 41.0, Target::Forward),
        (210.0, 38.0, Target::Next),
        (252.0, 34.0, Target::Mute),
    ] {
        if target == Target::Play || m.hover == Some(target) {
            rounded(ctx, Rect(x, 101.0, w, 28.0), 14.0);
            source(
                ctx,
                if target == Target::Play {
                    t.primary
                } else {
                    t.bright
                },
            );
            ctx.fill()?;
        }
    }
    text(ctx, "−10s", 66.0, 119.0, 41.0, 11.0, false, t.text)?;
    text(ctx, "+10s", 165.0, 119.0, 41.0, 11.0, false, t.text)?;
    source(ctx, t.text);
    ctx.set_line_width(1.7);
    for (x, sign) in [(42.0, -1.0), (230.0, 1.0)] {
        line(ctx, &[(x, 109.0), (x, 121.0)])?;
        ctx.move_to(x - sign * 2.0, 115.0);
        ctx.line_to(x - sign * 10.0, 109.0);
        ctx.line_to(x - sign * 10.0, 121.0);
        ctx.close_path();
        ctx.fill()?;
    }
    source(ctx, t.on_primary);
    if m.playing() {
        ctx.rectangle(130.0, 109.0, 4.0, 12.0);
        ctx.rectangle(138.0, 109.0, 4.0, 12.0);
        ctx.fill()?;
    } else {
        ctx.move_to(132.0, 108.0);
        ctx.line_to(132.0, 122.0);
        ctx.line_to(143.0, 115.0);
        ctx.close_path();
        ctx.fill()?;
    }
    source(ctx, t.text);
    ctx.rectangle(257.0, 112.0, 4.0, 6.0);
    ctx.fill()?;
    ctx.move_to(260.0, 112.0);
    ctx.line_to(266.0, 108.0);
    ctx.line_to(266.0, 122.0);
    ctx.line_to(260.0, 118.0);
    ctx.close_path();
    ctx.fill()?;
    if m.volume().unwrap_or(0.0) <= 0.001 {
        line(ctx, &[(270.0, 112.0), (276.0, 118.0)])?;
        line(ctx, &[(270.0, 118.0), (276.0, 112.0)])?;
    } else {
        ctx.arc(265.0, 115.0, 8.0, -0.8, 0.8);
        ctx.stroke()?;
    }
    slider(ctx, model::VOLUME, m.volume().unwrap_or(0.0), t)?;
    let volume = m
        .volume()
        .map_or_else(|| "—".into(), |v| format!("{:.0}%", v * 100.0));
    text(ctx, &volume, 382.0, 119.0, 36.0, 11.0, false, t.secondary)?;
    ctx.restore()?;
    Ok(())
}
impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        self.scale = factor.max(1);
        self.dirty = true;
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_ready = true;
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
    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.selected_output.as_ref() == Some(&output) {
            self.place();
            self.layer.commit();
        }
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.selected_output.as_ref() == Some(&output) {
            self.exit = true;
        }
    }
}
impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        c: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.width = if c.new_size.0 == 0 {
            WIDTH
        } else {
            c.new_size.0
        };
        self.height = if c.new_size.1 == 0 {
            self.height
        } else {
            c.new_size.1
        };
        self.configured = true;
        self.dirty = true;
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
        if cap == Capability::Pointer {
            self.pointer = None;
            self.close_at = Some(Instant::now());
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}
impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            let (x, y) = event.position;
            let old = self.model.open;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.close_at = None;
                    if matches!(event.kind, PointerEventKind::Enter { .. })
                        && !self.model.open
                        && model::ARROW.contains(x, y)
                    {
                        self.model.open = true;
                    }
                    let a = self.model.motion(x, y);
                    self.send(a);
                    if let Some(p) = &self.pointer {
                        let _ = p.set_cursor(
                            conn,
                            if self.model.hover.is_some() {
                                CursorIcon::Pointer
                            } else {
                                CursorIcon::Default
                            },
                        );
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.model.hover = None;
                    self.close_at = Some(Instant::now() + Duration::from_millis(240));
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    let a = self.model.press(x, y);
                    self.send(a);
                }
                PointerEventKind::Release { button: 0x110, .. } => {
                    let a = self.model.motion(x, y);
                    self.send(a);
                    let a = self.model.release(x, y);
                    self.send(a);
                }
                _ => (),
            }
            if let Err(e) = self.change_open(old, qh) {
                eprintln!("Spoti surface: {e}");
                self.exit = true;
            }
            self.dirty = true;
        }
    }
}
impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_compositor!(State);
delegate_layer!(State);
delegate_output!(State);
delegate_pointer!(State);
delegate_registry!(State);
delegate_seat!(State);
delegate_shm!(State);
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

#[cfg(target_env = "gnu")]
fn memory_setup() {
    unsafe {
        mallopt(-8, 1);
        mallopt(-1, 128 * 1024);
    }
}
#[cfg(not(target_env = "gnu"))]
fn memory_setup() {}
#[cfg(target_env = "gnu")]
fn memory_trim() {
    unsafe {
        malloc_trim(0);
    }
}
#[cfg(not(target_env = "gnu"))]
fn memory_trim() {}
#[cfg(target_env = "gnu")]
extern "C" {
    fn mallopt(param: i32, value: i32) -> i32;
    fn malloc_trim(pad: usize) -> i32;
}
