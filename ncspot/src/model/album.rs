use rand::{rng, seq::IteratorRandom};
use rspotify::model::Id;
use std::fmt;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use log::{debug, warn};
use rspotify::model::album::{FullAlbum, SavedAlbum, SimplifiedAlbum};

use crate::library::Library;
use crate::model::artist::Artist;
use crate::model::playable::Playable;
use crate::model::track::{Source, Track};
use crate::queue::Queue;
use crate::spotify::Spotify;
use crate::traits::{IntoBoxedViewExt, ListItem, ViewExt};
use crate::ui::{album::AlbumView, listview::ListView};

#[derive(Clone, Deserialize, Serialize)]
pub struct Album {
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub year: String,
    pub cover_url: Option<String>,
    pub url: Option<String>,
    pub tracks: Option<Vec<Track>>,
    pub added_at: Option<DateTime<Utc>>,
    total_tracks: Option<usize>,
    #[serde(default)]
    pub source: Source,
}

impl Album {
    /// Lo que no es de Spotify no se puede guardar ni quitar desde Spotan.
    pub fn is_read_only(&self) -> bool {
        self.source != Source::Spotify
    }

    /// Álbum de otro servicio (los campos privados impiden armarlo desde afuera).
    #[allow(clippy::too_many_arguments)]
    pub fn new_external(
        source: Source,
        id: String,
        title: String,
        artist: Option<String>,
        year: String,
        cover_url: Option<String>,
        url: Option<String>,
        total_tracks: usize,
    ) -> Album {
        Album {
            id: Some(id),
            title,
            artists: artist.into_iter().collect(),
            artist_ids: Vec::new(),
            year,
            cover_url,
            url,
            tracks: None,
            added_at: None,
            total_tracks: Some(total_tracks),
            source,
        }
    }

    pub fn load_all_tracks(&mut self, spotify: Spotify) {
        if self.tracks.is_some() && self.tracks.as_ref().map(|t| t.len()) == self.total_tracks {
            return;
        }

        if self.source != Source::Spotify {
            let Some(id) = self.id.as_deref() else { return };
            let loaded = match self.source {
                Source::YouTube => crate::external::yt_library::fetch_album_tracks(id),
                _ => crate::external::sc_library::fetch_playlist_tracks(id),
            };
            match loaded {
                Ok(tracks) => {
                    self.total_tracks = Some(tracks.len());
                    self.tracks = Some(tracks);
                }
                // se deja sin cargar para reintentar al abrir de nuevo
                Err(e) => warn!("Failed to load external album {id}: {e}"),
            }
            return;
        }

        if let Some(ref album_id) = self.id {
            let mut collected_tracks = Vec::new();
            if let Ok(full_album) = spotify.api.album(album_id) {
                let mut tracks_result = Some(full_album.tracks.clone());
                while let Some(ref tracks) = tracks_result {
                    for t in &tracks.items {
                        collected_tracks.push(Track::from_simplified_track(t, &full_album));
                    }

                    debug!("got {} tracks", tracks.items.len());

                    // load next batch if necessary
                    tracks_result = match tracks.next {
                        Some(_) => {
                            debug!("requesting tracks again..");
                            let next = spotify
                                .api
                                .album_tracks(
                                    album_id,
                                    50,
                                    tracks.offset + tracks.items.len() as u32,
                                )
                                .ok();
                            if next.is_none() {
                                // leave unloaded rather than store a partial list
                                warn!("Failed to load all tracks of album {album_id}");
                                return;
                            }
                            next
                        }
                        None => None,
                    }
                }
            } else {
                warn!("Failed to load album {album_id}");
                return;
            }

            self.total_tracks = Some(collected_tracks.len());
            self.tracks = Some(collected_tracks);
        }
    }
}

impl From<&SimplifiedAlbum> for Album {
    fn from(sa: &SimplifiedAlbum) -> Self {
        Self {
            id: sa.id.as_ref().map(|id| id.id().to_string()),
            title: sa.name.clone(),
            artists: sa.artists.iter().map(|sa| sa.name.clone()).collect(),
            artist_ids: sa
                .artists
                .iter()
                .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
                .collect(),
            year: sa
                .release_date
                .clone()
                .unwrap_or_default()
                .split('-')
                .next()
                .unwrap()
                .into(),
            cover_url: sa.images.first().map(|i| i.url.clone()),
            url: sa.id.as_ref().map(|id| id.url()),
            tracks: None,
            added_at: None,
            total_tracks: None,
            source: Source::Spotify,
        }
    }
}

