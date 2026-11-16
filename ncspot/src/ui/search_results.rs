use crate::application::ASYNC_RUNTIME;
use crate::command::Command;
use crate::commands::CommandResult;
use crate::events::EventManager;
use crate::library::Library;
use crate::model::album::Album;
use crate::model::artist::Artist;
use crate::model::episode::Episode;
use crate::model::playlist::Playlist;
use crate::model::show::Show;
use crate::external::{self, Page};
use crate::model::track::{Source, Track};
use crate::queue::Queue;
use crate::spotify::{Spotify, UriType};
use crate::spotify_url::SpotifyUrl;
use crate::traits::{ListItem, ViewExt};
use crate::ui::listview::ListView;
use crate::ui::pagination::Pagination;
use crate::ui::source_filter::{FilteredList, SourceFilter, build_results, merge_and_filter};
use crate::ui::tabbedview::TabbedView;
use cursive::Cursive;
use cursive::view::ViewWrapper;
use rspotify::model::SearchType;
use rspotify::model::search::SearchResult;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// Resultados de Tracks sin filtrar y cursores de paginación por servicio.
#[derive(Default)]
struct SearchState {
    /// Número de búsqueda: un hilo de una búsqueda vieja no escribe en la nueva.
    generation: u64,
    all: Vec<Track>,
    on: HashSet<Source>,
    /// `Some(token)` mientras el servicio tenga más páginas (Spotify: offset).
    next: HashMap<Source, Option<String>>,
}

type SharedState = Arc<Mutex<SearchState>>;

/// Recalcula la lista visible y el rango de paginación a partir de `all` y los filtros.
fn refresh(state: &Mutex<SearchState>, results: &RwLock<Vec<Track>>, pagination: &Pagination<Track>) {
    let st = state.lock().unwrap();
    let list = merge_and_filter(&st.all, &st.on);
    let len = list.len();
    let more = st.on.iter().any(|s| matches!(st.next.get(s), Some(Some(_))));
    // el rango se actualiza con la lista aún bloqueada: un dibujo nunca ve uno sin el otro
    let mut shown = results.write().unwrap();
    *shown = list;
    pagination.update_range(len, more.then_some(len + 1));
    drop(shown);
}

/// Agrega la página de `source` si la búsqueda sigue vigente; los fallos solo avisan.
fn apply_page(
    state: &Mutex<SearchState>,
    generation: u64,
    source: Source,
    result: Result<Page, String>,
    ev: &EventManager,
) -> bool {
    let mut st = state.lock().unwrap();
    if st.generation != generation {
        return false;
    }
    st.next.insert(source, result.as_ref().ok().and_then(|p| p.next.clone()));
    let (tracks, warnings) = build_results(vec![(source, result)]);
    st.all.extend(tracks);
    drop(st);
    for w in warnings {
        ev.notify(w);
    }
    true
}

fn fetch_page(spotify: &Spotify, source: Source, query: &str, token: Option<&str>) -> Result<Page, String> {
    match source {
        Source::Spotify => {
            let offset: u32 = token.and_then(|t| t.parse().ok()).unwrap_or(0);
            match spotify.api.search(SearchType::Track, query, crate::authentication::search_page_size(), offset) {
                Ok(SearchResult::Tracks(r)) => {
                    let tracks: Vec<Track> = r.items.iter().map(|ft| ft.into()).collect();
                    let loaded = offset + tracks.len() as u32;
                    let next = (!tracks.is_empty() && loaded < r.total).then(|| loaded.to_string());
                    Ok(Page { tracks, next })
                }
                _ => Err("Spotify".into()),
            }
        }
        Source::YouTube => external::youtube::search(query, token),
        Source::SoundCloud => external::soundcloud::search(query, token),
    }
}

pub struct SearchResultsView {
    search_term: String,
    results_tracks: Arc<RwLock<Vec<Track>>>,
    state: SharedState,
    pagination_tracks: Pagination<Track>,
    results_albums: Arc<RwLock<Vec<Album>>>,
    pagination_albums: Pagination<Album>,
    results_artists: Arc<RwLock<Vec<Artist>>>,
    pagination_artists: Pagination<Artist>,
    results_playlists: Arc<RwLock<Vec<Playlist>>>,
    pagination_playlists: Pagination<Playlist>,
    results_shows: Arc<RwLock<Vec<Show>>>,
    pagination_shows: Pagination<Show>,
    results_episodes: Arc<RwLock<Vec<Episode>>>,
    pagination_episodes: Pagination<Episode>,
    tabs: TabbedView,
    spotify: Spotify,
    events: EventManager,
}

