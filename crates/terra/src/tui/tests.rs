//! Tests of the TUI against the synthetic sample library, without a
//! terminal: keys go through the keymap and `update`, requests run on
//! `block_on`, and the screen renders to Ratatui's `TestBackend`. The player
//! is a fake that records its commands, unless a test gives another.

mod playback;

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use terra_core::testing::{MemoryLibrary, block_on};
use terra_core::{
    Album, AlbumId, AlbumKind, AlbumRef, LibraryBackend, Page, Track, TrackFilter, TrackId,
};

use super::app::{
    Action, App, Focus, Item, Load, Request, Response, View, handle_event, receive, update,
};
use super::fetch::fetch;
use super::keymap::{self, Context, KEYMAP};
use super::playback::Playback;
use super::playlist::Modal;
use super::render::{format_duration, help_line, render};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

/// The device name plays are recorded under in the tests.
const DEVICE: &str = "test-host";

/// A TUI on an 80×24 terminal over a library, with every request answered
/// before the next key.
struct Harness {
    app: App,
    library: MemoryLibrary,
    /// The fake player the TUI commands, unless it was given another.
    player: playback::FakePlayer,
}

impl Harness {
    fn new(library: MemoryLibrary) -> Self {
        let player = playback::FakePlayer::default();
        Self::with_player(library, Box::new(player.clone()), player)
    }

    fn with_player(
        library: MemoryLibrary,
        playback: Box<dyn Playback>,
        player: playback::FakePlayer,
    ) -> Self {
        let mut harness = Self {
            app: App::new("test library", playback, DEVICE),
            library,
            player,
        };
        harness.dispatch(Action::Resize {
            width: WIDTH,
            height: HEIGHT,
        });
        harness.dispatch(Action::Start);
        harness
    }

    fn sample() -> Self {
        Self::new(MemoryLibrary::sample())
    }

    fn dispatch(&mut self, action: Action) {
        let requests = update(&mut self.app, action);
        self.answer(requests);
    }

    /// Runs the requests, and those their responses lead to, in order.
    fn answer(&mut self, mut requests: Vec<Request>) {
        while !requests.is_empty() {
            let mut next = Vec::new();
            for request in requests {
                let response = block_on(fetch(&self.library, request));
                next.extend(receive(&mut self.app, response));
            }
            requests = next;
        }
    }

    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let requests = self.key_unanswered(code, modifiers);
        self.answer(requests);
    }

    fn press(&mut self, code: KeyCode) {
        self.key(code, KeyModifiers::NONE);
    }

    /// Presses a key and returns its requests without running them.
    fn key_unanswered(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Vec<Request> {
        let event = Event::Key(KeyEvent::new(code, modifiers));
        handle_event(&mut self.app, &event)
    }

    fn press_unanswered(&mut self, code: KeyCode) -> Vec<Request> {
        self.key_unanswered(code, KeyModifiers::NONE)
    }

    /// Types each character as a key, with Shift for capitals as a terminal
    /// reports them.
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let modifiers = if c.is_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            self.key(KeyCode::Char(c), modifiers);
        }
    }

    /// The names of the main list's rows.
    fn names(&self) -> Vec<String> {
        let Load::Ready(items) = self.app.rows() else {
            panic!("the list is not loaded: {:?}", self.app.rows());
        };
        items.iter().map(name).collect()
    }

    fn selected(&self) -> String {
        name(self.app.selected_item().expect("a selected row"))
    }

    fn buffer(&self) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal.draw(|frame| render(&self.app, frame)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn screen(&self) -> Vec<String> {
        lines(&self.buffer())
    }
}

fn name(item: &Item) -> String {
    match item {
        Item::Artist(artist) => artist.name.clone(),
        Item::Album(album) => album.title.clone(),
        Item::Track(track) => track.title.clone(),
        Item::Entry(entry) => entry.track.title.clone(),
    }
}

