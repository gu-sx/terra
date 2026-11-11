//! Tests of playback: the keys against a fake player that records its
//! commands, the bottom bar from a synthetic sequence of player events, the
//! recording of plays on the library, and a missing file against the real
//! player with no device.

use std::cell::RefCell;
use std::num::NonZero;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use terra_core::testing::{MemoryLibrary, block_on};
use terra_core::{ArtistRef, AudioSource, LibraryBackend, Track, TrackId};
use terra_player::{
    Capture, PlaybackState, Player, PlayerError, PlayerEvent, PlayerStatus, Queue, QueueItemId,
};

use super::super::app::{Payload, Request, Response, receive};
use super::super::playback::{PLAY_THRESHOLD, Playback, player_event};
use super::{DEVICE, Harness, row_of};

/// A command the fake player was given.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play(AudioSource),
    Append(AudioSource),
    InsertNext(AudioSource),
    Remove(QueueItemId),
    MoveTo(QueueItemId, usize),
    Toggle,
    Resume,
    Next,
    Previous,
    Seek(Duration),
    SetVolume(f32),
}

struct FakeState {
    queue: Queue<AudioSource>,
    state: PlaybackState,
    commands: Vec<Command>,
}

/// A player that records its commands and keeps a queue, with no audio and
/// no events. Clones share the same player.
#[derive(Clone)]
pub struct FakePlayer(Rc<RefCell<FakeState>>);

impl Default for FakePlayer {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(FakeState {
            queue: Queue::new(),
            state: PlaybackState::Stopped,
            commands: Vec::new(),
        })))
    }
}

impl FakePlayer {
    /// The commands since the last time, and clears them.
    fn commands(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut().commands)
    }

    /// Moves to the next item on its own, as at the end of an item.
    fn advance(&self) {
        let mut state = self.0.borrow_mut();
        if state.queue.next().is_none() {
            state.state = PlaybackState::Stopped;
        }
    }

    fn record(&self, command: Command) -> std::cell::RefMut<'_, FakeState> {
        let mut state = self.0.borrow_mut();
        state.commands.push(command);
        state
    }
}

impl Playback for FakePlayer {
    fn play(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        let mut state = self.record(Command::Play(source.clone()));
        state.queue.clear();
        let item = state.queue.append(source);
        state.queue.select(item);
        state.state = PlaybackState::Playing;
        Ok(item)
    }

    fn append(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        Ok(self
            .record(Command::Append(source.clone()))
            .queue
            .append(source))
    }

    fn insert_next(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        let mut state = self.record(Command::InsertNext(source.clone()));
        Ok(state.queue.insert_next(source))
    }

    fn remove(&mut self, item: QueueItemId) -> Result<(), PlayerError> {
        let mut state = self.record(Command::Remove(item));
        let current = state.queue.current_id() == Some(item);
        state.queue.remove(item);
        if current && state.state != PlaybackState::Stopped && state.queue.next().is_none() {
            state.state = PlaybackState::Stopped;
        }
        Ok(())
    }

    fn move_to(&mut self, item: QueueItemId, index: usize) -> Result<(), PlayerError> {
        self.record(Command::MoveTo(item, index))
            .queue
            .move_to(item, index);
        Ok(())
    }

    fn toggle(&mut self) -> Result<(), PlayerError> {
        let mut state = self.record(Command::Toggle);
        state.state = match state.state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
        };
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        let mut state = self.record(Command::Resume);
        if state.queue.current().is_none() {
            state.queue.next();
        }
        if state.queue.current().is_some() {
            state.state = PlaybackState::Playing;
        }
        Ok(())
    }

    fn next(&mut self) -> Result<(), PlayerError> {
        let mut state = self.record(Command::Next);
        if state.queue.next().is_none() {
            state.state = PlaybackState::Stopped;
        }
        Ok(())
    }

    fn previous(&mut self) -> Result<(), PlayerError> {
        self.record(Command::Previous).queue.previous();
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        self.record(Command::Seek(position));
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        self.record(Command::SetVolume(volume));
        Ok(())
    }

    fn status(&mut self) -> Result<PlayerStatus, PlayerError> {
        let state = self.0.borrow();
        Ok(PlayerStatus {
            state: state.state,
            items: state.queue.iter().map(|(item, _)| item).collect(),
            current: state.queue.current_id(),
        })
    }
}

