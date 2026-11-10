//! Tests of the TUI against the synthetic sample library, without a
//! terminal: keys go through the keymap and `update`, requests run on
//! `block_on`, and the screen renders to Ratatui's `TestBackend`.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use terra_core::testing::{MemoryLibrary, block_on};
use terra_core::{LibraryBackend, Track, TrackId};

use super::app::{Action, App, Focus, Item, Load, Request, Response, receive, update};
use super::fetch::fetch;
use super::keymap::{self, Context, KEYMAP};
use super::render::{format_duration, help_line, render};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

/// A TUI on an 80×24 terminal over a library, with every request answered
/// before the next key.
struct Harness {
    app: App,
    library: MemoryLibrary,
}

impl Harness {
    fn new(library: MemoryLibrary) -> Self {
        let mut harness = Self {
            app: App::new("test library"),
            library,
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
        for request in requests {
            let response = block_on(fetch(&self.library, request));
            receive(&mut self.app, response);
        }
    }

    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let event = Event::Key(KeyEvent::new(code, modifiers));
        if let Some(action) = keymap::action_for_event(&self.app, &event) {
            self.dispatch(action);
        }
    }

    fn press(&mut self, code: KeyCode) {
        self.key(code, KeyModifiers::NONE);
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
        "│Tab focus  Enter open  Backspace back  / search  ? help  q quit               │",
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
    // Open Albums instead.
    harness.type_text("hj");
    harness.press(KeyCode::Enter);
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
    let (id, view) = (harness.app.top().id, harness.app.top().view);
    receive(
        &mut harness.app,
        Response {
            request: Request::Screen { id, view },
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

    // The sidebar reaches the playlists; opening one changes nothing yet.
    harness.type_text("hG");
    assert_eq!(harness.app.sidebar, 6);
    assert!(harness.screen()[13].starts_with("┃> Favorites"));
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.focus, Focus::Sidebar);
    assert_eq!(harness.app.breadcrumb(), "LIBRARY / ALL TRACKS");
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
    let screen = harness.screen();
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