fn lines(buffer: &Buffer) -> Vec<String> {
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

/// The row of the buffer that contains `text`.
fn row_of(screen: &[String], text: &str) -> usize {
    screen
        .iter()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{text:?} is not on the screen:\n{}", screen.join("\n")))
}

#[test]
fn first_screen_shows_the_sidebar_and_every_sample_track() {
    let harness = Harness::sample();
    let expected = [
        "┌──────────────────────────────────────────────────────────────────────────────┐",
        "│◉ terra  test library                                     LIBRARY / ALL TRACKS│",
        "└──────────────────────────────────────────────────────────────────────────────┘",
        "┌────────────────────┐┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓",
        "│BROWSE              │┃  TITLE / ARTIST                                    TIME┃",
        "│> All tracks        │┃> Sunrise / Artist D                               06:14┃",
        "│  Artists           │┃  Last Light / Artist E                            05:06┃",
        "│  Albums            │┃  After Hours / Artist B                           06:18┃",
        "│  DJ Sets           │┃  Into the Deep / Artist C                         04:51┃",
        "│                    │┃  Midnight Drive / Artist A                        05:42┃",
        "│PLAYLISTS           │┃  Demo Tape / Artist E                             03:33┃",
        "│  No playlists      │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "│                    │┃                                                        ┃",
        "└────────────────────┘┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛",
        "┌──────────────────────────────────────────────────────────────────────────────┐",
        "│■ Nothing playing                                                             │",
        "│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│",
        "│Tab focus  Enter open  Backspace back  / search  a add  ? help  q quit        │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];
    assert_eq!(harness.screen(), expected);
}

#[test]
fn list_focus_is_visible() {
    let mut harness = Harness::sample();
    let buffer = harness.buffer();
    let screen = lines(&buffer);
    let list_row = u16::try_from(row_of(&screen, "> Sunrise")).unwrap();
    // The focused list has a thick accent border and a reversed selection;
    // the sidebar's selection is only bold.
    assert_eq!(buffer[(22, 3)].symbol(), "┏");
    assert_eq!(buffer[(22, 3)].fg, Color::Cyan);
    assert_eq!(buffer[(0, 3)].fg, Color::DarkGray);
    assert!(buffer[(24, list_row)].modifier.contains(Modifier::REVERSED));
    assert!(!buffer[(3, 5)].modifier.contains(Modifier::REVERSED));

    harness.press(KeyCode::Tab);
    assert_eq!(harness.app.focus, Focus::Sidebar);
    let buffer = harness.buffer();
    assert_eq!(buffer[(0, 3)].symbol(), "┏");
    assert_eq!(buffer[(0, 3)].fg, Color::Cyan);
    assert_eq!(buffer[(22, 3)].fg, Color::DarkGray);
    assert!(buffer[(3, 5)].modifier.contains(Modifier::REVERSED));
    assert!(!buffer[(24, list_row)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn keys_reach_artist_album_tracks_and_back() {
    let mut harness = Harness::sample();

    // Sidebar → Artists.
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.focus, Focus::List);
    assert_eq!(
        harness.names(),
        [
            "The Aardvarks",
            "Artist A",
            "Artist B",
            "Artist C",
            "Artist D",
            "Artist E",
            "Café Ensemble",
            "Various Artists"
        ]
    );

    // Artist A → its album.
    harness.type_text("j");
    assert_eq!(harness.selected(), "Artist A");
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ARTISTS / ARTIST A");
    assert_eq!(harness.names(), ["Nocturnes"]);

    // The album → its tracks.
    harness.press(KeyCode::Enter);
    assert_eq!(
        harness.app.breadcrumb(),
        "LIBRARY / ARTISTS / ARTIST A / NOCTURNES"
    );
    assert_eq!(harness.names(), ["Midnight Drive"]);
    assert!(harness.screen()[5].contains("> Midnight Drive / Artist A"));

    // Back, with Backspace and Esc, to the artists, where Artist A is still
    // selected.
    harness.press(KeyCode::Backspace);
    assert_eq!(harness.names(), ["Nocturnes"]);
    harness.press(KeyCode::Esc);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ARTISTS");
    assert_eq!(harness.selected(), "Artist A");

    // Back at the top level stays there.
    harness.press(KeyCode::Esc);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ARTISTS");
}

#[test]
fn an_artist_lists_the_albums_it_appears_on() {
    let mut harness = Harness::sample();
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
    // Artist B has a track on the compilation "Late Shift", whose album
    // artist is "Various Artists".
    harness.type_text("jj");
    assert_eq!(harness.selected(), "Artist B");
    harness.press(KeyCode::Enter);
    assert_eq!(harness.names(), ["Late Shift"]);
    harness.press(KeyCode::Enter);
    assert_eq!(harness.names(), ["After Hours", "Into the Deep"]);
}

#[test]
fn search_narrows_the_list_and_esc_restores_it() {
    let mut harness = Harness::sample();
    let all = harness.names();
    assert_eq!(all.len(), 6);

    harness.type_text("/mid");
    assert_eq!(harness.names(), ["Midnight Drive"]);
    let screen = harness.screen();
    assert!(screen[5].contains("> Midnight Drive / Artist A"));
    assert!(!screen.iter().any(|line| line.contains("Sunrise")));
    assert!(screen[22].contains("/mid█"));

    harness.press(KeyCode::Esc);
    assert!(harness.app.top().search.is_none());
    assert_eq!(harness.names(), all);
    assert!(harness.screen()[22].starts_with("│Tab focus"));
}

#[test]
fn search_keys_are_text_while_typing() {
    let mut harness = Harness::sample();
    // `q`, `j` and `G` go into the query instead of quitting or moving.
    harness.type_text("/qjG");
    assert!(!harness.app.quit);
    assert_eq!(harness.app.editing_search().unwrap().query, "qjG");

    // Deleting every character shows the whole list again.
    for _ in 0..3 {
        harness.press(KeyCode::Backspace);
    }
    assert_eq!(harness.names().len(), 6);
    harness.type_text("artist");
    // Every artist named "Artist …", the albums by them and every track.
    assert_eq!(harness.names().len(), 6 + 3 + 6);
}

#[test]
fn search_results_open_and_back_returns_to_them() {
    let mut harness = Harness::sample();
    harness.type_text("/artist a");
    assert_eq!(harness.selected(), "Artist A");
    harness.press(KeyCode::Enter);
    // Enter kept the results; now it opens the artist.
    assert!(harness.app.editing_search().is_none());
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALL TRACKS / ARTIST A");
    assert_eq!(harness.names(), ["Nocturnes"]);

    harness.press(KeyCode::Backspace);
    assert_eq!(harness.selected(), "Artist A");
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALL TRACKS / SEARCH");
    // `/` edits the same query again.
    harness.type_text("/");
    assert_eq!(harness.app.editing_search().unwrap().query, "artist a");
    harness.press(KeyCode::Esc);
    assert_eq!(harness.names().len(), 6);
}

#[test]
fn a_late_search_response_does_not_replace_a_newer_one() {
    let mut harness = Harness::sample();
    harness.type_text("/");
    let first = update(&mut harness.app, Action::SearchInput('a'));
    let second = update(&mut harness.app, Action::SearchInput('f'));
    let [first] = <[Request; 1]>::try_from(first).unwrap();
    let [second] = <[Request; 1]>::try_from(second).unwrap();

    let newer = block_on(fetch(&harness.library, second));
    receive(&mut harness.app, newer);
    assert_eq!(harness.names(), ["After Hours"]);
    let older = block_on(fetch(&harness.library, first));
    receive(&mut harness.app, older);
    assert_eq!(harness.names(), ["After Hours"]);
}

#[test]
fn a_response_for_a_closed_screen_is_dropped() {
    let mut harness = Harness::sample();
    // Open Artists, but leave its request unanswered.
    harness.type_text("hj");
    let stale = update(&mut harness.app, Action::Open);
    // Open Albums instead. Keys would wait for Artists to load, so the
    // actions go to `update` directly.
    for action in [Action::FocusSidebar, Action::MoveDown, Action::Open] {
        harness.dispatch(action);
    }
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALBUMS");
    for request in stale {
        let response = block_on(fetch(&harness.library, request));
        receive(&mut harness.app, response);
    }
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALBUMS");
    assert_eq!(
        harness.names(),
        ["Daybreak Session", "Late Shift", "Nocturnes"]
    );
}

#[test]
fn a_backend_error_is_shown_in_the_list() {
    let mut harness = Harness::sample();
    let top = harness.app.top();
    let (id, load, view) = (top.id, top.load, top.view);
    receive(
        &mut harness.app,
        Response {
            request: Request::Screen { id, load, view },
            result: Err("disk unplugged".to_owned()),
        },
    );
    let screen = harness.screen();
    assert!(screen[5].contains("Could not load the list: disk unplugged"));
}

#[test]
fn dj_sets_lists_the_dj_set_albums() {
    let mut harness = Harness::sample();
    harness.type_text("hjjj");
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / DJ SETS");
    assert_eq!(harness.names(), ["Daybreak Session"]);
    let screen = harness.screen();
    assert!(screen[5].contains("> Daybreak Session / Artist D"));
    assert!(screen[5].contains("2023"));
}

#[test]
fn playlists_are_listed_in_the_sidebar() {
    let library = MemoryLibrary::sample();
    for name in ["Favorites", "Driving", "Discoveries"] {
        block_on(library.create_playlist(name)).unwrap();
    }
    let mut harness = Harness::new(library);
    let screen = harness.screen();
    for (row, name) in [(11, "Discoveries"), (12, "Driving"), (13, "Favorites")] {
        assert!(
            screen[row].starts_with(&format!("│  {name}")),
            "{screen:#?}"
        );
    }

    // The sidebar reaches the playlists, and opening one shows it.
    harness.type_text("hG");
    assert_eq!(harness.app.sidebar, 6);
    assert!(harness.screen()[13].starts_with("┃> Favorites"));
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.focus, Focus::List);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / FAVORITES");
    assert!(harness.screen()[5].contains("Nothing here yet"));
}

/// A library with `count` tracks, numbered from 1, so the list scrolls.
fn long_library(count: i64) -> MemoryLibrary {
    let mut library = MemoryLibrary::sample();
    let (template, source) = terra_core::testing::sample::library().tracks.remove(0);
    for number in 1..=count {
        let track = Track {
            id: TrackId::new(100 + number),
            title: format!("Track {number:03}"),
            album: None,
            disc_no: None,
            track_no: None,
            duration: Duration::from_secs(60),
            ..template.clone()
        };
        library.insert_track(track, source.clone());
    }
    library
}

#[test]
fn only_the_visible_rows_are_drawn_and_the_cursor_stays_visible() {
    let mut harness = Harness::new(long_library(100));
    // 24 rows less the header, the bottom bar, the borders and the column
    // header leave 13 rows for the list.
    assert_eq!(harness.app.list_height, 13);
    assert_eq!(harness.names().len(), 106);

    harness.type_text("G");
    assert_eq!(harness.selected(), "Track 100");
    let screen = harness.screen();
    assert!(screen[17].contains("> Track 100"));
    assert!(screen[5].contains("  Track 088"));
    assert!(!screen.iter().any(|line| line.contains("Sunrise")));

    harness.type_text("g");
    assert_eq!(harness.selected(), "Sunrise");
    harness.key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(harness.app.cursor().selected, 6);
    harness.key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    harness.key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(harness.app.cursor().selected, 18);
    assert!(harness.screen()[17].contains("> "));
    harness.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(harness.app.cursor().selected, 12);
    harness.press(KeyCode::Up);
    harness.press(KeyCode::Down);
    harness.press(KeyCode::Down);
    assert_eq!(harness.app.cursor().selected, 13);
}

#[test]
fn moves_stop_at_the_ends_of_the_list() {
    let mut harness = Harness::sample();
    harness.type_text("k");
    assert_eq!(harness.selected(), "Sunrise");
    harness.type_text("jjjjjjjjjj");
    assert_eq!(harness.selected(), "Demo Tape");
}

#[test]
fn help_overlay_lists_every_keymap_entry() {
    let mut harness = Harness::sample();
    harness.type_text("?");
    assert!(harness.app.help);
    // The overlay is taller than the terminal: `j` scrolls it to its end.
    let mut screen = harness.screen();
    loop {
        harness.type_text("j");
        let next = harness.screen();
        if next.iter().all(|row| screen.contains(row)) {
            break;
        }
        screen.extend(next);
    }
    assert!(harness.app.help_scroll > 0);
    for binding in KEYMAP {
        let line = help_line(binding);
        assert!(
            screen.iter().any(|row| row.contains(&line)),
            "{line:?} is not in the help overlay:\n{}",
            screen.join("\n")
        );
    }
    for context in Context::ALL {
        row_of(&screen, context.title());
    }
    harness.type_text("g");
    let scrolled = harness.app.help_scroll;
    harness.type_text("kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk");
    assert_eq!(harness.app.help_scroll, 0);
    assert!(scrolled > 0);

    // Keys other than the help bindings do nothing while it is open, and
    // `q` closes it rather than quitting.
    harness.type_text("j/");
    assert!(harness.app.help);
    assert_eq!(harness.selected(), "Sunrise");
    assert!(harness.app.top().search.is_none());
    harness.type_text("q");
    assert!(!harness.app.help);
    assert!(!harness.app.quit);
}

#[test]
fn no_key_has_two_bindings_in_one_context() {
    for (index, binding) in KEYMAP.iter().enumerate() {
        for other in &KEYMAP[index + 1..] {
            if binding.context == other.context {
                for key in binding.keys {
                    assert!(
                        !other.keys.contains(key),
                        "{} is bound twice in {:?}",
                        key.name(),
                        binding.context
                    );
                }
            }
        }
    }
}

#[test]
fn playback_keys_are_not_shadowed_where_they_apply() {
    let playback = KEYMAP
        .iter()
        .filter(|binding| binding.context == Context::Playback);
    for binding in playback {
        for key in binding.keys {
            for context in [Context::Browse, Context::Queue] {
                assert_eq!(
                    keymap::action_for_key(context, *key),
                    Some(binding.action),
                    "{} does not reach the player in {context:?}",
                    key.name()
                );
            }
            assert_eq!(keymap::action_for_key(Context::Search, *key), {
                let KeyCode::Char(c) = key.code else {
                    unreachable!()
                };
                Some(Action::SearchInput(c))
            });
        }
    }
}

#[test]
fn q_quits() {
    let mut harness = Harness::sample();
    harness.type_text("q");
    assert!(harness.app.quit);
}

#[test]
fn durations_format_as_minutes_or_hours() {
    assert_eq!(format_duration(Duration::from_secs(342)), "05:42");
    assert_eq!(format_duration(Duration::from_secs(0)), "00:00");
    assert_eq!(format_duration(Duration::from_secs(3600 + 62)), "1:01:02");
}

#[test]
fn a_long_breadcrumb_keeps_the_library_name_and_its_own_end() {
    let mut harness = Harness::sample();
    harness.app.library = "demo library (synthetic sample)".to_owned();
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
    harness.type_text("j");
    harness.press(KeyCode::Enter);
    harness.press(KeyCode::Enter);
    let header = &harness.screen()[1];
    assert!(
        header.starts_with("│◉ terra  demo library (synthetic sample)  …"),
        "{header}"
    );
    assert!(header.ends_with("/ ARTIST A / NOCTURNES│"), "{header}");
}

#[test]
fn a_response_for_a_cleared_search_does_not_replace_a_new_one() {
    let mut harness = Harness::sample();
    // Search for "a", clear it before its results arrive, then search for
    // "m" on the same screen.
    harness.type_text("/");
    let abandoned = update(&mut harness.app, Action::SearchInput('a'));
    harness.press(KeyCode::Esc);
    harness.type_text("/");
    let current = update(&mut harness.app, Action::SearchInput('m'));

    for request in current.into_iter().chain(abandoned) {
        let response = block_on(fetch(&harness.library, request));
        receive(&mut harness.app, response);
    }
    assert_eq!(harness.app.editing_search().unwrap().query, "m");
    assert_eq!(harness.names(), ["Midnight Drive"]);
}

/// The sample library with two playlists: "Driving", holding Midnight Drive,
/// Sunrise and Last Light in that order, and "Favorites", empty. The sidebar
/// shows them at 4 and 5.
fn playlist_library() -> MemoryLibrary {
    let library = MemoryLibrary::sample();
    let tracks = block_on(library.list_tracks(TrackFilter::default(), Page::ALL)).unwrap();
    let id = |title: &str| tracks.iter().find(|track| track.title == title).unwrap().id;
    let driving = block_on(library.create_playlist("Driving")).unwrap();
    let entries = [id("Midnight Drive"), id("Sunrise"), id("Last Light")];
    block_on(library.add_to_playlist(driving.id, &entries, None)).unwrap();
    block_on(library.create_playlist("Favorites")).unwrap();
    library
}

impl Harness {
    fn playlists() -> Self {
        Self::new(playlist_library())
    }

    /// Opens the playlist at `index` of the sidebar's playlists.
    fn open_playlist(&mut self, index: usize) {
        self.type_text("hg");
        for _ in 0..4 + index {
            self.type_text("j");
        }
        self.press(KeyCode::Enter);
    }

    /// The playlists in the library, by name, with their entries' titles.
    fn stored(&self) -> Vec<(String, Vec<String>)> {
        let playlists = block_on(self.library.list_playlists()).unwrap();
        playlists
            .into_iter()
            .map(|playlist| {
                let contents = block_on(self.library.get_playlist(playlist.id)).unwrap();
                let titles = contents.entries.into_iter().map(|entry| entry.track.title);
                (playlist.name, titles.collect())
            })
            .collect()
    }

    /// The bottom bar's line of hints, prompts and messages.
    fn status_line(&self) -> String {
        self.screen()[22].clone()
    }
}

fn stored(entries: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    entries
        .iter()
        .map(|(name, titles)| {
            let titles = titles.iter().map(|title| (*title).to_owned()).collect();
            ((*name).to_owned(), titles)
        })
        .collect()
}

#[test]
fn selecting_a_playlist_opens_its_entries() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    assert_eq!(harness.app.focus, Focus::List);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / DRIVING");
    assert_eq!(harness.names(), ["Midnight Drive", "Sunrise", "Last Light"]);
    let screen = harness.screen();
    assert!(screen[1].ends_with("LIBRARY / DRIVING│"));
    assert!(screen[4].contains("TITLE / ARTIST"));
    assert!(screen[5].contains("> Midnight Drive / Artist A"));
    assert!(screen[5].contains("05:42"));
    // The open playlist is the sidebar's active entry, and All tracks no
    // longer is.
    let buffer = harness.buffer();
    let driving = u16::try_from(row_of(&screen, "│> Driving")).unwrap();
    assert_eq!(buffer[(4, driving)].fg, Color::Cyan);
    assert_ne!(buffer[(4, 5)].fg, Color::Cyan);
}

#[test]
fn n_creates_a_playlist_through_a_name_prompt() {
    let mut harness = Harness::playlists();
    harness.type_text("n");
    assert!(harness.status_line().starts_with("│New playlist: █"));
    assert!(harness.status_line().contains("Enter ok  Esc cancel│"));
    // Keys are text in the prompt: `q` doesn't quit and `j` doesn't move.
    harness.type_text("Road Trip qj");
    harness.press(KeyCode::Backspace);
    harness.press(KeyCode::Backspace);
    harness.press(KeyCode::Backspace);
    assert!(
        harness
            .status_line()
            .starts_with("│New playlist: Road Trip█")
    );
    harness.press(KeyCode::Enter);

    assert!(harness.app.modal.is_none());
    assert!(!harness.app.quit);
    assert!(harness.status_line().starts_with("│Created “Road Trip”  "));
    assert_eq!(
        harness.stored(),
        stored(&[
            ("Driving", &["Midnight Drive", "Sunrise", "Last Light"]),
            ("Favorites", &[]),
            ("Road Trip", &[])
        ])
    );
    assert!(harness.screen()[13].starts_with("│  Road Trip"));
    // The message goes with the next key.
    harness.type_text("j");
    assert!(harness.status_line().starts_with("│Tab focus"));
}

#[test]
fn a_duplicate_name_shows_the_error_in_the_status_line() {
    let mut harness = Harness::playlists();
    harness.type_text("nfavorites");
    harness.press(KeyCode::Enter);
    let line = harness.status_line();
    assert!(
        line.starts_with(r#"│invalid input: a playlist named "favorites" already exists"#),
        "{line}"
    );
    let buffer = harness.buffer();
    assert_eq!(buffer[(1, 22)].fg, Color::Red);
    assert_eq!(harness.stored().len(), 2);
}

#[test]
fn esc_cancels_the_name_prompt() {
    let mut harness = Harness::playlists();
    harness.type_text("nNever");
    let requests = update(&mut harness.app, Action::PromptCancel);
    assert!(requests.is_empty());
    assert!(harness.app.modal.is_none());
    harness.type_text("n");
    harness.press(KeyCode::Esc);
    assert!(harness.app.modal.is_none());
    assert!(harness.status_line().starts_with("│Tab focus"));
    assert_eq!(harness.stored().len(), 2);
}

#[test]
fn r_renames_the_selected_playlist_and_its_open_view() {
    let mut harness = Harness::playlists();
    // From the sidebar.
    harness.type_text("hjjjjj");
    harness.type_text("r");
    assert!(
        harness
            .status_line()
            .starts_with("│Rename “Favorites”: Favorites█")
    );
    for _ in 0.."Favorites".len() {
        harness.press(KeyCode::Backspace);
    }
    harness.type_text("Best");
    harness.press(KeyCode::Enter);
    assert!(harness.status_line().starts_with("│Renamed to “Best”"));
    assert_eq!(harness.stored()[0].0, "Best");

    // From the open playlist, whose title follows.
    harness.open_playlist(1);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / DRIVING");
    harness.type_text("r");
    harness.press(KeyCode::Backspace);
    harness.type_text("g Mix");
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / DRIVING MIX");
    assert_eq!(harness.names(), ["Midnight Drive", "Sunrise", "Last Light"]);
}

#[test]
fn rename_and_delete_need_a_playlist() {
    let mut harness = Harness::playlists();
    harness.type_text("r");
    assert!(harness.app.modal.is_none());
    assert!(
        harness
            .status_line()
            .starts_with("│Select a playlist to rename")
    );
    harness.type_text("hD");
    assert!(harness.app.modal.is_none());
    assert!(
        harness
            .status_line()
            .starts_with("│Select a playlist to delete")
    );
}

#[test]
fn shift_d_deletes_a_playlist_after_yes() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    harness.type_text("D");
    let line = harness.status_line();
    assert!(line.starts_with("│Delete “Driving”?"), "{line}");
    assert!(line.ends_with("y yes  n no│"), "{line}");
    // Other keys do nothing while it asks.
    harness.type_text("xjq");
    assert!(!harness.app.quit);
    assert_eq!(harness.app.cursor().selected, 0);
    harness.type_text("y");

    assert_eq!(harness.stored(), stored(&[("Favorites", &[])]));
    assert!(harness.status_line().starts_with("│Deleted “Driving”"));
    // The open playlist is gone, so the list goes back to all tracks.
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALL TRACKS");
    assert_eq!(harness.names().len(), 6);
    assert!(harness.screen()[11].starts_with("│> Favorites"));
}

#[test]
fn cancelling_the_delete_confirmation_keeps_the_playlist() {
    let mut harness = Harness::playlists();
    harness.type_text("hjjjjD");
    harness.type_text("n");
    assert!(harness.app.modal.is_none());
    harness.type_text("D");
    harness.press(KeyCode::Esc);
    assert!(harness.app.modal.is_none());
    assert_eq!(harness.stored().len(), 2);
    assert!(harness.status_line().starts_with("│Tab focus"));
}

#[test]
fn a_adds_the_selected_track_through_the_picker() {
    let mut harness = Harness::playlists();
    harness.type_text("j");
    assert_eq!(harness.selected(), "Last Light");
    harness.type_text("a");
    let screen = harness.screen();
    let top = row_of(&screen, " Add “Last Light” to ");
    assert!(screen[top + 1].contains("› █"));
    assert!(screen[top + 2].contains("> Driving"));
    assert!(screen[top + 3].contains("  Favorites"));

    // The filter narrows the playlists, ignoring case; `j` is text.
    harness.type_text("FAV");
    let screen = harness.screen();
    assert!(screen[top + 1].contains("› FAV█"));
    assert!(screen[top + 2].contains("> Favorites"));
    // The picker is 50 columns wide, in the middle of the screen.
    let inside: String = screen[top + 3].chars().skip(15).take(50).collect();
    assert_eq!(inside.trim_matches(|c| c == ' ' || c == '┃'), "");
    harness.press(KeyCode::Enter);

    assert!(harness.app.modal.is_none());
    assert!(
        harness
            .status_line()
            .starts_with("│Added “Last Light” to “Favorites”")
    );
    assert_eq!(
        harness.stored()[1],
        stored(&[("Favorites", &["Last Light"])])[0]
    );
}

#[test]
fn the_picker_moves_with_the_arrows_and_adds_from_any_track_list() {
    let mut harness = Harness::playlists();
    // A search result, added to the second playlist.
    harness.type_text("/sunrise");
    harness.press(KeyCode::Enter);
    harness.type_text("a");
    harness.press(KeyCode::Down);
    harness.press(KeyCode::Down);
    harness.press(KeyCode::Up);
    harness.press(KeyCode::Down);
    harness.press(KeyCode::Enter);
    // A playlist entry, added to its own playlist again.
    harness.open_playlist(0);
    harness.type_text("a");
    harness.press(KeyCode::Enter);
    assert_eq!(
        harness.stored(),
        stored(&[
            (
                "Driving",
                &["Midnight Drive", "Sunrise", "Last Light", "Midnight Drive"]
            ),
            ("Favorites", &["Sunrise"]),
        ])
    );
    assert_eq!(
        harness.names(),
        ["Midnight Drive", "Sunrise", "Last Light", "Midnight Drive"]
    );
}

#[test]
fn a_picker_filter_that_matches_nothing() {
    let mut harness = Harness::playlists();
    harness.type_text("azzz");
    let screen = harness.screen();
    row_of(&screen, "No playlists match “zzz”");
    // Enter adds nothing and leaves the picker open; Esc closes it.
    let requests = update(&mut harness.app, Action::PromptConfirm);
    assert!(requests.is_empty());
    assert!(matches!(harness.app.modal, Some(Modal::Picker(_))));
    harness.press(KeyCode::Down);
    harness.press(KeyCode::Backspace);
    harness.press(KeyCode::Backspace);
    harness.press(KeyCode::Backspace);
    assert!(
        harness
            .screen()
            .iter()
            .any(|line| line.contains("> Driving"))
    );
    harness.press(KeyCode::Esc);
    assert!(harness.app.modal.is_none());
    assert_eq!(harness.stored()[1].1.len(), 0);
}

#[test]
fn a_needs_a_track() {
    let mut harness = Harness::playlists();
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
    harness.type_text("a");
    assert!(harness.app.modal.is_none());
    assert!(harness.status_line().starts_with("│Select a track to add"));
    // From the sidebar, `a` doesn't reach into the list either.
    harness.type_text("hga");
    assert!(harness.app.modal.is_none());
}

#[test]
fn x_removes_the_selected_entry() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    harness.type_text("jx");
    assert_eq!(harness.names(), ["Midnight Drive", "Last Light"]);
    assert_eq!(harness.selected(), "Last Light");
    harness.type_text("x");
    assert_eq!(harness.selected(), "Midnight Drive");
    assert_eq!(
        harness.stored(),
        stored(&[("Driving", &["Midnight Drive"]), ("Favorites", &[])])
    );
    harness.type_text("x");
    assert!(harness.screen()[5].contains("Nothing here yet"));
    // Nothing is left to remove.
    let requests = update(&mut harness.app, Action::RemoveEntry);
    assert!(requests.is_empty());
}

#[test]
fn x_and_moves_do_nothing_outside_a_playlist() {
    let mut harness = Harness::playlists();
    for action in [Action::RemoveEntry, Action::MoveEntryDown] {
        assert!(update(&mut harness.app, action).is_empty());
    }
    assert_eq!(harness.names().len(), 6);
    // Nor on search results inside a playlist.
    harness.open_playlist(0);
    harness.type_text("/sunrise");
    harness.press(KeyCode::Enter);
    for action in [Action::RemoveEntry, Action::MoveEntryDown] {
        assert!(update(&mut harness.app, action).is_empty());
    }
    assert_eq!(harness.stored()[0].1.len(), 3);
}

#[test]
fn shift_j_and_k_move_the_entry_and_stop_at_the_edges() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    // Up at the top does nothing.
    assert!(update(&mut harness.app, Action::MoveEntryUp).is_empty());
    harness.type_text("J");
    assert_eq!(harness.names(), ["Sunrise", "Midnight Drive", "Last Light"]);
    assert_eq!(harness.selected(), "Midnight Drive");
    harness.type_text("J");
    assert_eq!(harness.names(), ["Sunrise", "Last Light", "Midnight Drive"]);
    // Down at the bottom does nothing.
    assert!(update(&mut harness.app, Action::MoveEntryDown).is_empty());
    harness.type_text("kK");
    assert_eq!(harness.names(), ["Last Light", "Sunrise", "Midnight Drive"]);
    assert_eq!(harness.selected(), "Last Light");
    assert!(harness.screen()[5].contains("> Last Light"));
    assert_eq!(
        harness.stored()[0].1,
        ["Last Light", "Sunrise", "Midnight Drive"]
    );
}