/// The sample library's file of a track.
fn sample(path: &str) -> AudioSource {
    AudioSource::LocalPath(PathBuf::from(format!("/music/{path}")))
}

impl Harness {
    /// Applies a player event and answers the requests it leads to.
    fn hear(&mut self, event: PlayerEvent) {
        let requests = player_event(&mut self.app, event);
        self.answer(requests);
    }

    /// Selects the track with this title in the main list.
    fn select(&mut self, title: &str) {
        self.press(KeyCode::Home);
        for _ in 0..self.names().len() {
            if self.selected() == title {
                return;
            }
            self.press(KeyCode::Down);
        }
        panic!("{title} is not in the list: {:?}", self.names());
    }

    /// The queue's items, in order.
    fn queue(&self) -> Vec<QueueItemId> {
        self.app.now.status.items.clone()
    }

    fn current(&self) -> QueueItemId {
        self.app.now.status.current.expect("a current item")
    }

    fn plays(&self) -> Vec<(String, String)> {
        let plays = block_on(self.library.recently_played(100)).unwrap();
        plays
            .into_iter()
            .map(|play| (play.track.title, play.device))
            .collect()
    }
}

#[test]
fn enter_plays_the_track_and_queues_the_rest_of_the_list() {
    let mut harness = Harness::sample();
    harness.select("After Hours");
    harness.press(KeyCode::Enter);
    assert_eq!(
        harness.player.commands(),
        [
            Command::Play(sample("artist-b/after-hours.mp3")),
            Command::Append(sample("artist-c/into-the-deep.mp3")),
            Command::Append(sample("artist-a/midnight-drive.flac")),
            Command::Append(sample("artist-e/demo-tape.flac")),
        ]
    );
    assert_eq!(harness.queue().len(), 4);
    assert_eq!(harness.current(), harness.queue()[0]);
    // The list stays as it was: Enter on a track does not open anything.
    assert_eq!(harness.app.stack.len(), 1);

    // Enter on another track replaces the queue.
    harness.select("Demo Tape");
    harness.press(KeyCode::Enter);
    assert_eq!(
        harness.player.commands(),
        [Command::Play(sample("artist-e/demo-tape.flac"))]
    );
    assert_eq!(harness.queue().len(), 1);
}

#[test]
fn playback_keys_give_the_player_commands() {
    let mut harness = Harness::sample();
    harness.select("Into the Deep");
    harness.press(KeyCode::Enter);
    harness.player.commands();

    harness.type_text(" ");
    assert_eq!(harness.player.commands(), [Command::Toggle]);
    assert_eq!(harness.app.now.status.state, PlaybackState::Paused);
    harness.type_text(" ");
    assert_eq!(harness.player.commands(), [Command::Toggle]);
    assert_eq!(harness.app.now.status.state, PlaybackState::Playing);

    let first = harness.current();
    harness.type_text(">");
    assert_eq!(harness.player.commands(), [Command::Next]);
    assert_ne!(harness.current(), first);
    harness.type_text("<");
    assert_eq!(harness.player.commands(), [Command::Previous]);
    assert_eq!(harness.current(), first);

    // Seeks count from the last position, and from each other until the
    // player reports one.
    harness.hear(PlayerEvent::Position {
        item: first,
        elapsed: Duration::from_secs(60),
        total: Some(Duration::from_secs(291)),
    });
    harness.type_text("]]");
    harness.type_text("[");
    harness.type_text("}");
    harness.type_text("{");
    let seconds = |s| Command::Seek(Duration::from_secs(s));
    assert_eq!(
        harness.player.commands(),
        [
            seconds(65),
            seconds(70),
            seconds(65),
            seconds(95),
            seconds(65)
        ]
    );
    // Back stops at the start; forward past the end skips to the next track.
    harness.type_text("{{{");
    assert_eq!(
        harness.player.commands(),
        [seconds(35), seconds(5), seconds(0)]
    );
    harness.hear(PlayerEvent::Position {
        item: first,
        elapsed: Duration::from_secs(280),
        total: Some(Duration::from_secs(291)),
    });
    harness.type_text("}");
    assert_eq!(harness.player.commands(), [Command::Next]);

    harness.type_text("-");
    assert_eq!(harness.player.commands(), [Command::SetVolume(0.95)]);
    assert_eq!(harness.app.status.as_ref().unwrap().text, "Volume 95%");
    harness.type_text("++");
    assert_eq!(
        harness.player.commands(),
        [Command::SetVolume(1.0), Command::SetVolume(1.0)]
    );
    assert_eq!(harness.app.now.volume, 100);
}