impl From<&FullAlbum> for Album {
    fn from(fa: &FullAlbum) -> Self {
        let tracks = Some(
            fa.tracks
                .items
                .iter()
                .map(|st| Track::from_simplified_track(st, fa))
                .collect(),
        );

        Self {
            id: Some(fa.id.id().to_string()),
            title: fa.name.clone(),
            artists: fa.artists.iter().map(|sa| sa.name.clone()).collect(),
            artist_ids: fa
                .artists
                .iter()
                .filter_map(|a| a.id.as_ref().map(|id| id.id().to_string()))
                .collect(),
            year: fa.release_date.split('-').next().unwrap().into(),
            cover_url: fa.images.first().map(|i| i.url.clone()),
            url: Some(fa.id.uri()),
            tracks,
            added_at: None,
            total_tracks: Some(fa.tracks.total as usize),
            source: Source::Spotify,
        }
    }
}

impl From<&SavedAlbum> for Album {
    fn from(sa: &SavedAlbum) -> Self {
        let mut album: Self = (&sa.album).into();
        album.added_at = Some(sa.added_at);
        album
    }
}

impl fmt::Display for Album {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {}", self.artists.join(", "), self.title)
    }
}

impl fmt::Debug for Album {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "({} - {} ({:?}))",
            self.artists.join(", "),
            self.title,
            self.id
        )
    }
}

impl ListItem for Album {
    fn is_playing(&self, queue: &Queue) -> bool {
        if let Some(tracks) = self.tracks.as_ref() {
            let playing: Vec<String> = queue
                .queue
                .read()
                .unwrap()
                .iter()
                .filter_map(|t| t.id())
                .collect();

            let ids: Vec<String> = tracks.iter().filter_map(|t| t.id.clone()).collect();
            !ids.is_empty() && playing == ids
        } else {
            false
        }
    }

    fn display_left(&self, _library: &Library) -> String {
        format!("{self}")
    }

    fn display_right(&self, library: &Library) -> String {
        if self.is_read_only() {
            return format!("[{}] {}", self.source.label(), self.year);
        }
        let saved = if library.is_saved_album(self) {
            if library.cfg.values().use_nerdfont.unwrap_or(false) {
                "\u{f012c} "
            } else {
                crate::utils::glyph("✓ ", "* ")
            }
        } else {
            ""
        };
        format!("{}{}", saved, self.year)
    }

