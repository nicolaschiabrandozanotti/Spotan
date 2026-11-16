use std::sync::{Arc, RwLock};

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
const NO_SOURCE_HINT: &str = "Iniciá sesión con Spotify o cargá tu perfil de SoundCloud en Ajustes (F4)";

fn new_bar(active: Vec<Source>, recompute: &Arc<dyn Fn(&HashSet<Source>) + Send + Sync>) -> SourceFilter {
    let mut bar = SourceFilter::new(active);
    let r = recompute.clone();
    bar.on_change(move |on| r(on));
    bar
}

/// Lista de la biblioteca con Spotify y SoundCloud mezclados; devuelve también la lista
/// derivada que muestra. Se recalcula al cambiar el filtro o `library.generation()`.
pub fn mixed_list<T: ListItem + Sourced + Clone>(
    queue: Arc<Queue>,
    library: Arc<Library>,
    spotify: Arc<RwLock<Vec<T>>>,
    sc: Arc<RwLock<Vec<T>>>,
) -> (FilteredList<T>, Arc<RwLock<Vec<T>>>) {
    let shown = Arc::new(RwLock::new(Vec::new()));
    let recompute: Arc<dyn Fn(&HashSet<Source>) + Send + Sync> = {
        let shown = shown.clone();
        Arc::new(move |on| {
            let mut all = spotify.read().unwrap().clone();
            all.extend(sc.read().unwrap().iter().cloned());
            *shown.write().unwrap() = merge_and_filter(&all, on);
        })
    };
    let bar = new_bar(library_sources(&library), &recompute);
    recompute(bar.on());
    let mut tab = FilteredList::new(bar, ListView::new(shown.clone(), queue, library.clone()));
    let shown2 = shown.clone();
    let mut seen = library.generation();
    let recompute2 = recompute.clone();
    tab.on_layout = Some(Box::new(move |bar, list| {
        let generation = library.generation();
        if generation != seen {
            seen = generation;
            let active = library_sources(&library);
            if active != bar.active() {
                *bar = new_bar(active, &recompute2);
            }
            recompute2(bar.on());
        }
        // la lista pudo achicarse: la selección no se pasa del final
        let sel = list.get_selected_index();
        if sel > 0 && sel >= shown2.read().unwrap().len() {
            list.move_focus_to(sel);
        }
    }));
    (tab, shown)
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
                    mixed_list(queue.clone(), library.clone(), library.tracks.clone(), library.sc_tracks.clone()).0,
                ),
                LibraryTab::Albums => tabview.add_tab(
                    "Albums",
                    ListView::new(library.albums.clone(), queue.clone(), library.clone()),
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
        let sc = self.library.sc_active();
        if !online && !sc {
            printer.print((1, 3), NO_SOURCE_HINT);
        } else if !online && !matches!(self.tabs.selected_title(), "Tracks" | "Playlists") {
            printer.print((1, 3), OFFLINE_HINT);
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
