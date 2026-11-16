//! Biblioteca de SoundCloud (Likes y playlists de un perfil): parseo de las respuestas de api-v2.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::Value;
use url::Url;

use super::soundcloud::{api, get_json, parse_track, ScError, API};
use crate::config::config_path;
use crate::model::album::Album;
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

/// Un set de SoundCloud: playlist común o álbum/EP/single.
pub enum ScSet {
    List(Playlist),
    Album(Album),
}

pub fn parse_set(p: &Value) -> Option<ScSet> {
    let is_album = p["is_album"].as_bool() == Some(true)
        || matches!(p["set_type"].as_str(), Some("album" | "ep" | "single" | "compilation"));
    if !is_album {
        return parse_playlist(p).map(ScSet::List);
    }
    let year = ["release_date", "published_at", "created_at", "last_modified"]
        .iter()
        .filter_map(|k| p[*k].as_str())
        .find(|d| !d.is_empty())
        .map(|d| d.chars().take(4).collect())
        .unwrap_or_default();
    Some(ScSet::Album(Album::new_external(
        Source::SoundCloud,
        p["id"].as_u64()?.to_string(),
        p["title"].as_str()?.to_string(),
        p["user"]["username"].as_str().map(String::from),
        year,
        p["artwork_url"].as_str().map(String::from),
        p["permalink_url"].as_str().map(String::from),
        p["track_count"].as_u64().unwrap_or(0) as usize,
    )))
}

/// Sets propios (`/users/{id}/playlists`).
pub fn parse_playlists(json: &Value) -> (Vec<ScSet>, Option<String>) {
    (collection(json).filter_map(parse_set).collect(), next_href(json))
}

/// Sets guardados: solo los items de `/likes` con clave `playlist`.
pub fn parse_liked_playlists(json: &Value) -> (Vec<ScSet>, Option<String>) {
    (collection(json).filter_map(|i| parse_set(i.get("playlist")?)).collect(), next_href(json))
}

/// Propios primero, después guardados, sin repetir id dentro de cada tipo.
pub fn split_sets(own: Vec<ScSet>, liked: Vec<ScSet>) -> (Vec<Playlist>, Vec<Album>) {
    let (mut lists, mut albums) = (Vec::new(), Vec::new());
    let (mut seen_l, mut seen_a) = (HashSet::new(), HashSet::new());
    for set in own.into_iter().chain(liked) {
        match set {
            ScSet::List(p) if seen_l.insert(p.id.clone()) => lists.push(p),
            ScSet::Album(a) if seen_a.insert(a.id.clone()) => albums.push(a),
            _ => {}
        }
    }
    (lists, albums)
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
    let by_id: HashMap<String, Track> = known.into_iter().filter_map(|t| Some((t.id.clone()?, t))).collect();
    order.iter().filter_map(|id| by_id.get(id).cloned()).collect()
}

/// Respuesta de `/tracks?ids=` (arreglo en la raíz).
pub fn parse_tracks(json: &Value) -> Vec<Track> {
    json.as_array().into_iter().flatten().filter_map(parse_track).collect()
}

const PROFILE_FILE: &str = "soundcloud_profile";
const MAX_PAGES: usize = 100;

#[derive(Clone, Debug)]
pub struct ScUser {
    pub id: u64,
    pub username: String,
    pub likes: u64,
    pub playlists: u64,
}

pub fn resolve(profile_url: &str) -> Result<ScUser, String> {
    let json = api(|cid| {
        let mut u = Url::parse(&format!("{API}/resolve")).expect("url");
        u.query_pairs_mut().append_pair("url", profile_url).append_pair("client_id", cid);
        get_json(u.as_str()).map_err(|e| match e {
            ScError::NotFound => ScError::Other("No existe ese perfil de SoundCloud".into()),
            e => e,
        })
    })?;
    Ok(ScUser {
        id: json["id"].as_u64().ok_or("SoundCloud no devolvió el usuario")?,
        username: json["username"].as_str().unwrap_or_default().to_string(),
        likes: json["likes_count"].as_u64().unwrap_or(0),
        playlists: json["playlist_count"].as_u64().unwrap_or(0),
    })
}

fn get(url: &str) -> Result<Value, String> {
    api(|cid| get_json(&format!("{url}{}client_id={cid}", if url.contains('?') { "&" } else { "?" })))
}

/// Todas las páginas de un listado; si una falla o `next_href` no termina, error (nada parcial).
fn all_pages<T>(first: String, parse: impl Fn(&Value) -> (Vec<T>, Option<String>)) -> Result<Vec<T>, String> {
    let (mut out, mut url) = (Vec::new(), first);
    for _ in 0..MAX_PAGES {
        let (items, next) = parse(&get(&url)?);
        out.extend(items);
        match next {
            Some(n) => url = n,
            None => return Ok(out),
        }
    }
    Err("SoundCloud devolvió demasiadas páginas".into())
}

pub struct ScLibrary {
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub albums: Vec<Album>,
}

pub fn fetch_library(user_id: u64) -> Result<ScLibrary, String> {
    let list = |what: &str| format!("{API}/users/{user_id}/{what}?limit=200&linked_partitioning=1");
    let likes = all_pages(list("track_likes"), parse_track_likes)?;
    let own = all_pages(list("playlists"), parse_playlists)?;
    let saved = all_pages(list("likes"), parse_liked_playlists)?;
    let (playlists, albums) = split_sets(own, saved);
    Ok(ScLibrary { tracks: likes, playlists, albums })
}