    fn play(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            let tracks: Vec<Playable> = tracks
                .iter()
                .map(|track| Playable::Track(track.clone()))
                .collect();
            queue.play_all(&tracks, true, true);
        }
    }

    fn play_next(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for t in tracks.iter().rev() {
                queue.insert_after_current(Playable::Track(t.clone()));
            }
        }
    }

    fn queue(&mut self, queue: &Queue) {
        self.load_all_tracks(queue.get_spotify());

        if let Some(tracks) = self.tracks.as_ref() {
            for t in tracks {
                queue.append(Playable::Track(t.clone()));
            }
        }
    }

    fn toggle_saved(&mut self, library: &Library) {
        if self.is_read_only() {
            return;
        }
        if library.is_saved_album(self) {
            library.unsave_album(self);
        } else {
            library.save_album(self);
        }
    }

    fn save(&mut self, library: &Library) {
        if self.is_read_only() {
            return;
        }
        library.save_album(self);
    }

    fn unsave(&mut self, library: &Library) {
        if self.is_read_only() {
            return;
        }
        library.unsave_album(self);
    }

    fn open(&self, queue: Arc<Queue>, library: Arc<Library>) -> Option<Box<dyn ViewExt>> {
        Some(AlbumView::new(queue, library, self).into_boxed_view_ext())
    }

    fn open_recommendations(
        &mut self,
        queue: Arc<Queue>,
        library: Arc<Library>,
    ) -> Option<Box<dyn ViewExt>> {
        if self.is_read_only() {
            return None;
        }
        self.load_all_tracks(queue.get_spotify());
        const MAX_SEEDS: usize = 5;
        let track_ids: Vec<&str> = self
            .tracks
            .as_ref()?
            .iter()
            .filter_map(|t| t.id.as_deref())
            // spotify allows at max 5 seed items, so choose 4 random tracks...
            .sample(&mut rng(), MAX_SEEDS - 1);

        let artist_id: Option<String> = self
            .artist_ids
            .iter()
            .cloned()
            // ...and one artist
            .choose(&mut rng());

        if track_ids.is_empty() && artist_id.is_some() {
            return None;
        }

        let spotify = queue.get_spotify();
        let recommendations: Option<Vec<Track>> = spotify
            .api
            .recommendations(
                artist_id.as_ref().map(|aid| vec![aid.as_str()]),
                None,
                Some(track_ids),
            )
            .ok()
            .map(|r| r.tracks)
            .map(|tracks| tracks.iter().map(Track::from).collect());
        recommendations.map(|tracks| {
            ListView::new(
                Arc::new(RwLock::new(tracks)),
                queue.clone(),
                library.clone(),
            )
            .with_title(&format!("Similar to Album \"{}\"", self.title))
            .into_boxed_view_ext()
        })
    }

    fn share_url(&self) -> Option<String> {
        if self.is_read_only() {
            return self.url.clone();
        }
        self.id
            .clone()
            .map(|id| format!("https://open.spotify.com/album/{id}"))
    }

    fn read_only_source(&self) -> Option<Source> {
        self.is_read_only().then_some(self.source)
    }

    fn artists(&self) -> Option<Vec<Artist>> {
        Some(
            self.artist_ids
                .iter()
                .zip(self.artists.iter())
                .map(|(id, name)| Artist::new(id.clone(), name.clone()))
                .collect(),
        )
    }

    #[inline]
    fn is_saved(&self, library: &Library) -> Option<bool> {
        if self.is_read_only() {
            return None;
        }
        Some(library.is_saved_album(self))
    }

    #[inline]
    fn is_playable(&self) -> bool {
        true
    }

    fn as_listitem(&self) -> Box<dyn ListItem> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::events::EventManager;

    #[test]
    fn old_album_cache_defaults_to_spotify() {
        let a: Album = serde_json::from_str(r#"{"id":"x","title":"t","artists":[],"artist_ids":[],"year":"2020","cover_url":null,"url":null,"tracks":null,"added_at":null,"total_tracks":null}"#).unwrap();
        assert_eq!(a.source, Source::Spotify);
        assert!(!a.is_read_only());
    }

    #[test]
    fn sc_album_is_read_only() {
        let a = Album::new_external(Source::SoundCloud, "1815863".into(), "Ecclesia".into(), Some("Forss".into()), "2015".into(), None, Some("https://soundcloud.com/forss/sets/ecclesia".into()), 10);
        assert!(a.is_read_only());
        assert_eq!(a.read_only_source(), Some(Source::SoundCloud));
        assert_eq!(a.share_url().as_deref(), Some("https://soundcloud.com/forss/sets/ecclesia"));
        assert!(a.artists().unwrap().is_empty());
        assert_eq!(format!("{a}"), "Forss - Ecclesia");
    }

    #[test]
    fn sc_album_save_unsave_never_touch_spotify_albums() {
        let (_g, _d) = crate::config::test_sandbox();
        let cfg = Config::new_for_test();
        let ev = EventManager::new_for_test();
        let lib = Library::new_for_test(ev.clone(), Spotify::new_for_test(cfg.clone(), ev), cfg);
        *lib.is_done.write().unwrap() = true;
        let mut a = Album::new_external(Source::SoundCloud, "1".into(), "t".into(), None, "".into(), None, None, 1);
        a.save(&lib);
        a.toggle_saved(&lib);
        a.unsave(&lib);
        assert!(lib.albums.read().unwrap().is_empty());
        assert_eq!(a.is_saved(&lib), None);
    }
}
