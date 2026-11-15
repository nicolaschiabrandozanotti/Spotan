//! Offline rendering allocation profile. No player connection or network access.
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use sha2::{Digest, Sha256};
use spoti_tools::{lyrics::App, theme::Theme, State, Track};
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell, time::Instant};

thread_local! {
    static COUNTS: Cell<(bool, u64, u64)> = const { Cell::new((false, 0, 0)) };
}
struct Allocator;
fn count(size: usize) {
    let _ = COUNTS.try_with(|counts| {
        let (enabled, calls, bytes) = counts.get();
        if enabled { counts.set((true, calls + 1, bytes + size as u64)); }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() { count(layout.size()); }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() { count(layout.size()); }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, layout, size) };
        if !result.is_null() { count(size); }
        result
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn main() {
    let canvas_mode = std::env::args().any(|arg| arg == "--canvas");
    let mut results = Vec::new();
    for width in [20, 40, 80, 132, 180] {
        let mut app = App::new();
        app.theme = Theme::default();
        app.disconnected = false;
        app.message.clear();
        app.background.enabled = false;
        app.state = State {
            mode: json!({"Paused":{"secs":90,"nanos":0}}),
            playable: Some(Track {
                kind: "Track".into(), id: "fixture".into(),
                title: "Prueba de renderizado".into(), artists: vec!["Spoti".into()],
                duration: 240000, ..Default::default()
            }),
        };
        app.rows = (0..96).map(|i| format!(
            "Línea {i}: texto inventado con palabras largas, acentos, 漢字 y símbolos ♪ e\u{301}."
        )).collect();
        app.times = (0..96).map(|i| i as f64 * 2.0).collect();
        let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
        let mut canvas = spoti_tools::ratatui_bridge::CellCanvas::default();
        let mut draw = |app: &mut App| {
            if canvas_mode { canvas.draw(app, width, 32); }
            else { terminal.draw(|f| app.draw(f)).unwrap(); }
        };
        for _ in 0..20 { draw(&mut app); }
        COUNTS.with(|c| c.set((true, 0, 0)));
        let start = Instant::now();
        let frames = 300;
        for _ in 0..frames { draw(&mut app); }
        let elapsed = start.elapsed();
        let (_, calls, bytes) = COUNTS.with(|c| { let v = c.get(); c.set((false, 0, 0)); v });
        let snapshot = if canvas_mode { format!("{:?}",canvas.buffer()) }
            else { format!("{:?}", terminal.backend().buffer()) };
        let hash = format!("{:x}", Sha256::digest(snapshot.as_bytes()));
        let buffer = if canvas_mode { canvas.buffer() } else { terminal.backend().buffer() };
        let mut visible = Sha256::new();
        for y in 0..32 {
            let mut x = 0;
            while x < width {
                let cell = &buffer[(x,y)];
                visible.update(format!("{x},{y}:{cell:?}").as_bytes());
                x += unicode_width::UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
            }
        }
        let visible_hash = format!("{:x}", visible.finalize());
        results.push(json!({"width":width,"height":32,"frames":frames,
            "allocations_per_frame":calls as f64 / frames as f64,
            "requested_bytes_per_frame":bytes as f64 / frames as f64,
            "microseconds_per_frame":elapsed.as_micros() as f64 / frames as f64,
            "cell_sha256":hash,"visible_cell_sha256":visible_hash}));
    }
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
}