pub fn fetch_playlist_tracks(id: &str) -> Result<Vec<Track>, String> {
    let (order, mut known) = playlist_order(&get(&format!("{API}/playlists/{id}"))?);
    let have: HashSet<_> = known.iter().filter_map(|t| t.id.clone()).collect();
    let missing: Vec<_> = order.iter().filter(|i| !have.contains(*i)).cloned().collect();
    for batch in missing.chunks(50) {
        known.extend(parse_tracks(&get(&format!("{API}/tracks?ids={}", batch.join(",")))?));
    }
    Ok(order_tracks(&order, known))
}

fn profile_path() -> std::path::PathBuf {
    config_path(PROFILE_FILE)
}

pub fn profile() -> Option<String> {
    std::fs::read_to_string(profile_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `None` borra el archivo.
pub fn set_profile(url: Option<&str>) -> Result<(), String> {
    let path = profile_path();
    match url {
        Some(u) => std::fs::write(path, u).map_err(|e| e.to_string()),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    }
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

    fn ids(sets: Vec<ScSet>) -> Vec<String> {
        sets.into_iter()
            .map(|s| match s {
                ScSet::List(p) => p.id,
                ScSet::Album(a) => a.id.unwrap(),
            })
            .collect()
    }

    #[test]
    fn liked_playlists_skip_tracks() {
        let (p, _) = parse_liked_playlists(&fx("sc_likes"));
        assert_eq!(ids(p), ["272741983", "729308781"]);
    }

    #[test]
    fn own_playlists() {
        let (p, next) = parse_playlists(&fx("sc_playlists"));
        let Some(ScSet::List(p)) = p.into_iter().next() else { panic!("playlist") };
        assert_eq!((p.id.as_str(), p.name.as_str(), p.num_tracks), ("2050462", "Ecclesia Inspiration", 7));
        assert_eq!(p.owner_name.as_deref(), Some("Forss"));
        assert_eq!(p.source, Source::SoundCloud);
        assert!(p.tracks.is_none() && next.is_none());
    }

    #[test]
    fn split_sets_dedups_by_id() {
        let (own, _) = parse_playlists(&fx("sc_playlists"));
        let (mut liked, _) = parse_playlists(&fx("sc_playlists"));
        liked.extend(parse_liked_playlists(&fx("sc_likes")).0);
        let (lists, _) = split_sets(own, liked);
        assert_eq!(lists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["2050462", "272741983", "729308781"]);
    }

    #[test]
    fn album_goes_to_albums_once() {
        let (own, _) = parse_playlists(&fx("sc_playlists"));
        let (liked, _) = parse_playlists(&fx("sc_playlists")); // el mismo álbum también con Like
        let (lists, albums) = split_sets(own, liked);
        assert_eq!(lists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["2050462"]);
        assert_eq!(albums.len(), 1);
        let a = &albums[0];
        assert_eq!((a.id.as_deref(), a.title.as_str(), a.year.as_str()), (Some("1815863"), "Ecclesia", "2015"));
        assert_eq!(a.artists, ["Forss"]);
        assert_eq!(a.source, Source::SoundCloud);
    }

    #[test]
    fn set_type_ep_is_album_and_missing_dates_give_empty_year() {
        let ep = serde_json::json!({"id": 9, "title": "E", "track_count": 3, "user": {"id": 1, "username": "u"}, "is_album": false, "set_type": "ep"});
        assert!(matches!(parse_set(&ep), Some(ScSet::Album(a)) if a.year.is_empty()));
        let pl = serde_json::json!({"id": 8, "title": "P", "track_count": 3, "user": {"id": 1, "username": "u"}, "is_album": false, "set_type": ""});
        assert!(matches!(parse_set(&pl), Some(ScSet::List(_))));
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
    fn repeated_track_in_playlist_is_kept() {
        let (order, known) = playlist_order(&fx("sc_playlist"));
        let first = order[0].clone();
        let mut order = order[..5].to_vec();
        order.push(first.clone());
        let t = order_tracks(&order, known);
        assert_eq!(t.len(), 6);
        assert_eq!(t[0].id, t[5].id);
    }

    #[test]
    fn profile_roundtrip() {
        let (_g, _d) = crate::config::test_sandbox();
        assert_eq!(profile(), None);
        set_profile(Some("https://soundcloud.com/forss")).unwrap();
        assert_eq!(profile().as_deref(), Some("https://soundcloud.com/forss"));
        set_profile(None).unwrap();
        assert_eq!(profile(), None);
        set_profile(None).unwrap(); // borrar sin archivo no es error
    }

    /// Red real: lo corre el usuario con `docs/superpowers/scripts/sc-fetch.sh`.
    #[test]
    #[ignore]
    fn sc_live_forss() {
        let u = resolve("https://soundcloud.com/forss").unwrap();
        assert!(resolve("https://soundcloud.com/zzz-no-existe-9f3a1c").is_err());
        let lib = fetch_library(u.id).unwrap();
        println!("{} ({}): {} likes, {} playlists, {} albums", u.username, u.id, lib.tracks.len(), lib.playlists.len(), lib.albums.len());
        assert!(lib.tracks.len() >= 200 && lib.playlists.len() + lib.albums.len() >= 4);
        assert_eq!(fetch_playlist_tracks("2050462").unwrap().len(), 7);
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
