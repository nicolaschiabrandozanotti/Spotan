use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use cursive::Cursive;
use cursive::view::{View, ViewWrapper};
use strum::IntoEnumIterator;

use crate::command::Command;
use crate::commands::CommandResult;
use crate::config::LibraryTab;
use crate::library::Library;
use crate::queue::Queue;
use crate::traits::ViewExt;
use crate::ui::browse::BrowseView;
use crate::ui::listview::ListView;
use crate::ui::playlists::PlaylistsView;
use crate::ui::source_filter::{FilteredList, SourceFilter, Sourced, library_sources, merge_and_filter};
use crate::ui::tabbedview::TabbedView;
use crate::traits::ListItem;
use std::collections::HashSet;
use crate::model::track::Source;

const OFFLINE_HINT: &str = "Iniciá sesión con Spotify en Ajustes (F4) para ver tu biblioteca";
const NO_SOURCE_HINT: &str = "Iniciá sesión con Spotify o configurá SoundCloud o YouTube en Ajustes (F4)";

fn new_bar(active: Vec<Source>, recompute: &Arc<dyn Fn(&HashSet<Source>) + Send + Sync>) -> SourceFilter {
    let mut bar = SourceFilter::new(active);
    let r = recompute.clone();
    bar.on_change(move |on| r(on));
    bar
}

/// Aviso de sesión según el nombre interno de la pestaña (no el traducido).
fn hint_for(online: bool, external_active: bool, tab: &str) -> Option<&'static str> {
    if !online && !external_active {
        Some(NO_SOURCE_HINT)
    } else if !online && !matches!(tab, "Tracks" | "Albums" | "Playlists") {
        Some(OFFLINE_HINT)
    } else {
        None
    }
}

/// Sin otro servicio activo la lista muestra el `Arc` de Spotify tal cual, sin copia derivada (RAM).
fn show_spotify_directly(active: &[Source]) -> bool {
    active.iter().all(|s| *s == Source::Spotify)
}

/// Servicios de una pestaña: Spotify (si hay sesión) y los activos que tienen datos en ella.
fn tab_sources(active: Vec<Source>, nonempty: &[Source]) -> Vec<Source> {
    active.into_iter().filter(|s| *s == Source::Spotify || nonempty.contains(s)).collect()
}

/// Lista de la biblioteca con Spotify y los demás servicios mezclados. Se recalcula al cambiar el
/// filtro o `library.generation()`; sin otros servicios activos no hay copia.
pub fn mixed_list<T: ListItem + Sourced + Clone>(
    queue: Arc<Queue>,
    library: Arc<Library>,
    spotify: Arc<RwLock<Vec<T>>>,
    others: Vec<(Source, Arc<RwLock<Vec<T>>>)>,
) -> FilteredList<T> {
    let shown = Arc::new(RwLock::new(Vec::new()));
    let recompute: Arc<dyn Fn(&HashSet<Source>) + Send + Sync> = {
        let (shown, spotify, others) = (shown.clone(), spotify.clone(), others.clone());
        Arc::new(move |on| {
            // cada almacén tiene un solo servicio: el orden se conserva al concatenar
            let mut v = merge_and_filter(&spotify.read().unwrap(), on);
            for (_, o) in &others {
                v.extend(merge_and_filter(&o.read().unwrap(), on));
            }
            *shown.write().unwrap() = v;
        })
    };
    let tab_active = {
        let (library, others) = (library.clone(), others.clone());
        move || {
            let nonempty: Vec<Source> = others.iter().filter(|(_, o)| !o.read().unwrap().is_empty()).map(|(s, _)| *s).collect();
            tab_sources(library_sources(&library), &nonempty)
        }
    };
    let active = tab_active();
    let bar = new_bar(active.clone(), &recompute);
    let content = if show_spotify_directly(&active) {
        spotify.clone()
    } else {
        recompute(bar.on());
        shown.clone()
    };
    let mut tab = FilteredList::new(bar, ListView::new(content, queue, library.clone()));
    let mut seen = library.generation();
    let mut last = Instant::now();
    tab.on_layout = Some(Box::new(move |bar, list| {
        let generation = library.generation();
        let loading = !*library.is_done.read().unwrap();
        if should_recompute(generation != seen, loading, last.elapsed()) {
            seen = generation;
            last = Instant::now();
            let active = tab_active();
            if active != bar.active() {
                *bar = new_bar(active.clone(), &recompute);
            }
            if show_spotify_directly(&active) {
                *shown.write().unwrap() = Vec::new();
                list.set_content(spotify.clone());
            } else {
                recompute(bar.on());
                list.set_content(shown.clone());
            }
        }
        // la lista pudo achicarse: la selección no se pasa del final
        let sel = list.get_selected_index();
        if sel > 0 && sel >= list.content().read().unwrap().len() {
            list.move_focus_to(sel);
        }
    }));
    tab
}

