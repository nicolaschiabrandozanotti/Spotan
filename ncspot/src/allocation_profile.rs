//! Opt-in, test-only current-thread requested heap accounting, never shipped.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Counts {
    pub allocations: u64,
    pub requested_bytes: u64,
    pub retained_bytes: i64,
}
thread_local! {static COUNTS:Cell<Option<Counts>>=const {Cell::new(None)};}
fn record(allocated: usize, freed: usize, call: bool) {
    let _ = COUNTS.try_with(|cell| {
        if let Some(mut c) = cell.get() {
            c.allocations += u64::from(call);
            c.requested_bytes += allocated as u64;
            c.retained_bytes += allocated as i64 - freed as i64;
            cell.set(Some(c));
        }
    });
}
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            record(l.size(), 0, true)
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            record(l.size(), 0, true)
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        record(0, l.size(), false);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            record(n, l.size(), true)
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
pub(crate) fn measure<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTS.with(|c| c.set(None));
        }
    }
    COUNTS.with(|c| assert!(c.replace(Some(Counts::default())).is_none()));
    let reset = Reset;
    let value = f();
    let counts = COUNTS.with(|c| c.get().unwrap());
    drop(reset);
    (value, counts)
}

#[test]
#[ignore = "opt-in read-only audit of cached music metadata"]
fn audit_metadata() {
    use crate::{
        config::UserState,
        model::{
            album::Album, artist::Artist, playable::Playable, playlist::Playlist, track::Track,
        },
    };
    use std::{collections::HashSet, mem::size_of, path::PathBuf};
    let directory = PathBuf::from(
        std::env::var("NCSPOT_METADATA_PROFILE_DIR").expect("metadata-only snapshot directory"),
    );
    // File buffers and reporting allocations are outside the tracked scope.
    let tracks_bytes = std::fs::read(directory.join("tracks.db")).unwrap();
    let artists_bytes = std::fs::read(directory.join("artists.db")).unwrap();
    let albums_bytes = std::fs::read(directory.join("albums.db")).unwrap();
    let playlists_bytes = std::fs::read(directory.join("playlists.db")).unwrap();
    let state_bytes = std::fs::read(directory.join("userstate.cbor")).unwrap();
    let (tracks, t) = measure(|| serde_json::from_slice::<Vec<Track>>(&tracks_bytes).unwrap());
    let (artists, a) = measure(|| serde_json::from_slice::<Vec<Artist>>(&artists_bytes).unwrap());
    let (albums, al) = measure(|| serde_json::from_slice::<Vec<Album>>(&albums_bytes).unwrap());
    let (playlists, p) =
        measure(|| serde_json::from_slice::<Vec<Playlist>>(&playlists_bytes).unwrap());
    let (state, s) = measure(|| serde_cbor::from_slice::<UserState>(&state_bytes).unwrap());
    // Queue::new clones this while the Config still owns the persisted copy.
    let (queue, q) = measure(|| state.queuestate.clone());
    let mut all: Vec<&Track> = tracks.iter().collect();
    for artist in &artists {
        if let Some(ts) = &artist.tracks {
            all.extend(ts);
        }
    }
    for album in &albums {
        if let Some(ts) = &album.tracks {
            all.extend(ts);
        }
    }
    let playlist_items: usize = playlists
        .iter()
        .map(|p| p.tracks.as_ref().map_or(0, Vec::len))
        .sum();
    for items in playlists
        .iter()
        .filter_map(|p| p.tracks.as_ref())
        .chain([&state.queuestate.queue, &queue.queue])
    {
        for item in items {
            if let Playable::Track(track) = item {
                all.push(track);
            }
        }
    }
    let mut ids = HashSet::new();
    let mut fingerprints = HashSet::new();
    let mut repeated = 0;
    let mut repeated_deep_heap = 0i64;
    for track in &all {
        if let Some(id) = &track.id {
            ids.insert(id.as_str());
        }
        let mut v = serde_json::to_value(track).unwrap();
        let object = v.as_object_mut().unwrap();
        object.remove("list_index");
        object.remove("added_at");
        let fingerprint = serde_json::to_string(&v).unwrap();
        if !fingerprints.insert(fingerprint) {
            repeated += 1;
            let (copy, c) = measure(|| (*track).clone());
            repeated_deep_heap += c.retained_bytes;
            drop(copy);
        }
    }
    let report = serde_json::json!({
        "scope":"typed deserialization of metadata-only cache snapshot plus Queue::new clone; requested heap, not live RSS",
        "tracks":{"count":tracks.len(),"heap":t},
        "artists":{"count":artists.len(),"nested_tracks":artists.iter().map(|a|a.tracks.as_ref().map_or(0,Vec::len)).sum::<usize>(),"heap":a},
        "albums":{"count":albums.len(),"heap":al},
        "playlists":{"count":playlists.len(),"nested_items":playlist_items,"heap":p},
        "config_state":{"queue_count":state.queuestate.queue.len(),"heap":s},
        "runtime_queue_clone":{"queue_count":queue.queue.len(),"heap":q},
        "total_requested_retained_bytes":t.retained_bytes+a.retained_bytes+al.retained_bytes+p.retained_bytes+s.retained_bytes+q.retained_bytes,
        "track_occurrences":all.len(),"unique_track_ids":ids.len(),
        "unique_metadata_ignoring_list_index_and_added_at":fingerprints.len(),
        "repeated_metadata_occurrences":repeated,"track_inline_size":size_of::<Track>(),
        "repeated_deep_heap_bytes":repeated_deep_heap,
        "repeated_inline_plus_deep_upper_bound_bytes":repeated_deep_heap+repeated as i64*size_of::<Track>() as i64,
        "upper_bound_limit":"sharing still needs per-list context, handles, lookup/cache synchronization; this is not realizable RSS savings"
    });
    let out = std::env::var("NCSPOT_METADATA_PROFILE_OUT").expect("NCSPOT_METADATA_PROFILE_OUT");
    std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