#[test]
fn e_appends_the_selected_track_and_shift_e_plays_it_next() {
    let mut harness = Harness::sample();
    harness.select("Sunrise");
    harness.press(KeyCode::Enter);
    harness.select("Midnight Drive");
    harness.player.commands();

    harness.type_text("e");
    assert_eq!(
        harness.player.commands(),
        [Command::Append(sample("artist-a/midnight-drive.flac"))]
    );
    assert_eq!(
        harness.app.status.as_ref().unwrap().text,
        "Queued “Midnight Drive”"
    );
    harness.select("Demo Tape");
    harness.type_text("E");
    assert_eq!(
        harness.player.commands(),
        [Command::InsertNext(sample("artist-e/demo-tape.flac"))]
    );
    let titles: Vec<&str> = harness
        .queue()
        .iter()
        .map(|item| harness.app.now.tracks[item].title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Sunrise",
            "Demo Tape",
            "Last Light",
            "After Hours",
            "Into the Deep",
            "Midnight Drive",
            "Demo Tape",
            "Midnight Drive",
        ]
    );

    // In the sidebar there is no track to queue.
    harness.press(KeyCode::Tab);
    harness.type_text("e");
    assert_eq!(harness.player.commands(), []);
}

#[test]
fn playback_keys_act_while_a_list_loads() {
    let mut harness = Harness::sample();
    harness.select("Sunrise");
    harness.press(KeyCode::Enter);
    harness.player.commands();
    // Albums is loading: the space is not held for it.
    harness.type_text("hjj");
    let requests = harness.press_unanswered(KeyCode::Enter);
    harness.type_text(" ");
    assert_eq!(harness.player.commands(), [Command::Toggle]);
    harness.answer(requests);
}

#[test]
fn the_queue_view_waits_for_the_keys_held_before_it() {
    let mut harness = Harness::playlists();
    harness.select("Sunrise");
    harness.press(KeyCode::Enter);
    harness.player.commands();
    let queued = harness.queue();
    // "Driving" is loading while `j`, `x` and `Q` are typed.
    harness.type_text("hgjjjj");
    let requests = harness.press_unanswered(KeyCode::Enter);
    harness.type_text("jxQ");
    assert!(harness.app.queue_view.is_none());
    harness.answer(requests);

    // `j` and `x` removed the playlist's second entry, then `Q` opened the
    // queue, which is as it was.
    assert_eq!(harness.names(), ["Midnight Drive", "Last Light"]);
    assert_eq!(
        harness.stored()[0],
        (
            "Driving".to_owned(),
            vec!["Midnight Drive".to_owned(), "Last Light".to_owned()]
        )
    );
    assert!(harness.app.queue_view.is_some());
    assert_eq!(harness.player.commands(), []);
    assert_eq!(harness.queue(), queued);
}

