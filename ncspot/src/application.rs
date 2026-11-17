use std::error::Error;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use cursive::traits::Nameable;
use cursive::{Cursive, CursiveRunner};
use librespot_core::authentication::Credentials;
use log::{error, info, trace};

#[cfg(unix)]
use signal_hook::{consts::SIGHUP, consts::SIGTERM, iterator::Signals};

use crate::command::Command;
use crate::commands::CommandManager;
use crate::config::{Config, PlaybackState};
use crate::external;
use crate::events::{Event, EventManager};
use crate::library::Library;
use crate::queue::Queue;
use crate::spotify::{PlayerEvent, Spotify};
use crate::ui::create_cursive;
use crate::{authentication, ui, utils};
use crate::{command, queue, spotify};

#[cfg(feature = "mpris")]
use crate::mpris::MprisManager;

#[cfg(unix)]
use crate::ipc::{self, IpcSocket};

/// Set up the global logger to log to `filename`.
pub fn setup_logging(filename: &Path) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        // Perform allocation-free log formatting
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        // Add blanket level filter -
        .level(log::LevelFilter::Debug)
        // Set runtime log level for modules
        .level_for("ncspot", log::LevelFilter::Trace)
        // Output to stdout, files, and other Dispatch configurations
        .chain(fern::log_file(filename)?)
        // Apply globally
        .apply()?;
    Ok(())
}

pub type UserData = Rc<UserDataInner>;
pub struct UserDataInner {
    pub cmd: CommandManager,
}

/// The global Tokio runtime for running asynchronous tasks.
pub static ASYNC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// How the app starts, given the cached Spotify credentials and the enabled external services.
#[derive(Debug, PartialEq, Eq)]
enum Startup {
    Online,
    /// No Spotify, but YouTube/SoundCloud are on: skip the start screen.
    Offline,
    StartScreen,
}

fn startup_mode(has_credentials: bool, services: &[crate::model::track::Source]) -> Startup {
    match (has_credentials, services.is_empty()) {
        (true, _) => Startup::Online,
        (false, false) => Startup::Offline,
        (false, true) => Startup::StartScreen,
    }
}