#[test]
fn edits_run_one_at_a_time_and_reloads_wait_for_the_last() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    // Two moves typed before the backend answers: the second waits.
    let first = update(&mut harness.app, Action::MoveEntryDown);
    let second = update(&mut harness.app, Action::MoveEntryDown);
    assert!(matches!(first.as_slice(), [Request::Edit(_)]));
    assert!(second.is_empty());
    assert_eq!(harness.names(), ["Sunrise", "Last Light", "Midnight Drive"]);

    // A reload sent before the edits is dropped.
    let top = harness.app.top();
    let stale = Request::Screen {
        id: top.id,
        load: top.load - 1,
        view: top.view,
    };

    let [first] = <[Request; 1]>::try_from(first).unwrap();
    let response = block_on(fetch(&harness.library, first));
    let next = receive(&mut harness.app, response);
    assert!(matches!(next.as_slice(), [Request::Edit(_)]));
    let response = block_on(fetch(&harness.library, stale));
    assert!(receive(&mut harness.app, response).is_empty());
    assert_eq!(harness.names(), ["Sunrise", "Last Light", "Midnight Drive"]);

    let [second] = <[Request; 1]>::try_from(next).unwrap();
    let response = block_on(fetch(&harness.library, second));
    let reloads = receive(&mut harness.app, response);
    assert!(matches!(
        reloads.as_slice(),
        [Request::Playlists { .. }, Request::Screen { .. }]
    ));
    harness.answer(reloads);
    assert_eq!(harness.names(), ["Sunrise", "Last Light", "Midnight Drive"]);
    assert_eq!(harness.selected(), "Midnight Drive");
    assert_eq!(
        harness.stored()[0].1,
        ["Sunrise", "Last Light", "Midnight Drive"]
    );
}