#[test]
fn the_queue_view_removes_and_reorders_items() {
    let mut harness = Harness::sample();
    harness.select("Last Light");
    harness.press(KeyCode::Enter);
    harness.player.commands();
    let items = harness.queue();
    assert_eq!(items.len(), 5);

    harness.type_text("Q");
    let screen = harness.screen();
    row_of(&screen, " Queue: 5 tracks ");
    let current = row_of(&screen, "> ▶ Last Light / Artist E");
    assert_eq!(row_of(&screen, "    After Hours / Artist B"), current + 1);

    // Down to After Hours, then move it down and remove the item after it.
    harness.type_text("jJ");
    assert_eq!(harness.player.commands(), [Command::MoveTo(items[1], 2)]);
    assert_eq!(
        harness.queue(),
        [items[0], items[2], items[1], items[3], items[4]]
    );
    assert_eq!(harness.app.queue_view.unwrap().selected, 2);
    harness.type_text("jx");
    assert_eq!(harness.player.commands(), [Command::Remove(items[3])]);
    assert_eq!(harness.queue(), [items[0], items[2], items[1], items[4]]);
    // Moving up at the top changes nothing.
    harness.type_text("gK");
    assert_eq!(harness.player.commands(), []);
    // Playback keys work in the queue view.
    harness.type_text(" ");
    assert_eq!(harness.player.commands(), [Command::Toggle]);

    harness.press(KeyCode::Esc);
    assert!(harness.app.queue_view.is_none());
    // The main list's selection did not move.
    assert_eq!(harness.selected(), "Last Light");
    harness.type_text("QQ");
    assert!(harness.app.queue_view.is_none());
}

#[test]
fn the_bar_shows_the_track_its_time_and_progress() {
    let mut harness = Harness::sample();
    harness.select("Midnight Drive");
    harness.press(KeyCode::Enter);
    let [a, b] = harness.queue()[..] else {
        panic!("two items: {:?}", harness.queue());
    };
    harness.hear(PlayerEvent::TrackStarted { item: a });
    harness.hear(PlayerEvent::Position {
        item: a,
        elapsed: Duration::from_secs(133),
        total: Some(Duration::from_secs(342)),
    });
    let bar = |screen: Vec<String>| screen[19..].to_vec();
    assert_eq!(
        bar(harness.screen()),
        [
            "┌──────────────────────────────────────────────────────────────────────────────┐",
            "│▶ Midnight Drive — Artist A                                        02:13/05:42│",
            "│━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│",
            "│Tab focus  Enter open  Backspace back  / search  a add  ? help  q quit        │",
            "└──────────────────────────────────────────────────────────────────────────────┘",
        ]
    );

    harness.type_text(" ");
    assert!(harness.screen()[20].starts_with("│‖ Midnight Drive — Artist A"));
    harness.type_text(" ");

    // The gapless join: the first track ends and the second starts.
    harness.hear(PlayerEvent::Position {
        item: a,
        elapsed: Duration::from_secs(342),
        total: Some(Duration::from_secs(342)),
    });
    assert!(harness.screen()[21].contains(&"━".repeat(78)));
    harness.player.advance();
    harness.hear(PlayerEvent::TrackEnded { item: a });
    harness.hear(PlayerEvent::TrackStarted { item: b });
    assert_eq!(
        bar(harness.screen())[1..3],
        [
            "│▶ Demo Tape — Artist E                                             00:00/03:33│",
            "│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│",
        ]
    );

    // The queue runs out.
    harness.player.advance();
    harness.hear(PlayerEvent::TrackEnded { item: b });
    harness.hear(PlayerEvent::Stopped);
    assert!(harness.screen()[20].starts_with("│■ Nothing playing"));
}

#[test]
fn crossing_the_play_threshold_records_one_play() {
    let mut harness = Harness::sample();
    // 03:33 long: its play counts from 01:46.5.
    harness.select("Demo Tape");
    harness.press(KeyCode::Enter);
    let item = harness.current();
    harness.hear(PlayerEvent::TrackStarted { item });
    let at = |seconds: f64| PlayerEvent::Position {
        item,
        elapsed: Duration::from_secs_f64(seconds),
        total: Some(Duration::from_secs(213)),
    };
    for seconds in [0.0, 50.0, 106.4] {
        harness.hear(at(seconds));
    }
    assert_eq!(harness.plays(), []);
    harness.hear(at(106.5));
    let play = ("Demo Tape".to_owned(), DEVICE.to_owned());
    assert_eq!(harness.plays(), std::slice::from_ref(&play));
    for seconds in [106.6, 150.0, 213.0] {
        harness.hear(at(seconds));
    }
    assert_eq!(harness.plays(), std::slice::from_ref(&play));

    // Playing it again is another play.
    harness.hear(PlayerEvent::TrackStarted { item });
    harness.hear(at(120.0));
    assert_eq!(harness.plays(), [play.clone(), play]);
}