/// Standalone login screen shown when there are no (valid) cached credentials. Returns the
/// credentials once they have connected to Spotify, or `None` for "Entrar sin Spotify".
fn start_screen(
    configuration: &Arc<Config>,
    theme: &cursive::theme::Theme,
    initial_error: Option<String>,
) -> Result<Option<Credentials>, String> {
    let (mut cursive, mut pacing) = create_cursive().map_err(|e| e.to_string())?;
    cursive.set_theme(theme.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let sink = cursive.cb_sink().clone();
    let configuration = configuration.clone();
    let view = ui::settings::SettingsView::new(
        ui::settings::SettingsMode::Start,
        None,
        Box::new(move |credentials| {
            let _ = sink.send(Box::new(|s| {
                s.call_on_name(ui::settings::MSG, |v: &mut cursive::views::TextView| {
                    v.set_content(format!("Conectando{}", crate::utils::glyph("…", "...")))
                });
            }));
            Spotify::test_credentials(&configuration, credentials.clone()).map_err(|e| e.to_string())?;
            let _ = tx.send(credentials);
            let _ = sink.send(Box::new(|s| s.quit()));
            Ok(())
        }),
        // The Library is created later and loads SoundCloud by itself.
        Box::new(|_| ()),
    );
    cursive.add_fullscreen_layer(view);
    if let Some(e) = initial_error {
        ui::settings::set_message(&mut cursive, e);
    }
    while cursive.is_running() {
        pacing.step(&mut cursive);
    }
    // Closed without logging in: leave quietly once the terminal is restored.
    if let Ok(credentials) = rx.try_recv() {
        return Ok(Some(credentials));
    }
    if cursive.take_user_data::<ui::settings::EnterWithoutSpotify>().is_some() {
        return Ok(None);
    }
    drop(cursive);
    std::process::exit(0)
}

/// The representation of an ncspot application.
pub struct Application {
    /// The music queue which controls playback order.
    queue: Arc<Queue>,
    /// Internally shared
    spotify: Spotify,
    /// Internally shared
    event_manager: EventManager,
    /// An IPC implementation using the D-Bus MPRIS protocol, used to control and inspect ncspot.
    #[cfg(unix)]
    ipc: Option<IpcSocket>,
    /// The object to render to the terminal.
    cursive: CursiveRunner<Cursive>,
    draw_pacing: ui::DrawPacing,
}

impl Application {
    /// Create a new ncspot application.
    ///
    /// # Arguments
    ///
    /// * `configuration_file_path` - Relative path to the configuration file inside the base path
    pub fn new(configuration_file_path: Option<String>) -> Result<Self, Box<dyn Error>> {
        // Things here may cause the process to abort; we must do them before creating curses
        // windows otherwise the error message will not be seen by a user

        ASYNC_RUNTIME
            .set(
                tokio::runtime::Builder::new_multi_thread()
                    // Network tasks are asynchronous; a worker per CPU retains
                    // unnecessary thread stacks and allocator arenas on desktops.
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let configuration = Arc::new(Config::new(configuration_file_path));
        let theme = configuration.build_theme();

        let mut start_error = None;
        let mut credentials = authentication::get_credentials()?;
        if let Some(c) = credentials.clone() {
            // Cursive is not drawn yet, so the console is still ours.
            println!("Conectando a Spotify{}", crate::utils::glyph("…", "..."));
            if let Err(e) = Spotify::test_credentials(&configuration, c) {
                error!("Cached credentials rejected: {e}");
                start_error = Some(format!("Error: {e}"));
                credentials = None;
            }
        }
        let mut offline_notice = None;
        let credentials = match startup_mode(credentials.is_some(), &crate::external::enabled()) {
            Startup::Online => credentials,
            Startup::Offline => {
                // rejected credentials: tell the user once the UI is up
                offline_notice = start_error.as_ref().map(|e| {
                    let e = e.strip_prefix("Error: ").unwrap_or(e);
                    format!("Spotify: no se pudo conectar — {e}; seguís con YouTube/SoundCloud")
                });
                None
            }
            Startup::StartScreen => start_screen(&configuration, &theme, start_error)?,
        };

        // DON'T USE STDOUT AFTER THIS CALL!
        let (mut cursive, draw_pacing) = create_cursive().map_err(|error| error.to_string())?;

        cursive.set_theme(theme.clone());

        #[cfg(all(unix, feature = "pancurses_backend"))]
        cursive.add_global_callback(cursive::event::Event::CtrlChar('z'), |_s| unsafe {
            libc::raise(libc::SIGTSTP);
        });

        let event_manager = EventManager::new(cursive.cb_sink().clone());

        let mut spotify = match credentials {
            Some(credentials) => {
                spotify::Spotify::new(event_manager.clone(), credentials, configuration.clone())?
            }
            None => spotify::Spotify::new_offline(event_manager.clone(), configuration.clone()),
        };

        if let Some(notice) = offline_notice {
            event_manager.notify(notice);
        }

        let library = Arc::new(Library::new(
            event_manager.clone(),
            spotify.clone(),
            configuration.clone(),
        ));

        let queue = Arc::new(queue::Queue::new(
            spotify.clone(),
            configuration.clone(),
            library.clone(),
        ));

        #[cfg(feature = "mpris")]
        let mpris_manager = MprisManager::new(
            event_manager.clone(),
            queue.clone(),
            library.clone(),
            spotify.clone(),
        );

        #[cfg(feature = "mpris")]
        spotify.set_mpris(mpris_manager.clone());

        // Load the last played track into the player
        let playback_state = configuration.state().playback_state.clone();
        let queue_state = configuration.state().queuestate.clone();

        // A track that can't play now (no Spotify session, service off) stays unloaded
        if let Some(playable) = queue.get_current().filter(|p| spotify.can_play(p)) {
            spotify.load(
                &playable,
                playback_state == PlaybackState::Playing,
                queue_state.track_progress.as_millis() as u32,
            );
            spotify.update_track();
            match playback_state {
                PlaybackState::Stopped => {
                    spotify.stop();
                }
                PlaybackState::Paused | PlaybackState::Playing | PlaybackState::Default => {
                    spotify.pause();
                }
            }
        }

        #[cfg(unix)]
        let ipc = if let Ok(runtime_directory) = utils::create_runtime_directory() {
            Some(
                ipc::IpcSocket::new(
                    ASYNC_RUNTIME.get().unwrap().handle(),
                    runtime_directory.join("ncspot.sock"),
                    event_manager.clone(),
                    spotify.clone(),
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            error!("failed to create IPC socket: no suitable user runtime directory found");
            None
        };

        let mut cmd_manager = CommandManager::new(
            spotify.clone(),
            queue.clone(),
            library.clone(),
            configuration.clone(),
            event_manager.clone(),
        );

        cmd_manager.register_all();
        cmd_manager.register_keybindings(&mut cursive);

        let pending_count = cmd_manager.pending_count();

        cursive.set_user_data(Rc::new(UserDataInner { cmd: cmd_manager }));

        let search =
            ui::search::SearchView::new(event_manager.clone(), queue.clone(), library.clone());

        let libraryview = ui::library::LibraryView::new(queue.clone(), library.clone());

        let settingsview = ui::settings::SettingsView::new(
            ui::settings::SettingsMode::Screen,
            spotify
                .is_online()
                .then(|| spotify.api.user().unwrap_or_else(|| "sesión activa".into())),
            // Login from Ajustes (no session yet): store the credentials, then the app relaunches.
            Box::new({
                let configuration = configuration.clone();
                move |credentials| {
                    Spotify::login_and_store(&configuration, credentials).map_err(|e| e.to_string())
                }
            }),
            Box::new({
                let library = library.clone();
                move |change| match change {
                    ui::settings::LibraryChange::ScLoaded => library.update_soundcloud(),
                    ui::settings::LibraryChange::ScRemoved => library.clear_soundcloud(),
                    ui::settings::LibraryChange::ScToggled => library.sc_visibility_changed(),
                    ui::settings::LibraryChange::YtLinksChanged if external::yt_library::links().is_empty() => {
                        library.clear_youtube()
                    }
                    ui::settings::LibraryChange::YtLinksChanged => library.update_youtube(),
                    ui::settings::LibraryChange::YtToggled => library.yt_visibility_changed(),
                }
            }),
        );

        let queueview = ui::queue::QueueView::new(queue.clone(), library.clone());

        #[cfg(feature = "cover")]
        let coverview = ui::cover::CoverView::new(queue.clone(), library.clone(), &configuration);

        let status = ui::statusbar::StatusBar::new(
            queue.clone(),
            Arc::clone(&library),
            pending_count.clone(),
        );

        let mut layout = ui::layout::Layout::new(
            status,
            &event_manager,
            theme,
            Arc::clone(&configuration),
            pending_count,
        )
        .screen("search", search.with_name("search"))
        .screen("library", libraryview.with_name("library"))
        .screen("queue", queueview)
        .screen("settings", settingsview.with_name("settings"));

        #[cfg(feature = "cover")]
        layout.add_screen("cover", coverview.with_name("cover"));

        // initial screen is library
        let initial_screen = configuration
            .values()
            .initial_screen
            .clone()
            .unwrap_or_else(|| "library".to_string());
        if layout.has_screen(&initial_screen) {
            layout.set_screen(initial_screen);
        } else {
            error!("Invalid screen name: {initial_screen}");
            layout.set_screen("library");
        }

        // Builds without the Wayland window always show the panels in the terminal.
        #[cfg(feature = "panels")]
        if cfg!(not(feature = "native")) || std::env::args().any(|arg|arg=="--native"||arg=="--panels") {
            cursive.add_fullscreen_layer(ui::native_panels::NativePanels::new(layout.with_name("main"),queue.clone(),spotify.clone()).with_name("spoti-native-panels"));
        } else {
            cursive.add_fullscreen_layer(layout.with_name("main"));
        }
        #[cfg(not(feature = "panels"))]
        cursive.add_fullscreen_layer(layout.with_name("main"));

        Ok(Self {
            queue,
            spotify,
            event_manager,
            #[cfg(unix)]
            ipc,
            cursive,
            draw_pacing,
        })
    }

    /// Start the application and run the event loop.
    pub fn run(&mut self) -> Result<(), String> {
        let mut last_reclaim = std::time::Instant::now();
        #[cfg(unix)]
        let mut signals =
            Signals::new([SIGTERM, SIGHUP]).expect("could not register signal handler");

        // cursive event loop
        while self.cursive.is_running() {
            self.draw_pacing.step(&mut self.cursive);
            if last_reclaim.elapsed() >= std::time::Duration::from_secs(15) {
                crate::memory::reclaim_unused();
                last_reclaim = std::time::Instant::now();
            }
            #[cfg(feature = "cover")]
            self.cursive
                .call_on_name("cover", |view: &mut ui::cover::CoverView| {
                    view.render_to_terminal();
                });
            #[cfg(unix)]
            for signal in signals.pending() {
                if signal == SIGTERM || signal == SIGHUP {
                    info!("Caught {signal}, cleaning up and closing");
                    if let Some(data) = self.cursive.user_data::<UserData>().cloned() {
                        data.cmd.handle(&mut self.cursive, Command::Quit);
                    }
                }
            }
            for event in self.event_manager.msg_iter() {
                match event {
                    Event::Player(state) => {
                        trace!("event received: {state:?}");
                        self.spotify.update_status(state.clone());

                        #[cfg(unix)]
                        if let Some(ref ipc) = self.ipc {
                            ipc.publish(&state, self.queue.get_current());
                        }

                        if state == PlayerEvent::FinishedTrack {
                            self.queue.next(false);
                        }
                    }
                    Event::Queue(event) => {
                        self.queue.handle_event(event);
                    }
                    Event::SessionDied => {
                        if self.spotify.start_worker(None).is_err() {
                            let data: UserData = self
                                .cursive
                                .user_data()
                                .cloned()
                                .expect("user data should be set");
                            data.cmd.handle(&mut self.cursive, Command::Quit);
                        };
                    }
                    Event::IpcInput(input) => match command::parse(&input) {
                        Ok(commands) => {
                            if let Some(data) = self.cursive.user_data::<UserData>().cloned() {
                                for cmd in commands {
                                    info!("Executing command from IPC: {cmd}");
                                    data.cmd.handle(&mut self.cursive, cmd);
                                }
                            }
                        }
                        Err(e) => error!("Parsing error: {e}"),
                    },
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::track::Source;

    #[test]
    fn startup_mode_decides_by_credentials_and_services() {
        assert_eq!(startup_mode(true, &[]), Startup::Online);
        assert_eq!(startup_mode(true, &[Source::YouTube]), Startup::Online);
        assert_eq!(startup_mode(false, &[Source::YouTube]), Startup::Offline);
        assert_eq!(startup_mode(false, &[]), Startup::StartScreen);
    }
}
