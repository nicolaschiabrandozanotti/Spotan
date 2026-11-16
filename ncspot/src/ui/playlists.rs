use std::sync::{Arc, RwLock};

use cursive::Cursive;
use cursive::view::{Margins, ViewWrapper};
use cursive::views::Dialog;

use crate::command::Command;
use crate::commands::CommandResult;
use crate::library::Library;
use crate::model::playlist::Playlist;
use crate::queue::Queue;
use crate::traits::ViewExt;
use crate::ui::library::mixed_list;
use crate::ui::modal::Modal;
use crate::ui::source_filter::FilteredList;

pub struct PlaylistsView {
    list: FilteredList<Playlist>,
    /// Lista derivada (Spotify + SoundCloud filtrados): la selección se resuelve acá.
    shown: Arc<RwLock<Vec<Playlist>>>,
    library: Arc<Library>,
}

impl PlaylistsView {
    pub fn new(queue: Arc<Queue>, library: Arc<Library>) -> Self {
        let (list, shown) =
            mixed_list(queue, library.clone(), library.playlists.clone(), library.sc_playlists.clone());
        Self { list, shown, library }
    }

    pub fn delete_dialog(&mut self) -> Option<Modal<Dialog>> {
        let playlists = self.shown.read().unwrap();
        let current = playlists.get(self.list.list.get_selected_index());

        if let Some(playlist) = current {
            let library = self.library.clone();
            let id = playlist.id.clone();
            let dialog = Dialog::text("Are you sure you want to delete this playlist?")
                .padding(Margins::lrtb(1, 1, 1, 0))
                .title("Delete playlist")
                .dismiss_button("No")
                .button("Yes", move |s: &mut Cursive| {
                    library.delete_playlist(&id);
                    s.pop_layer();
                });
            Some(Modal::new(dialog))
        } else {
            None
        }
    }
}

impl ViewWrapper for PlaylistsView {
    wrap_impl!(self.list: FilteredList<Playlist>);
}

impl ViewExt for PlaylistsView {
    fn title(&self) -> String {
        "Playlists".to_string()
    }

    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        if let Command::Delete = cmd {
            let is_sc = self
                .shown
                .read()
                .unwrap()
                .get(self.list.list.get_selected_index())
                .is_some_and(|p| p.is_read_only());
            if is_sc {
                return Err("SoundCloud: solo lectura".into());
            }
            if let Some(dialog) = self.delete_dialog() {
                s.add_layer(dialog);
            }
            return Ok(CommandResult::Consumed(None));
        }

        self.list.on_command(s, cmd)
    }
}
