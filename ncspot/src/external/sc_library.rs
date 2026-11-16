//! Biblioteca de SoundCloud (Likes y playlists de un perfil): parseo de las respuestas de api-v2.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::Value;

use super::soundcloud::parse_track;
use crate::model::playlist::Playlist;
use crate::model::track::{Source, Track};

/// `Some("https://soundcloud.com/<usuario>")` para un perfil (usuario o URL); `None` si no lo es.
pub fn normalize_profile(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.split(['?', '#']).next().unwrap_or("");
    let user = match s.split_once("://").map_or(s, |(_, rest)| rest).split_once('/') {
        // con dominio: el usuario es el primer segmento del path
        Some((host, path)) if host.contains('.') => {
            let host = host.trim_start_matches("www.").trim_start_matches("m.");
            if host != "soundcloud.com" {
                return None;
            }
            path.split('/').next().unwrap_or("")
        }
        Some(_) => return None,
        None if s.contains('.') => {
            let host = s.split_once("://").map_or(s, |(_, r)| r);
            let host = host.trim_start_matches("www.").trim_start_matches("m.");
            if host == "soundcloud.com" { "" } else { return None }
        }
        None => s,
    };
    let valid = Regex::new(r"^[A-Za-z0-9_-]+$").expect("regex");
    valid.is_match(user).then(|| format!("https://soundcloud.com/{}", user.to_lowercase()))
}

fn next_href(json: &Value) -> Option<String> {
    json["next_href"].as_str().map(String::from)
}

fn collection(json: &Value) -> impl Iterator<Item = &Value> {
    json["collection"].as_array().into_iter().flatten()
}

/// Temas con Like (`/users/{id}/track_likes`); `added_at` es la fecha del Like.
pub fn parse_track_likes(json: &Value) -> (Vec<Track>, Option<String>) {
    let tracks = collection(json)
        .filter_map(|item| {
            let mut t = parse_track(&item["track"])?;
            t.added_at = item["created_at"].as_str().and_then(|d| d.parse::<DateTime<Utc>>().ok());
            Some(t)
        })
        .collect();
    (tracks, next_href(json))
}

pub fn parse_playlist(p: &Value) -> Option<Playlist> {
    Some(Playlist {
        id: p["id"].as_u64()?.to_string(),
        name: p["title"].as_str()?.to_string(),
        owner_id: p["user"]["id"].as_u64().map(|i| i.to_string()).unwrap_or_default(),
        owner_name: p["user"]["username"].as_str().map(String::from),
        snapshot_id: p["last_modified"].as_str().unwrap_or_default().to_string(),
        num_tracks: p["track_count"].as_u64().unwrap_or(0) as usize,
        tracks: None,
        collaborative: false,
        source: Source::SoundCloud,
    })
}

/// Playlists propias (`/users/{id}/playlists`).
pub fn parse_playlists(json: &Value) -> (Vec<Playlist>, Option<String>) {
    (collection(json).filter_map(parse_playlist).collect(), next_href(json))
}

/// Playlists guardadas: solo los items de `/likes` con clave `playlist`.
pub fn parse_liked_playlists(json: &Value) -> (Vec<Playlist>, Option<String>) {
    (collection(json).filter_map(|i| parse_playlist(i.get("playlist")?)).collect(), next_href(json))
}

/// Propias primero, después guardadas, sin repetir id.
pub fn merge_playlists(own: Vec<Playlist>, liked: Vec<Playlist>) -> Vec<Playlist> {
    let mut seen = HashSet::new();
    own.into_iter().chain(liked).filter(|p| seen.insert(p.id.clone())).collect()
}

/// Ids de todos los temas de una playlist, en orden, y los que ya vienen completos
/// (el resto son stubs que hay que pedir por `/tracks?ids=`).
pub fn playlist_order(json: &Value) -> (Vec<String>, Vec<Track>) {
    let items = json["tracks"].as_array().into_iter().flatten();
    let order = items.clone().filter_map(|t| t["id"].as_u64()).map(|i| i.to_string()).collect();
    (order, items.filter_map(parse_track).collect())
}

/// Temas en el orden de `order`; los ids sin tema se omiten.
pub fn order_tracks(order: &[String], known: Vec<Track>) -> Vec<Track> {
    let mut by_id: HashMap<String, Track> = known.into_iter().filter_map(|t| Some((t.id.clone()?, t))).collect();
    order.iter().filter_map(|id| by_id.remove(id)).collect()
}

