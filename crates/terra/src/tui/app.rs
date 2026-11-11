//! The TUI's state, the actions that change it and the backend requests they
//! lead to.
//!
//! [`update`] is synchronous and never calls the backend. An action that needs
//! data returns a [`Request`]; the event loop runs it with
//! [`fetch`](super::fetch::fetch) and hands the [`Response`] to [`receive`].
//! A response whose screen or search is gone by the time it arrives is
//! dropped, so a slow backend never overwrites a newer list.
//!
//! Keys reach the state through [`handle_event`]. While the main list is
//! loading, a key typed to browse it is held, and the held keys are replayed
//! in order once [`receive`] has loaded the list or failed to, so that each
//! key acts on the list it was typed for. Quit and back act at once, and back
//! drops the held keys, as does leaving the screen in any other way.
//!
//! Quit waits for the playlist edits that are still outstanding, unless it is
//! pressed again; meanwhile every other key is ignored.
//!
//! The playback keys act at once, even while a list loads, and the player's
//! events reach the state through [`player_event`](super::playback::player_event).

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use crossterm::event::{Event, KeyEventKind};
use terra_core::{
    Album, AlbumFilter, AlbumKind, Artist, ArtistId, AudioSource, Playlist, PlaylistEntry,
    PlaylistId, Track, TrackFilter,
};

use super::keymap::{self, Context, Key};
use super::playback::{Job, NowPlaying, Playback, QueueView, Queueing};
use super::playlist::{Edit, Modal};
use super::render;

/// Which pane has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    List,
}

/// The entries of the sidebar's BROWSE section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browse {
    AllTracks,
    Artists,
    Albums,
    DjSets,
}

impl Browse {
    /// The entries in the order the sidebar shows them.
    pub const ALL: [Self; 4] = [Self::AllTracks, Self::Artists, Self::Albums, Self::DjSets];

    pub fn label(self) -> &'static str {
        match self {
            Self::AllTracks => "All tracks",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::DjSets => "DJ Sets",
        }
    }

    pub fn view(self) -> View {
        match self {
            Self::AllTracks => View::Tracks(TrackFilter::default()),
            Self::Artists => View::Artists,
            Self::Albums => View::Albums(AlbumFilter::default()),
            Self::DjSets => View::Albums(AlbumFilter {
                kind: Some(AlbumKind::DjSet),
                ..AlbumFilter::default()
            }),
        }
    }
}

/// What a screen of the main list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The tracks that match the filter.
    Tracks(TrackFilter),
    /// Every artist.
    Artists,
    /// The albums that match the filter.
    Albums(AlbumFilter),
    /// The albums of an artist: those it is the album artist of, and those
    /// it has a track on.
    ArtistAlbums(ArtistId),
    /// The entries of a playlist.
    Playlist(PlaylistId),
}

/// One row of the main list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Artist(Artist),
    Album(Album),
    Track(Track),
    Entry(PlaylistEntry),
}

impl Item {
    /// The track the row is or holds.
    pub fn track(&self) -> Option<&Track> {
        match self {
            Self::Track(track) => Some(track),
            Self::Entry(entry) => Some(&entry.track),
            Self::Artist(_) | Self::Album(_) => None,
        }
    }
}

/// Data that the backend is still loading, has loaded, or failed to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    /// The backend's error, as text.
    Failed(String),
}

/// The selected row of a list, and the first row shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub selected: usize,
    pub offset: usize,
}

impl Cursor {
    /// Moves the selection to `to`, kept within `len` rows, and scrolls so
    /// that it stays within the `height` rows shown.
    pub(super) fn select(&mut self, to: usize, len: usize, height: usize) {
        self.selected = to.min(len.saturating_sub(1));
        self.offset = visible_offset(self.selected, self.offset, height);
    }
}

/// The first row to show so that `selected` is among the `height` rows
/// shown, scrolling as little as possible from `offset`.
pub fn visible_offset(selected: usize, offset: usize, height: usize) -> usize {
    if selected < offset {
        selected
    } else if height > 0 && selected >= offset + height {
        selected + 1 - height
    } else {
        offset
    }
}

