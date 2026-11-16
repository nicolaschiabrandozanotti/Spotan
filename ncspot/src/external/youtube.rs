//! Cliente de YouTube sin yt-dlp: búsqueda y enlace de audio (itag 140) vía youtubei.

use std::sync::Mutex;

use serde_json::{json, Value};

use super::{http, parse_duration, Page};
use crate::model::track::{Source, Track};

// Todo lo que YouTube puede romper vive acá (verificado el 2026-11-16).
pub(crate) const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const API: &str = "https://www.youtube.com/youtubei/v1";
const HOME: &str = "https://www.youtube.com/";
const CLIENT_NAME_ID: &str = "101";
const CLIENT_VERSION: &str = "1.02";
const AUDIO_ITAG: u64 = 140;

pub struct StreamInfo {
    pub url: String,
    pub content_length: u64,
    pub duration_ms: u32,
}

static VISITOR: Mutex<Option<String>> = Mutex::new(None);

/// Extrae `"VISITOR_DATA":"…"` del HTML de la portada.
pub fn parse_visitor_data(html: &str) -> Option<String> {
    let rest = html.split("\"VISITOR_DATA\":\"").nth(1)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string()).filter(|s| !s.is_empty())
}

fn text_of(v: &Value) -> Option<&str> {
    v["runs"][0]["text"].as_str().or_else(|| v["simpleText"].as_str())
}

fn walk(v: &Value, tracks: &mut Vec<Track>, next: &mut Option<String>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                match k.as_str() {
                    "compactVideoRenderer" => {
                        if let Some(t) = video(child) {
                            tracks.push(t);
                        }
                    }
                    "nextContinuationData" => {
                        if next.is_none() {
                            *next = child["continuation"].as_str().map(String::from);
                        }
                    }
                    _ => walk(child, tracks, next),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| walk(c, tracks, next)),
        _ => {}
    }
}

/// Sin `lengthText` (en vivo) se descarta.
fn video(r: &Value) -> Option<Track> {
    let id = r["videoId"].as_str()?;
    let duration = parse_duration(text_of(&r["lengthText"])?)?;
    Some(Track::external(
        Source::YouTube,
        id.to_string(),
        text_of(&r["title"])?.to_string(),
        text_of(&r["longBylineText"]).unwrap_or_default().to_string(),
        duration,
        r["thumbnail"]["thumbnails"][0]["url"].as_str().map(String::from),
        format!("https://www.youtube.com/watch?v={id}"),
    ))
}

pub fn parse_search(json: &Value) -> Page {
    let (mut tracks, mut next) = (Vec::new(), None);
    walk(json, &mut tracks, &mut next);
    Page { tracks, next }
}

pub fn parse_player(json: &Value) -> Result<StreamInfo, String> {
    let status = &json["playabilityStatus"];
    if status["status"].as_str() != Some("OK") {
        return Err(status["reason"].as_str().unwrap_or("YouTube no permite reproducir este video").to_string());
    }
    let f = json["streamingData"]["adaptiveFormats"]
        .as_array()
        .and_then(|a| a.iter().find(|f| f["itag"].as_u64() == Some(AUDIO_ITAG)))
        .ok_or("YouTube no ofrece audio para este video")?;
    let num = |v: &Value| v.as_str().and_then(|s| s.parse::<u64>().ok());
    Ok(StreamInfo {
        url: f["url"].as_str().ok_or("YouTube no dio enlace de audio")?.to_string(),
        content_length: num(&f["contentLength"]).ok_or("falta el tamaño del audio")?,
        duration_ms: num(&f["approxDurationMs"]).unwrap_or(0) as u32,
    })
}

fn visitor_data(refresh: bool) -> Result<String, String> {
    let mut cached = VISITOR.lock().unwrap();
    if !refresh {
        if let Some(v) = cached.as_ref() {
            return Ok(v.clone());
        }
    }
    let html = http().get(HOME).header("User-Agent", UA).send().and_then(|r| r.text()).map_err(|e| e.to_string())?;
    let vd = parse_visitor_data(&html).ok_or("YouTube no dio visitorData")?;
    *cached = Some(vd.clone());
    Ok(vd)
}

