//! UI interaction is platform-independent; Wayland is an optional Linux adapter.
pub mod model;
#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
#[cfg(all(target_os = "linux", feature = "wayland"))]
pub use wayland::run;
#[cfg(not(all(target_os = "linux", feature = "wayland")))]
pub fn run() -> spoti_control::Result<()> {
    Err("This build has no desktop adapter. The Athena popup currently supports Linux Wayland layer-shell.".into())
}