/// Identifies a screen, so that a response finds the screen it was for.
pub type ScreenId = u64;

/// One level of the main list's drill-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub id: ScreenId,
    /// The name the header shows for this level.
    pub title: String,
    pub view: View,
    /// Identifies the latest load of the rows, so that only its response
    /// counts.
    pub load: u64,
    pub rows: Load<Vec<Item>>,
    pub cursor: Cursor,
    pub search: Option<Search>,
}

/// An incremental search over the library, started on a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    /// Whether keys go into the query.
    pub editing: bool,
    /// Identifies the latest query sent, so that only its results count.
    /// Unique across every search of the session, so that a response for a
    /// cleared search never matches a later one.
    pub seq: u64,
    pub rows: Load<Vec<Item>>,
    pub cursor: Cursor,
}

impl Search {
    /// Whether the query has a term to search for. Without one the screen
    /// shows its own list.
    pub fn is_active(&self) -> bool {
        has_terms(&self.query)
    }
}

/// Whether `query` has a search term: the backend splits queries at every
/// character that is not alphanumeric and matches nothing without a term.
fn has_terms(query: &str) -> bool {
    query.chars().any(char::is_alphanumeric)
}

/// What the user asked for, from the keymap or the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Loads the first screen and the playlists.
    Start,
    MoveDown,
    MoveUp,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
    ToggleFocus,
    FocusSidebar,
    FocusList,
    /// Opens the selected entry: a sidebar view, an artist or an album. On
    /// a track, plays it and queues the tracks after it in the list.
    Open,
    /// Clears the search, or goes back one level.
    Back,
    /// Starts a search, or edits the current one again.
    StartSearch,
    SearchInput(char),
    SearchDeleteChar,
    /// Stops editing the query and keeps the results.
    ConfirmSearch,
    ClearSearch,
    ToggleHelp,
    /// Asks for the name of a new playlist.
    NewPlaylist,
    /// Asks for a new name for the selected or open playlist.
    RenamePlaylist,
    /// Asks whether to delete the selected or open playlist.
    DeletePlaylist,
    /// Opens the picker of the playlist to add the selected track to.
    AddToPlaylist,
    /// Removes the selected entry from the open playlist.
    RemoveEntry,
    MoveEntryDown,
    MoveEntryUp,
    /// Types into the name prompt or the picker's filter.
    PromptInput(char),
    PromptDeleteChar,
    /// Accepts the prompt: the name, the deletion or the picked playlist.
    PromptConfirm,
    PromptCancel,
    /// Adds the selected track at the end of the queue.
    QueueTrack,
    /// Adds the selected track to play after the current one.
    PlayNext,
    /// Opens or closes the queue view.
    ToggleQueue,
    /// Pauses or resumes.
    TogglePause,
    NextTrack,
    PreviousTrack,
    /// Seeks this many seconds forward or back in the current track.
    Seek {
        forward: bool,
        seconds: u64,
    },
    VolumeUp,
    VolumeDown,
    Quit,
    /// The terminal has this many columns and rows.
    Resize {
        width: u16,
        height: u16,
    },
}

/// Data the state needs from the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Playlists {
        load: u64,
    },
    Screen {
        id: ScreenId,
        load: u64,
        view: View,
    },
    Search {
        screen: ScreenId,
        seq: u64,
        query: String,
    },
    Edit(Edit),
    /// Where the audio of these tracks is, for a playback job.
    Resolve {
        job: u64,
        tracks: Vec<Track>,
    },
    /// Records that the device played the track.
    RecordPlay {
        track: Track,
        device: String,
        at: SystemTime,
    },
}

impl Action {
    /// Whether the action drives the player, so that it acts at once rather
    /// than wait for a list to load.
    pub fn is_playback(self) -> bool {
        matches!(
            self,
            Self::ToggleQueue
                | Self::TogglePause
                | Self::NextTrack
                | Self::PreviousTrack
                | Self::Seek { .. }
                | Self::VolumeUp
                | Self::VolumeDown
        )
    }
}

