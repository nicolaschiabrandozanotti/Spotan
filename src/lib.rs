use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[cfg(unix)]
use std::{net::Shutdown, os::unix::net::UnixStream};
use unicode_width::UnicodeWidthChar;

pub mod theme;
pub mod background;
pub mod braille;
pub mod controls;
pub mod memory;
#[cfg(unix)]
pub mod terminal;
pub mod lyrics;
pub mod ratatui_bridge;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn home() -> PathBuf {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).expect("HOME is not set")
}

pub fn config_home() -> PathBuf {
    // Same roots as ncspot's platform-dirs on Windows: %APPDATA%\ncspot, %LOCALAPPDATA%.
    #[cfg(windows)]
    if let Some(dir) = env::var_os("APPDATA") { return dir.into(); }
    env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

pub fn cache_home() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = env::var_os("LOCALAPPDATA") { return dir.into(); }
    env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache"))
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

pub fn ipc_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", env::var("UID").unwrap_or_else(|_| "1000".into()))))
        .join("ncspot/ncspot.sock")
}

#[cfg(unix)]
fn connection() -> Result<BufReader<UnixStream>> {
    let socket = UnixStream::connect(ipc_path())?;
    socket.set_read_timeout(Some(Duration::from_millis(800)))?;
    socket.set_write_timeout(Some(Duration::from_millis(800)))?;
    Ok(BufReader::new(socket))
}

pub fn read_snapshot(reader: &mut impl BufRead) -> Result<State> {
    let mut bytes = Vec::new();
    reader.take(262_144).read_until(b'\n', &mut bytes)?;
    if !bytes.ends_with(b"\n") { return Err("ncspot devolvió un estado incompleto".into()); }
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(unix)]
pub fn current_state() -> Result<State> {
    read_snapshot(&mut connection()?)
}

#[cfg(unix)]
pub fn player_command(command: &str) -> Result<()> {
    let mut stream = connection()?;
    // Drain ncspot's initial snapshot so closing cannot reset unread data.
    read_snapshot(&mut stream)?;
    writeln!(stream.get_mut(), "{command}")?;
    stream.get_ref().shutdown(Shutdown::Write)?;
    Ok(())
}

pub use spoti_control::{State, Track, elapsed};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Record {
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Cached { saved: f64, record: Option<Record> }

fn query(track: &Track) -> Result<String> {
    let artist = track.artists.first().ok_or("La canción no tiene artista")?;
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("track_name", &track.title).append_pair("artist_name", artist).append_pair("album_name", &track.album);
    let duration = (track.duration as f64 / 1000.0).round() as u64;
    if (1..=3600).contains(&duration) { query.append_pair("duration", &duration.to_string()); }
    Ok(query.finish())
}

pub fn lookup(track: &Track) -> Result<Option<Record>> {
    let query = query(track)?;
    let folder = cache_home().join("spoti-lyrics");
    fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{:x}.json", Sha256::digest(query.as_bytes())));
    if let Ok(file) = fs::File::open(&path) {
        if let Ok(cached) = serde_json::from_reader::<_, Cached>(file) {
            let ttl = if cached.record.is_some() { 604800.0 } else { 900.0 };
            if now() - cached.saved < ttl { return Ok(cached.record); }
        }
    }
    let cooldown = folder.join("retry-after");
    if let Ok(until) = fs::read_to_string(&cooldown).unwrap_or_default().parse::<f64>() {
        if until > now() { return Err("LRCLIB está limitando consultas. Probá en unos segundos.".into()); }
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15))
        .tls_connector(std::sync::Arc::new(native_tls::TlsConnector::new()?))
        .user_agent("SpotiLyrics/0.1 (ncspot helper; https://github.com/hrkfdn/ncspot)").build();
    let record = match agent.get(&format!("https://lrclib.net/api/get?{query}")).call() {
        Ok(response) => Some(serde_json::from_reader(response.into_reader().take(2_000_000))?),
        Err(ureq::Error::Status(404, _)) => None,
        Err(ureq::Error::Status(code @ (429 | 503), response)) => {
            let delay = response.header("Retry-After").and_then(|s| s.parse::<f64>().ok()).unwrap_or(60.0).max(1.0);
            fs::write(cooldown, (now() + delay).to_string())?;
            return Err(format!("LRCLIB respondió {code}; esperá {delay:.0} segundos.").into());
        }
        Err(error) => return Err(format!("No pude consultar LRCLIB: {error}").into()),
    };
    let cached = Cached { saved: now(), record };
    let mut temporary = tempfile::NamedTempFile::new_in(&folder)?;
    serde_json::to_writer(temporary.as_file_mut(), &cached)?;
    temporary.persist(path)?;
    Ok(cached.record)
}

