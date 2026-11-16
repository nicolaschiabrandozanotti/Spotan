//! Cliente de SoundCloud sin yt-dlp: búsqueda y lista HLS (AAC en fMP4) vía api-v2.

use std::fs;
use std::sync::Mutex;

use regex::Regex;
use serde_json::Value;
use url::Url;

use super::hls::{parse_m3u8, Hls};
use super::{http, Page};
use crate::config::cache_path;
use crate::model::track::{Source, Track};

// Todo lo que SoundCloud puede romper vive acá (verificado el 2026-11-16).
const HOME: &str = "https://soundcloud.com/";
const API: &str = "https://api-v2.soundcloud.com";
const SCRIPT_RE: &str = r#"https://a-v2\.sndcdn\.com/assets/[^"'\s]+\.js"#;
const CLIENT_ID_RE: &str = r#"client_id\s*:\s*"([0-9a-zA-Z]{32})""#;
const PRESETS: [&str; 2] = ["aac_160k", "aac_96k"];
const CACHE_FILE: &str = "soundcloud_client_id";

pub enum ScError {
    Unauthorized,
    Other(String),
}

/// Scripts de la portada, en orden inverso (el `client_id` suele estar en los últimos).
pub fn parse_script_urls(html: &str) -> Vec<String> {
    let re = Regex::new(SCRIPT_RE).expect("regex");
    let mut v: Vec<String> = re.find_iter(html).map(|m| m.as_str().to_string()).collect();
    v.reverse();
    v
}

pub fn parse_client_id(js: &str) -> Option<String> {
    let re = Regex::new(CLIENT_ID_RE).expect("regex");
    Some(re.captures(js)?[1].to_string())
}

/// URL del transcoding HLS fMP4 (AAC 160k, si no 96k); nunca el cifrado.
pub fn pick_transcoding(track: &Value) -> Option<String> {
    let list = track["media"]["transcodings"].as_array()?;
    PRESETS.iter().find_map(|preset| {
        list.iter().find(|t| {
            t["preset"] == *preset
                && t["format"]["protocol"] == "hls"
                && t["format"]["mime_type"].as_str().is_some_and(|m| m.starts_with("audio/mp4"))
        })?["url"]
            .as_str()
            .map(String::from)
    })
}

pub fn parse_track(t: &Value) -> Option<Track> {
    let mut track = Track::external(
        Source::SoundCloud,
        t["id"].as_u64()?.to_string(),
        t["title"].as_str()?.to_string(),
        t["user"]["username"].as_str().unwrap_or_default().to_string(),
        t["duration"].as_u64().unwrap_or(0) as u32,
        t["artwork_url"].as_str().map(String::from),
        t["permalink_url"].as_str().unwrap_or_default().to_string(),
    );
    track.is_playable = Some(pick_transcoding(t).is_some());
    Some(track)
}

pub fn parse_search(json: &Value) -> Page {
    let tracks = json["collection"].as_array().into_iter().flatten().filter_map(parse_track).collect();
    Page { tracks, next: json["next_href"].as_str().map(String::from) }
}

/// Usa el `client_id` guardado; ante un 401/403 pide uno nuevo y reintenta una sola vez.
fn with_client_id<T>(
    cached: Option<String>,
    get: impl Fn(&str) -> Result<T, ScError>,
    refresh: impl Fn() -> Result<String, String>,
) -> Result<T, String> {
    let cid = match cached {
        Some(c) => c,
        None => refresh()?,
    };
    match get(&cid) {
        Err(ScError::Unauthorized) => match get(&refresh()?) {
            Err(ScError::Unauthorized) => Err("SoundCloud rechazó el client_id".into()),
            Err(ScError::Other(e)) => Err(e),
            Ok(v) => Ok(v),
        },
        Err(ScError::Other(e)) => Err(e),
        Ok(v) => Ok(v),
    }
}

static CLIENT_ID: Mutex<Option<String>> = Mutex::new(None);