/// What the backend returned for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    Playlists(Vec<Playlist>),
    Items(Vec<Item>),
    /// The playlist an edit created or renamed.
    Playlist(Playlist),
    /// An edit that returns nothing succeeded.
    Done,
    /// The audio source of each track of a [`Request::Resolve`], in order,
    /// or the backend's error.
    Sources(Vec<Result<AudioSource, String>>),
}

/// A message for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

/// A request and its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub request: Request,
    pub result: Result<Payload, String>,
}

/// The whole state of the TUI.
pub struct App {
    /// Names the library the TUI is connected to.
    pub library: String,
    pub focus: Focus,
    /// The selected sidebar entry: the BROWSE entries, then the playlists.
    pub sidebar: usize,
    pub playlists: Load<Vec<Playlist>>,
    /// Identifies the latest load of the playlists.
    pub(super) playlists_load: u64,
    /// The drill-down, from the screen of the sidebar entry that was opened.
    /// Never empty.
    pub stack: Vec<Screen>,
    /// The prompt that has the keyboard, if any.
    pub modal: Option<Modal>,
    /// The edits not yet done, in order. The first is running.
    pub(super) edits: VecDeque<Edit>,
    /// Keys typed to browse the main list while it was loading, in order.
    pub(super) held: VecDeque<Key>,
    /// The screen whose list the held keys wait for.
    held_for: ScreenId,
    /// The result of the last edit, until the next key.
    pub status: Option<Status>,
    pub help: bool,
    /// How many lines the help overlay is scrolled down.
    pub help_scroll: usize,
    /// How many lines of the help overlay fit on the terminal.
    pub(super) help_height: usize,
    /// The player, which the playback keys command.
    pub(super) player: Box<dyn Playback>,
    /// The name plays are recorded under: this machine's.
    pub(super) device: String,
    pub now: NowPlaying,
    /// The playback jobs not yet done, in order. The first is running.
    pub(super) jobs: VecDeque<Job>,
    /// How many plays are being recorded. Quitting waits for them.
    pub(super) recording: usize,
    /// The queue view, while it is open.
    pub queue_view: Option<QueueView>,
    /// Whether the TUI waits for the outstanding edits to quit.
    pub quitting: bool,
    pub quit: bool,
    /// The error of an edit or a play that failed while quitting, which the
    /// TUI returns.
    pub(super) quit_error: Option<String>,
    /// How many rows of the main list fit on the terminal.
    pub list_height: usize,
    next_screen: ScreenId,
    /// Numbers searches and loads. Unique across the session.
    next_seq: u64,
}

impl App {
    /// A TUI connected to the library named `library`, showing all tracks,
    /// that plays on `player` and records plays as `device`. Nothing is
    /// loaded until [`Action::Start`].
    pub fn new(
        library: impl Into<String>,
        player: Box<dyn Playback>,
        device: impl Into<String>,
    ) -> Self {
        let mut app = Self {
            library: library.into(),
            focus: Focus::List,
            sidebar: 0,
            playlists: Load::Loading,
            playlists_load: 0,
            stack: Vec::new(),
            modal: None,
            edits: VecDeque::new(),
            held: VecDeque::new(),
            held_for: 0,
            status: None,
            help: false,
            help_scroll: 0,
            help_height: 0,
            player,
            device: device.into(),
            now: NowPlaying::new(),
            jobs: VecDeque::new(),
            recording: 0,
            queue_view: None,
            quitting: false,
            quit: false,
            quit_error: None,
            list_height: 0,
            next_screen: 0,
            next_seq: 0,
        };
        let root = app.screen(Browse::AllTracks.label(), Browse::AllTracks.view());
        app.stack.push(root);
        app
    }

    /// The screen the main list shows.
    pub fn top(&self) -> &Screen {
        self.stack.last().expect("the stack is never empty")
    }

    /// The view of the sidebar entry the main list descends from.
    pub fn root_view(&self) -> View {
        self.stack[0].view
    }

    pub(super) fn top_mut(&mut self) -> &mut Screen {
        self.stack.last_mut().expect("the stack is never empty")
    }