/// Respuesta de `/tracks?ids=` (arreglo en la raíz).
pub fn parse_tracks(json: &Value) -> Vec<Track> {
    json.as_array().into_iter().flatten().filter_map(parse_track).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> Value {
        let text = match name {
            "sc_track_likes" => include_str!("fixtures/sc_track_likes.json"),
            "sc_likes" => include_str!("fixtures/sc_likes.json"),
            "sc_playlists" => include_str!("fixtures/sc_playlists.json"),
            "sc_playlist" => include_str!("fixtures/sc_playlist.json"),
            "sc_tracks_ids" => include_str!("fixtures/sc_tracks_ids.json"),
            _ => panic!("fixture {name}"),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn normalize_profile_variants() {
        let ok = Some("https://soundcloud.com/forss".to_string());
        for i in [
            "forss",
            " forss ",
            "soundcloud.com/forss",
            "https://soundcloud.com/forss",
            "https://m.soundcloud.com/forss/",
            "https://soundcloud.com/forss/likes",
            "http://www.soundcloud.com/forss?ref=x",
        ] {
            assert_eq!(normalize_profile(i), ok, "{i}");
        }
        for i in ["", "https://open.spotify.com/user/x", "https://soundcloud.com/", "a b"] {
            assert_eq!(normalize_profile(i), None, "{i}");
        }
    }

    #[test]
    fn track_likes_page() {
        let (t, next) = parse_track_likes(&fx("sc_track_likes"));
        assert_eq!(t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>()[0], "Warm");
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].id.as_deref(), Some("201247503"));
        assert_eq!(t[0].source, Source::SoundCloud);
        assert_eq!(t[0].added_at.unwrap().to_rfc3339(), "2019-10-21T05:44:51+00:00");
        assert!(next.is_some());
    }

    #[test]
    fn liked_playlists_skip_tracks() {
        let (p, _) = parse_liked_playlists(&fx("sc_likes"));
        assert_eq!(p.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["272741983", "729308781"]);
    }

    #[test]
    fn own_playlists() {
        let (p, next) = parse_playlists(&fx("sc_playlists"));
        assert_eq!((p[0].id.as_str(), p[0].name.as_str(), p[0].num_tracks), ("2050462", "Ecclesia Inspiration", 7));
        assert_eq!(p[0].owner_name.as_deref(), Some("Forss"));
        assert_eq!(p[0].source, Source::SoundCloud);
        assert!(p[0].tracks.is_none() && next.is_none());
    }

    #[test]
    fn merge_playlists_dedups_by_id() {
        let (own, _) = parse_playlists(&fx("sc_playlists"));
        let mut liked = own[..1].to_vec();
        liked.extend(parse_liked_playlists(&fx("sc_likes")).0);
        let ids: Vec<_> = merge_playlists(own, liked).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["2050462", "1815863", "272741983", "729308781"]);
    }

    #[test]
    fn playlist_stubs_completed_in_order() {
        let (order, known) = playlist_order(&fx("sc_playlist"));
        assert_eq!(order.len(), 7);
        assert_eq!(known.len(), 5);
        let mut all = known;
        all.extend(parse_tracks(&fx("sc_tracks_ids")));
        let t = order_tracks(&order, all);
        assert_eq!(t.len(), 7);
        assert_eq!(t[5].title, "Bach - Cello Suite No.1 Prelude");
        assert_eq!(t[6].id.as_deref(), Some("17523896"));
    }

    #[test]
    fn missing_tracks_are_skipped() {
        let (order, known) = playlist_order(&fx("sc_playlist"));
        let t = order_tracks(&order, known); // sin completar los 2 stubs
        assert_eq!(t.len(), 5);
        assert_eq!(t.iter().map(|t| t.id.clone().unwrap()).collect::<Vec<_>>(), order[..5]);
    }

    #[test]
    fn old_playlist_cache_defaults_to_spotify() {
        let p: Playlist = serde_json::from_str(
            r#"{"id":"x","name":"n","owner_id":"o","owner_name":null,"snapshot_id":"s","num_tracks":0,"tracks":null,"collaborative":false}"#,
        )
        .unwrap();
        assert_eq!(p.source, Source::Spotify);
    }
}
