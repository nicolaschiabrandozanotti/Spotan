//! Biblioteca (me gusta, playlists, ítems) cargada por el spclient interno de librespot,
//! porque la Web API responde 429 a todo.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use librespot_core::{SpotifyId, SpotifyUri};
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata;
use librespot_protocol::playlist4_external::{MetaItem, SelectedListContent};
use log::warn;
use protobuf::{EnumOrUnknown, Message};

use crate::application::ASYNC_RUNTIME;
use crate::model::episode::Episode;
use crate::model::playable::Playable;
use crate::model::playlist::Playlist;
use crate::model::track::Track;
use crate::spotify::Spotify;

const METADATA_BATCH: usize = 100;
const CONCURRENT_FALLBACK: usize = 8;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b62(gid: &[u8]) -> Option<String> {
    SpotifyId::from_raw(gid).ok()?.to_base62().ok()
}

pub fn timestamp_ms(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}

fn cover(group: &metadata::ImageGroup) -> Option<String> {
    let img = group.image.iter().max_by_key(|i| i.size() as i32)?;
    Some(format!("https://i.scdn.co/image/{}", hex(img.file_id())))
}

fn playlist_from_meta(uri: &str, meta: &MetaItem) -> Option<Playlist> {
    let id = uri.strip_prefix("spotify:playlist:")?.to_string();
    Some(Playlist {
        id,
        name: meta.attributes.name().to_string(),
        owner_id: meta.owner_username().to_string(),
        owner_name: Some(meta.owner_username().to_string()),
        snapshot_id: hex(meta.revision()),
        num_tracks: meta.length().max(0) as usize,
        tracks: None,
        collaborative: meta.attributes.collaborative(),
    })
}

/// Una playlist por item `spotify:playlist:` (carpetas fuera). Si falta el `MetaItem` del
/// índice se conserva la playlist con nombre/dueño vacíos en vez de perderla.
fn playlists_from_content(content: &SelectedListContent) -> Vec<Playlist> {
    let items = &content.contents.items;
    let metas = &content.contents.meta_items;
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| playlist_from_meta(item.uri(), metas.get(i).unwrap_or(&MetaItem::new())))
        .collect()
}

fn track_from_metadata(m: &metadata::Track, added_at: Option<DateTime<Utc>>) -> Track {
    let id = b62(m.gid());
    Track {
        uri: format!("spotify:track:{}", id.as_deref().unwrap_or_default()),
        url: format!("https://open.spotify.com/track/{}", id.as_deref().unwrap_or_default()),
        id,
        title: m.name().to_string(),
        track_number: m.number().max(0) as u32,
        disc_number: m.disc_number(),
        duration: m.duration().max(0) as u32,
        artists: m.artist.iter().map(|a| a.name().to_string()).collect(),
        artist_ids: m.artist.iter().filter_map(|a| b62(a.gid())).collect(),
        album: m.album.as_ref().map(|a| a.name().to_string()),
        album_id: m.album.as_ref().and_then(|a| b62(a.gid())),
        album_artists: m.album.artist.iter().map(|a| a.name().to_string()).collect(),
        cover_url: m.album.cover_group.as_ref().and_then(cover),
        added_at,
        list_index: 0,
        is_local: false,
        is_playable: Some(true),
    }
}

fn episode_from_metadata(m: &metadata::Episode, added_at: Option<DateTime<Utc>>) -> Episode {
    let id = b62(m.gid()).unwrap_or_default();
    let d = &m.publish_time;
    Episode {
        uri: format!("spotify:episode:{id}"),
        id,
        duration: m.duration().max(0) as u32,
        name: m.name().to_string(),
        description: m.description().to_string(),
        release_date: format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()),
        cover_url: m.cover_image.as_ref().and_then(cover),
        added_at,
        list_index: 0,
    }
}

/// Playables en el orden de `uris`; sin metadata o `spotify:local:` se omiten.
fn order_playables(
    uris: &[(String, Option<DateTime<Utc>>)],
    tracks: &HashMap<String, metadata::Track>,
    episodes: &HashMap<String, metadata::Episode>,
) -> Vec<Playable> {
    let mut out: Vec<Playable> = uris
        .iter()
        .filter_map(|(uri, added)| {
            if let Some(m) = tracks.get(uri) {
                Some(Playable::Track(track_from_metadata(m, *added)))
            } else {
                episodes.get(uri).map(|m| Playable::Episode(episode_from_metadata(m, *added)))
            }
        })
        .collect();
    for (i, p) in out.iter_mut().enumerate() {
        match p {
            Playable::Track(t) => t.list_index = i,
            Playable::Episode(e) => e.list_index = i,
        }
    }
    out
}

fn batches(uris: &[String]) -> Vec<Vec<String>> {
    uris.chunks(METADATA_BATCH).map(<[String]>::to_vec).collect()
}