    /// The search that is being typed, if any.
    pub fn editing_search(&self) -> Option<&Search> {
        self.top().search.as_ref().filter(|search| search.editing)
    }

    /// Whether the main list shows search results.
    pub fn searching(&self) -> bool {
        self.top().search.as_ref().is_some_and(Search::is_active)
    }

    /// The rows the main list shows: the search results while a search with
    /// a term is open, otherwise the screen's own rows.
    pub fn rows(&self) -> &Load<Vec<Item>> {
        let top = self.top();
        match &top.search {
            Some(search) if search.is_active() => &search.rows,
            _ => &top.rows,
        }
    }

    /// The main list's cursor, as [`rows`](Self::rows) chooses the list.
    pub fn cursor(&self) -> Cursor {
        let top = self.top();
        match &top.search {
            Some(search) if search.is_active() => search.cursor,
            _ => top.cursor,
        }
    }

    fn rows_and_cursor_mut(&mut self) -> (usize, &mut Cursor) {
        let top = self.top_mut();
        let (rows, cursor) = match &mut top.search {
            Some(search) if search.is_active() => (&search.rows, &mut search.cursor),
            _ => (&top.rows, &mut top.cursor),
        };
        let len = match rows {
            Load::Ready(items) => items.len(),
            Load::Loading | Load::Failed(_) => 0,
        };
        (len, cursor)
    }

    /// The selected row of the main list.
    pub fn selected_item(&self) -> Option<&Item> {
        match self.rows() {
            Load::Ready(items) => items.get(self.cursor().selected),
            Load::Loading | Load::Failed(_) => None,
        }
    }

    /// The header's path to the current list, such as
    /// `LIBRARY / ARTISTS / ARTIST A`.
    pub fn breadcrumb(&self) -> String {
        let mut path = String::from("LIBRARY");
        for screen in &self.stack {
            path.push_str(" / ");
            path.push_str(&screen.title.to_uppercase());
        }
        if self.top().search.is_some() {
            path.push_str(" / SEARCH");
        }
        path
    }

    /// How many entries the sidebar has: the BROWSE entries, then the
    /// playlists.
    pub fn sidebar_len(&self) -> usize {
        Browse::ALL.len() + self.playlist_count()
    }

    fn playlist_count(&self) -> usize {
        self.playlist_list().len()
    }

    /// The playlists, or none while they are not loaded.
    pub fn playlist_list(&self) -> &[Playlist] {
        match &self.playlists {
            Load::Ready(playlists) => playlists,
            Load::Loading | Load::Failed(_) => &[],
        }
    }

    /// A new number for a search or a load.
    pub(super) fn seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn screen(&mut self, title: impl Into<String>, view: View) -> Screen {
        let id = self.next_screen;
        self.next_screen += 1;
        Screen {
            id,
            title: title.into(),
            view,
            load: self.seq(),
            rows: Load::Loading,
            cursor: Cursor::default(),
            search: None,
        }
    }

    /// Puts a new screen on top of the stack and asks for its rows.
    pub(super) fn push(&mut self, title: impl Into<String>, view: View) -> Request {
        let screen = self.screen(title, view);
        let request = Request::Screen {
            id: screen.id,
            load: screen.load,
            view,
        };
        self.stack.push(screen);
        request
    }

    fn half_page(&self) -> usize {
        (self.list_height / 2).max(1)
    }

    /// Moves the focused pane's selection with `to`, which gets the current
    /// selection and the length of the list.
    fn move_selection(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        if self.help {
            let max = render::help_rows().saturating_sub(self.help_height);
            self.help_scroll = to(self.help_scroll, max + 1).min(max);
            return;
        }
        if let Some(Modal::Picker(picker)) = &self.modal {
            let len = picker.matches(self.playlist_list()).len();
            if let Some(Modal::Picker(picker)) = &mut self.modal {
                picker.selected = to(picker.selected, len).min(len.saturating_sub(1));
            }
            return;
        }
        if self.queue_view.is_some() {
            self.move_queue_selection(to);
            return;
        }
        match self.focus {
            Focus::Sidebar => {
                let len = self.sidebar_len();
                self.sidebar = to(self.sidebar, len).min(len.saturating_sub(1));
            }
            Focus::List => {
                let height = self.list_height;
                let (len, cursor) = self.rows_and_cursor_mut();
                let target = to(cursor.selected, len);
                cursor.select(target, len, height);
            }
        }
    }

