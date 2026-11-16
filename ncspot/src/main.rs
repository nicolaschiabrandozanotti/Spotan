#[macro_use]
extern crate cursive;
#[macro_use]
extern crate serde;

use std::{path::PathBuf, process::exit};

use application::{Application, setup_logging};
use config::set_configuration_base_path;
use log::error;
use ncspot::program_arguments;

#[cfg(test)]
mod allocation_profile;
mod application;
mod authentication;
mod cli;
mod command;
mod commands;
mod config;
mod events;
mod external;
mod ext_traits;
mod library;
mod memory;
#[cfg(feature = "native")]
mod native_window;
mod model;
mod oauth_login;
mod panic;
mod queue;
mod restart;
mod serialization;
mod sharing;
mod spotify;
mod spotify_api;
mod spclient_library;
mod spotify_url;
mod spotify_worker;
mod theme;
mod traits;
mod ui;
mod utils;

#[cfg(unix)]
mod ipc;

#[cfg(feature = "mpris")]
mod mpris;

fn main() -> Result<(), String> {
    // Librespot creates a separate Tokio runtime with Runtime::new(). Keep its
    // default bounded too; an explicit per-process override takes precedence.
    if std::env::var_os("TOKIO_WORKER_THREADS").is_none() {
        // SAFETY: this is the single-threaded entry point, before any worker,
        // graphical backend, authentication or network client is started.
        unsafe { std::env::set_var("TOKIO_WORKER_THREADS", "1") };
    }
    #[cfg(all(windows, feature = "panels"))]
    spoti_tools::winstart::init();
    memory::configure();
    // Set a custom backtrace hook that writes the backtrace to a file instead of stdout, since
    // stdout is most likely in use by Cursive.
    panic::register_backtrace_panic_handler();

    // Parse the command line arguments.
    let matches = program_arguments().get_matches();
    #[cfg(not(feature = "native"))]
    if matches.get_flag("native") || matches.get_flag("native-demo") {
        return Err("Esta compilación no incluye la ventana nativa; compilá con --features native".into());
    }
    #[cfg(feature = "native")]
    if matches.get_flag("native-demo") {
        return native_window::demo().map_err(|error|error.to_string());
    }

    // Enable debug logging to a file if specified on the command line.
    if let Some(filename) = matches.get_one::<PathBuf>("debug") {
        setup_logging(filename).expect("logger could not be initialized");
    }

    // Set the configuration base path. All configuration files are read/written relative to this
    // path.
    set_configuration_base_path(matches.get_one::<PathBuf>("basepath").cloned());

    match matches.subcommand() {
        Some(("info", _subcommand_matches)) => cli::info(),
        Some((_, _)) => unreachable!(),
        None => {
            // Export proxy environment variables from the configuration file
            // before the tokio runtime and its worker threads are spawned in
            // `Application::new` (set_var is not thread safe).
            config::preload_proxy_environment(matches.get_one::<String>("config").cloned());

            // Create the application.
            let mut application =
                match Application::new(matches.get_one::<String>("config").cloned()) {
                    Ok(application) => application,
                    Err(error) => {
                        eprintln!("{error}");
                        error!("{error}");
                        exit(-1);
                    }
                };

            // Start the application event loop.
            let result = application.run();
            // Drop first so cursive restores the terminal before relaunching.
            drop(application);
            if restart::requested() {
                restart::logout_and_restart();
            }
            result
        }
    }?;

    Ok(())
}
