//! Biblioteca de YouTube por links públicos (sin sesión): links, playlists, álbumes y canales.
//! Parseo de las respuestas de YouTube Music (`WEB_REMIX`) y, para canales comunes, de `WEB`.

use regex::Regex;
use serde_json::Value;
use url::Url;

use crate::model::album::Album;
use crate::model::playlist::Playlist;
use crate::model::track::{Source, Track};

/// Un link de YouTube que se puede agregar a la biblioteca.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YtLink {
    /// Id de playlist (`PL…`, `OLAK5uy_…`, …).
    Playlist(String),
    /// Id de canal (`UC…`).
    Channel(String),
    /// Handle sin `@`.
    Handle(String),
}

fn valid(s: &str, extra: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || extra.contains(c))
}

pub fn parse_link(input: &str) -> Option<YtLink> {
    let s = input.trim();
    let url = Url::parse(&if s.contains("://") { s.to_string() } else { format!("https://{s}") }).ok()?;
    let host = url.host_str()?;
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).or_else(|| host.strip_prefix("music.")).unwrap_or(host);
    if host != "youtube.com" {
        return None;
    }
    let mut seg = url.path_segments()?;
    match (seg.next()?, seg.next()) {
        ("playlist" | "watch", _) => {
            let list = url.query_pairs().find(|(k, _)| k == "list")?.1.into_owned();
            valid(&list, "").then_some(YtLink::Playlist(list))
        }
        ("channel", Some(id)) => (id.starts_with("UC") && valid(id, "")).then(|| YtLink::Channel(id.to_string())),
        (h, _) => {
            let h = h.strip_prefix('@')?;
            valid(h, ".").then(|| YtLink::Handle(h.to_string()))
        }
    }
}

/// Forma canónica que se guarda en la biblioteca.
pub fn link_url(l: &YtLink) -> String {
    match l {
        YtLink::Playlist(id) => format!("https://www.youtube.com/playlist?list={id}"),
        YtLink::Channel(id) => format!("https://www.youtube.com/channel/{id}"),
        YtLink::Handle(h) => format!("https://www.youtube.com/@{h}"),
    }
}

