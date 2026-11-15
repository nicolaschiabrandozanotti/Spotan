//! Windows start-up helpers: config bootstrap, Windows Terminal relaunch and safe glyphs.
//! Everything platform-independent is testable on Linux; call sites are `cfg(windows)`.
use std::{fs, io, path::Path, sync::OnceLock};

const PACKAGED_CONFIG: &str = include_str!("../packaging/config.toml");

/// Create `path` from the packaged config (no Nerd Font) unless it already exists.
pub fn bootstrap_config(path: &Path) -> io::Result<()> {
    if path.exists() { return Ok(()); }
    if let Some(dir) = path.parent() { fs::create_dir_all(dir)?; }
    fs::write(path, PACKAGED_CONFIG.replace("use_nerdfont = true", "use_nerdfont = false"))
}

/// True on Windows outside Windows Terminal (conhost lacks many glyphs). Decided once.
pub fn safe_glyphs() -> bool {
    static SAFE: OnceLock<bool> = OnceLock::new();
    *SAFE.get_or_init(|| cfg!(windows) && std::env::var_os("WT_SESSION").is_none())
}

/// `fancy` normally, `plain` when `safe_glyphs()`.
pub fn glyph(fancy: &'static str, plain: &'static str) -> &'static str {
    if safe_glyphs() { plain } else { fancy }
}

/// Background cell for a braille dot pattern; plain ASCII density where braille would be tofu.
pub fn dot_char(bits: u8) -> char {
    if safe_glyphs() { density(bits) } else { char::from_u32(0x2800 + bits as u32).unwrap() }
}

fn density(bits: u8) -> char {
    match bits.count_ones() { 0 => ' ', 1..=2 => '.', 3..=5 => ':', _ => '+' }
}

/// Windows entry point: write the default config if missing, then relaunch in Windows Terminal
/// when double-clicked from its own conhost window. May exit the process.
#[cfg(windows)]
pub fn init() {
    let _ = bootstrap_config(&crate::config_home().join("ncspot").join("config.toml"));
    relaunch_in_windows_terminal();
}

#[cfg(windows)]
fn relaunch_in_windows_terminal() {
    use std::{env, path::PathBuf, process::Command};
    #[link(name = "kernel32")]
    extern "system" { fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32; }
    let mut list = [0u32; 2];
    // SAFETY: valid buffer of `count` u32s.
    let attached = unsafe { GetConsoleProcessList(list.as_mut_ptr(), 2) };
    if attached != 1 || env::var_os("WT_SESSION").is_some() || env::var_os("SPOTI_RELAUNCHED").is_some() { return; }
    let local = env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft\\WindowsApps\\wt.exe"));
    let in_path = env::var_os("PATH").and_then(|p| env::split_paths(&p).map(|d| d.join("wt.exe")).find(|f| f.is_file()));
    let Some(wt) = in_path.or(local.filter(|f| f.is_file())) else { return };
    let Ok(exe) = env::current_exe() else { return };
    let spawned = Command::new(wt).args(["-w", "new", "--title", "Spoti"]).arg(exe)
        .args(env::args_os().skip(1)).env("SPOTI_RELAUNCHED", "1").spawn();
    if spawned.is_ok() { std::process::exit(0); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_is_ascii_and_grows_with_dots() {
        let chars: Vec<char> = [0u8, 0b1, 0b111, 0xff].iter().map(|&b| density(b)).collect();
        assert_eq!(chars, [' ', '.', ':', '+']);
    }

    #[test]
    fn bootstrap_writes_once_without_nerdfont() {
        let dir = std::env::temp_dir().join(format!("spoti-winstart-{}", std::process::id()));
        let path = dir.join("ncspot").join("config.toml");
        let _ = fs::remove_dir_all(&dir);
        bootstrap_config(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("use_nerdfont = false") && !text.contains("use_nerdfont = true"));
        fs::write(&path, "mine").unwrap();
        bootstrap_config(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
        fs::remove_dir_all(&dir).unwrap();
    }
}