/// Metadata de `uris` (todas del mismo tipo `kind`) por `get_extended_metadata` en lotes.
/// Si un lote falla se piden de a uno, 8 en paralelo; `Err` si alguno de esos pedidos falla
/// (así el llamador no pisa su caché con datos parciales). Devuelve uri -> mensaje.
async fn fetch_metadata<M: Message>(
    spclient: &librespot_core::spclient::SpClient,
    kind: ExtensionKind,
    prefix: &str,
    gid_of: fn(&M) -> &[u8],
    uris: &[String],
) -> Result<HashMap<String, M>, ()> {
    let mut out = HashMap::new();
    let mut add = |value: &[u8]| {
        if let Ok(m) = M::parse_from_bytes(value) {
            if let Some(id) = b62(gid_of(&m)) {
                out.insert(format!("{prefix}{id}"), m);
            }
        }
    };
    for batch in batches(uris) {
        let req = BatchedEntityRequest {
            entity_request: batch
                .iter()
                .map(|uri| EntityRequest {
                    entity_uri: uri.clone(),
                    query: vec![ExtensionQuery { extension_kind: EnumOrUnknown::new(kind), ..Default::default() }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        match spclient.get_extended_metadata(req).await {
            Ok(res) => res
                .extended_metadata
                .iter()
                .flat_map(|arr| arr.extension_data.iter())
                .filter_map(|d| d.extension_data.as_ref())
                .for_each(|any| add(&any.value)),
            Err(e) => {
                warn!("metadata en lote falló ({e}); pidiendo de a uno");
                let singles: Vec<_> = futures::stream::iter(batch.iter().filter_map(|u| SpotifyUri::from_uri(u).ok()))
                    .map(|uri| async move { spclient.get_metadata(kind, &uri).await })
                    .buffered(CONCURRENT_FALLBACK)
                    .collect()
                    .await;
                all_ok(&singles)?;
                singles.iter().flatten().for_each(|b| add(b));
            }
        }
    }
    Ok(out)
}

/// `Err` si algún pedido individual falló (por red/429), no si sólo faltó metadata.
fn all_ok<T, E>(results: &[Result<T, E>]) -> Result<(), ()> {
    results.iter().all(Result::is_ok).then_some(()).ok_or(())
}

async fn fetch_tracks(sp: &librespot_core::spclient::SpClient, uris: &[String]) -> Result<HashMap<String, metadata::Track>, ()> {
    fetch_metadata(sp, ExtensionKind::TRACK_V4, "spotify:track:", |m: &metadata::Track| m.gid(), uris).await
}

async fn fetch_episodes(sp: &librespot_core::spclient::SpClient, uris: &[String]) -> Result<HashMap<String, metadata::Episode>, ()> {
    fetch_metadata(sp, ExtensionKind::EPISODE_V4, "spotify:episode:", |m: &metadata::Episode| m.gid(), uris).await
}

/// Sólo desde hilos std, nunca desde dentro del runtime tokio (`block_on` entraría en pánico).
fn runtime() -> &'static tokio::runtime::Runtime {
    ASYNC_RUNTIME.get().unwrap()
}

/// Me gusta, en el orden de Spotify. Reutiliza los `Track` de `cached` por id. Bloqueante: sólo desde hilos std.
pub fn liked_tracks(spotify: &Spotify, cached: &[Track]) -> Result<Vec<Track>, ()> {
    let session = spotify.session().ok_or(())?;
    runtime().block_on(async {
        let sp = session.spclient();
        let ctx = sp
            .get_context(&format!("spotify:user:{}:collection", session.username()))
            .await
            .map_err(|e| warn!("me gusta: {e}"))?;
        let mut uris: Vec<String> = vec![];
        let mut visited = std::collections::HashSet::new();
        for page in &ctx.pages {
            uris.extend(page.tracks.iter().map(|t| t.uri().to_string()));
            // Página sin pistas (sólo `page_url`) o con `next_page_url`: se siguen como JSON.
            let mut next = match (page.tracks.is_empty(), page.next_page_url(), page.page_url()) {
                (_, n, _) if !n.is_empty() => n.to_string(),
                (true, _, p) => p.to_string(),
                _ => String::new(),
            };
            while !next.is_empty() {
                if !visited.insert(next.clone()) {
                    warn!("me gusta: página repetida {next}");
                    return Err(());
                }
                let body = sp.get_next_page(&next).await.map_err(|e| warn!("me gusta (página): {e}"))?;
                let json: serde_json::Value = serde_json::from_slice(&body).map_err(|_| ())?;
                let found = json["tracks"].as_array().into_iter().flatten().filter_map(|t| t["uri"].as_str());
                uris.extend(found.map(String::from));
                next = json["next_page_url"].as_str().unwrap_or_default().to_string();
            }
        }
        let mut seen = std::collections::HashSet::new();
        uris.retain(|u| seen.insert(u.clone()));
        let known: HashMap<&str, &Track> = cached.iter().filter_map(|t| Some((t.id.as_deref()?, t))).collect();
        let missing: Vec<String> = uris
            .iter()
            .filter(|u| u.starts_with("spotify:track:") && !known.contains_key(&u["spotify:track:".len()..]))
            .cloned()
            .collect();
        let fetched = fetch_tracks(&sp, &missing).await?;
        Ok(uris
            .iter()
            .filter_map(|u| {
                let id = u.strip_prefix("spotify:track:")?;
                known.get(id).map(|t| (*t).clone()).or_else(|| fetched.get(u).map(|m| track_from_metadata(m, None)))
            })
            .enumerate()
            .map(|(i, mut t)| {
                t.list_index = i;
                t
            })
            .collect())
    })
}

/// Playlists del usuario (sin sus pistas) desde la rootlist. Bloqueante: sólo desde hilos std.
pub fn playlists(spotify: &Spotify) -> Result<Vec<Playlist>, ()> {
    let session = spotify.session().ok_or(())?;
    runtime().block_on(async {
        let bytes = session.spclient().get_rootlist(0, Some(1000)).await.map_err(|e| warn!("rootlist: {e}"))?;
        let content = SelectedListContent::parse_from_bytes(&bytes).map_err(|e| warn!("rootlist: {e}"))?;
        Ok(playlists_from_content(&content))
    })
}

/// Ítems de la playlist `id` (base62) en orden, con metadata en lote. Bloqueante: sólo desde hilos std.
pub fn playlist_items(spotify: &Spotify, id: &str) -> Result<Vec<Playable>, ()> {
    let session = spotify.session().ok_or(())?;
    let id = SpotifyId::from_base62(id).map_err(|_| ())?;
    runtime().block_on(async {
        let sp = session.spclient();
        let bytes = sp.get_playlist(&id).await.map_err(|e| warn!("playlist: {e}"))?;
        let content = SelectedListContent::parse_from_bytes(&bytes).map_err(|e| warn!("playlist: {e}"))?;
        let uris: Vec<(String, Option<DateTime<Utc>>)> = content
            .contents
            .items
            .iter()
            .map(|i| (i.uri().to_string(), i.attributes.as_ref().and_then(|a| timestamp_ms(a.timestamp()))))
            .collect();
        let of = |p: &str| uris.iter().filter(|(u, _)| u.starts_with(p)).map(|(u, _)| u.clone()).collect::<Vec<_>>();
        let (track_uris, episode_uris) = (of("spotify:track:"), of("spotify:episode:"));
        let (tracks, episodes) = futures::join!(fetch_tracks(&sp, &track_uris), fetch_episodes(&sp, &episode_uris));
        let (tracks, episodes) = (tracks?, episodes?);
        Ok(order_playables(&uris, &tracks, &episodes))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::metadata;
    use librespot_protocol::playlist4_external::{Item, ListAttributes, MetaItem, SelectedListContent};
    use protobuf::{EnumOrUnknown, MessageField};

    fn gid(n: u8) -> Vec<u8> {
        let mut g = vec![0u8; 16];
        g[15] = n;
        g
    }

    fn artist(name: &str, n: u8) -> metadata::Artist {
        metadata::Artist { name: Some(name.into()), gid: Some(gid(n)), ..Default::default() }
    }

    fn image(file: u8, size: metadata::image::Size) -> metadata::Image {
        metadata::Image { file_id: Some(vec![file; 20]), size: Some(EnumOrUnknown::new(size)), ..Default::default() }
    }

    fn track(n: u8) -> metadata::Track {
        metadata::Track { gid: Some(gid(n)), name: Some(format!("T{n}")), ..Default::default() }
    }

    #[test]
    fn track_from_metadata_maps_fields() {
        let album = metadata::Album {
            gid: Some(gid(9)),
            name: Some("Alb".into()),
            artist: vec![artist("AA", 7)],
            cover_group: MessageField::some(metadata::ImageGroup {
                image: vec![image(0x11, metadata::image::Size::SMALL), image(0xab, metadata::image::Size::LARGE)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let m = metadata::Track {
            gid: Some(gid(1)),
            name: Some("Canción".into()),
            number: Some(3),
            disc_number: Some(1),
            duration: Some(231533),
            artist: vec![artist("A1", 5), artist("A2", 6)],
            album: MessageField::some(album),
            ..Default::default()
        };
        let t = track_from_metadata(&m, None);
        let id = SpotifyId::from_raw(&gid(1)).unwrap().to_base62().unwrap();
        assert_eq!(t.id.as_deref(), Some(id.as_str()));
        assert_eq!(t.uri, format!("spotify:track:{id}"));
        assert_eq!(t.url, format!("https://open.spotify.com/track/{id}"));
        assert_eq!(t.title, "Canción");
        assert_eq!(t.track_number, 3);
        assert_eq!(t.duration, 231533);
        assert_eq!(t.artists, ["A1", "A2"]);
        assert_eq!(t.artist_ids.len(), 2);
        assert_eq!(t.album.as_deref(), Some("Alb"));
        assert_eq!(t.album_artists, ["AA"]);
        assert_eq!(t.cover_url, Some(format!("https://i.scdn.co/image/{}", "ab".repeat(20))));
        assert!(!t.is_local);
        assert_eq!(t.is_playable, Some(true));
    }

    #[test]
    fn rootlist_skips_folders() {
        let uris = ["spotify:start-group:x:Carpeta", "spotify:playlist:37i9dQZF1EYkqdzj48dyYq", "spotify:end-group:x"];
        let mut c = SelectedListContent::new();
        for u in uris {
            c.contents.mut_or_insert_default().items.push(Item { uri: Some(u.into()), ..Default::default() });
            c.contents.mut_or_insert_default().meta_items.push(MetaItem {
                revision: Some(vec![0x00, 0xab, 0xff]),
                length: Some(42),
                owner_username: Some("dueño".into()),
                attributes: MessageField::some(ListAttributes {
                    name: Some("Mi lista".into()),
                    collaborative: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        let pls = playlists_from_content(&c);
        assert_eq!(pls.len(), 1);
        let p = &pls[0];
        assert_eq!(p.id, "37i9dQZF1EYkqdzj48dyYq");
        assert_eq!(p.name, "Mi lista");
        assert_eq!(p.num_tracks, 42);
        assert_eq!(p.snapshot_id, "00abff");
        assert_eq!(p.owner_id, "dueño");
        assert!(p.collaborative);
        assert!(p.tracks.is_none());
    }

    #[test]
    fn rootlist_without_meta_keeps_playlist() {
        let mut c = SelectedListContent::new();
        c.contents.mut_or_insert_default().items.push(Item {
            uri: Some("spotify:playlist:37i9dQZF1EYkqdzj48dyYq".into()),
            ..Default::default()
        });
        assert_eq!(playlists_from_content(&c).len(), 1);
    }

    fn uris_of(n: &[u8]) -> (HashMap<String, metadata::Track>, Vec<(String, Option<DateTime<Utc>>)>) {
        let mut tracks = HashMap::new();
        let mut uris = vec![];
        for &i in n {
            let t = track(i);
            let uri = format!("spotify:track:{}", SpotifyId::from_raw(&gid(i)).unwrap().to_base62().unwrap());
            tracks.insert(uri.clone(), t);
            uris.push((uri, None));
        }
        (tracks, uris)
    }

    #[test]
    fn playlist_items_skip_missing_metadata() {
        let (tracks, mut uris) = uris_of(&[1, 2]);
        uris.insert(1, ("spotify:track:0000000000000000000000".into(), None));
        uris.push(("spotify:local:a:b:c:12".into(), None));
        let out = order_playables(&uris, &tracks, &HashMap::new());
        assert_eq!(out.len(), 2);
        let titles: Vec<_> = out.iter().map(|p| match p { Playable::Track(t) => t.title.clone(), _ => String::new() }).collect();
        assert_eq!(titles, ["T1", "T2"]);
        let idx: Vec<_> = out.iter().map(|p| p.list_index()).collect();
        assert_eq!(idx, [0, 1]);
    }

    #[test]
    fn playlist_items_keep_episodes() {
        let ep = metadata::Episode { gid: Some(gid(3)), name: Some("Ep".into()), duration: Some(1000), ..Default::default() };
        let uri = format!("spotify:episode:{}", SpotifyId::from_raw(&gid(3)).unwrap().to_base62().unwrap());
        let eps = HashMap::from([(uri.clone(), ep)]);
        let out = order_playables(&[(uri, None)], &HashMap::new(), &eps);
        assert!(matches!(out[0], Playable::Episode(_)));
    }

    #[test]
    fn chunks_keep_order() {
        let uris: Vec<String> = (0..250).map(|i| i.to_string()).collect();
        let b = batches(&uris);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), [100, 100, 50]);
        assert_eq!(b.concat(), uris);
    }

    #[test]
    fn all_ok_fails_on_any_request_error() {
        assert!(all_ok::<u8, ()>(&[Ok(1), Ok(2)]).is_ok());
        assert!(all_ok(&[Ok(1), Err(())]).is_err());
    }

    #[test]
    fn timestamp_ms_converts() {
        assert_eq!(timestamp_ms(1_700_000_000_000).unwrap().timestamp(), 1_700_000_000);
    }
}