#[test]
fn a_failed_edit_drops_the_queued_ones_and_reloads() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    let first = update(&mut harness.app, Action::RemoveEntry);
    let queued = update(&mut harness.app, Action::MoveEntryDown);
    assert!(queued.is_empty());
    assert_eq!(harness.names(), ["Last Light", "Sunrise"]);
    let [Request::Edit(edit)] = <[Request; 1]>::try_from(first).unwrap() else {
        panic!("not an edit");
    };
    let reloads = receive(
        &mut harness.app,
        Response {
            request: Request::Edit(edit),
            result: Err("library storage failed".to_owned()),
        },
    );
    assert!(harness.status_line().starts_with("│library storage failed"));
    // Until the reload arrives, the rows that showed the failed removal
    // can't be edited.
    assert_eq!(harness.app.rows(), &Load::Loading);
    assert!(update(&mut harness.app, Action::MoveEntryUp).is_empty());
    harness.answer(reloads);
    assert_eq!(harness.names(), ["Midnight Drive", "Sunrise", "Last Light"]);
    assert_eq!(harness.stored()[0].1.len(), 3);
}

#[test]
fn the_playlist_keys_are_in_the_keymap() {
    let browse = [
        ('n', Action::NewPlaylist),
        ('r', Action::RenamePlaylist),
        ('D', Action::DeletePlaylist),
        ('a', Action::AddToPlaylist),
        ('x', Action::RemoveEntry),
        ('J', Action::MoveEntryDown),
        ('K', Action::MoveEntryUp),
    ];
    for (c, action) in browse {
        let key = keymap::Key::from_event(&KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert_eq!(keymap::action_for_key(Context::Browse, key), Some(action));
    }
    let key = |c| keymap::Key::from_event(&KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    assert_eq!(
        keymap::action_for_key(Context::Prompt, key('n')),
        Some(Action::PromptInput('n'))
    );
    assert_eq!(
        keymap::action_for_key(Context::Confirm, key('y')),
        Some(Action::PromptConfirm)
    );
    assert_eq!(keymap::action_for_key(Context::Confirm, key('x')), None);
}

#[test]
fn a_playlist_opened_during_an_edit_waits_for_it() {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    let removal = update(&mut harness.app, Action::RemoveEntry);
    // Open the playlist again before the removal is done. Its rows, read
    // before the removal, are dropped, so nothing can be edited on them.
    harness.type_text("h");
    for request in update(&mut harness.app, Action::Open) {
        let response = block_on(fetch(&harness.library, request));
        assert!(receive(&mut harness.app, response).is_empty());
    }
    assert_eq!(harness.app.rows(), &Load::Loading);
    assert!(update(&mut harness.app, Action::MoveEntryDown).is_empty());

    harness.answer(removal);
    assert_eq!(harness.names(), ["Sunrise", "Last Light"]);
    harness.type_text("J");
    assert_eq!(harness.stored()[0].1, ["Last Light", "Sunrise"]);
}

/// The sample library with a second album by Artist A, "Twilight", after
/// "Nocturnes", holding "Dusk" and "Starlight" in that order.
fn two_album_library() -> MemoryLibrary {
    let mut library = MemoryLibrary::sample();
    let (template, source) = terra_core::testing::sample::library().tracks.remove(0);
    let album = Album {
        id: AlbumId::new(4),
        title: "Twilight".to_owned(),
        album_artist: template.artist.clone(),
        year: Some(2024),
        kind: AlbumKind::Album,
    };
    library.insert_album(album.clone());
    for (number, title) in [(1, "Dusk"), (2, "Starlight")] {
        let track = Track {
            id: TrackId::new(100 + number),
            title: title.to_owned(),
            album: Some(AlbumRef {
                id: album.id,
                title: album.title.clone(),
            }),
            track_no: Some(u32::try_from(number).unwrap()),
            ..template.clone()
        };
        library.insert_track(track, source.clone());
    }
    library
}

/// Artists, with Artist A selected.
fn artists_at_artist_a(library: MemoryLibrary) -> Harness {
    let mut harness = Harness::new(library);
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
    harness.type_text("j");
    assert_eq!(harness.selected(), "Artist A");
    harness
}

/// Runs a request and returns the requests its response leads to, without
/// running them.
fn respond(harness: &mut Harness, request: Request) -> Vec<Request> {
    let response = block_on(fetch(&harness.library, request));
    receive(&mut harness.app, response)
}

#[test]
fn keys_typed_while_a_list_loads_act_on_it_once_it_has() {
    let mut harness = artists_at_artist_a(two_album_library());
    let artist = harness.press_unanswered(KeyCode::Enter);
    // Artist A's albums are loading: `j`, `Enter` and `j` wait for them.
    assert!(harness.press_unanswered(KeyCode::Char('j')).is_empty());
    assert!(harness.press_unanswered(KeyCode::Enter).is_empty());
    assert!(harness.press_unanswered(KeyCode::Char('j')).is_empty());
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ARTISTS / ARTIST A");

    let [artist] = <[Request; 1]>::try_from(artist).unwrap();
    let album = respond(&mut harness, artist);
    // `j` selected the second album and `Enter` opened it.
    let [Request::Screen { view, .. }] = album.as_slice() else {
        panic!("expected the album's tracks to be requested: {album:?}");
    };
    assert_eq!(
        *view,
        View::Tracks(TrackFilter {
            album: Some(AlbumId::new(4)),
            ..TrackFilter::default()
        })
    );
    assert_eq!(
        harness.app.breadcrumb(),
        "LIBRARY / ARTISTS / ARTIST A / TWILIGHT"
    );

    // The last `j` waits for the album's tracks.
    assert_eq!(harness.app.rows(), &Load::Loading);
    harness.answer(album);
    assert_eq!(harness.names(), ["Dusk", "Starlight"]);
    assert_eq!(harness.selected(), "Starlight");
}

#[test]
fn back_while_a_list_loads_acts_at_once_and_drops_the_held_keys() {
    for back in [KeyCode::Esc, KeyCode::Backspace] {
        let mut harness = artists_at_artist_a(two_album_library());
        let artist = harness.press_unanswered(KeyCode::Enter);
        assert!(harness.press_unanswered(KeyCode::Char('j')).is_empty());
        assert!(harness.press_unanswered(KeyCode::Enter).is_empty());

        assert!(harness.press_unanswered(back).is_empty());
        assert_eq!(harness.app.breadcrumb(), "LIBRARY / ARTISTS", "{back:?}");
        // The held `j` and `Enter` are gone: the late response is dropped,
        // and the next key acts at once.
        for request in artist {
            assert!(respond(&mut harness, request).is_empty(), "{back:?}");
        }
        assert_eq!(harness.selected(), "Artist A", "{back:?}");
        harness.type_text("j");
        assert_eq!(harness.selected(), "Artist B", "{back:?}");
    }
}

#[test]
fn quit_while_a_list_loads_acts_at_once() {
    for (code, modifiers) in [
        (KeyCode::Char('q'), KeyModifiers::NONE),
        (KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let mut harness = artists_at_artist_a(MemoryLibrary::sample());
        harness.press_unanswered(KeyCode::Enter);
        assert!(harness.press_unanswered(KeyCode::Char('j')).is_empty());
        assert!(harness.key_unanswered(code, modifiers).is_empty());
        assert!(harness.app.quit, "{code:?}");
    }
}

#[test]
fn search_typing_is_not_held_but_keys_after_it_wait_for_the_results() {
    let mut harness = Harness::sample();
    let mut searches = harness.press_unanswered(KeyCode::Char('/'));
    for c in "artist".chars() {
        searches.extend(harness.press_unanswered(KeyCode::Char(c)));
    }
    // Every character went into the query while the results were loading.
    assert_eq!(harness.app.editing_search().unwrap().query, "artist");
    assert_eq!(searches.len(), 6);

    // Enter keeps the results, and `j` waits for them.
    assert!(harness.press_unanswered(KeyCode::Enter).is_empty());
    assert!(harness.app.editing_search().is_none());
    assert!(harness.press_unanswered(KeyCode::Char('j')).is_empty());
    harness.answer(searches);
    // The results start at Artist A; the held `j` moved down from it.
    assert_eq!(harness.selected(), "Artist B");
}

const SAVING: &str = "│Saving playlist changes… (q again to quit now)";

/// "Driving" open, and `J` pressed with its edit returned unanswered.
fn moved_once() -> (Harness, Request) {
    let mut harness = Harness::playlists();
    harness.open_playlist(0);
    let edit = harness.key_unanswered(KeyCode::Char('J'), KeyModifiers::SHIFT);
    let [edit] = <[Request; 1]>::try_from(edit).unwrap();
    (harness, edit)
}

#[test]
fn quit_waits_for_pending_edits() {
    let (mut harness, first) = moved_once();
    let queued = harness.key_unanswered(KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert!(queued.is_empty());
    assert!(harness.press_unanswered(KeyCode::Char('q')).is_empty());
    assert!(!harness.app.quit);
    assert!(harness.status_line().starts_with(SAVING));

    let second = respond(&mut harness, first);
    assert!(matches!(second.as_slice(), [Request::Edit(_)]));
    assert!(!harness.app.quit);
    assert!(harness.status_line().starts_with(SAVING));

    let [second] = <[Request; 1]>::try_from(second).unwrap();
    // The last edit quits instead of reloading.
    assert!(respond(&mut harness, second).is_empty());
    assert!(harness.app.quit);
    assert!(super::exit(&harness.app).is_ok());
    assert_eq!(
        harness.stored()[0].1,
        ["Sunrise", "Last Light", "Midnight Drive"]
    );
}

#[test]
fn a_second_quit_quits_at_once() {
    for (code, modifiers) in [
        (KeyCode::Char('q'), KeyModifiers::NONE),
        (KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let (mut harness, _edit) = moved_once();
        assert!(harness.key_unanswered(code, modifiers).is_empty());
        assert!(!harness.app.quit, "{code:?}");
        assert!(harness.key_unanswered(code, modifiers).is_empty());
        assert!(harness.app.quit, "{code:?}");
        assert!(super::exit(&harness.app).is_ok(), "{code:?}");
    }
}

#[test]
fn a_failed_edit_while_quitting_quits_and_reports_it() {
    let (mut harness, first) = moved_once();
    let queued = harness.key_unanswered(KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert!(queued.is_empty());
    assert!(harness.press_unanswered(KeyCode::Char('q')).is_empty());
    let Request::Edit(edit) = first else {
        panic!("not an edit: {first:?}");
    };
    let requests = receive(
        &mut harness.app,
        Response {
            request: Request::Edit(edit),
            result: Err("library storage failed".to_owned()),
        },
    );
    // The queued move is dropped, and nothing reloads.
    assert!(requests.is_empty());
    assert!(harness.app.quit);
    let error = super::exit(&harness.app).unwrap_err();
    assert_eq!(
        error.to_string(),
        "playlist changes not saved: library storage failed"
    );
}

#[test]
fn keys_are_ignored_while_quitting() {
    let (mut harness, edit) = moved_once();
    assert!(harness.press_unanswered(KeyCode::Char('q')).is_empty());
    let names = harness.names();
    let selected = harness.selected();
    for code in [KeyCode::Char('j'), KeyCode::Char('x')] {
        assert!(harness.press_unanswered(code).is_empty(), "{code:?}");
    }
    assert_eq!(harness.names(), names);
    assert_eq!(harness.selected(), selected);
    assert!(harness.status_line().starts_with(SAVING));
    // A resize still applies.
    assert!(handle_event(&mut harness.app, &Event::Resize(WIDTH, HEIGHT + 10)).is_empty());
    assert_eq!(
        harness.app.list_height,
        super::render::list_height(WIDTH, HEIGHT + 10)
    );
    assert_ne!(
        harness.app.list_height,
        super::render::list_height(WIDTH, HEIGHT)
    );
    assert!(!harness.app.quit);
    harness.answer(vec![edit]);
    assert!(harness.app.quit);
    assert_eq!(
        harness.stored()[0].1,
        ["Sunrise", "Midnight Drive", "Last Light"]
    );
}

#[test]
fn quitting_from_a_prompt_closes_it_so_that_q_quits() {
    let (mut harness, _edit) = moved_once();
    harness.type_text("n");
    assert!(matches!(harness.app.modal, Some(Modal::Name { .. })));
    assert!(
        harness
            .key_unanswered(KeyCode::Char('c'), KeyModifiers::CONTROL)
            .is_empty()
    );
    assert!(harness.app.modal.is_none());
    assert!(harness.status_line().starts_with(SAVING));
    assert!(harness.press_unanswered(KeyCode::Char('q')).is_empty());
    assert!(harness.app.quit);
}
