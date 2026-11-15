use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, EnableMouseCapture, DisableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    cursor::{Hide, Show},
};
use ratatui::{backend::CrosstermBackend,Terminal};
use spoti_tools::{self as api,Result,Track,theme::Theme};
#[cfg(test)]
use spoti_tools::{State,Record,controls::Action};
#[cfg(test)]
use crossterm::event::{KeyEvent,MouseEvent,MouseButton,MouseEventKind};
use std::{
    env, io,
    sync::{mpsc, Arc, atomic::{AtomicBool, Ordering}},
    thread, time::{Duration, Instant},
};

use spoti_tools::lyrics::{App,MouseResult};

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, Show, LeaveAlternateScreen);
    }
}

fn view() -> Result<()> {
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(io::stdout(), EnterAlternateScreen, Hide, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new();
    let (updates, snapshots) = mpsc::sync_channel(1);
    let (finished, lyrics) = mpsc::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    thread::Builder::new().name("ncspot-ipc".into()).spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            if updates.try_send(api::current_state().map_err(|e| e.to_string())).is_err() && stop.load(Ordering::Relaxed) { break; }
            thread::sleep(Duration::from_millis(300));
        }
    })?;
    // Commands run separately so a stalled player never freezes the view.
    let (controls, commands) = mpsc::sync_channel::<String>(8);
    thread::Builder::new().name("ncspot-controls".into()).spawn(move || {
        for command in commands { let _ = api::player_command(&command); }
    })?;
    let outcome = (|| -> Result<()> {
        let mut needs_lookup = false;
        let mut pending = false;
        let mut last_theme = Instant::now();
        let mut last_draw=Instant::now()-Duration::from_secs(1);
        let mut input_dirty=true;
        loop {
            while let Ok(snapshot) = snapshots.try_recv() {
                match snapshot {
                    Ok(state) => { app.state = state; app.disconnected = false; }
                    Err(_) => { app.disconnected = true; }
                }
            }
            let key = app.state.playable.as_ref().map_or(String::new(), Track::key);
            if key != app.active { app.active = key; app.accept_track(); needs_lookup = true; }
            while let Ok((key,result)) = lyrics.try_recv() {
                pending = false;
                if key == app.active { app.accept_lyrics(result); needs_lookup = false; }
            }
            if !pending && needs_lookup {
                if let Some(track) = app.state.playable.as_ref().filter(|t| t.kind == "Track") {
                    needs_lookup = false; pending = true;
                    let track = track.clone(); let send = finished.clone(); let key = app.active.clone();
                    thread::Builder::new().name("lrclib".into()).spawn(move || {
                        let result = api::lookup(&track).map_err(|e| e.to_string()); let _ = send.send((key,result));
                    })?;
                }
            }
            if last_theme.elapsed() >= Duration::from_secs(1) { app.theme = Theme::read(); last_theme = Instant::now(); }
            if last_draw.elapsed()>=Duration::from_millis(125) || input_dirty && last_draw.elapsed()>=Duration::from_millis(33) {
                terminal.draw(|frame| app.draw(frame))?;last_draw=Instant::now();input_dirty=false;
            }
            let interval=Duration::from_millis(if input_dirty {33}else{125});
            let wait=interval.saturating_sub(last_draw.elapsed());
            if event::poll(wait)? {
                input_dirty=true;
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if matches!(key.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc)
                            || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) { break; }
                        let height = terminal.size()?.height.saturating_sub(10) as usize;
                        if let Some(command) = app.key(key,height.max(1)) { let _ = controls.try_send(command.to_owned()); }
                    }
                    Event::Mouse(mouse) => match app.mouse(mouse) {
                        MouseResult::Command(command)=>{let _=controls.try_send(command);}
                        MouseResult::Close=>break,
                        MouseResult::None=>(),
                    },
                    _ => (),
                }
            }
        }
        Ok(())
    })();
    stopped.store(true, Ordering::Relaxed);
    outcome
}

