//! Borrow only the fields used by the lyrics UI; retain them between frames.
use crate::{model::playable::Playable, spotify::PlayerEvent};
use spoti_tools::{State, Track};

#[derive(Default)]
pub(super) struct StateCache {
    mode: Option<PlayerEvent>,
}

struct Metadata<'a> {
    kind: &'static str,
    id: &'a str,
    title: &'a str,
    artists: &'a [String],
    album: &'a str,
    duration: u64,
}
impl<'a> Metadata<'a> {
    fn from_playable(playable: &'a Playable) -> Option<Self> {
        Some(match playable {
            Playable::Track(track) => Self {
                kind: "Track",
                // Match the previous serde bridge: null String fields rejected the track.
                id: track.id.as_deref()?,
                title: &track.title,
                artists: &track.artists,
                album: track.album.as_deref()?,
                duration: track.duration.into(),
            },
            Playable::Episode(episode) => Self {
                kind: "Episode",
                id: &episode.id,
                title: "",
                artists: &[],
                album: "",
                duration: episode.duration.into(),
            },
        })
    }
    fn matches(&self, track: &Track) -> bool {
        track.kind == self.kind
            && track.id == self.id
            && track.title == self.title
            && track.artists == self.artists
            && track.album == self.album
            && track.duration == self.duration
    }
    fn to_owned(&self) -> Track {
        Track {
            kind: self.kind.into(),
            id: self.id.into(),
            title: self.title.into(),
            artists: self.artists.to_vec(),
            album: self.album.into(),
            duration: self.duration,
        }
    }
}
impl StateCache {
    /// Returns whether lyrics metadata changed. Unchanged frames allocate nothing.
    pub(super) fn update(
        &mut self,
        state: &mut State,
        mode: PlayerEvent,
        playable: Option<&Playable>,
    ) -> bool {
        if self.mode.as_ref() != Some(&mode) {
            state.mode = serde_json::to_value(&mode).unwrap_or_default();
            self.mode = Some(mode);
        }
        let metadata = playable.and_then(Metadata::from_playable);
        let changed = match (&metadata, &state.playable) {
            (Some(metadata), Some(track)) => !metadata.matches(track),
            (None, None) => false,
            _ => true,
        };
        if changed {
            state.playable = metadata.map(|m| m.to_owned());
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        hint::black_box,
        time::{Duration, Instant, UNIX_EPOCH},
    };
    fn fixture() -> Playable {
        serde_json::from_value(serde_json::json!({
            "type":"Track", "id":"fixture", "uri":"spotify:track:fixture", "title":"Prueba 漢字 e\u{301}",
            "track_number":1,"disc_number":1,"duration":240000,"artists":["Artista", "Otro"],
            "artist_ids":["a","b"],"album":"Álbum","album_id":"album","album_artists":["Artista"],
            "cover_url":"https://example.invalid/cover","url":"https://example.invalid/track",
            "added_at":null,"list_index":7,"is_local":false,"is_playable":true
        })).unwrap()
    }
    fn legacy(mode: &PlayerEvent, playable: Option<&Playable>) -> State {
        State {
            mode: serde_json::to_value(mode).unwrap_or_default(),
            playable: playable
                .cloned()
                .and_then(|p| serde_json::to_value(p).ok())
                .and_then(|v| serde_json::from_value(v).ok()),
        }
    }
    fn assert_state(
        cache: &mut StateCache,
        state: &mut State,
        mode: PlayerEvent,
        playable: Option<&Playable>,
    ) {
        let expected = legacy(&mode, playable);
        cache.update(state, mode, playable);
        assert_eq!(
            serde_json::to_value(&*state).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    #[test]
    fn direct_metadata_matches_serde_for_tracks_episodes_and_missing_fields() {
        let mut cache = StateCache::default();
        let mut state = State::default();
        let track = fixture();
        let episode=serde_json::from_value(serde_json::json!({"type":"Episode", "id":"ep",
            "uri":"spotify:episode:ep","name":"Episode", "duration":5000,"description":"Description",
            "release_date":"2026-01-01","cover_url":null,"added_at":null,"list_index":0})).unwrap();
        let mut cases = vec![Some(track.clone()), Some(episode), None];
        for field in ["id", "album"] {
            let mut v = serde_json::to_value(&track).unwrap();
            v[field] = serde_json::Value::Null;
            cases.push(Some(serde_json::from_value(v).unwrap()));
        }
        let mut local = track.clone();
        if let Playable::Track(t) = &mut local {
            t.is_local = true;
            t.id = Some(String::new());
            t.album = Some(String::new());
        }
        cases.push(Some(local));
        for p in &cases {
            for mode in [
                PlayerEvent::Stopped,
                PlayerEvent::FinishedTrack,
                PlayerEvent::Paused(Duration::new(9, 42)),
                PlayerEvent::Playing(UNIX_EPOCH + Duration::new(100, 123)),
                PlayerEvent::Playing(UNIX_EPOCH - Duration::from_secs(1)),
            ] {
                assert_state(&mut cache, &mut state, mode, p.as_ref());
            }
        }
    }
    #[test]
    fn cache_updates_all_visible_metadata_and_preserves_unchanged_storage() {
        let mut cache = StateCache::default();
        let mut state = State::default();
        let mut p = fixture();
        let mode = PlayerEvent::Paused(Duration::from_secs(42));
        assert!(cache.update(&mut state, mode.clone(), Some(&p)));
        let ptr = state.playable.as_ref().unwrap().title.as_ptr();
        assert!(!cache.update(&mut state, mode.clone(), Some(&p)));
        assert_eq!(state.playable.as_ref().unwrap().title.as_ptr(), ptr);
        for field in 0..6 {
            if let Playable::Track(t) = &mut p {
                match field {
                    0 => t.id = Some("changed".into()),
                    1 => t.title.push('!'),
                    2 => t.artists.push("new".into()),
                    3 => t.album = Some("other".into()),
                    4 => t.duration += 1,
                    _ => t.title.clear(),
                }
            }
            assert!(cache.update(&mut state, mode.clone(), Some(&p)));
            assert_state(&mut cache, &mut state, mode.clone(), Some(&p));
        }
        if let Playable::Track(t) = &mut p {
            t.list_index += 1;
            t.cover_url = None;
        }
        assert!(!cache.update(&mut state, mode, Some(&p)));
        assert!(cache.update(&mut state, PlayerEvent::Stopped, None));
        assert!(state.playable.is_none());
    }
    #[test]
    #[ignore = "opt-in allocation and CPU profile; no player or network"]
    fn profile_state_conversion() {
        let p = fixture();
        let mode = PlayerEvent::Paused(Duration::from_secs(90));
        let mut cache = StateCache::default();
        let mut state = State::default();
        cache.update(&mut state, mode.clone(), Some(&p));
        let mut active = state.playable.as_ref().unwrap().key();
        let frames = 50000;
        let (_, before) = crate::allocation_profile::measure(|| {
            for _ in 0..frames {
                let s = legacy(black_box(&mode), Some(black_box(&p)));
                black_box(s.playable.as_ref().unwrap().key());
                black_box(s);
            }
        });
        let (_, after) = crate::allocation_profile::measure(|| {
            for _ in 0..frames {
                if cache.update(&mut state, black_box(mode.clone()), Some(black_box(&p))) {
                    active = state.playable.as_ref().unwrap().key();
                }
                black_box((&state, &active));
            }
        });
        // Time separately, with allocation instrumentation disabled.
        let mut timings = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            for _ in 0..frames {
                let s = legacy(black_box(&mode), Some(black_box(&p)));
                black_box(s.playable.as_ref().unwrap().key());
                black_box(s);
            }
            let old = start.elapsed().as_nanos() as f64 / frames as f64;
            let start = Instant::now();
            for _ in 0..frames {
                if cache.update(&mut state, black_box(mode.clone()), Some(black_box(&p))) {
                    active = state.playable.as_ref().unwrap().key();
                }
                black_box((&state, &active));
            }
            timings.push(serde_json::json!({"legacy_ns_per_frame":old,"direct_ns_per_frame":start.elapsed().as_nanos() as f64/frames as f64}));
        }
        assert_eq!(after.allocations, 0);
        let result = serde_json::json!({"frames":frames,"legacy":before,"direct":after,"timings":timings,
            "scope":"state conversion plus track key, stable track/mode; not whole player CPU or RSS"});
        let path = std::env::var("NCSPOT_STATE_PROFILE_OUT").expect("NCSPOT_STATE_PROFILE_OUT");
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}
