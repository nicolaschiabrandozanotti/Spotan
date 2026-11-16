use std::fmt;
use std::sync::{Arc, RwLock};

use crate::config;
use crate::utils::ms_to_hms;
use chrono::{DateTime, Utc};
use rspotify::model::Id;
use rspotify::model::album::FullAlbum;
use rspotify::model::track::{FullTrack, SavedTrack, SimplifiedTrack};

use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::queue::Queue;
use crate::traits::{IntoBoxedViewExt, ListItem, ViewExt};
use crate::ui::listview::ListView;

/// Servicio de origen de un tema.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Spotify,
    YouTube,
    SoundCloud,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spotify => "S",
            Self::YouTube => "YT",
            Self::SoundCloud => "SC",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Spotify => "Spotify",
            Self::YouTube => "YouTube",
            Self::SoundCloud => "SoundCloud",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Spotify => "spotify",
            Self::YouTube => "youtube",
            Self::SoundCloud => "soundcloud",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Spotify, Self::YouTube, Self::SoundCloud]
            .into_iter()
            .find(|s| s.key() == key)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Track {
    pub id: Option<String>,
    pub uri: String,
    pub title: String,
    pub track_number: u32,
    pub disc_number: i32,
    pub duration: u32,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artists: Vec<String>,
    pub cover_url: Option<String>,
    pub url: String,
    pub added_at: Option<DateTime<Utc>>,
    pub list_index: usize,
    pub is_local: bool,
    pub is_playable: Option<bool>,
    #[serde(default)]
    pub source: Source,
}

impl Track {
    pub fn external(
        source: Source,
        id: String,
        title: String,
        artist: String,
        duration_ms: u32,
        cover_url: Option<String>,
        url: String,
    ) -> Self {
        Self {
            uri: format!("{}:track:{id}", source.key()),
            id: Some(id),
            title,
            track_number: 0,
            disc_number: 0,
            duration: duration_ms,
            artists: vec![artist],
            artist_ids: Vec::new(),
            album: None,
            album_id: None,
            album_artists: Vec::new(),
            cover_url,
            url,
            added_at: None,
            list_index: 0,
            is_local: false,
            is_playable: Some(true),
            source,
        }
    }

    pub fn from_simplified_track(track: &SimplifiedTrack, album: &FullAlbum) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
            .collect::<Vec<String>>();
        let album_artists = album
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: Some(album.name.clone()),
            album_id: Some(album.id.id().to_string()),
            album_artists,
            cover_url: album.images.first().map(|img| img.url.clone()),
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
            source: Source::Spotify,
        }
    }

    pub fn duration_str(&self) -> String {
        ms_to_hms(self.duration)
    }
}

impl From<&SimplifiedTrack> for Track {
    fn from(track: &SimplifiedTrack) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|a| a.id().to_string()))
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: None,
            album_id: None,
            album_artists: Vec::new(),
            cover_url: None,
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
            source: Source::Spotify,
        }
    }
}

impl From<&FullTrack> for Track {
    fn from(track: &FullTrack) -> Self {
        let artists = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();
        let artist_ids = track
            .artists
            .iter()
            .filter_map(|a| a.id.as_ref().map(|a| a.id().to_string()))
            .collect::<Vec<String>>();
        let album_artists = track
            .album
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<String>>();

        Self {
            id: track.id.as_ref().map(|id| id.id().to_string()),
            uri: track.id.as_ref().map(|id| id.uri()).unwrap_or_default(),
            title: track.name.clone(),
            track_number: track.track_number,
            disc_number: track.disc_number,
            duration: track.duration.num_milliseconds() as u32,
            artists,
            artist_ids,
            album: Some(track.album.name.clone()),
            album_id: track.album.id.as_ref().map(|a| a.id().to_string()),
            album_artists,
            cover_url: track.album.images.first().map(|img| img.url.clone()),
            url: track.id.as_ref().map(|id| id.url()).unwrap_or_default(),
            added_at: None,
            list_index: 0,
            is_local: track.is_local,
            is_playable: track.is_playable,
            source: Source::Spotify,
        }
    }
}

impl From<&SavedTrack> for Track {
    fn from(st: &SavedTrack) -> Self {
        let mut track: Self = (&st.track).into();
        track.added_at = Some(st.added_at);
        track
    }
}

impl fmt::Display for Track {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {}", self.artists.join(", "), self.title)
    }
}

impl ListItem for Track {
    fn is_playing(&self, queue: &Queue) -> bool {
        let current = queue.get_current();
        current.map(|t| t.id() == self.id).unwrap_or(false)
    }

    fn display_left(&self, library: &Library) -> String {
        let formatting = library
            .cfg
            .values()
            .track_format
            .clone()
            .unwrap_or_default();
        let default = config::TrackFormat::default().left.unwrap();
        let left = formatting.left.unwrap_or_else(|| default.clone());
        if left != default {
            Playable::format(&Playable::Track(self.clone()), &left, library)
        } else {
            format!("{self}")
        }
    }

    fn display_center(&self, library: &Library) -> String {
        let formatting = library
            .cfg
            .values()
            .track_format
            .clone()
            .unwrap_or_default();
        let default = config::TrackFormat::default().center.unwrap();
        let center = formatting.center.unwrap_or_else(|| default.clone());
        if center != default {
            Playable::format(&Playable::Track(self.clone()), &center, library)
        } else {
            self.album.clone().unwrap_or_default()
        }
    }