fn stored_client_id() -> Option<String> {
    let mut slot = CLIENT_ID.lock().unwrap();
    if slot.is_none() {
        *slot = fs::read_to_string(cache_path(CACHE_FILE)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }
    slot.clone()
}

fn refresh_client_id() -> Result<String, String> {
    *CLIENT_ID.lock().unwrap() = None;
    let _ = fs::remove_file(cache_path(CACHE_FILE));
    let html = http().get(HOME).send().and_then(|r| r.text()).map_err(|e| e.to_string())?;
    for script in parse_script_urls(&html) {
        let Ok(js) = http().get(&script).send().and_then(|r| r.text()) else { continue };
        if let Some(cid) = parse_client_id(&js) {
            let _ = fs::write(cache_path(CACHE_FILE), &cid);
            *CLIENT_ID.lock().unwrap() = Some(cid.clone());
            return Ok(cid);
        }
    }
    Err("no se encontró el client_id de SoundCloud".into())
}

fn get_json(url: &str) -> Result<Value, ScError> {
    let resp = http().get(url).send().map_err(|e| ScError::Other(e.to_string()))?;
    match resp.status().as_u16() {
        401 | 403 => Err(ScError::Unauthorized),
        s if s >= 400 => Err(ScError::Other(format!("SoundCloud respondió {s}"))),
        _ => resp.json().map_err(|e| ScError::Other(e.to_string())),
    }
}

fn api<T>(f: impl Fn(&str) -> Result<T, ScError>) -> Result<T, String> {
    with_client_id(stored_client_id(), f, refresh_client_id)
}

pub fn search(query: &str, next: Option<&str>) -> Result<Page, String> {
    let limit = crate::authentication::search_page_size();
    let json = api(|cid| {
        let url = match next {
            Some(n) => format!("{n}&client_id={cid}"),
            None => {
                let mut u = Url::parse(&format!("{API}/search/tracks")).expect("url");
                u.query_pairs_mut()
                    .append_pair("q", query)
                    .append_pair("client_id", cid)
                    .append_pair("limit", &limit.to_string())
                    .append_pair("linked_partitioning", "1");
                u.to_string()
            }
        };
        get_json(&url)
    })?;
    Ok(parse_search(&json))
}

pub fn playlist(track_id: &str) -> Result<Hls, String> {
    let m3u8_url = api(|cid| {
        let track = get_json(&format!("{API}/tracks/{track_id}?client_id={cid}"))?;
        let transcoding = pick_transcoding(&track).ok_or_else(|| ScError::Other("tema sin audio AAC sin cifrar".into()))?;
        let auth = track["track_authorization"].as_str().unwrap_or_default();
        let resp = get_json(&format!("{transcoding}?client_id={cid}&track_authorization={auth}"))?;
        resp["url"].as_str().map(String::from).ok_or_else(|| ScError::Other("respuesta sin url".into()))
    })?;
    let text = http().get(&m3u8_url).send().and_then(|r| r.text()).map_err(|e| e.to_string())?;
    parse_m3u8(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("fixtures/sc_search.json")).unwrap()
    }

    #[test]
    fn parse_search_marks_drm_unplayable() {
        let page = parse_search(&fixture());
        assert_eq!(page.tracks.len(), 3);
        assert_eq!(page.tracks[0].title, "Dâu Hào Môn Lofi");
        assert_eq!(page.tracks[2].title, "Tema con DRM");
        let playable: Vec<_> = page.tracks.iter().map(|t| t.is_playable).collect();
        assert_eq!(playable, [Some(true), Some(true), Some(false)]);
        assert!(page.next.is_some());
    }

    #[test]
    fn pick_transcoding_prefers_160k() {
        let f = fixture();
        let url = pick_transcoding(&f["collection"][1]).unwrap();
        assert!(url.contains("fake0"), "{url}");
        assert!(pick_transcoding(&f["collection"][2]).is_none());
    }

    #[test]
    fn parse_client_id_from_js() {
        let js = r#"x={client_id:"abcdefghijklmnopqrstuvwxyz012345"},y"#;
        assert_eq!(parse_client_id(js).as_deref(), Some("abcdefghijklmnopqrstuvwxyz012345"));
        assert_eq!(parse_client_id("client_id:\"corto\""), None);
    }

    #[test]
    fn parse_script_urls_reversed() {
        let html = r#"<script src="https://a-v2.sndcdn.com/assets/a-1.js"></script><script crossorigin src="https://a-v2.sndcdn.com/assets/b-2.js">"#;
        assert_eq!(parse_script_urls(html), ["https://a-v2.sndcdn.com/assets/b-2.js", "https://a-v2.sndcdn.com/assets/a-1.js"]);
    }

    #[test]
    fn retries_once_with_fresh_client_id() {
        let (gets, refreshes) = (Cell::new(0), Cell::new(0));
        let refresh = || {
            refreshes.set(refreshes.get() + 1);
            Ok("nuevo".to_string())
        };
        let ok = with_client_id(
            Some("viejo".into()),
            |c| {
                gets.set(gets.get() + 1);
                if c == "viejo" { Err(ScError::Unauthorized) } else { Ok(7) }
            },
            refresh,
        );
        assert_eq!(ok.ok(), Some(7));
        assert_eq!((gets.get(), refreshes.get()), (2, 1));

        let (gets, refreshes) = (Cell::new(0), Cell::new(0));
        let err = with_client_id(
            Some("viejo".into()),
            |_| -> Result<u8, ScError> {
                gets.set(gets.get() + 1);
                Err(ScError::Unauthorized)
            },
            || {
                refreshes.set(refreshes.get() + 1);
                Ok("nuevo".to_string())
            },
        );
        assert!(err.is_err());
        assert_eq!((gets.get(), refreshes.get()), (2, 1));
    }

    // Red real: la corre el dueño a mano (`cargo test ... -- --ignored`).
    #[test]
    #[ignore]
    fn live_soundcloud_search_and_playlist() {
        let page = search("lofi", None).unwrap();
        let t = page.tracks.iter().find(|t| t.is_playable == Some(true)).expect("tema reproducible");
        let hls = playlist(t.id.as_deref().expect("id")).unwrap();
        assert!(hls.init.split('?').next().unwrap().ends_with("init.mp4"), "{}", hls.init);
    }
}
