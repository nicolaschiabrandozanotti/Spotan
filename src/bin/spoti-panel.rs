use serde_json::Value;
use spoti_tools::{self as api, Result};
use std::{env, path::{Path, PathBuf}, process::{Command, ExitCode}};
use std::{thread,time::Duration};

fn remote(arguments: &[&str]) -> Result<String> {
    let address=env::var("KITTY_LISTEN_ON").unwrap_or_default();
    let fd=address.strip_prefix("fd:").and_then(|s|s.parse::<i32>().ok())
        .ok_or("Usá Shift + L dentro de ncspot en Kitty.")?;
    if fd<3 {return Err("Canal de Kitty inválido".into());}
    // Kitty supplies an inheritable socketpair. Rust preserves that descriptor
    // when spawning kitten; no global socket or remote-control permission needed.
    let output=Command::new("/usr/bin/kitten").args(["@","--to",&address]).args(arguments).output()?;
    if !output.status.success() {return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned().into());}
    Ok(String::from_utf8(output.stdout)?)
}

fn is_player(window: &Value) -> bool {
    let command_is_player=|command: &Value| command.get(0).and_then(Value::as_str)
        .and_then(|path|Path::new(path).file_name()).is_some_and(|name|name=="ncspot");
    command_is_player(&window["cmdline"]) || window["foreground_processes"].as_array()
        .is_some_and(|processes|processes.iter().any(|process|command_is_player(&process["cmdline"])))
}

fn target(windows: &Value) -> Option<(Value,Value)> {
    let mut target=None;
    let mut preferred=false;
    for window in windows.as_array()? {
        for tab in window["tabs"].as_array().into_iter().flatten() {
            for player in tab["windows"].as_array().into_iter().flatten().filter(|w|is_player(w)) {
                let focused=window["is_focused"].as_bool().unwrap_or(false) && tab["is_active"].as_bool().unwrap_or(false);
                if target.is_none() || focused && !preferred {target=Some((tab.clone(),player.clone()));preferred=focused;}
            }
        }
    }
    target
}

fn configure_window() -> Result<()> {
    let theme=api::theme::Theme::read();
    let rgb=|color,fallback:&str|match color {
        ratatui::style::Color::Rgb(r,g,b)=>format!("#{r:02X}{g:02X}{b:02X}"),
        _=>fallback.to_owned(),
    };
    let settings=[
        "focus_follows_mouse=yes".to_owned(),
        "window_padding_width=0".to_owned(),
        "window_margin_width=0".to_owned(),
        "single_window_margin_width=0".to_owned(),
        "background_opacity=1".to_owned(),
        format!("background={}",rgb(theme.background,"#18181F")),
        format!("foreground={}",rgb(theme.primary,"#E8DFDF")),
        format!("active_border_color={}",rgb(theme.highlight_bg,"#745151")),
        format!("inactive_border_color={}",rgb(theme.highlight_bg,"#745151")),
    ];
    let mut arguments=vec!["load-config"];
    for setting in &settings {arguments.extend(["--override",setting.as_str()]);}
    remote(&arguments)?;
    Ok(())
}