    fn display_right(&self, library: &Library) -> String {
        let formatting = library
            .cfg
            .values()
            .track_format
            .clone()
            .unwrap_or_default();
        let default = config::TrackFormat::default().right.unwrap();
        let right = formatting.right.unwrap_or_else(|| default.clone());
        if right != default {
            Playable::format(&Playable::Track(self.clone()), &right, library)
        } else {
            let saved = if library.is_saved_track(&Playable::Track(self.clone())) {
                if library.cfg.values().use_nerdfont.unwrap_or(false) {
                    "\u{f012c}"
                } else {
                    crate::utils::glyph("✓", "*")
                }
            } else {
                ""
            };
            let tag = match self.source {
                Source::Spotify => String::new(),
                other => format!("[{}] ", other.label()),
            };
            format!("{tag}{saved} {}", self.duration_str())
        }
    }

    fn play(&mut self, queue: &Queue) {
        let index = queue.append_next(&[Playable::Track(self.clone())]);
        queue.play(index, true, false);
    }

    fn play_next(&mut self, queue: &Queue) {
        queue.insert_after_current(Playable::Track(self.clone()));
    }

    fn queue(&mut self, queue: &Queue) {
        queue.append(Playable::Track(self.clone()));
    }

    fn toggle_saved(&mut self, library: &Library) {
        if self.source != Source::Spotify {
            return;
        }
        if library.is_saved_track(&Playable::Track(self.clone())) {
            library.unsave_tracks(&[self]);
        } else {
            library.save_tracks(&[self]);
        }
    }

    fn save(&mut self, library: &Library) {
        if self.source != Source::Spotify {
            return;
        }
        library.save_tracks(&[self]);
    }

    fn unsave(&mut self, library: &Library) {
        if self.source != Source::Spotify {
            return;
        }
        library.unsave_tracks(&[self]);
    }

    fn open(&self, _queue: Arc<Queue>, _library: Arc<Library>) -> Option<Box<dyn ViewExt>> {
        None
    }

    fn open_recommendations(
        &mut self,
        queue: Arc<Queue>,
        library: Arc<Library>,
    ) -> Option<Box<dyn ViewExt>> {
        if self.source != Source::Spotify {
            return None;
        }
        let spotify = queue.get_spotify();

        let recommendations: Option<Vec<Self>> = if let Some(id) = &self.id {
            spotify
                .api
                .recommendations(None, None, Some(vec![id]))
                .ok()
                .map(|r| r.tracks)
                .map(|tracks| tracks.iter().map(Self::from).collect())
        } else {
            None
        };

        recommendations.map(|tracks| {
            ListView::new(
                Arc::new(RwLock::new(tracks)),
                queue.clone(),
                library.clone(),
            )
            .with_title(&format!(
                "Similar to \"{} - {}\"",
                self.artists.join(", "),
                self.title
            ))
            .into_boxed_view_ext()
        })
    }

    fn share_url(&self) -> Option<String> {
        if self.source != Source::Spotify {
            return Some(self.url.clone());
        }
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/track/{id}"))
    }

    fn album(&self, queue: &Queue) -> Option<Album> {
        if self.source != Source::Spotify {
            return None;
        }
        let spotify = queue.get_spotify();

        match self.album_id {
            Some(ref album_id) => spotify.api.album(album_id).map(|ref fa| fa.into()).ok(),
            None => None,
        }
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        if self.source != Source::Spotify {
            return None;
        }
        Some(
            self.artist_ids
                .iter()
                .zip(self.artists.iter())
                .map(|(id, name)| Artist::new(id.clone(), name.clone()))
                .collect(),
        )
    }

    fn track(&self) -> Option<Track> {
        Some(self.clone())
    }

    #[inline]
    fn is_saved(&self, library: &Library) -> Option<bool> {
        if self.source != Source::Spotify {
            return None;
        }
        Some(library.is_saved_track(&Playable::Track(self.clone())))
    }

    #[inline]
    fn is_playable(&self) -> bool {
        self.is_playable == Some(true)
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::playable::Playable;

    fn yt() -> Track {
        Track::external(
            Source::YouTube,
            "fJ9rUzIMcZQ".into(),
            "Bohemian Rhapsody".into(),
            "Queen".into(),
            354_000,
            None,
            "https://www.youtube.com/watch?v=fJ9rUzIMcZQ".into(),
        )
    }

    #[test]
    fn track_without_source_deserializes_as_spotify() {
        let mut v = serde_json::to_value(yt()).unwrap();
        v.as_object_mut().unwrap().remove("source");
        let t: Track = serde_json::from_value(v).unwrap();
        assert_eq!(t.source, Source::Spotify);
    }

    #[test]
    fn external_track_roundtrips_cbor() {
        let t = yt();
        let back: Track = serde_cbor::from_slice(&serde_cbor::to_vec(&t).unwrap()).unwrap();
        assert_eq!(back.uri, "youtube:track:fJ9rUzIMcZQ");
        assert_eq!(back.source, Source::YouTube);
        assert_eq!(back.title, t.title);
        assert_eq!(back.artists, t.artists);
        assert_eq!(Source::from_key("soundcloud"), Some(Source::SoundCloud));
    }

    #[test]
    fn legacy_reader_ignores_source_field() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Legacy {
            id: Option<String>,
            uri: String,
            title: String,
        }
        let bytes = serde_cbor::to_vec(&yt()).unwrap();
        let l: Legacy = serde_cbor::from_slice(&bytes).unwrap();
        assert_eq!(l.uri, "youtube:track:fJ9rUzIMcZQ");
    }

    #[test]
    fn external_track_has_no_spotify_id() {
        let id: Option<rspotify::prelude::PlayableId> = (&Playable::Track(yt())).into();
        assert!(id.is_none());
    }
}