    fn open(&mut self) -> Vec<Request> {
        match self.focus {
            Focus::Sidebar => {
                let (title, view) = match Browse::ALL.get(self.sidebar) {
                    Some(browse) => (browse.label().to_owned(), browse.view()),
                    None => match self.sidebar_playlist() {
                        Some(playlist) => (playlist.name.clone(), View::Playlist(playlist.id)),
                        None => return Vec::new(),
                    },
                };
                self.stack.clear();
                let request = self.push(title, view);
                self.focus = Focus::List;
                vec![request]
            }
            Focus::List => {
                let request = match self.selected_item() {
                    Some(Item::Artist(artist)) => {
                        let (title, view) = (artist.name.clone(), View::ArtistAlbums(artist.id));
                        self.push(title, view)
                    }
                    Some(Item::Album(album)) => {
                        let title = album.title.clone();
                        let view = View::Tracks(TrackFilter {
                            album: Some(album.id),
                            ..TrackFilter::default()
                        });
                        self.push(title, view)
                    }
                    Some(Item::Track(_) | Item::Entry(_)) => return self.play_from_selected(),
                    None => return Vec::new(),
                };
                let parent = self.stack.len() - 2;
                if let Some(search) = &mut self.stack[parent].search {
                    search.editing = false;
                }
                vec![request]
            }
        }
    }

    fn back(&mut self) {
        if self.top_mut().search.take().is_none() && self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn start_search(&mut self) {
        self.focus = Focus::List;
        let top = self.top_mut();
        match &mut top.search {
            Some(search) => search.editing = true,
            None => {
                top.search = Some(Search {
                    query: String::new(),
                    editing: true,
                    seq: 0,
                    rows: Load::Loading,
                    cursor: Cursor::default(),
                });
            }
        }
    }

    /// Changes the query being typed and asks for its results.
    fn edit_query(&mut self, edit: impl FnOnce(&mut String)) -> Vec<Request> {
        let seq = self.seq();
        let top = self.top_mut();
        let screen = top.id;
        let Some(search) = top.search.as_mut().filter(|search| search.editing) else {
            return Vec::new();
        };
        edit(&mut search.query);
        search.seq = seq;
        if !search.is_active() {
            search.rows = Load::Loading;
            search.cursor = Cursor::default();
            return Vec::new();
        }
        vec![Request::Search {
            screen,
            seq: search.seq,
            query: search.query.clone(),
        }]
    }

    pub(super) fn confirm_search(&mut self) {
        let top = self.top_mut();
        match &mut top.search {
            Some(search) if search.is_active() => search.editing = false,
            Some(_) => top.search = None,
            None => {}
        }
    }

    /// Applies a key in the current context, or holds it while the main list
    /// it would browse is loading, or while other keys are held. While
    /// quitting, only quit acts, so that no edit joins the queue.
    pub(super) fn press(&mut self, key: Key) -> Vec<Request> {
        let context = Context::of(self);
        let action = keymap::action_for_key(context, key);
        if self.quitting {
            return match action {
                Some(Action::Quit) => update(self, Action::Quit),
                _ => Vec::new(),
            };
        }
        let hold = !self.held.is_empty()
            || (context == Context::Browse && matches!(self.rows(), Load::Loading));
        match action {
            Some(Action::Back) => {
                self.held.clear();
                update(self, Action::Back)
            }
            Some(Action::Quit) => update(self, Action::Quit),
            Some(action) if action.is_playback() => update(self, action),
            _ if hold => {
                if self.held.is_empty() {
                    self.held_for = self.top().id;
                }
                self.held.push_back(key);
                Vec::new()
            }
            Some(action) => update(self, action),
            None => Vec::new(),
        }
    }

    /// Replays the held keys once the list they wait for has loaded or
    /// failed, and drops them once its screen is gone. A replayed key that
    /// opens another loading screen holds the keys after it for that screen.
    fn replay(&mut self) -> Vec<Request> {
        if self.held.is_empty() {
            return Vec::new();
        }
        if self.top().id != self.held_for {
            self.held.clear();
            return Vec::new();
        }
        if matches!(self.rows(), Load::Loading) {
            return Vec::new();
        }
        let keys = std::mem::take(&mut self.held);
        keys.into_iter().flat_map(|key| self.press(key)).collect()
    }
}

/// Applies a terminal event: a key through the keymap, held while the main
/// list loads, or a resize, at once.
pub fn handle_event(app: &mut App, event: &Event) -> Vec<Request> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => app.press(Key::from_event(key)),
        Event::Resize(width, height) => update(
            app,
            Action::Resize {
                width: *width,
                height: *height,
            },
        ),
        _ => Vec::new(),
    }
}