fn restart_player(tab:&Value,player:&Value,player_id:&str,tab_id:&str) -> Result<()> {
    let old_state=api::current_state()?;
    let was_playing=old_state.mode.get("Playing").is_some();
    let position=api::elapsed(&old_state,api::now());
    let old_track=old_state.playable.as_ref().map(|track|track.id.clone());
    let direct=player["cmdline"].get(0).and_then(Value::as_str)
        .and_then(|path|Path::new(path).file_name()).is_some_and(|name|name=="ncspot");
    let pid=if direct {player["pid"].as_u64()} else {
        player["foreground_processes"].as_array().into_iter().flatten()
            .find(|process| process["cmdline"].get(0).and_then(Value::as_str)
                .and_then(|path|Path::new(path).file_name()).is_some_and(|name|name=="ncspot"))
            .and_then(|process|process["pid"].as_u64())
    }.ok_or("No pude identificar el proceso de ncspot")?;
    let binary=env::current_exe()?.with_file_name("ncspot");
    let binary=binary.to_str().ok_or("Ruta de ncspot inválida")?;
    // Keep the existing window group alive, including a possible Fish session
    // underneath the player, without introducing another split.
    remote(&["focus-window","--match",player_id])?;
    let pid_argument=pid.to_string();
    remote(&["launch","--match",tab_id,"--type=overlay-main","--title=ncspot","/bin/sh","-c",
        "while [ -d \"/proc/$1\" ]; do sleep 0.05; done; exec \"$2\"","spoti-restart",&pid_argument,binary])?;
    api::player_command("quit")?;
    let process=PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..100 {if !process.exists(){break;} thread::sleep(Duration::from_millis(50));}
    if process.exists() {return Err("ncspot todavía está guardando el estado; no se forzó su cierre".into());}
    let _=tab;
    for _ in 0..200 {
        if let Ok(state)=api::current_state() {
            if state.playable.as_ref().map(|track|track.id.clone())==old_track {
                api::player_command(&format!("seek {position:.3}s"))?;
                // `play` starts the selected library row; toggle playback of
                // the restored queue item instead.
                if state.mode.get("Playing").is_some()!=was_playing {
                    api::player_command("playpause")?;
                }
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("ncspot se abrió, pero todavía no publicó la canción restaurada".into())
}

fn run() -> Result<()> {
    if env::args().nth(1).as_deref()==Some("--launch") {return api::terminal::launch();}
    if env::args().nth(1).as_deref()==Some("--launch-terminal") {return api::terminal::launch_terminal();}
    if env::args().nth(1).as_deref()==Some("--tmux") {
        let socket=env::args().nth(2).ok_or("Falta el socket de Spoti")?;
        let pane=env::args().nth(3).ok_or("Falta el panel de Spoti")?;
        return api::terminal::toggle(&socket,&pane);
    }
    if env::args().nth(1).as_deref()==Some("--inspect") {
        let data=remote(&["ls"])?;
        std::fs::write(api::cache_home().join("spoti-lyrics/terminal-layout.json"),data)?;
        return Ok(());
    }
    if env::args().nth(1).as_deref()==Some("--memory") {
        api::memory::print_report();return Ok(());
    }
    if env::args().nth(1).as_deref()==Some("--version") {
        println!("spoti-panel {} (Rust)",env!("CARGO_PKG_VERSION"));return Ok(());
    }
    let windows: Value=serde_json::from_str(&remote(&["ls"])?)?;
    let (tab,player)=target(&windows).ok_or("No hay un ncspot abierto en esta instancia de Kitty")?;
    let player_id=format!("id:{}",player["id"].as_u64().ok_or("ID de panel inválido")?);
    let tab_id=format!("id:{}",tab["id"].as_u64().ok_or("ID de pestaña inválido")?);
    // Apply the app's frame and background only to this Kitty instance.
    configure_window()?;
    if env::args().nth(1).as_deref()==Some("--configure") {return Ok(());}
    if env::args().nth(1).as_deref()==Some("--restart-player") {return restart_player(&tab,&player,&player_id,&tab_id);}
    let mut closed=false;
    for window in tab["windows"].as_array().into_iter().flatten() {
        if window["user_vars"]["spoti_lyrics"].as_str()==Some("1") {
            let id=format!("id:{}",window["id"].as_u64().ok_or("ID de letras inválido")?);
            remote(&["close-window","--match",&id])?;closed=true;
        }
    }
    if closed {remote(&["focus-window","--match",&player_id])?;return Ok(());}
    remote(&["goto-layout","--match",&tab_id,"splits"])?;
    let lyrics=env::current_exe()?.with_file_name("spoti-lyrics");
    let lyrics=lyrics.to_str().ok_or("Ruta de letras inválida")?;
    remote(&["launch","--match",&tab_id,"--type=window","--location=vsplit","--next-to",&player_id,
        "--bias=55","--keep-focus","--title=Spoti · Letras","--var=spoti_lyrics=1",lyrics,"--view"])?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Spoti: {error}");
            // Background hotkey actions have no terminal for their errors.
            let path: PathBuf=api::cache_home().join("spoti-lyrics/panel-error.log");
            if let Some(parent)=path.parent() {let _=std::fs::create_dir_all(parent);}
            let _=std::fs::write(path,format!("{} {error}\n",api::now()));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn finds_ncspot_inside_fish_and_prefers_focused_tab() {
        let windows=json!([
            {"is_focused":false,"tabs":[{"id":1,"windows":[{"id":1,"cmdline":["/bin/ncspot"]}]}]},
            {"is_focused":true,"tabs":[{"id":2,"is_active":true,"windows":[{"id":2,"cmdline":["/bin/fish"],"foreground_processes":[{"cmdline":["/home/user/.local/bin/ncspot"]}]}]}]}
        ]);
        let (tab,player)=target(&windows).unwrap();assert_eq!(tab["id"],2);assert_eq!(player["id"],2);
        assert!(target(&json!([{"tabs":[{"windows":[{"cmdline":["/bin/fish"]}]}]}])).is_none());
    }
}