type SearchHandler<I> =
    Box<dyn Fn(&Spotify, &Arc<RwLock<Vec<I>>>, &str, usize, bool) -> u32 + Send + Sync>;

impl SearchResultsView {
    pub fn new(
        search_term: String,
        events: EventManager,
        queue: Arc<Queue>,
        library: Arc<Library>,
    ) -> Self {
        let results_tracks = Arc::new(RwLock::new(Vec::new()));
        let results_albums = Arc::new(RwLock::new(Vec::new()));
        let results_artists = Arc::new(RwLock::new(Vec::new()));
        let results_playlists = Arc::new(RwLock::new(Vec::new()));
        let results_shows = Arc::new(RwLock::new(Vec::new()));
        let results_episodes = Arc::new(RwLock::new(Vec::new()));

        let list_tracks = ListView::new(results_tracks.clone(), queue.clone(), library.clone());
        let pagination_tracks = list_tracks.get_pagination().clone();
        let state: SharedState = Arc::default();
        let mut active = Vec::new();
        if queue.get_spotify().is_online() {
            active.push(Source::Spotify);
        }
        active.extend(external::enabled());
        state.lock().unwrap().on = active.iter().copied().collect();
        let mut bar = SourceFilter::new(active);
        {
            let (state, results, pagination) = (state.clone(), results_tracks.clone(), pagination_tracks.clone());
            bar.on_change(move |on| {
                state.lock().unwrap().on = on.clone();
                refresh(&state, &results, &pagination);
            });
        }
        let tracks_tab = FilteredList::new(bar, list_tracks);
        let list_albums = ListView::new(results_albums.clone(), queue.clone(), library.clone());
        let pagination_albums = list_albums.get_pagination().clone();
        let list_artists = ListView::new(results_artists.clone(), queue.clone(), library.clone());
        let pagination_artists = list_artists.get_pagination().clone();
        let list_playlists =
            ListView::new(results_playlists.clone(), queue.clone(), library.clone());
        let pagination_playlists = list_playlists.get_pagination().clone();
        let list_shows = ListView::new(results_shows.clone(), queue.clone(), library.clone());
        let pagination_shows = list_shows.get_pagination().clone();
        let list_episodes = ListView::new(results_episodes.clone(), queue.clone(), library);
        let pagination_episodes = list_episodes.get_pagination().clone();

        let mut tabs = TabbedView::new();
        tabs.add_tab("Tracks", tracks_tab);
        tabs.add_tab("Albums", list_albums);
        tabs.add_tab("Artists", list_artists);
        tabs.add_tab("Playlists", list_playlists);
        tabs.add_tab("Shows", list_shows);
        tabs.add_tab("Episodes", list_episodes);

        let mut view = Self {
            search_term,
            results_tracks,
            state,
            pagination_tracks,
            results_albums,
            pagination_albums,
            results_artists,
            pagination_artists,
            results_playlists,
            pagination_playlists,
            results_shows,
            pagination_shows,
            results_episodes,
            pagination_episodes,
            tabs,
            spotify: queue.get_spotify(),
            events,
        };

        view.run_search();
        view
    }

    fn get_album(
        spotify: &Spotify,
        albums: &Arc<RwLock<Vec<Album>>>,
        query: &str,
        _offset: usize,
        _append: bool,
    ) -> u32 {
        if let Ok(results) = spotify.api.album(query) {
            let a = vec![(&results).into()];
            let mut r = albums.write().unwrap();
            *r = a;
            return 1;
        }
        0
    }