fn request_body(extra: Value, vd: &str) -> Value {
    let mut body = json!({"context": {"client": {
        "clientName": "VISIONOS", "clientVersion": CLIENT_VERSION,
        "deviceMake": "Apple", "deviceModel": "RealityDevice17,1",
        "osName": "visionOS", "osVersion": "26.5.23O471",
        "userAgent": UA, "hl": "es", "visitorData": vd,
    }}});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    body
}

fn call(endpoint: &str, extra: Value, vd: &str) -> Result<Value, String> {
    http()
        .post(format!("{API}/{endpoint}?prettyPrint=false"))
        .header("User-Agent", UA)
        .header("X-YouTube-Client-Name", CLIENT_NAME_ID)
        .header("X-YouTube-Client-Version", CLIENT_VERSION)
        .header("X-Goog-Visitor-Id", vd)
        .json(&request_body(extra, vd))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<Value>())
        .map_err(|e| e.to_string())
}

pub fn search(query: &str, continuation: Option<&str>) -> Result<Page, String> {
    let extra = match continuation {
        Some(c) => json!({"continuation": c}),
        None => json!({"query": query}),
    };
    Ok(parse_search(&call("search", extra, &visitor_data(false)?)?))
}

pub fn stream(video_id: &str) -> Result<StreamInfo, String> {
    let extra = json!({"videoId": video_id, "contentCheckOk": true, "racyCheckOk": true});
    let first = call("player", extra.clone(), &visitor_data(false)?)?;
    if first["playabilityStatus"]["status"].as_str() == Some("LOGIN_REQUIRED") {
        // visitorData vencido: se pide uno nuevo y se reintenta una sola vez.
        return parse_player(&call("player", extra, &visitor_data(true)?)?);
    }
    parse_player(&first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn parse_search_reads_compact_videos() {
        let page = parse_search(&fixture(include_str!("fixtures/yt_search.json")));
        assert_eq!(page.tracks.len(), 3);
        let t = &page.tracks[0];
        assert_eq!(t.id.as_deref(), Some("fJ9rUzIMcZQ"));
        assert_eq!(t.title, "Queen – Bohemian Rhapsody (Official Video Remastered)");
        assert_eq!(t.artists, vec!["Queen Official".to_string()]);
        assert_eq!(t.duration, 360000);
        assert_eq!(t.cover_url.as_deref(), Some("https://i.ytimg.com/vi/fJ9rUzIMcZQ/mqdefault.jpg"));
        assert!(page.tracks.iter().all(|t| t.id.as_deref() != Some("LIVEXXXXXXX")));
        assert_eq!(page.next.as_deref(), Some("CONT_TOKEN_1"));
    }

    #[test]
    fn parse_player_picks_itag_140() {
        let s = parse_player(&fixture(include_str!("fixtures/yt_player.json"))).unwrap();
        assert!(s.url.contains("itag=140"));
        assert_eq!(s.content_length, 5818745);
        assert_eq!(s.duration_ms, 359491);
    }

    #[test]
    fn parse_player_login_required_is_err() {
        let e = parse_player(&fixture(include_str!("fixtures/yt_player_login_required.json"))).err().unwrap();
        assert!(e.contains("bot"));
    }

    #[test]
    fn parse_visitor_data_from_html() {
        let html = r#"x{"INNERTUBE":1,"VISITOR_DATA":"Cgtabc%3D","Y":2}"#;
        assert_eq!(parse_visitor_data(html).as_deref(), Some("Cgtabc%3D"));
        assert_eq!(parse_visitor_data("nada"), None);
    }

    #[test]
    #[ignore = "usa la red"]
    fn live_youtube_search_and_stream() {
        let first = search("bohemian rhapsody", None).unwrap().tracks.remove(0);
        let s = stream(first.id.as_deref().unwrap()).unwrap();
        let r = http().get(&s.url).header("Range", "bytes=0-99999").send().unwrap();
        assert_eq!(r.status().as_u16(), 206);
    }
}
