#![allow(dead_code)]

use std::{fmt::Write, path::PathBuf};

/// Returns a human readable String of a Duration
///
/// Example: `3h 12m 53s`
pub fn format_duration(d: &std::time::Duration) -> String {
    let mut s = String::new();
    let mut append_unit = |value, unit| {
        if value > 0 {
            let _ = write!(s, "{value}{unit}");
        }
    };

    let seconds = d.as_secs() % 60;
    let minutes = (d.as_secs() / 60) % 60;
    let hours = (d.as_secs() / 60) / 60;

    append_unit(hours, "h ");
    append_unit(minutes, "m ");
    append_unit(seconds, "s ");

    s.trim_end().to_string()
}

/// Returns a human readable String of milliseconds in the HH:MM:SS format.
pub fn ms_to_hms(duration: u32) -> String {
    let mut formated_time = String::new();

    let total_seconds = duration / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;

    if hours > 0 {
        formated_time.push_str(&format!("{hours}:{minutes:02}:"));
    } else {
        formated_time.push_str(&format!("{minutes}:"));
    }
    formated_time.push_str(&format!("{seconds:02}"));

    formated_time
}

pub fn cache_path_for_url(url: String) -> std::path::PathBuf {
    let mut path = crate::config::cache_path("covers");
    path.push(url.split('/').next_back().unwrap());
    path
}

pub fn download(url: String, path: std::path::PathBuf) -> Result<(), std::io::Error> {
    let mut resp = reqwest::blocking::get(url).map_err(std::io::Error::other)?;

    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut file = std::fs::File::create(path)?;

    std::io::copy(&mut resp, &mut file)?;
    Ok(())
}

/// Create the application specific runtime directory and return the path to it.
///
/// If the directory already exists and has the correct permissions, this function just returns the
/// existing directory. The contents stored in this directory are not necessarily persisted across
/// reboots. Stored files should be small since they could reside in memory (like on a tmpfs mount).
#[cfg(unix)]
pub fn create_runtime_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    use std::{
        fs::{self, Permissions},
        os::unix::prelude::PermissionsExt,
    };

    let user_runtime_directory = user_runtime_directory().ok_or("no runtime directory found")?;

    let creation_result = fs::create_dir(&user_runtime_directory);

    if creation_result.is_ok()
        || matches!(
            creation_result.as_ref().unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        )
    {
        // Needed when created inside a world readable directory, to prevent unauthorized access.
        // Doesn't hurt otherwise.
        fs::set_permissions(&user_runtime_directory, Permissions::from_mode(0o700))?;

        Ok(user_runtime_directory)
    } else {
        #[allow(clippy::unnecessary_unwrap)]
        Err(Box::new(creation_result.unwrap_err()))
    }
}

/// Return the path to the current user's runtime directory, or None if it couldn't be found.
/// This function does not guarantee correct ownership or permissions of the directory.
#[cfg(unix)]
pub fn user_runtime_directory() -> Option<PathBuf> {
    let linux_runtime_directory =
        PathBuf::from(format!("/run/user/{}/", unsafe { libc::getuid() }));
    let unix_runtime_directory = PathBuf::from("/tmp/");

    if let Some(xdg_runtime_directory) = xdg_runtime_directory() {
        Some(xdg_runtime_directory.join("ncspot"))
    } else if cfg!(target_os = "linux") && linux_runtime_directory.exists() {
        Some(linux_runtime_directory.join("ncspot"))
    } else if unix_runtime_directory.exists() {
        Some(unix_runtime_directory.join(format!("ncspot-{}", unsafe { libc::getuid() })))
    } else {
        None
    }
}

#[cfg(unix)]
fn xdg_runtime_directory() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR").ok().map(Into::into)
}

#[cfg(test)]
mod download_tests {
    #[test]
    #[ignore = "requires a public HTTPS image URL in NCSPOT_TEST_COVER_URL"]
    fn downloads_https_cover_with_system_certificates() {
        let url = std::env::var("NCSPOT_TEST_COVER_URL").expect("set NCSPOT_TEST_COVER_URL");
        assert!(url.starts_with("https://"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("covers/image");
        super::download(url, path.clone()).expect("HTTPS cover download");
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]) || bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[cfg(feature = "native")]
    #[test]
    #[ignore = "requires network and a track in NCSPOT_TEST_LYRICS_TRACK; use an isolated XDG_CACHE_HOME"]
    fn lyrics_https_works_with_the_selected_crypto_provider() {
        let track = std::env::var("NCSPOT_TEST_LYRICS_TRACK").unwrap();
        let track: spoti_tools::Track = serde_json::from_str(&track).unwrap();
        // A missing lyric is a valid HTTPS result too; transport/TLS errors are not.
        spoti_tools::lookup(&track).expect("HTTPS lyrics lookup");
    }
}