    fn search_album(
        spotify: &Spotify,
        albums: &Arc<RwLock<Vec<Album>>>,
        query: &str,
        offset: usize,
        append: bool,
    ) -> u32 {
        if let Ok(SearchResult::Albums(results)) =
            spotify
                .api
                .search(SearchType::Album, query, crate::authentication::search_page_size(), offset as u32)
        {
            let mut a = results.items.iter().map(|sa| sa.into()).collect();
            let mut r = albums.write().unwrap();

            if append {
                r.append(&mut a);
            } else {
                *r = a;
            }
            return results.total;
        }
        0
    }

    fn get_artist(
        spotify: &Spotify,
        artists: &Arc<RwLock<Vec<Artist>>>,
        query: &str,
        _offset: usize,
        _append: bool,
    ) -> u32 {
        if let Ok(results) = spotify.api.artist(query) {
            let a = vec![(&results).into()];
            let mut r = artists.write().unwrap();
            *r = a;
            return 1;
        }
        0
    }

    fn search_artist(
        spotify: &Spotify,
        artists: &Arc<RwLock<Vec<Artist>>>,
        query: &str,
        offset: usize,
        append: bool,
    ) -> u32 {
        if let Ok(SearchResult::Artists(results)) =
            spotify
                .api
                .search(SearchType::Artist, query, crate::authentication::search_page_size(), offset as u32)
        {
            let mut a = results.items.iter().map(|fa| fa.into()).collect();
            let mut r = artists.write().unwrap();

            if append {
                r.append(&mut a);
            } else {
                *r = a;
            }
            return results.total;
        }
        0
    }

    fn get_playlist(
        spotify: &Spotify,
        playlists: &Arc<RwLock<Vec<Playlist>>>,
        query: &str,
        _offset: usize,
        _append: bool,
    ) -> u32 {
        if let Ok(result) = spotify.api.playlist(query).as_ref() {
            let pls = vec![result.into()];
            let mut r = playlists.write().unwrap();
            *r = pls;
            return 1;
        }
        0
    }

    fn search_playlist(
        spotify: &Spotify,
        playlists: &Arc<RwLock<Vec<Playlist>>>,
        query: &str,
        offset: usize,
        append: bool,
    ) -> u32 {
        if let Ok(SearchResult::Playlists(results)) =
            spotify
                .api
                .search(SearchType::Playlist, query, crate::authentication::search_page_size(), offset as u32)
        {
            let mut pls = results.items.iter().map(|sp| sp.into()).collect();
            let mut r = playlists.write().unwrap();

            if append {
                r.append(&mut pls);
            } else {
                *r = pls;
            }
            return results.total;
        }
        0
    }

    fn get_show(
        spotify: &Spotify,
        shows: &Arc<RwLock<Vec<Show>>>,
        query: &str,
        _offset: usize,
        _append: bool,
    ) -> u32 {
        if let Ok(result) = spotify.api.show(query).as_ref() {
            let pls = vec![result.into()];
            let mut r = shows.write().unwrap();
            *r = pls;
            return 1;
        }
        0
    }

    fn search_show(
        spotify: &Spotify,
        shows: &Arc<RwLock<Vec<Show>>>,
        query: &str,
        offset: usize,
        append: bool,
    ) -> u32 {
        if let Ok(SearchResult::Shows(results)) =
            spotify
                .api
                .search(SearchType::Show, query, crate::authentication::search_page_size(), offset as u32)
        {
            let mut pls = results.items.iter().map(|sp| sp.into()).collect();
            let mut r = shows.write().unwrap();

            if append {
                r.append(&mut pls);
            } else {
                *r = pls;
            }
            return results.total;
        }
        0
    }

    fn get_episode(
        spotify: &Spotify,
        episodes: &Arc<RwLock<Vec<Episode>>>,
        query: &str,
        _offset: usize,
        _append: bool,
    ) -> u32 {
        if let Ok(result) = spotify.api.episode(query).as_ref() {
            let e = vec![result.into()];
            let mut r = episodes.write().unwrap();
            *r = e;
            return 1;
        }
        0
    }

    fn search_episode(
        spotify: &Spotify,
        episodes: &Arc<RwLock<Vec<Episode>>>,
        query: &str,
        offset: usize,
        append: bool,
    ) -> u32 {
        if let Ok(SearchResult::Episodes(results)) =
            spotify
                .api
                .search(SearchType::Episode, query, crate::authentication::search_page_size(), offset as u32)
        {
            let mut e = results.items.iter().map(|se| se.into()).collect();
            let mut r = episodes.write().unwrap();

            if append {
                r.append(&mut e);
            } else {
                *r = e;
            }
            return results.total;
        }
        0
    }