fn main() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("--view") => view(),
        Some("--check") => {
            let state = api::current_state()?;
            let track = state.playable.filter(|t| t.kind == "Track").ok_or("Poné una canción en ncspot")?;
            let record = api::lookup(&track)?;
            let timeline = api::parse_lrc(record.as_ref().and_then(|r| r.synced_lyrics.as_deref()).unwrap_or(""));
            println!("{} — {}\nLRCLIB: {}; sincronización: {} líneas", api::clean(&track.title),api::clean(&track.artists.join(", ")),
                if record.is_some() { "encontrada" } else { "sin resultado" },timeline.len());
            Ok(())
        }
        Some("--version") => { println!("spoti-lyrics {} (Rust)",env!("CARGO_PKG_VERSION")); Ok(()) }
        _ => { println!("Shift + L en ncspot abre el panel.\nspoti-lyrics --view | --check | --version"); Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    #[test]
    fn responsive_view_paints_the_entire_ncspot_background() {
        for (width,height) in [(1,1),(19,8),(20,11),(30,12),(80,35),(150,45)] {
            let mut terminal=Terminal::new(TestBackend::new(width,height)).unwrap();
            let mut app=App::new();
            app.theme=Theme::default();
            app.state=State { mode:json!({"Paused":{"secs":12}}),playable:Some(Track {title:"Prueba".into(),duration:30000,..Default::default()}) };
            app.rows=vec!["Primer verso".into(),"línea muy larga ".repeat(20),"tercera línea".into()];
            app.times=vec![0.0,10.0,20.0]; app.message.clear();
            terminal.draw(|frame|app.draw(frame)).unwrap();
            let buffer=terminal.backend().buffer();
            let mut selected=false;
            for y in 0..height { for x in 0..width {
                let cell=&buffer[(x,y)];
                assert!([app.theme.background,app.theme.highlight_bg,app.theme.statusbar_bg].contains(&cell.bg));
                selected |= cell.bg==app.theme.highlight_bg;
            }}
            if height>12 && width>=30 {assert!(selected);}
        }
    }

    #[test]
    fn song_changes_reset_lyrics_and_manual_offset() {
        let mut app=App::new(); app.rows.push("old".into());app.times.push(0.0);app.offset=3.0;app.follow=false;app.scroll=100;
        app.accept_track();assert!(app.rows.is_empty());assert!(app.times.is_empty());assert_eq!(app.offset,0.0);assert!(app.follow);assert_eq!(app.scroll,0);
        app.accept_lyrics(Ok(Some(Record{plain_lyrics:Some("sin tiempos".into()),..Default::default()})));
        assert_eq!(app.rows,vec!["sin tiempos"]);assert!(app.times.is_empty());
    }

    #[test]
    fn controls_and_manual_scrolling_stay_available() {
        let mut app=App::new();
        assert_eq!(app.key(KeyEvent::new(KeyCode::Char(' '),KeyModifiers::NONE),10),Some("playpause"));
        assert_eq!(app.key(KeyEvent::new(KeyCode::Right,KeyModifiers::NONE),10),Some("seek +5s"));
        app.key(KeyEvent::new(KeyCode::Down,KeyModifiers::NONE),10);assert!(!app.follow);assert_eq!(app.scroll,1);
        app.key(KeyEvent::new(KeyCode::Char('f'),KeyModifiers::NONE),10);assert!(app.follow);
    }

    #[test]
    fn background_toggle_preserves_the_active_lyric_and_controls() {
        let mut terminal=Terminal::new(TestBackend::new(80,35)).unwrap();
        let mut app=App::new();
        app.state=State { mode:json!({"Paused":{"secs":12}}),..Default::default() };
        app.rows=vec!["Earlier".into(),"Active lyric".into(),"Later".into()];
        app.times=vec![0.0,10.0,20.0];app.message.clear();app.disconnected=false;
        terminal.draw(|frame|app.draw(frame)).unwrap();
        let animated=terminal.backend().buffer().clone();
        assert!(animated.content.iter().any(|cell| cell.symbol().chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))));
        app.key(KeyEvent::new(KeyCode::Char('v'),KeyModifiers::NONE),10);
        assert!(!app.background.enabled);
        terminal.draw(|frame|app.draw(frame)).unwrap();
        let plain=terminal.backend().buffer();
        assert!(!plain.content.iter().any(|cell| cell.symbol().chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))));
        for y in 0..app.progress.y {for x in 0..80 {
            if animated[(x,y)].bg==app.theme.highlight_bg { assert_eq!(animated[(x,y)],plain[(x,y)]); }
        }}
        assert_eq!(app.key(KeyEvent::new(KeyCode::Char(' '),KeyModifiers::NONE),10),Some("playpause"));
        assert_eq!(app.times,vec![0.0,10.0,20.0]);
    }
}


#[cfg(test)]
mod mouse_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use serde_json::json;
    fn mouse(kind:MouseEventKind,column:u16,row:u16)->MouseEvent {MouseEvent {kind,column,row,modifiers:KeyModifiers::NONE}}
    #[test]
    fn mouse_controls_dispatch_once_and_seek_across_the_entire_track() {
        let mut app=App::new();app.state=State {mode:json!({"Paused":{"secs":12}}),playable:Some(Track {duration:120000,..Default::default()})};
        let mut terminal=Terminal::new(TestBackend::new(80,35)).unwrap();terminal.draw(|frame|app.draw(frame)).unwrap();
        let pause=app.buttons.iter().find(|button|button.action==Action::Player("playpause")).unwrap().area;
        assert_eq!(app.mouse(mouse(MouseEventKind::Down(MouseButton::Left),pause.x,pause.y)),MouseResult::Command("playpause".into()));
        assert_eq!(app.mouse(mouse(MouseEventKind::Up(MouseButton::Left),pause.x,pause.y)),MouseResult::None);
        let bar=app.progress;
        assert_eq!(app.mouse(mouse(MouseEventKind::Down(MouseButton::Left),bar.x,bar.y)),MouseResult::Command("seek 0ms".into()));
        assert_eq!(app.mouse(mouse(MouseEventKind::Up(MouseButton::Left),bar.right()-1,bar.y)),MouseResult::Command("seek 120000ms".into()));
        let background=app.buttons.iter().find(|button|button.action==Action::Background).unwrap().area;
        let previous=app.background.enabled;app.mouse(mouse(MouseEventKind::Down(MouseButton::Left),background.x,background.y));assert_ne!(app.background.enabled,previous);
        app.mouse(mouse(MouseEventKind::ScrollDown,20,10));assert!(!app.follow);
        let follow=app.buttons.iter().find(|button|button.action==Action::Follow).unwrap().area;
        app.mouse(mouse(MouseEventKind::Down(MouseButton::Left),follow.x,follow.y));assert!(app.follow);
    }
}