/// Applies an action to the state and returns the data it needs loaded.
pub fn update(app: &mut App, action: Action) -> Vec<Request> {
    let half_page = app.half_page();
    if !matches!(action, Action::Start | Action::Resize { .. }) {
        app.status = None;
    }
    match action {
        Action::Start => {
            app.refresh();
            let top = app.top();
            return vec![
                Request::Playlists {
                    load: app.playlists_load,
                },
                Request::Screen {
                    id: top.id,
                    load: top.load,
                    view: top.view,
                },
            ];
        }
        Action::MoveDown => app.move_selection(|at, _| at.saturating_add(1)),
        Action::MoveUp => app.move_selection(|at, _| at.saturating_sub(1)),
        Action::Top => app.move_selection(|_, _| 0),
        Action::Bottom => app.move_selection(|_, len| len.saturating_sub(1)),
        Action::HalfPageDown => app.move_selection(|at, _| at.saturating_add(half_page)),
        Action::HalfPageUp => app.move_selection(|at, _| at.saturating_sub(half_page)),
        Action::ToggleFocus => {
            app.focus = match app.focus {
                Focus::Sidebar => Focus::List,
                Focus::List => Focus::Sidebar,
            };
        }
        Action::FocusSidebar => app.focus = Focus::Sidebar,
        Action::FocusList => app.focus = Focus::List,
        Action::Open => return app.open(),
        Action::Back => app.back(),
        Action::StartSearch => app.start_search(),
        Action::SearchInput(c) => return app.edit_query(|query| query.push(c)),
        Action::SearchDeleteChar => {
            return app.edit_query(|query| {
                query.pop();
            });
        }
        Action::ConfirmSearch => app.confirm_search(),
        Action::ClearSearch => app.top_mut().search = None,
        Action::ToggleHelp => {
            app.help = !app.help;
            app.help_scroll = 0;
        }
        Action::NewPlaylist => app.new_playlist(),
        Action::RenamePlaylist => app.rename_playlist(),
        Action::DeletePlaylist => app.delete_playlist(),
        Action::AddToPlaylist => app.pick_playlist(),
        Action::RemoveEntry if app.queue_view.is_some() => app.remove_queued(),
        Action::MoveEntryDown if app.queue_view.is_some() => app.move_queued(true),
        Action::MoveEntryUp if app.queue_view.is_some() => app.move_queued(false),
        Action::RemoveEntry => return app.remove_entry(),
        Action::MoveEntryDown => return app.move_entry(true),
        Action::MoveEntryUp => return app.move_entry(false),
        Action::PromptInput(c) => app.edit_prompt(|text| text.push(c)),
        Action::PromptDeleteChar => app.edit_prompt(|text| {
            text.pop();
        }),
        Action::PromptConfirm => return app.confirm_prompt(),
        Action::PromptCancel => app.modal = None,
        Action::QueueTrack => return app.queue_selected(Queueing::Append),
        Action::PlayNext => return app.queue_selected(Queueing::Next),
        Action::ToggleQueue => app.toggle_queue(),
        Action::TogglePause => app.toggle_pause(),
        Action::NextTrack => app.skip(true),
        Action::PreviousTrack => app.skip(false),
        Action::Seek { forward, seconds } => {
            app.seek_by(Duration::from_secs(seconds), forward);
        }
        Action::VolumeUp => app.change_volume(true),
        Action::VolumeDown => app.change_volume(false),
        Action::Quit => app.quit_after_edits(),
        Action::Resize { width, height } => {
            app.list_height = render::list_height(width, height);
            app.help_height = render::help_height(width, height);
            let max = render::help_rows().saturating_sub(app.help_height);
            app.help_scroll = app.help_scroll.min(max);
            let height = app.list_height;
            for screen in &mut app.stack {
                screen.cursor.offset = visible_offset(screen.cursor.selected, 0, height);
                if let Some(search) = &mut screen.search {
                    search.cursor.offset = visible_offset(search.cursor.selected, 0, height);
                }
            }
        }
    }
    Vec::new()
}