/// Todos los valores con la clave `key`, en orden, sin entrar en los ya encontrados.
/// No entra en `header`: ahí hay tokens de continuación de otros paneles (descripción).
fn collect<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    fn go<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == key {
                        out.push(x);
                    } else if k != "header" {
                        go(x, key, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| go(x, key, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(v, key, &mut out);
    out
}

fn text_of(v: &Value) -> String {
    v["runs"].as_array().into_iter().flatten().filter_map(|r| r["text"].as_str()).collect()
}

fn runs_texts(v: &Value) -> impl Iterator<Item = &str> {
    v["runs"].as_array().into_iter().flatten().filter_map(|r| r["text"].as_str())
}

/// URL de la última miniatura (la más grande).
fn cover(v: &Value) -> Option<String> {
    let thumbs = collect(v, "thumbnails").into_iter().next()?;
    thumbs.as_array()?.last()?["url"].as_str().map(String::from)
}

/// Último token de 4 dígitos del subtítulo ("Álbum • 2024").
fn year_of(subtitle: &Value) -> String {
    let is_year = |t: &&str| t.len() == 4 && t.chars().all(|c| c.is_ascii_digit());
    runs_texts(subtitle).filter(is_year).last().unwrap_or_default().to_string()
}

/// Token de la próxima página (primer `continuationItemRenderer` fuera de `header`).
fn next_token(json: &Value) -> Option<String> {
    let item = collect(json, "continuationItemRenderer").into_iter().next()?;
    item["continuationEndpoint"]["continuationCommand"]["token"].as_str().map(String::from)
}

/// Cantidad de "198 pistas", "6 vídeos", "13 episodios"…
fn count_in(text: &str) -> Option<usize> {
    let re = Regex::new(r"^\s*([\d.,\u{a0}]+)\s*(pista|canci|v[ií]deo|episodio)").expect("regex");
    let digits: String = re.captures(text)?[1].chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

pub fn parse_track(item: &Value) -> Option<Track> {
    let id = item["playlistItemData"]["videoId"].as_str()?.to_string();
    let col = |i: usize| &item["flexColumns"][i]["musicResponsiveListItemFlexColumnRenderer"]["text"];
    let duration = item["fixedColumns"][0]["musicResponsiveListItemFixedColumnRenderer"]["text"]["runs"][0]["text"]
        .as_str()
        .and_then(super::parse_duration)?;
    let url = format!("https://music.youtube.com/watch?v={id}");
    Some(Track::external(Source::YouTube, id, text_of(col(0)), text_of(col(1)), duration, cover(&item["thumbnail"]), url))
}

/// Primera página o continuación de una playlist/álbum; el `String` es el token de la próxima.
pub fn parse_track_page(json: &Value) -> (Vec<Track>, Option<String>) {
    let tracks = collect(json, "musicResponsiveListItemRenderer").into_iter().filter_map(parse_track).collect();
    (tracks, next_token(json))
}

fn header(json: &Value) -> Option<&Value> {
    collect(json, "musicResponsiveHeaderRenderer").into_iter().next()
}

/// Encabezado de la respuesta `VL<id>`; la cantidad sale de "N pistas".
pub fn parse_playlist_header(json: &Value, id: &str) -> Option<Playlist> {
    let h = header(json)?;
    Some(Playlist {
        id: id.to_string(),
        name: text_of(&h["title"]),
        owner_id: String::new(),
        owner_name: Some(text_of(&h["straplineTextOne"])).filter(|s| !s.is_empty()),
        snapshot_id: String::new(),
        num_tracks: runs_texts(&h["secondSubtitle"]).find_map(count_in).unwrap_or(0),
        tracks: None,
        collaborative: false,
        source: Source::YouTube,
    })
}

fn album_url(id: &str) -> Option<String> {
    Some(format!("https://music.youtube.com/browse/{id}"))
}

/// Encabezado de un álbum `MPREb_…`.
pub fn parse_album_header(json: &Value, browse_id: &str) -> Option<Album> {
    let h = header(json)?;
    let artist = Some(text_of(&h["straplineTextOne"])).filter(|s| !s.is_empty());
    Some(Album::new_external(
        Source::YouTube,
        browse_id.to_string(),
        text_of(&h["title"]),
        artist,
        year_of(&h["subtitle"]),
        cover(&h["thumbnail"]),
        album_url(browse_id),
        0,
    ))
}

fn two_row_items(json: &Value) -> Vec<&Value> {
    collect(json, "musicTwoRowItemRenderer")
}

fn browse_id(item: &Value) -> Option<&str> {
    item["title"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()
}

fn album_from_item(item: &Value, artist: &str) -> Option<Album> {
    let id = browse_id(item).filter(|i| i.starts_with("MPRE"))?;
    Some(Album::new_external(
        Source::YouTube,
        id.to_string(),
        text_of(&item["title"]),
        Some(artist.to_string()),
        year_of(&item["subtitle"]),
        cover(&item["thumbnailRenderer"]),
        album_url(id),
        0,
    ))
}

fn playlist_from_item(item: &Value, owner: &str) -> Option<Playlist> {
    let id = browse_id(item)?.strip_prefix("VL")?;
    Some(Playlist {
        id: id.to_string(),
        name: text_of(&item["title"]),
        owner_id: String::new(),
        owner_name: Some(owner.to_string()),
        snapshot_id: String::new(),
        num_tracks: 0,
        tracks: None,
        collaborative: false,
        source: Source::YouTube,
    })
}

pub fn parse_album_grid(json: &Value, artist: &str) -> Vec<Album> {
    two_row_items(json).into_iter().filter_map(|i| album_from_item(i, artist)).collect()
}

pub fn parse_playlist_grid(json: &Value, owner: &str) -> Vec<Playlist> {
    two_row_items(json).into_iter().filter_map(|i| playlist_from_item(i, owner)).collect()
}

/// Página de un artista de YouTube Music: carruseles de álbumes, singles y listas con su "ver todo".
pub struct ArtistPage {
    pub name: String,
    pub albums: Vec<Album>,
    pub albums_more: Option<(String, String)>,
    pub singles: Vec<Album>,
    pub singles_more: Option<(String, String)>,
    pub playlists: Vec<Playlist>,
    pub playlists_more: Option<(String, String)>,
}

/// `None` si la página no tiene carruseles (canal común, no musical).
pub fn parse_artist(json: &Value) -> Option<ArtistPage> {
    let carousels = collect(json, "musicCarouselShelfRenderer");
    if carousels.is_empty() {
        return None;
    }
    let name = json["header"].as_object().and_then(|m| m.values().next()).map(|h| text_of(&h["title"])).unwrap_or_default();
    let mut page = ArtistPage {
        name,
        albums: vec![],
        albums_more: None,
        singles: vec![],
        singles_more: None,
        playlists: vec![],
        playlists_more: None,
    };
    for c in carousels {
        let title = &c["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"]["runs"][0];
        let more = || {
            let e = &title["navigationEndpoint"]["browseEndpoint"];
            Some((e["browseId"].as_str()?.to_string(), e["params"].as_str()?.to_string()))
        };
        let name = title["text"].as_str().unwrap_or_default();
        let items = c["contents"].as_array().into_iter().flatten().filter_map(|i| i.get("musicTwoRowItemRenderer"));
        if name.starts_with("Álbumes") {
            page.albums = items.filter_map(|i| album_from_item(i, &page.name)).collect();
            page.albums_more = more();
        } else if name.starts_with("Singles") {
            page.singles = items.filter_map(|i| album_from_item(i, &page.name)).collect();
            page.singles_more = more();
        } else if name.starts_with("Listas de reproducción") {
            page.playlists = items.filter_map(|i| playlist_from_item(i, &page.name)).collect();
            page.playlists_more = more();
        }
    }
    Some(page)
}

/// Playlists de un canal común (pestaña Playlists de la web); incluye las series (`SHOW`),
/// que también son playlists. `num_tracks` sale de "N vídeos" / "N episodios".
pub fn parse_web_playlists(json: &Value, owner: &str) -> (Vec<Playlist>, Option<String>) {
    let lists = collect(json, "lockupViewModel")
        .into_iter()
        .filter(|l| matches!(l["contentType"].as_str(), Some("LOCKUP_CONTENT_TYPE_PLAYLIST" | "LOCKUP_CONTENT_TYPE_SHOW")))
        .filter_map(|l| {
            let count = collect(l, "thumbnailBadgeViewModel").into_iter().find_map(|b| count_in(b["text"].as_str()?));
            Some(Playlist {
                id: l["contentId"].as_str()?.to_string(),
                name: l["metadata"]["lockupMetadataViewModel"]["title"]["content"].as_str()?.to_string(),
                owner_id: String::new(),
                owner_name: Some(owner.to_string()),
                snapshot_id: String::new(),
                num_tracks: count.unwrap_or(0),
                tracks: None,
                collaborative: false,
                source: Source::YouTube,
            })
        })
        .collect();
    (lists, next_token(json))
}

/// `UC…` del canal que resuelve `navigation/resolve_url`.
pub fn parse_resolve(json: &Value) -> Option<String> {
    let id = json["endpoint"]["browseEndpoint"]["browseId"].as_str()?;
    id.starts_with("UC").then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(name: &str) -> Value {
        let text = match name {
            "yt_playlist" => include_str!("fixtures/yt_playlist.json"),
            "yt_playlist_cont" => include_str!("fixtures/yt_playlist_cont.json"),
            "yt_album" => include_str!("fixtures/yt_album.json"),
            "yt_artist" => include_str!("fixtures/yt_artist.json"),
            "yt_grid" => include_str!("fixtures/yt_grid.json"),
            "yt_web_playlists" => include_str!("fixtures/yt_web_playlists.json"),
            "yt_resolve" => include_str!("fixtures/yt_resolve.json"),
            _ => panic!("fixture desconocido: {name}"),
        };
        serde_json::from_str(text).expect("fixture JSON")
    }

    #[test]
    fn links_normalize() {
        let pl = Some(YtLink::Playlist("PLD4WCKZfeUfs".into()));
        for i in ["https://www.youtube.com/playlist?list=PLD4WCKZfeUfs", " music.youtube.com/playlist?list=PLD4WCKZfeUfs&si=x ",
                  "https://m.youtube.com/watch?v=abc&list=PLD4WCKZfeUfs"] { assert_eq!(parse_link(i), pl, "{i}"); }
        assert_eq!(parse_link("https://www.youtube.com/@LofiGirl"), Some(YtLink::Handle("LofiGirl".into())));
        assert_eq!(parse_link("https://music.youtube.com/channel/UCIaFw5VBEK8qaW6nRpx_qnw"), Some(YtLink::Channel("UCIaFw5VBEK8qaW6nRpx_qnw".into())));
        for i in ["", "https://open.spotify.com/playlist/x", "https://www.youtube.com/watch?v=abc", "hola"] { assert_eq!(parse_link(i), None, "{i}"); }
        assert_eq!(link_url(&pl.unwrap()), "https://www.youtube.com/playlist?list=PLD4WCKZfeUfs");
    }

    #[test]
    fn playlist_page_and_continuation() {
        let (t, next) = parse_track_page(&fx("yt_playlist"));
        assert_eq!((t[0].id.as_deref(), t[0].title.as_str(), t[0].duration), (Some("34G5nvk5SJc"), "Intersect", 178_000));
        assert_eq!(t[0].source, Source::YouTube);
        assert_eq!(t.len(), 3);
        assert!(next.is_some());
        assert!(parse_track_page(&fx("yt_playlist_cont")).1.is_none());
        let p = parse_playlist_header(&fx("yt_playlist"), "PLD4WCKZfeUfs").unwrap();
        assert_eq!((p.name.as_str(), p.num_tracks, p.source), ("lofi rain", 198, Source::YouTube));
    }

    #[test]
    fn album_header() {
        let a = parse_album_header(&fx("yt_album"), "MPREb_pnVvyDhXXd0").unwrap();
        assert_eq!((a.title.as_str(), a.year.as_str(), a.artists.clone()), ("Moon Music", "2024", vec!["Coldplay".to_string()]));
        assert_eq!(a.source, Source::YouTube);
    }

    #[test]
    fn artist_page_sections() {
        let p = parse_artist(&fx("yt_artist")).unwrap();
        assert_eq!(p.albums[0].title, "Moon Music");
        assert_eq!(p.albums_more, Some(("MPADUCIaFw5VBEK8qaW6nRpx_qnw".into(), "ggMIegYIARoCAQI%3D".into())));
        assert!(!p.singles.is_empty() && !p.playlists.is_empty());
        assert!(p.playlists.iter().all(|l| !l.id.starts_with("VL")));
        assert!(parse_artist(&serde_json::json!({"contents": {}})).is_none());
    }

    #[test]
    fn artist_extras() {
        let p = parse_artist(&fx("yt_artist")).unwrap();
        assert_eq!(p.name, "Coldplay");
        assert_eq!((p.singles[0].year.as_str(), p.albums.len(), p.playlists.len()), ("2025", 2, 2));
        assert!(p.singles_more.is_some() && p.playlists_more.is_some());
        // "Destacado en" y "Vídeos" no entran como listas
        assert!(p.playlists.iter().all(|l| l.name.contains("Coldplay")));
    }

    #[test]
    fn grids() {
        let a = parse_album_grid(&fx("yt_grid"), "Coldplay");
        assert_eq!((a.len(), a[0].title.as_str(), a[0].year.as_str()), (2, "Moon Music", "2024"));
        assert_eq!(a[0].id.as_deref(), Some("MPREb_pnVvyDhXXd0"));
    }

    #[test]
    fn web_playlists_of_common_channel() {
        let (p, next) = parse_web_playlists(&fx("yt_web_playlists"), "MKBHD");
        assert_eq!(p[0].id, "PLBsP89CPrMeNIuvr9j3_EQaMwaNJ6rhuQ");
        assert!(p[0].num_tracks > 0 && next.is_some());
        assert_eq!((p.len(), p[1].num_tracks), (2, 6));
    }

    #[test]
    fn resolve_handle() {
        assert_eq!(parse_resolve(&fx("yt_resolve")).as_deref(), Some("UCDPM_n1atn2ijUwHd0NNRQw"));
    }
}
