use cursive::{Cursive, CursiveRunner};
use ncspot::BIN_NAME;
use std::{cell::Cell, rc::Rc, time::Duration};

/// Honor compositor backpressure while continuing to process input and callbacks.
#[derive(Default)]
pub struct DrawPacing {
    ready: Option<Rc<Cell<bool>>>,
    pending: bool,
}

impl DrawPacing {
    #[cfg(any(feature = "native", test))]
    pub fn native(ready: Rc<Cell<bool>>) -> Self {
        Self { ready: Some(ready), pending: false }
    }

    pub fn step(&mut self, runner: &mut CursiveRunner<Cursive>) {
        let received = runner.process_events();
        self.pending |= received;
        if self.ready.as_ref().is_none_or(|ready| ready.get()) {
            runner.post_events(std::mem::take(&mut self.pending));
        } else if !received {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

pub mod album;
pub mod artist;
pub mod browse;
pub mod contextmenu;
pub mod help;
pub mod layout;
pub mod library;
pub mod listview;
pub mod modal;
pub mod navigation;
#[cfg(feature = "panels")]
pub mod native_panels;
#[cfg(feature = "panels")]
mod native_state;
pub mod pagination;
pub mod playlist;
pub mod playlists;
pub mod queue;
pub mod search;
pub mod search_results;
pub mod settings;
pub mod show;
pub mod source_filter;
pub mod statusbar;
pub mod spoti_background;
pub mod tabbedview;

#[cfg(feature = "cover")]
pub mod cover;

/// Create a CursiveRunner which implements the drawing logic and event loop.
pub fn create_cursive() -> Result<(CursiveRunner<Cursive>, DrawPacing), Box<dyn std::error::Error>> {
    #[cfg(feature = "native")]
    if std::env::args().any(|arg|arg=="--native") {
        let backend=crate::native_window::NativeBackend::new()?;
        let pacing=DrawPacing::native(backend.draw_ready());
        let mut runner=CursiveRunner::new(Cursive::new(),Box::new(backend));
        runner.set_fps(8);
        runner.add_global_callback(crate::native_window::close_event(),|s|{
            match s.user_data::<crate::application::UserData>().cloned(){Some(data)=>data.cmd.handle(s,crate::command::Command::Quit),None=>s.quit()}
        });
        // Override Cursive's default Ctrl+C quit so the playback state is saved.
        runner.set_on_pre_event(cursive::event::Event::CtrlChar('c'),|s|{
            match s.user_data::<crate::application::UserData>().cloned(){Some(data)=>data.cmd.handle(s,crate::command::Command::Quit),None=>s.quit()}
        });
        return Ok((runner,pacing));
    }
    let backend = cursive::backends::try_default()?;
    let mut cursive_runner = CursiveRunner::new(cursive::Cursive::new(), backend);

    cursive_runner.set_window_title(BIN_NAME);
    cursive_runner.set_fps(8);

    Ok((cursive_runner,DrawPacing::default()))
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    use cursive::{traits::Nameable, views::TextView};

    #[test]
    fn blocked_frames_keep_callbacks_and_pending_updates_until_resume() {
        let backend = cursive::backends::puppet::Backend::init(Some((40, 10).into()));
        let frames = backend.stream();
        let _ = frames.try_recv(); // Puppet's initial empty frame.
        let ready = Rc::new(Cell::new(false));
        let mut pacing = DrawPacing::native(ready.clone());
        let mut runner = CursiveRunner::new(Cursive::new(), backend);
        runner.add_fullscreen_layer(TextView::new("before").with_name("text"));
        runner.cb_sink().send(Box::new(|s| {
            s.call_on_name("text", |v: &mut TextView| v.set_content("after"));
        })).unwrap();
        pacing.step(&mut runner);
        assert!(frames.try_recv().is_err());
        assert_eq!(runner.call_on_name("text", |v: &mut TextView| v.get_content().source().to_owned()).unwrap(), "after");
        ready.set(true);
        pacing.step(&mut runner);
        assert!(frames.try_recv().is_ok());
        ready.set(false);
        runner.cb_sink().send(Box::new(|s| s.quit())).unwrap();
        pacing.step(&mut runner);
        assert!(!runner.is_running());
    }
}
