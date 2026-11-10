//! Managing playlists: the prompts that create, rename and delete them, the
//! picker that adds a track to one, and the edits of the open playlist.
//!
//! An edit goes to the backend as a [`Request::Edit`]. Edits run one at a
//! time, in the order they were made, so that a fast sequence of moves
//! reaches the backend as it was typed. Removing and moving an entry show at
//! once; while edits are outstanding, loads of the playlists and of every
//! playlist view are dropped, so a playlist opened meanwhile stays loading,
//! and once the last edit is done the playlists and every open playlist load
//! again.

use terra_core::{Playlist, PlaylistEntryId, PlaylistId, Track};

use super::app::{App, Browse, Cursor, Focus, Item, Load, Payload, Request, Status, View};

/// A prompt that has the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Typing a playlist's name: a new playlist's, or a new name for the
    /// playlist `renaming`.
    Name {
        renaming: Option<Playlist>,
        text: String,
    },
    /// Asking whether to delete the playlist.
    Delete(Playlist),
    /// Choosing the playlist to add a track to.
    Picker(Picker),
}

/// The playlist picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub track: Track,
    pub query: String,
    /// The selected row of [`matches`](Self::matches).
    pub selected: usize,
}

impl Picker {
    /// The playlists whose name contains the query, ignoring case.
    pub fn matches<'a>(&self, playlists: &'a [Playlist]) -> Vec<&'a Playlist> {
        let query = self.query.trim().to_lowercase();
        playlists
            .iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&query))
            .collect()
    }
}

/// A change to the playlists that the backend makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Create(String),
    Rename(PlaylistId, String),
    Delete(Playlist),
    Add {
        playlist: Playlist,
        track: Track,
    },
    Remove {
        playlist: PlaylistId,
        entry: PlaylistEntryId,
    },
    Move {
        playlist: PlaylistId,
        entry: PlaylistEntryId,
        to: usize,
    },
}

fn info(text: String) -> Option<Status> {
    Some(Status { text, error: false })
}

impl App {
    /// The playlist selected in the sidebar, if a playlist is.
    pub fn sidebar_playlist(&self) -> Option<&Playlist> {
        let index = self.sidebar.checked_sub(Browse::ALL.len())?;
        self.playlist_list().get(index)
    }

    /// The playlist that rename and delete act on: the one selected in the
    /// sidebar, or the open one when the list has the focus.
    fn target_playlist(&self) -> Option<Playlist> {
        let playlist = match (self.focus, self.root_view()) {
            (Focus::Sidebar, _) => self.sidebar_playlist(),
            (Focus::List, View::Playlist(id)) => self
                .playlist_list()
                .iter()
                .find(|playlist| playlist.id == id),
            (Focus::List, _) => None,
        };
        playlist.cloned()
    }

    pub(super) fn new_playlist(&mut self) {
        self.modal = Some(Modal::Name {
            renaming: None,
            text: String::new(),
        });
    }

    pub(super) fn rename_playlist(&mut self) {
        match self.target_playlist() {
            Some(playlist) => {
                let text = playlist.name.clone();
                self.modal = Some(Modal::Name {
                    renaming: Some(playlist),
                    text,
                });
            }
            None => self.status = info("Select a playlist to rename".to_owned()),
        }
    }

    pub(super) fn delete_playlist(&mut self) {
        match self.target_playlist() {
            Some(playlist) => self.modal = Some(Modal::Delete(playlist)),
            None => self.status = info("Select a playlist to delete".to_owned()),
        }
    }

    /// Opens the picker for the selected track of the main list.
    pub(super) fn pick_playlist(&mut self) {
        let track = match self.focus {
            Focus::List => self.selected_item().and_then(Item::track),
            Focus::Sidebar => None,
        };
        match track {
            Some(track) => {
                self.modal = Some(Modal::Picker(Picker {
                    track: track.clone(),
                    query: String::new(),
                    selected: 0,
                }));
            }
            None => self.status = info("Select a track to add".to_owned()),
        }
    }

    /// Changes the text of the name prompt or the picker's query.
    pub(super) fn edit_prompt(&mut self, edit: impl FnOnce(&mut String)) {
        match &mut self.modal {
            Some(Modal::Name { text, .. }) => edit(text),
            Some(Modal::Picker(picker)) => {
                edit(&mut picker.query);
                picker.selected = 0;
            }
            Some(Modal::Delete(_)) | None => {}
        }
    }

    pub(super) fn confirm_prompt(&mut self) -> Vec<Request> {
        let edit = match self.modal.take() {
            Some(Modal::Name {
                renaming: None,
                text,
            }) => Edit::Create(text),
            Some(Modal::Name {
                renaming: Some(playlist),
                text,
            }) => Edit::Rename(playlist.id, text),
            Some(Modal::Delete(playlist)) => Edit::Delete(playlist),
            Some(Modal::Picker(picker)) => {
                let matches = picker.matches(self.playlist_list());
                let Some(&playlist) = matches.get(picker.selected) else {
                    // Nothing matches: the picker stays open.
                    self.modal = Some(Modal::Picker(picker));
                    return Vec::new();
                };
                Edit::Add {
                    playlist: playlist.clone(),
                    track: picker.track,
                }
            }
            None => return Vec::new(),
        };
        self.enqueue(edit)
    }

