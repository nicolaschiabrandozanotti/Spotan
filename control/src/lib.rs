//! No GUI or compositor dependencies. Transports are platform adapters.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub mode: Value,
    pub playable: Option<Track>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Track {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration: u64,
}

impl Track {
    pub fn key(&self) -> String {
        format!("{}\0{}\0{}", self.id, self.title, self.artists.join("\0"))
    }
}

pub fn elapsed(state: &State, at: f64) -> f64 {
    if let Some(start) = state.mode.get("Playing") {
        (at - start["secs_since_epoch"].as_f64().unwrap_or(at)
            - start["nanos_since_epoch"].as_f64().unwrap_or(0.0) / 1e9)
            .max(0.0)
    } else if let Some(paused) = state.mode.get("Paused") {
        paused["secs"].as_f64().unwrap_or(0.0) + paused["nanos"].as_f64().unwrap_or(0.0) / 1e9
    } else {
        0.0
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    pub state: State,
    pub volume: Option<f64>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    PlayPause,
    Previous,
    Next,
    Seek(f64),
    SeekRelative(i32),
    Volume(f64),
}
impl Action {
    pub fn command(self) -> String {
        match self {
            Self::PlayPause => "playpause".into(),
            Self::Previous => "previous".into(),
            Self::Next => "next".into(),
            Self::Seek(s) => format!("seek {}ms", (finite(s).max(0.0) * 1000.0).round() as u64),
            Self::SeekRelative(s) => format!("seek {s:+}s"),
            Self::Volume(v) => format!("volume {:.4}", finite(v).clamp(0.0, 1.0) * 100.0),
        }
    }
}
fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}
#[cfg(unix)]
pub mod unix;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_stopped_snapshot() {
        let s: Snapshot = serde_json::from_str(r#"{"mode":"Stopped","playable":null}"#).unwrap();
        assert!(s.volume.is_none());
        assert_eq!(elapsed(&s.state, 100.0), 0.0);
    }
    #[test]
    fn commands_are_bounded() {
        assert_eq!(Action::Volume(2.0).command(), "volume 100.0000");
        assert_eq!(Action::Volume(f64::NAN).command(), "volume 0.0000");
        assert_eq!(Action::Seek(-10.0).command(), "seek 0ms");
        assert_eq!(Action::SeekRelative(-10).command(), "seek -10s");
    }
}