    fn perform_search<I: ListItem + Clone>(
        &self,
        handler: SearchHandler<I>,
        results: &Arc<RwLock<Vec<I>>>,
        query: &str,
        paginator: Option<&Pagination<I>>,
    ) {
        let spotify = self.spotify.clone();
        let query = query.to_owned();
        let results = results.clone();
        let ev = self.events.clone();
        let paginator = paginator.cloned();

        std::thread::spawn(move || {
            let total_items = handler(&spotify, &results, &query, 0, false) as usize;

            // register paginator if the API has more than one page of results
            if let Some(mut paginator) = paginator {
                let loaded_items = results.read().unwrap().len();
                if total_items > loaded_items {
                    let ev = ev.clone();

                    // paginator callback
                    let cb = move |items: Arc<RwLock<Vec<I>>>| {
                        let offset = items.read().unwrap().len();
                        handler(&spotify, &results, &query, offset, true);
                        ev.trigger();
                    };
                    paginator.set(loaded_items, total_items, Box::new(cb));
                } else {
                    paginator.clear()
                }
            }
            ev.trigger();
        });
    }

    /// Nueva búsqueda de Tracks: invalida hilos viejos y devuelve su número.
    fn begin_tracks(&self) -> u64 {
        let mut st = self.state.lock().unwrap();
        st.generation += 1;
        st.all.clear();
        st.next.clear();
        st.generation
    }

    /// Una URI/URL de pista de Spotify va a la misma lista que el resto.
    fn spotify_track(&self, id: &str) {
        let generation = self.begin_tracks();
        let (spotify, id, ev) = (self.spotify.clone(), id.to_owned(), self.events.clone());
        let (state, results, pagination) =
            (self.state.clone(), self.results_tracks.clone(), self.pagination_tracks.clone());
        std::thread::spawn(move || {
            let page = spotify
                .api
                .track(&id)
                .map(|t| Page { tracks: vec![(&t).into()], next: None })
                .map_err(|_| "Spotify".to_string());
            if apply_page(&state, generation, Source::Spotify, page, &ev) {
                refresh(&state, &results, &pagination);
                ev.trigger();
            }
        });
    }

    /// Un hilo por servicio (Spotify solo si hay sesión); la paginación pide la
    /// página siguiente de cada servicio prendido que tenga más.
    fn search_mixed(&self, query: &str) {
        let generation = self.begin_tracks();
        let mut sources = Vec::new();
        if self.spotify.is_online() {
            sources.push(Source::Spotify);
        }
        sources.extend(external::enabled());

        let (state, results, pagination) =
            (self.state.clone(), self.results_tracks.clone(), self.pagination_tracks.clone());
        {
            let (spotify, query, ev) = (self.spotify.clone(), query.to_owned(), self.events.clone());
            let (state, results, pg) = (state.clone(), results.clone(), pagination.clone());
            pagination.set(
                0,
                0,
                Box::new(move |_| {
                    let pending: Vec<(Source, String)> = {
                        let st = state.lock().unwrap();
                        st.on
                            .iter()
                            .filter_map(|s| Some((*s, st.next.get(s)?.clone()?)))
                            .collect()
                    };
                    std::thread::scope(|scope| {
                        for (source, token) in pending {
                            let (spotify, query, ev, state) = (&spotify, &query, &ev, &state);
                            scope.spawn(move || {
                                let page = fetch_page(spotify, source, query, Some(&token));
                                apply_page(state, generation, source, page, ev);
                            });
                        }
                    });
                    refresh(&state, &results, &pg);
                    ev.trigger();
                }),
            );
        }
        for source in sources {
            let (spotify, query, ev) = (self.spotify.clone(), query.to_owned(), self.events.clone());
            let (state, results, pagination) = (state.clone(), results.clone(), pagination.clone());
            std::thread::spawn(move || {
                let page = fetch_page(&spotify, source, &query, None);
                if apply_page(&state, generation, source, page, &ev) {
                    refresh(&state, &results, &pagination);
                    ev.trigger();
                }
            });
        }
    }