    /// The open playlist, its rows and its cursor, when the main list has
    /// the focus and shows them.
    fn open_entries(&mut self) -> Option<(PlaylistId, &mut Vec<Item>, &mut Cursor)> {
        if self.focus != Focus::List || self.searching() {
            return None;
        }
        let top = self.top_mut();
        let View::Playlist(id) = top.view else {
            return None;
        };
        let Load::Ready(rows) = &mut top.rows else {
            return None;
        };
        Some((id, rows, &mut top.cursor))
    }

    pub(super) fn remove_entry(&mut self) -> Vec<Request> {
        let height = self.list_height;
        let Some((playlist, rows, cursor)) = self.open_entries() else {
            return Vec::new();
        };
        let at = cursor.selected;
        let Some(Item::Entry(entry)) = rows.get(at) else {
            return Vec::new();
        };
        let entry = entry.id;
        rows.remove(at);
        cursor.select(at, rows.len(), height);
        self.enqueue(Edit::Remove { playlist, entry })
    }

    /// Moves the selected entry one place down or up. At the end it moves
    /// towards, it stays.
    pub(super) fn move_entry(&mut self, down: bool) -> Vec<Request> {
        let height = self.list_height;
        let Some((playlist, rows, cursor)) = self.open_entries() else {
            return Vec::new();
        };
        let from = cursor.selected;
        let Some(Item::Entry(entry)) = rows.get(from) else {
            return Vec::new();
        };
        let entry = entry.id;
        let to = if down {
            from.checked_add(1)
        } else {
            from.checked_sub(1)
        };
        let Some(to) = to.filter(|&to| to < rows.len()) else {
            return Vec::new();
        };
        rows.swap(from, to);
        cursor.select(to, rows.len(), height);
        self.enqueue(Edit::Move {
            playlist,
            entry,
            to,
        })
    }

    /// Queues an edit, and starts it unless another is running.
    fn enqueue(&mut self, edit: Edit) -> Vec<Request> {
        self.edits.push_back(edit);
        match self.edits.len() {
            1 => vec![Request::Edit(self.edits[0].clone())],
            _ => Vec::new(),
        }
    }

    /// Gives the playlists and every open playlist a new load, so that the
    /// responses of earlier ones are dropped, and returns the requests.
    fn reload_playlists(&mut self) -> Vec<Request> {
        self.playlists_load = self.seq();
        let mut requests = vec![Request::Playlists {
            load: self.playlists_load,
        }];
        for index in 0..self.stack.len() {
            if let View::Playlist(_) = self.stack[index].view {
                let load = self.seq();
                let screen = &mut self.stack[index];
                screen.load = load;
                requests.push(Request::Screen {
                    id: screen.id,
                    load,
                    view: screen.view,
                });
            }
        }
        requests
    }

    /// Takes the result of the running edit, starts the next one and, after
    /// the last, reloads the playlists. After an error the queued edits are
    /// dropped, since they were made on top of the failed one.
    pub(super) fn finish_edit(
        &mut self,
        edit: &Edit,
        result: Result<Payload, String>,
    ) -> Vec<Request> {
        self.edits.pop_front();
        let mut requests = Vec::new();
        match result {
            Ok(payload) => {
                self.status = match (edit, payload) {
                    (Edit::Create(_), Payload::Playlist(playlist)) => {
                        info(format!("Created “{}”", playlist.name))
                    }
                    (Edit::Rename(..), Payload::Playlist(playlist)) => {
                        for screen in &mut self.stack {
                            if screen.view == View::Playlist(playlist.id) {
                                screen.title.clone_from(&playlist.name);
                            }
                        }
                        info(format!("Renamed to “{}”", playlist.name))
                    }
                    (Edit::Delete(playlist), _) => {
                        if self.root_view() == View::Playlist(playlist.id) {
                            self.stack.clear();
                            let browse = Browse::AllTracks;
                            requests.push(self.push(browse.label(), browse.view()));
                        }
                        info(format!("Deleted “{}”", playlist.name))
                    }
                    (Edit::Add { playlist, track }, _) => {
                        info(format!("Added “{}” to “{}”", track.title, playlist.name))
                    }
                    _ => None,
                };
            }
            Err(error) => {
                self.status = Some(Status {
                    text: error,
                    error: true,
                });
                self.edits.clear();
            }
        }
        match self.edits.front() {
            Some(next) => requests.push(Request::Edit(next.clone())),
            None => requests.extend(self.reload_playlists()),
        }
        requests
    }
}