pub fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Parse LRC timestamps, offsets, repeated stamps and instrumental gaps.
pub fn parse_lrc(text: &str) -> Vec<(f64, String)> {
    let offset = text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("[offset:").and_then(|s| s.strip_suffix(']')).and_then(|s| s.parse::<f64>().ok())
    }).unwrap_or(0.0) / 1000.0;
    let mut result = Vec::new();
    for line in text.lines() {
        let mut stamps = Vec::new();
        let mut rest = line.trim();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((stamp, tail)) = after.split_once(']') else { break };
            let Some((minutes, seconds)) = stamp.split_once(':') else { break };
            let (Ok(minutes), Ok(seconds)) = (minutes.parse::<u32>(), seconds.parse::<f64>()) else { break };
            if !seconds.is_finite() || !(0.0..60.0).contains(&seconds) { break; }
            stamps.push(minutes as f64 * 60.0 + seconds + offset);
            rest = tail;
        }
        let content = clean(rest).trim().to_string();
        for stamp in stamps { result.push((stamp, if content.is_empty() { "♪".into() } else { content.clone() })); }
    }
    result.sort_by(|a, b| a.0.total_cmp(&b.0));
    result
}

pub fn active_line(times: &[f64], position: f64) -> Option<usize> {
    times.partition_point(|time| *time <= position).checked_sub(1)
}

pub fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let word_width: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
        if !row.is_empty() && used + 1 + word_width > width {
            rows.push(std::mem::take(&mut row)); used = 0;
        }
        if !row.is_empty() { row.push(' '); used += 1; }
        for character in word.chars() {
            let character = if character.width().unwrap_or(0) > width { '?' } else { character };
            let size = character.width().unwrap_or(0);
            if used + size > width { rows.push(std::mem::take(&mut row)); used = 0; }
            row.push(character); used += size;
        }
    }
    if !row.is_empty() || rows.is_empty() { rows.push(row); }
    rows
}

pub fn no_lyrics(record: Option<&Record>) -> String {
    match record {
        None => "No encontré la letra de esta versión en LRCLIB.".into(),
        Some(record) if record.instrumental => "Este tema figura como instrumental en LRCLIB.".into(),
        Some(record) => record.plain_lyrics.as_deref().map(clean).unwrap_or_else(|| "Todavía no hay una letra disponible para este tema.".into()),
    }
}

pub fn io_error(message: impl Into<String>) -> io::Error { io::Error::other(message.into()) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lrc_offsets_and_gaps() {
        assert_eq!(parse_lrc("[offset:250]\n[00:01.25][00:03.50]test\n[00:05.00]"),
            vec![(1.5, "test".into()), (3.75, "test".into()), (5.25, "♪".into())]);
        assert!(parse_lrc("[ar:Artist]\n[00:NaN]bad\n[00:65]bad").is_empty());
        assert_eq!(active_line(&[1.0, 2.0, 2.0, 3.0], 2.0), Some(2));
        assert_eq!(active_line(&[1.0], 0.0), None);
    }

    #[test]
    fn playback_tracks_pause_and_seek() {
        let mut state = State { mode: json!({"Playing":{"secs_since_epoch":100,"nanos_since_epoch":500000000}}), ..Default::default() };
        assert_eq!(elapsed(&state, 112.0), 11.5);
        state.mode = json!({"Paused":{"secs":5,"nanos":250000000}});
        assert_eq!(elapsed(&state, 999.0), 5.25);
        state.mode = json!({"Playing":{"secs_since_epoch":180,"nanos_since_epoch":0}});
        assert_eq!(elapsed(&state, 200.0), 20.0);
        state.mode = json!("Stopped"); assert_eq!(elapsed(&state, 999.0), 0.0);
    }

    #[test]
    fn wraps_unicode_at_all_widths() {
        for width in [1, 2, 8, 30, 80, 150] {
            for line in wrap_line("Música e\u{301} 中文 palabrasssssssssssssssssssss para probar", width) {
                assert!(unicode_width::UnicodeWidthStr::width(line.as_str()) <= width);
            }
        }
        assert_eq!(clean("title\u{1b}\u{7}\nnext"), "title\nnext");
    }

    #[test]
    fn reads_only_complete_bounded_ipc_snapshots() {
        let data = b"{\"mode\":\"Stopped\",\"playable\":null}\n";
        assert!(read_snapshot(&mut &data[..]).unwrap().playable.is_none());
        assert!(read_snapshot(&mut &data[..data.len()-1]).is_err());
    }

    #[test]
    fn reuses_python_cache_query_format() {
        let track = Track { title: "The Kill (Bury Me)".into(), artists: vec!["Thirty Seconds To Mars".into()], album: "A Beautiful Lie".into(), duration: 231533, ..Default::default() };
        assert_eq!(query(&track).unwrap(), "track_name=The+Kill+%28Bury+Me%29&artist_name=Thirty+Seconds+To+Mars&album_name=A+Beautiful+Lie&duration=232");
    }
}