    pub fn run_search(&mut self) {
        let query = self.search_term.clone();

        // check if API token refresh is necessary before commencing multiple
        // requests to avoid deadlock, as the parallel requests might
        // simultaneously try to refresh the token
        let online = self.spotify.is_online();
        if online {
            self.spotify
                .api
                .update_token()
                .map(move |h| ASYNC_RUNTIME.get().unwrap().block_on(h).ok());
        }

        // is the query a Spotify URI?
        if online && let Ok(uritype) = query.parse() {
            match uritype {
                UriType::Track => {
                    self.spotify_track(&query);
                    self.tabs.set_selected(0);
                }
                UriType::Album => {
                    self.perform_search(
                        Box::new(Self::get_album),
                        &self.results_albums,
                        &query,
                        None,
                    );
                    self.tabs.set_selected(1);
                }
                UriType::Artist => {
                    self.perform_search(
                        Box::new(Self::get_artist),
                        &self.results_artists,
                        &query,
                        None,
                    );
                    self.tabs.set_selected(2);
                }
                UriType::Playlist => {
                    self.perform_search(
                        Box::new(Self::get_playlist),
                        &self.results_playlists,
                        &query,
                        None,
                    );
                    self.tabs.set_selected(3);
                }
                UriType::Show => {
                    self.perform_search(
                        Box::new(Self::get_show),
                        &self.results_shows,
                        &query,
                        None,
                    );
                    self.tabs.set_selected(4);
                }
                UriType::Episode => {
                    self.perform_search(
                        Box::new(Self::get_episode),
                        &self.results_episodes,
                        &query,
                        None,
                    );
                    self.tabs.set_selected(5);
                }
            }
        // Is the query a spotify URL?
        // https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC
        } else if online && let Some(url) = SpotifyUrl::from_url(&query) {
            match url.uri_type {
                UriType::Track => {
                    self.spotify_track(&url.id);
                    self.tabs.set_selected(0);
                }
                UriType::Album => {
                    self.perform_search(
                        Box::new(Self::get_album),
                        &self.results_albums,
                        &url.id,
                        None,
                    );
                    self.tabs.set_selected(1);
                }
                UriType::Artist => {
                    self.perform_search(
                        Box::new(Self::get_artist),
                        &self.results_artists,
                        &url.id,
                        None,
                    );
                    self.tabs.set_selected(2);
                }
                UriType::Playlist => {
                    self.perform_search(
                        Box::new(Self::get_playlist),
                        &self.results_playlists,
                        &url.id,
                        None,
                    );
                    self.tabs.set_selected(3);
                }
                UriType::Show => {
                    self.perform_search(
                        Box::new(Self::get_show),
                        &self.results_shows,
                        &url.id,
                        None,
                    );
                    self.tabs.set_selected(4);
                }
                UriType::Episode => {
                    self.perform_search(
                        Box::new(Self::get_episode),
                        &self.results_episodes,
                        &url.id,
                        None,
                    );
                    self.tabs.set_selected(5);
                }
            }
        } else {
            self.search_mixed(&query);
            if !online {
                return;
            }
            self.perform_search(
                Box::new(Self::search_album),
                &self.results_albums,
                &query,
                Some(&self.pagination_albums),
            );
            self.perform_search(
                Box::new(Self::search_artist),
                &self.results_artists,
                &query,
                Some(&self.pagination_artists),
            );
            self.perform_search(
                Box::new(Self::search_playlist),
                &self.results_playlists,
                &query,
                Some(&self.pagination_playlists),
            );
            self.perform_search(
                Box::new(Self::search_show),
                &self.results_shows,
                &query,
                Some(&self.pagination_shows),
            );
            self.perform_search(
                Box::new(Self::search_episode),
                &self.results_episodes,
                &query,
                Some(&self.pagination_episodes),
            );
        }
    }
}

impl ViewWrapper for SearchResultsView {
    wrap_impl!(self.tabs: TabbedView);
}

impl ViewExt for SearchResultsView {
    fn title(&self) -> String {
        format!("Search: {}", self.search_term)
    }
    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        self.tabs.on_command(s, cmd)
    }
}