/// Durante la primera carga de Spotify cada lista nueva sube la generación: se recalcula
/// a lo sumo cada 500 ms (`seen` no cambia al saltear, el próximo layout se pone al día).
fn should_recompute(changed: bool, loading: bool, since_last: Duration) -> bool {
    changed && (!loading || since_last >= Duration::from_millis(500))
}

pub struct LibraryView {
    tabs: TabbedView,
    display_name: Arc<RwLock<Option<String>>>,
    library: Arc<Library>,
}

impl LibraryView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        let mut tabview = TabbedView::new();
        let selected_tabs = library
            .cfg
            .values()
            .library_tabs
            .clone()
            .unwrap_or_else(|| Vec::from_iter(LibraryTab::iter()));

        for tab in selected_tabs {
            match tab {
                LibraryTab::Tracks => tabview.add_tab(
                    "Tracks",
                    mixed_list(queue.clone(), library.clone(), library.tracks.clone(), vec![(Source::SoundCloud, library.sc_tracks.clone())]),
                ),
                LibraryTab::Albums => tabview.add_tab(
                    "Albums",
                    mixed_list(queue.clone(), library.clone(), library.albums.clone(), vec![(Source::YouTube, library.yt_albums.clone()), (Source::SoundCloud, library.sc_albums.clone())]),
                ),
                LibraryTab::Artists => tabview.add_tab(
                    "Artists",
                    ListView::new(library.artists.clone(), queue.clone(), library.clone()),
                ),
                LibraryTab::Playlists => tabview.add_tab(
                    "Playlists",
                    PlaylistsView::new(queue.clone(), library.clone()),
                ),
                LibraryTab::Podcasts => tabview.add_tab(
                    "Podcasts",
                    ListView::new(library.shows.clone(), queue.clone(), library.clone()),
                ),
                LibraryTab::Browse => {
                    tabview.add_tab("Browse", BrowseView::new(queue.clone(), library.clone()))
                }
            }
        }

        Self {
            tabs: tabview,
            display_name: library.display_name.clone(),
            library,
        }
    }
}

impl ViewWrapper for LibraryView {
    wrap_impl!(self.tabs: TabbedView);

    fn wrap_draw(&self, printer: &cursive::Printer) {
        self.tabs.draw(printer);
        let online = self.library.is_online();
        let hint = hint_for(online, !online && (self.library.sc_active() || self.library.yt_active()), self.tabs.selected_name());
        if let Some(hint) = hint {
            printer.print((1, 3), hint);
        }
    }
}

impl ViewExt for LibraryView {
    fn title(&self) -> String {
        if let Some(name) = &*self.display_name.read().unwrap() {
            format!("Biblioteca · {} · {name}",self.tabs.selected_title())
        } else {
            format!("Biblioteca · {}",self.tabs.selected_title())
        }
    }

    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        self.tabs.on_command(s, cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_list_is_shown_directly_without_soundcloud() {
        assert!(show_spotify_directly(&[]));
        assert!(show_spotify_directly(&[Source::Spotify]));
        assert!(!show_spotify_directly(&[Source::SoundCloud]));
        assert!(!show_spotify_directly(&[Source::YouTube]));
        assert!(!show_spotify_directly(&[Source::Spotify, Source::SoundCloud]));
    }

    #[test]
    fn tab_sources_only_with_data_in_that_tab() {
        let all = vec![Source::Spotify, Source::YouTube, Source::SoundCloud];
        // Temas: YouTube activo pero sin temas propios
        assert_eq!(tab_sources(all.clone(), &[Source::SoundCloud]), [Source::Spotify, Source::SoundCloud]);
        assert_eq!(tab_sources(all.clone(), &[]), [Source::Spotify]);
        assert_eq!(tab_sources(all, &[Source::SoundCloud, Source::YouTube]).len(), 3);
        assert_eq!(tab_sources(vec![Source::YouTube], &[]), []);
    }

    #[test]
    fn recompute_is_throttled_only_while_loading() {
        let (short, long) = (Duration::from_millis(10), Duration::from_millis(600));
        assert!(!should_recompute(false, false, long));
        assert!(should_recompute(true, false, short));
        assert!(!should_recompute(true, true, short));
        assert!(should_recompute(true, true, long));
    }

    #[test]
    fn hint_depends_on_internal_tab_name() {
        assert_eq!(hint_for(false, false, "Tracks"), Some(NO_SOURCE_HINT));
        assert_eq!(hint_for(false, true, "Tracks"), None);
        assert_eq!(hint_for(false, true, "Playlists"), None);
        assert_eq!(hint_for(false, true, "Albums"), None);
        assert_eq!(hint_for(false, true, "Artists"), Some(OFFLINE_HINT));
        assert_eq!(hint_for(false, false, "Albums"), Some(NO_SOURCE_HINT));
        assert_eq!(hint_for(true, false, "Albums"), None);
    }
}
