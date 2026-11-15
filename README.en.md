<div align="center">

**English** | [Español](README.md)

<img src="docs/img/banner.svg" alt="Spoti" width="720">

**A lightweight Spotify client in Rust with synced lyrics and an animated background.**

![Rust](https://img.shields.io/badge/Rust-1.99-orange?logo=rust)
![MIT License](https://img.shields.io/badge/license-MIT-green)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20Windows-blue)

</div>

## What it is

Spoti is a lightweight Spotify client written in Rust. It started as a fork of [ncspot](https://github.com/hrkfdn/ncspot) 1.5.1 and plays audio through [librespot](https://github.com/librespot-org/librespot). **A Spotify Premium account is required.**

## Features

- 🪟 **Native Wayland window**, drawn with Cairo and shared memory: no terminal, no GTK.
- 🎤 **Synced lyrics** from [LRCLIB](https://lrclib.net), toggled with `Shift+L`. In narrow windows it opens a full-screen lyrics panel.
- 🌌 **Animated Unicode braille background** (toggle with `v`).
- 🎨 **Athena palette**, read live from the `[theme]` section of ncspot's `config.toml`.
- 🖱️ **Focus follows the mouse** between panels; clickable controls and progress bar.
- 💻 **X11 or terminal**: the same panels inside a terminal with `ncspot --panels`.
- 🪟 **Windows**: runs in Windows Terminal with the same lyrics and background panels (no native window there; hover-focus may not work in terminals, click does).
- 🧩 Optional **Athena popover** for Hyprland (`spoti-popover`, wlr-layer-shell).

## Metrics

<div align="center">
<img src="docs/img/metrics-en.svg" alt="Spoti metrics" width="720">
</div>

| Measurement | Before | Now |
|---|---|---|
| Executable | 34.8 MiB | 12.9 MiB |
| Player RAM (PSS+SwapPss) | ~31.6 MiB | ~29 MiB (+3–4 MiB with the Athena popover) |
| CPU, window hidden | 2.63% of one core | 1.06% |
| Threads | 18 | 8 |
| Allocations per lyrics frame | 62 | 0 |

Playing: ~0.7% of one core. Paused: ~0.15%. **320 kbps** bitrate and **gapless** playback are kept.

> Measured on a single Linux machine with the native window. Windows was not measured. Values vary with window size, library and track.

## Download and install

You need **Spotify Premium**. Every version is on [Releases](https://github.com/nicolaschiabrandozanotti/Spotan/releases).

### Linux (x86_64) — one command

```sh
curl -fsSL https://github.com/nicolaschiabrandozanotti/Spotan/releases/latest/download/spoti-linux-x86_64.tar.gz | tar xz && ./spoti-linux-x86_64/install.sh
```

Then open it with `spoti` or from your app menu. It installs per user, **no sudo**: everything goes to `~/.local`, plus a desktop entry and the JetBrains Mono Nerd Font. Tested on Ubuntu 22.04/24.04, Debian 12, Fedora and Arch (glibc ≥ 2.34). It only needs PulseAudio or PipeWire, cairo and xkbcommon, which ship with any desktop. To uninstall: `./spoti-linux-x86_64/uninstall.sh`.

### Windows 10/11 (x64) — one PowerShell command

```powershell
irm https://github.com/nicolaschiabrandozanotti/Spotan/releases/latest/download/spoti-windows-x86_64.zip -OutFile "$env:TEMP\spoti.zip"; Expand-Archive "$env:TEMP\spoti.zip" "$env:LOCALAPPDATA\Spoti" -Force; & "$env:LOCALAPPDATA\Spoti\spoti-windows-x86_64\spoti.bat"
```

To open it again: `%LOCALAPPDATA%\Spoti\spoti-windows-x86_64\spoti.bat`. You can also download the `.zip` from Releases, unzip it and double-click `spoti.bat`; SmartScreen may warn because the executable is unsigned (*More info → Run anyway*). On first run it copies the theme to `%APPDATA%\ncspot\config.toml` without overwriting an existing config and opens Windows Terminal if available.

On first run Spoti asks you to log in to Spotify in your browser.

## Keys

These are ncspot's default keybindings plus Spoti's additions.

| Key | Action |
|---|---|
| `Shift+L` | Lyrics |
| `Space` / `p` | Pause / resume |
| `n` / `b` | Next / previous |
| `←` / `→` | Seek -/+ 5 s |
| `+` / `-` | Volume |
| `↑` / `↓` or `j` / `k` | Scroll lyrics |
| `f` | Follow current track |
| `[` / `]` | Lyrics sync offset (250 ms) |
| `v` | Toggle braille background |
| `q` / `Esc` | Close lyrics / quit |
| `?` | Help |

## Build from source

```sh
packaging/build.sh
```

Requires Docker. It builds Linux inside Debian 12 (for an older glibc) and cross-compiles Windows with mingw-w64. Output goes to `dist/`.

Cargo features in `ncspot/`:

- `native`: Wayland window (implies `panels`).
- `panels`: lyrics and background inside any terminal.

The Windows build uses `--no-default-features --features panels,rodio_backend,crossterm_backend`.

To publish a version: `git tag v0.1.0 && git push origin v0.1.0`. The `.github/workflows/release.yml` workflow builds and uploads both packages to Releases.

The (Spanish) development log is in [docs/NOTAS-TECNICAS.md](docs/NOTAS-TECNICAS.md).

## Acknowledgements

Spoti stands on other people's work:

- **[ncspot](https://github.com/hrkfdn/ncspot)**, by Henrik Friedrichsen and contributors: the base this project started from.
- [librespot](https://github.com/librespot-org/librespot): the playback engine.
- [cursive](https://github.com/gyscos/cursive) and [ratatui](https://ratatui.rs): terminal UI.
- [Smithay Client Toolkit](https://github.com/Smithay/client-toolkit): Wayland client.
- [LRCLIB](https://lrclib.net): synced lyrics.
- [JetBrains Mono](https://www.jetbrains.com/lp/mono/) and [Nerd Fonts](https://www.nerdfonts.com) (OFL / MIT).

Spotify is a trademark of Spotify AB. This project is not affiliated with Spotify.

## License

Spoti's own code is MIT (see [LICENSE](LICENSE)). The ncspot fork in `ncspot/` remains under BSD-2-Clause ([ncspot/LICENSE](ncspot/LICENSE)). Bundled fonts keep their own licenses (`packaging/fonts`).