#[test]
fn a_long_track_counts_as_played_after_four_minutes() {
    let mut harness = Harness::sample();
    harness.select("Sunrise");
    harness.press(KeyCode::Enter);
    let item = harness.current();
    harness.hear(PlayerEvent::TrackStarted { item });
    // The player says the set is an hour long, longer than the library does.
    let at = |elapsed: Duration| PlayerEvent::Position {
        item,
        elapsed,
        total: Some(Duration::from_secs(3600)),
    };
    harness.hear(at(PLAY_THRESHOLD - Duration::from_millis(100)));
    assert_eq!(harness.plays(), []);
    harness.hear(at(PLAY_THRESHOLD));
    assert_eq!(harness.plays(), [("Sunrise".to_owned(), DEVICE.to_owned())]);
}

#[test]
fn a_track_the_backend_cannot_resolve_is_reported_and_skipped() {
    let mut harness = Harness::sample();
    harness.select("After Hours");
    let requests = harness.press_unanswered(KeyCode::Enter);
    let [Request::Resolve { job, tracks }] = &requests[..] else {
        panic!("one resolve: {requests:?}");
    };
    assert_eq!(tracks.len(), 1);
    let response = Response {
        request: Request::Resolve {
            job: *job,
            tracks: tracks.clone(),
        },
        result: Ok(Payload::Sources(vec![
            Err("the file is missing".to_owned()),
        ])),
    };
    let requests = receive(&mut harness.app, response);
    assert_eq!(harness.player.commands(), []);
    let status = harness.app.status.clone().unwrap();
    assert!(status.error);
    assert_eq!(
        status.text,
        "Cannot play “After Hours”: the file is missing"
    );

    // The next track plays instead, alone first, then the rest.
    harness.answer(requests);
    assert_eq!(
        harness.player.commands(),
        [
            Command::Play(sample("artist-c/into-the-deep.mp3")),
            Command::Append(sample("artist-a/midnight-drive.flac")),
            Command::Append(sample("artist-e/demo-tape.flac")),
        ]
    );
    assert!(row_of(&harness.screen(), "Cannot play “After Hours”") > 19);
}

#[test]
fn without_a_player_the_keys_say_why() {
    let library = MemoryLibrary::sample();
    let error = PlayerError::Output("no device".to_owned());
    let no_player = super::super::NoPlayer(error);
    let mut harness = Harness::with_player(library, Box::new(no_player), FakePlayer::default());
    let expected = "cannot open the audio output: no device";
    assert_eq!(harness.app.status.as_ref().unwrap().text, expected);
    harness.type_text("j");
    harness.type_text(" ");
    assert_eq!(harness.app.status.as_ref().unwrap().text, expected);
    harness.press(KeyCode::Enter);
    assert_eq!(harness.app.status.as_ref().unwrap().text, expected);
}

/// A library of three tracks by Artist A: "Missing", whose file does not
/// exist, then "Tone A" and "Tone B", the player's 1.5 s fixtures.
fn missing_file_library(missing: PathBuf) -> MemoryLibrary {
    let (template, _) = terra_core::testing::sample::library().tracks.remove(0);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../terra-player/tests/fixtures");
    let mut library = MemoryLibrary::new();
    let tracks = [
        ("Missing", missing),
        ("Tone A", fixtures.join("a.flac")),
        ("Tone B", fixtures.join("b.flac")),
    ];
    for ((title, path), id) in tracks.into_iter().zip(1..) {
        let track = Track {
            id: TrackId::new(id),
            title: title.to_owned(),
            artist: ArtistRef {
                id: template.artist.id,
                name: "Artist A".to_owned(),
            },
            album: None,
            disc_no: None,
            track_no: None,
            duration: Duration::from_millis(1500),
            ..template.clone()
        };
        library.insert_track(track, AudioSource::LocalPath(path));
    }
    library
}