/// Stores what the backend returned, unless its screen, search or load is
/// gone, replays the keys held for the list it loaded, and returns the data
/// that the result makes stale and the replayed keys need.
pub fn receive(app: &mut App, response: Response) -> Vec<Request> {
    let mut requests = store(app, response);
    requests.extend(app.replay());
    requests
}

/// Stores what the backend returned, unless its screen, search or load is
/// gone, and returns the data that the result makes stale.
fn store(app: &mut App, response: Response) -> Vec<Request> {
    let Response { request, result } = response;
    let to_rows = |result: Result<Payload, String>| match result {
        Ok(Payload::Items(items)) => Load::Ready(items),
        Ok(_) => Load::Failed("unexpected response".to_owned()),
        Err(error) => Load::Failed(error),
    };
    match request {
        Request::Playlists { load } => {
            // While edits are outstanding the playlists may be about to
            // change; they load again after the last edit.
            if load != app.playlists_load || !app.edits.is_empty() {
                return Vec::new();
            }
            app.playlists = match result {
                Ok(Payload::Playlists(playlists)) => Load::Ready(playlists),
                Ok(_) => Load::Failed("unexpected response".to_owned()),
                Err(error) => Load::Failed(error),
            };
            app.sidebar = app.sidebar.min(app.sidebar_len() - 1);
        }
        Request::Screen { id, load, view } => {
            if matches!(view, View::Playlist(_)) && !app.edits.is_empty() {
                return Vec::new();
            }
            let height = app.list_height;
            let screen = app
                .stack
                .iter_mut()
                .find(|screen| screen.id == id && screen.load == load);
            if let Some(screen) = screen {
                // A reload keeps the selection where it was.
                let reload = matches!(screen.rows, Load::Ready(_));
                screen.rows = to_rows(result);
                if reload {
                    let len = match &screen.rows {
                        Load::Ready(items) => items.len(),
                        Load::Loading | Load::Failed(_) => 0,
                    };
                    let selected = screen.cursor.selected;
                    screen.cursor.select(selected, len, height);
                } else {
                    screen.cursor = Cursor::default();
                }
            }
        }
        Request::Search { screen, seq, .. } => {
            let search = app
                .stack
                .iter_mut()
                .find(|candidate| candidate.id == screen)
                .and_then(|screen| screen.search.as_mut())
                .filter(|search| search.seq == seq);
            if let Some(search) = search {
                search.rows = to_rows(result);
                search.cursor = Cursor::default();
            }
        }
        Request::Edit(edit) => return app.finish_edit(&edit, result),
        Request::Resolve { job, tracks } => return app.finish_resolve(job, tracks, result),
        Request::RecordPlay { track, .. } => {
            app.recording = app.recording.saturating_sub(1);
            if let Err(error) = result {
                let text = format!("Could not record the play of “{}”: {error}", track.title);
                if app.quitting {
                    app.quit_error = Some(text.clone());
                }
                app.status = Some(Status { text, error: true });
            }
            app.quit_when_saved();
        }
    }
    Vec::new()
}
