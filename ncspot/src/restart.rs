//! Logout and relaunch of the process.
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static KEEP_SESSION: AtomicBool = AtomicBool::new(false);

/// Ask `main` to relaunch once the UI has been torn down.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Relaunch keeping the Spotify credentials (after logging in from Ajustes).
pub fn request_keep_session() {
    KEEP_SESSION.store(true, Ordering::SeqCst);
    request();
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

fn restart_args_from(args: impl Iterator<Item = OsString>) -> Vec<OsString> {
    args.skip(1).collect()
}

/// Delete the Spotify credentials (unless `request_keep_session`) and relaunch. The caller must already have left cursive
/// (so the terminal is restored); state is saved by `Command::Quit` before that.
pub fn logout_and_restart() -> ! {
    if !KEEP_SESSION.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(crate::config::cache_path("librespot").join("credentials.json"));
        crate::authentication::api_logout();
    }
    let args = restart_args_from(std::env::args_os());
    let exe = std::env::current_exe().unwrap_or_else(|_| "spoti".into());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _error = std::process::Command::new(exe).args(args).exec();
        std::process::exit(1)
    }
    #[cfg(not(unix))]
    {
        let code = std::process::Command::new(exe)
            .args(args)
            .env("SPOTI_RELAUNCHED", "1")
            .status()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(1);
        std::process::exit(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_args_keep_user_arguments() {
        let args = ["spoti", "--basepath", "/x", "-d", "log"].map(OsString::from);
        assert_eq!(restart_args_from(args.into_iter()), ["--basepath", "/x", "-d", "log"].map(OsString::from));
    }
}
