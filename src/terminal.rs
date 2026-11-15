//! Native Spoti launcher and explicit legacy terminal compatibility.
use crate::{self as api, Result};
use serde_json::Value;
use std::{
    env, fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::Command,
};

fn runtime() -> PathBuf {
    api::home().join(".local/share/spoti-runtime")
}
fn libraries() -> PathBuf {
    runtime().join("usr/lib/x86_64-linux-gnu")
}
pub fn socket() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/user/1000"))
        .join("spoti/tmux.sock")
}
pub fn command(socket: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(runtime().join("usr/bin/tmux"))
        .env("LD_LIBRARY_PATH", libraries())
        .args(["-S", socket])
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_owned()
            .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
fn focus_existing() -> Result<bool> {
    if api::current_state().is_ok() {
        let output = Command::new("hyprctl").args(["-j", "clients"]).output()?;
        let clients: Value = serde_json::from_slice(&output.stdout)?;
        if let Some(client) = clients.as_array().into_iter().flatten().find(|client| {
            client["class"] == "ncspot"
                || client["title"] == "ncspot"
                || client["title"] == "Spoti · Letras"
        }) {
            let address = client["address"].as_str().ok_or("Ventana sin dirección")?;
            if client["workspace"]["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("special:"))
            {
                let output = Command::new("hyprctl")
                    .args(["-j", "activeworkspace"])
                    .output()?;
                let workspace: Value = serde_json::from_slice(&output.stdout)?;
                let id = workspace["id"].as_i64().filter(|id| *id > 0).unwrap_or(1);
                Command::new("hyprctl").args(["dispatch",&format!("hl.dsp.window.move({{ window = \"address:{address}\", workspace = {id}, follow = false }})")]).output()?;
            }
            Command::new("hyprctl")
                .args([
                    "dispatch",
                    &format!("hl.dsp.focus({{ window = \"address:{address}\" }})"),
                ])
                .output()?;
            return Ok(true);
        }
        return Err("ncspot ya está abierto en otra terminal".into());
    }
    Ok(false)
}
pub fn launch() -> Result<()> {
    if focus_existing()? {
        return Ok(());
    }
    let mut command = Command::new(api::home().join(".local/bin/ncspot"));
    command.arg("--native").env_remove("NO_COLOR");
    if env::var_os("LD_LIBRARY_PATH").is_some_and(|path| path == libraries().as_os_str()) {
        command.env_remove("LD_LIBRARY_PATH");
    }
    Err(command.exec().into())
}
/// Explicit compatibility launcher; normal Spoti does not start a terminal.
pub fn launch_terminal() -> Result<()> {
    if focus_existing()? {
        return Ok(());
    }
    let socket = socket();
    let parent = socket.parent().ok_or("Ruta de sesión inválida")?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let config = api::config_home().join("spoti");
    let error = Command::new(runtime().join("usr/bin/foot"))
        .env("LD_LIBRARY_PATH", libraries())
        // A launcher called from tooling can inherit NO_COLOR; this is always
        // an interactive graphical window and must retain the user's theme.
        .env_remove("NO_COLOR")
        .env("COLORTERM", "truecolor")
        .args(["--app-id=ncspot", "--title=Spoti"])
        .arg(format!("--config={}", config.join("foot.ini").display()))
        .arg(runtime().join("usr/bin/tmux"))
        .arg("-S")
        .arg(socket)
        .arg("-f")
        .arg(config.join("tmux.conf"))
        .args(["new-session", "-A", "-s", "spoti"])
        .arg(api::home().join(".local/bin/ncspot"))
        .exec();
    Err(error.into())
}
pub fn toggle(socket: &str, pane: &str) -> Result<()> {
    let panes = command(
        socket,
        &[
            "list-panes",
            "-t",
            pane,
            "-F",
            "#{pane_id}\t#{pane_current_command}\t#{@spoti_lyrics}",
        ],
    )?;
    let panes: Vec<Vec<&str>> = panes
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    let player = panes
        .iter()
        .find(|item| item.get(1) == Some(&"ncspot"))
        .and_then(|item| item.first())
        .copied()
        .ok_or("No hay reproductor en este panel")?;
    if let Some(lyrics) = panes.iter().find(|item| item.get(2) == Some(&"1")) {
        command(socket, &["kill-pane", "-t", lyrics[0]])?;
        command(socket, &["select-pane", "-t", player])?;
    } else {
        let lyrics = api::home().join(".local/bin/spoti-lyrics");
        let id = command(
            socket,
            &[
                "split-window",
                "-h",
                "-p",
                "55",
                "-t",
                player,
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                lyrics.to_str().ok_or("Ruta inválida")?,
                "--view",
            ],
        )?;
        command(
            socket,
            &["set-option", "-p", "-t", id.trim(), "@spoti_lyrics", "1"],
        )?;
        command(socket, &["resize-pane", "-t", player, "-x", "45%"])?;
    }
    Ok(())
}