/// Pulls 10 ms of audio at a time and applies the player's events, until
/// `done`. Panics after 10 s of audio.
fn play_until(
    harness: &mut Harness,
    player: &Player,
    capture: &mut Capture,
    events: &Receiver<PlayerEvent>,
    done: impl Fn(&PlayerEvent) -> bool,
) {
    for _ in 0..1000 {
        capture.by_ref().take(480).for_each(drop);
        // The status goes through the player's thread after the events the
        // engine sent so far.
        player.status().unwrap();
        for event in events.try_iter() {
            let finished = done(&event);
            harness.hear(event);
            if finished {
                return;
            }
        }
    }
    panic!("not done after 10 s of audio");
}

#[test]
fn a_missing_file_shows_the_error_and_the_queue_moves_on() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing.flac");
    let (player, events, mut capture) =
        Player::capture(NonZero::new(1).unwrap(), NonZero::new(48_000).unwrap()).unwrap();
    let library = missing_file_library(missing.clone());
    let mut harness =
        Harness::with_player(library, Box::new(player.clone()), FakePlayer::default());
    assert_eq!(harness.names(), ["Missing", "Tone A", "Tone B"]);

    harness.press(KeyCode::Enter);
    let [missing_item, tone_a, _] = harness.queue()[..] else {
        panic!("three items: {:?}", harness.queue());
    };
    play_until(&mut harness, &player, &mut capture, &events, |event| {
        *event == PlayerEvent::TrackStarted { item: tone_a }
    });

    assert_eq!(harness.current(), tone_a);
    assert_eq!(harness.queue()[0], missing_item);
    let screen = harness.screen();
    assert!(screen[20].starts_with("│▶ Tone A — Artist A"), "{screen:?}");
    let status = harness.app.status.clone().unwrap();
    assert!(status.error);
    let expected = format!("Cannot play “Missing”: cannot decode {}", missing.display());
    assert!(status.text.starts_with(&expected), "{}", status.text);
    row_of(&screen, "Cannot play “Missing”");
}

/// "Demo Tape" playing past its threshold, with the request that records
/// its play returned unanswered.
fn recording_demo_tape() -> (Harness, Request) {
    let mut harness = Harness::sample();
    harness.select("Demo Tape");
    harness.press(KeyCode::Enter);
    let item = harness.current();
    harness.hear(PlayerEvent::TrackStarted { item });
    let requests = player_event(
        &mut harness.app,
        PlayerEvent::Position {
            item,
            elapsed: Duration::from_secs(120),
            total: None,
        },
    );
    let [record] = <[Request; 1]>::try_from(requests).unwrap();
    assert!(matches!(record, Request::RecordPlay { .. }));
    (harness, record)
}

#[test]
fn quit_waits_for_a_play_being_recorded() {
    let (mut harness, record) = recording_demo_tape();
    harness.type_text("Q");
    harness.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!harness.app.quit);
    // The queue view closes, so that `q` quits.
    assert!(harness.app.queue_view.is_none());
    row_of(
        &harness.screen(),
        "Saving the play history… (q again to quit now)",
    );
    harness.answer(vec![record]);
    assert!(harness.app.quit);
    assert!(super::super::exit(&harness.app).is_ok());
    assert_eq!(
        harness.plays(),
        [("Demo Tape".to_owned(), DEVICE.to_owned())]
    );
}

#[test]
fn a_play_that_fails_to_record_while_quitting_is_reported() {
    let (mut harness, record) = recording_demo_tape();
    harness.type_text("q");
    assert!(!harness.app.quit);
    receive(
        &mut harness.app,
        Response {
            request: record,
            result: Err("library storage failed".to_owned()),
        },
    );
    assert!(harness.app.quit);
    let error = super::super::exit(&harness.app).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not record the play of “Demo Tape”: library storage failed"
    );
}
