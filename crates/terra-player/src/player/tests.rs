//! The player's logic on a fake engine, driven synchronously: each test
//! gives the core commands and engine events and checks what it told the
//! engine and what it reported.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use terra_core::AudioSource;

use super::{Command, Core, State};
use crate::engine::{Engine, EngineEvent};
use crate::{PlayerError, PlayerEvent, QueueItemId, Repeat};

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Load(QueueItemId),
    SetNext(Option<QueueItemId>),
    Clear,
    Play,
    Pause,
    Seek(Duration),
    Gain(f32),
}

/// Records every call. Sources under the path "broken" fail to open.
#[derive(Default)]
struct Fake {
    calls: Vec<Call>,
}

fn opens(source: &AudioSource) -> Result<(), PlayerError> {
    match source {
        AudioSource::LocalPath(path) if path.starts_with("broken") => {
            Err(PlayerError::Decode(path.display().to_string()))
        }
        _ => Ok(()),
    }
}

impl Engine for Fake {
    fn load(&mut self, item: QueueItemId, source: &AudioSource) -> Result<(), PlayerError> {
        opens(source)?;
        self.calls.push(Call::Load(item));
        Ok(())
    }

    fn set_next(&mut self, next: Option<(QueueItemId, &AudioSource)>) -> Result<(), PlayerError> {
        if let Some((_, source)) = next {
            opens(source)?;
        }
        self.calls.push(Call::SetNext(next.map(|(item, _)| item)));
        Ok(())
    }

    fn clear(&mut self) {
        self.calls.push(Call::Clear);
    }

    fn play(&mut self) {
        self.calls.push(Call::Play);
    }

    fn pause(&mut self) {
        self.calls.push(Call::Pause);
    }

    fn seek(&mut self, position: Duration) {
        self.calls.push(Call::Seek(position));
    }

    fn set_gain(&mut self, gain: f32) {
        self.calls.push(Call::Gain(gain));
    }
}

struct Harness {
    core: Core<Fake>,
    events: Receiver<PlayerEvent>,
}

fn source(name: &str) -> AudioSource {
    AudioSource::LocalPath(PathBuf::from(name))
}

impl Harness {
    fn new() -> Self {
        let (sender, events) = mpsc::channel();
        Self {
            core: Core::new(Fake::default(), sender),
            events,
        }
    }

    fn play(&mut self, name: &str) -> QueueItemId {
        let item = QueueItemId::fresh();
        self.core.command(Command::Play(item, source(name)));
        item
    }

    fn append(&mut self, name: &str) -> QueueItemId {
        let item = QueueItemId::fresh();
        self.core.command(Command::Append(item, source(name)));
        item
    }

    fn insert_next(&mut self, name: &str) -> QueueItemId {
        let item = QueueItemId::fresh();
        self.core.command(Command::InsertNext(item, source(name)));
        item
    }

    /// The engine starts an item.
    fn start(&mut self, item: QueueItemId) {
        self.core.engine_event(EngineEvent::Started(item));
    }

    /// The engine ends an item.
    fn end(&mut self, item: QueueItemId) {
        self.core.engine_event(EngineEvent::Ended(item));
    }

    /// The engine's calls since the last time, and clears them.
    fn calls(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.core.engine.calls)
    }

    /// The player's events since the last time.
    fn events(&mut self) -> Vec<PlayerEvent> {
        self.events.try_iter().collect()
    }
}

fn started(item: QueueItemId) -> PlayerEvent {
    PlayerEvent::TrackStarted { item }
}

fn ended(item: QueueItemId) -> PlayerEvent {
    PlayerEvent::TrackEnded { item }
}

#[test]
fn play_loads_the_item_and_reports_its_start() {
    let mut h = Harness::new();
    let a = h.play("a");
    assert_eq!(h.calls(), [Call::Load(a), Call::Play]);
    assert_eq!(h.events(), []);
    h.start(a);
    assert_eq!(h.events(), [started(a)]);
    assert_eq!(h.core.state, State::Playing);
}

#[test]
fn play_replaces_the_queue() {
    let mut h = Harness::new();
    h.play("a");
    h.append("b");
    let c = h.play("c");
    assert_eq!(h.core.queue.len(), 1);
    assert_eq!(h.core.queue.current_id(), Some(c));
}

#[test]
fn the_next_item_is_handed_over_while_the_current_one_plays() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    h.calls();
    let b = h.append("b");
    assert_eq!(h.calls(), [Call::SetNext(Some(b))]);
    // A later item does not change what comes next.
    h.append("c");
    assert_eq!(h.calls(), []);
    // Inserting one to play next does.
    let d = h.insert_next("d");
    assert_eq!(h.calls(), [Call::SetNext(Some(d))]);
}

#[test]
fn the_engine_moves_to_the_handed_over_item() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    let c = h.append("c");
    h.start(a);
    h.calls();
    h.end(a);
    h.start(b);
    assert_eq!(h.events(), [started(a), ended(a), started(b)]);
    assert_eq!(h.core.queue.current_id(), Some(b));
    // The engine used up its next item; the player hands over the one after.
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
}

#[test]
fn the_queue_running_out_stops() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    h.calls();
    h.end(a);
    assert_eq!(h.events(), [started(a), ended(a), PlayerEvent::Stopped]);
    assert_eq!(h.calls(), [Call::Clear]);
    assert_eq!(h.core.state, State::Stopped);
    assert_eq!(h.core.queue.current_id(), None);
}

#[test]
fn an_item_that_cannot_be_opened_is_reported_and_skipped() {
    let mut h = Harness::new();
    let a = h.play("a");
    let broken = h.append("broken");
    let c = h.append("c");
    h.start(a);
    // The broken item could not be handed over, so the engine has none.
    assert_eq!(h.core.next, None);
    h.calls();
    h.end(a);
    let events = h.events();
    assert_eq!(events[..2], [started(a), ended(a)]);
    assert!(matches!(
        events[2],
        PlayerEvent::Error { item: Some(item), error: PlayerError::Decode(_) } if item == broken
    ));
    assert_eq!(events.len(), 3);
    assert_eq!(h.calls()[..2], [Call::Load(c), Call::Play]);
    assert_eq!(h.core.queue.current_id(), Some(c));
}

#[test]
fn a_queue_of_items_that_cannot_be_opened_stops_with_repeat() {
    let mut h = Harness::new();
    h.core.command(Command::SetRepeat(Repeat::All));
    h.append("broken/1");
    h.append("broken/2");
    h.core.command(Command::Resume);
    let events = h.events();
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[2], PlayerEvent::Stopped);
}

#[test]
fn removing_the_current_item_plays_the_one_that_followed() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.core.command(Command::Remove(a));
    assert_eq!(h.calls(), [Call::Load(b), Call::Play]);
    h.start(b);
    assert_eq!(h.events(), [started(a), started(b)]);
}

#[test]
fn removing_the_last_current_item_stops() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    h.core.command(Command::Remove(a));
    assert_eq!(h.core.state, State::Stopped);
    assert!(h.core.queue.is_empty());
}

#[test]
fn removing_the_handed_over_item_hands_over_the_next_one() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    let c = h.append("c");
    h.start(a);
    h.calls();
    h.core.command(Command::Remove(b));
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
}

#[test]
fn an_item_removed_as_the_engine_started_it_is_skipped() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    let c = h.append("c");
    h.start(a);
    // The engine reaches the join before it learns that b was removed.
    h.core.queue.remove(b);
    h.end(a);
    h.calls();
    h.start(b);
    assert_eq!(h.calls()[..2], [Call::Load(c), Call::Play]);
    assert_eq!(h.events(), [started(a), ended(a)]);
}

#[test]
fn moving_an_item_after_the_current_one_hands_it_over() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.append("b");
    let c = h.append("c");
    h.start(a);
    h.calls();
    h.core.command(Command::Move(c, 1));
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
}

#[test]
fn shuffling_while_an_item_plays_hands_over_the_new_next_item() {
    let mut h = Harness::new();
    h.core.rng = fastrand::Rng::with_seed(7);
    let a = h.play("a");
    let b = h.append("b");
    for name in ["c", "d", "e", "f", "g", "h"] {
        h.append(name);
    }
    h.start(a);
    h.calls();
    h.core.command(Command::Shuffle);
    let next = h.core.queue.peek_next();
    // The seed makes another item come next.
    assert_ne!(next, Some(b));
    assert_eq!(h.calls(), [Call::SetNext(next)]);
    assert_eq!(h.core.queue.current_id(), Some(a));
    assert_eq!(h.events(), [started(a)]);
}

#[test]
fn events_of_an_item_replaced_before_it_started_are_dropped() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.play("b");
    // The engine started a before it loaded b.
    h.start(a);
    h.core.engine_event(EngineEvent::Position {
        item: a,
        elapsed: Duration::from_millis(100),
        total: None,
    });
    h.end(a);
    h.start(b);
    assert_eq!(h.events(), [started(b)]);
}

#[test]
fn positions_of_the_current_item_are_reported() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    let position = PlayerEvent::Position {
        item: a,
        elapsed: Duration::from_millis(100),
        total: Some(Duration::from_secs(3)),
    };
    h.core.engine_event(EngineEvent::Position {
        item: a,
        elapsed: Duration::from_millis(100),
        total: Some(Duration::from_secs(3)),
    });
    assert_eq!(h.events(), [started(a), position]);
}

#[test]
fn next_skips_and_stops_at_the_end_without_repeat() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.core.command(Command::Next);
    assert_eq!(h.calls(), [Call::Load(b), Call::Play]);
    h.start(b);
    h.core.command(Command::Next);
    assert_eq!(h.calls(), [Call::Clear]);
    assert_eq!(h.core.state, State::Stopped);
}

#[test]
fn next_wraps_around_with_repeat() {
    let mut h = Harness::new();
    h.core.command(Command::SetRepeat(Repeat::All));
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.core.command(Command::Next);
    h.start(b);
    h.calls();
    h.core.command(Command::Next);
    assert_eq!(h.calls()[..2], [Call::Load(a), Call::Play]);
}

#[test]
fn repeat_hands_over_the_first_item_at_the_end() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.core.command(Command::Next);
    h.start(b);
    h.calls();
    h.core.command(Command::SetRepeat(Repeat::All));
    assert_eq!(h.calls(), [Call::SetNext(Some(a))]);
}

#[test]
fn previous_goes_back_and_restarts_the_first_item() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.core.command(Command::Next);
    h.start(b);
    h.calls();
    h.core.command(Command::Previous);
    assert_eq!(h.calls()[..2], [Call::Load(a), Call::Play]);
    h.start(a);
    h.calls();
    h.core.command(Command::Previous);
    assert_eq!(h.calls()[..2], [Call::Load(a), Call::Play]);
}

#[test]
fn pause_resume_and_toggle() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    h.calls();
    h.core.command(Command::Pause);
    assert_eq!(h.core.state, State::Paused);
    h.core.command(Command::Resume);
    assert_eq!(h.core.state, State::Playing);
    h.core.command(Command::Toggle);
    h.core.command(Command::Toggle);
    assert_eq!(
        h.calls(),
        [Call::Pause, Call::Play, Call::Pause, Call::Play]
    );
}

#[test]
fn skipping_while_paused_stays_paused() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.core.command(Command::Pause);
    h.calls();
    h.core.command(Command::Next);
    assert_eq!(h.calls(), [Call::Load(b), Call::Pause]);
    assert_eq!(h.core.state, State::Paused);
}

#[test]
fn stop_keeps_the_queue_and_resume_plays_the_current_item_again() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.append("b");
    h.start(a);
    h.calls();
    h.core.command(Command::Stop);
    assert_eq!(h.calls(), [Call::Clear]);
    assert_eq!(h.events(), [started(a), PlayerEvent::Stopped]);
    assert_eq!(h.core.queue.current_id(), Some(a));
    h.core.command(Command::Resume);
    assert_eq!(h.calls()[..2], [Call::Load(a), Call::Play]);
}

#[test]
fn resume_with_no_current_item_plays_the_next_one() {
    let mut h = Harness::new();
    let a = h.append("a");
    h.core.command(Command::Resume);
    assert_eq!(h.calls()[..2], [Call::Load(a), Call::Play]);
}

#[test]
fn seek_goes_to_the_engine_only_while_an_item_plays() {
    let mut h = Harness::new();
    h.core.command(Command::Seek(Duration::from_secs(1)));
    assert_eq!(h.calls(), []);
    let a = h.play("a");
    h.start(a);
    h.calls();
    h.core.command(Command::Seek(Duration::from_secs(1)));
    assert_eq!(h.calls(), [Call::Seek(Duration::from_secs(1))]);
}

#[test]
fn volume_is_cubed_clamped_and_nan_is_ignored() {
    let mut h = Harness::new();
    h.core.command(Command::SetVolume(0.5));
    h.core.command(Command::SetVolume(2.0));
    h.core.command(Command::SetVolume(-1.0));
    h.core.command(Command::SetVolume(f32::NAN));
    assert_eq!(
        h.calls(),
        [Call::Gain(0.125), Call::Gain(1.0), Call::Gain(0.0)]
    );
}

#[test]
fn clearing_the_queue_stops() {
    let mut h = Harness::new();
    let a = h.play("a");
    h.start(a);
    h.calls();
    h.core.command(Command::ClearQueue);
    assert_eq!(h.calls(), [Call::Clear]);
    assert!(h.core.queue.is_empty());
    assert_eq!(h.core.state, State::Stopped);
}

#[test]
fn an_item_replaced_as_the_engine_started_it_gives_way_to_its_replacement() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    let c = h.append("c");
    h.start(a);
    h.calls();
    h.core.command(Command::Remove(b));
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
    // The engine reached the join before the replacement arrived.
    h.end(a);
    h.start(b);
    assert_eq!(h.calls()[..2], [Call::Load(c), Call::Play]);
    h.start(c);
    assert_eq!(h.events(), [started(a), ended(a), started(c)]);
    assert_eq!(h.core.queue.current_id(), Some(c));
}

#[test]
fn an_item_dropped_as_the_engine_started_it_stops_at_the_end() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.core.command(Command::Remove(b));
    assert_eq!(h.calls(), [Call::SetNext(None)]);
    h.end(a);
    h.start(b);
    assert_eq!(h.calls(), [Call::Clear]);
    assert_eq!(h.events(), [started(a), ended(a), PlayerEvent::Stopped]);
}

#[test]
fn an_item_inserted_next_between_items_plays_before_the_handed_over_one() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.end(a);
    let c = h.insert_next("c");
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
    // The engine started b before the replacement arrived.
    h.start(b);
    assert_eq!(
        h.calls(),
        [Call::Load(c), Call::Play, Call::SetNext(Some(b))]
    );
    h.start(c);
    h.end(c);
    h.start(b);
    assert_eq!(
        h.events(),
        [started(a), ended(a), started(c), ended(c), started(b)]
    );
}

#[test]
fn an_item_moved_next_between_items_plays_before_the_handed_over_one() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    let c = h.append("c");
    h.start(a);
    h.calls();
    h.end(a);
    h.core.command(Command::Move(c, 1));
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
    h.start(b);
    assert_eq!(
        h.calls(),
        [Call::Load(c), Call::Play, Call::SetNext(Some(b))]
    );
    h.start(c);
    assert_eq!(h.events(), [started(a), ended(a), started(c)]);
}

#[test]
fn a_replacement_between_items_that_makes_the_join_plays_first() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.end(a);
    let c = h.insert_next("c");
    assert_eq!(h.calls(), [Call::SetNext(Some(c))]);
    // The engine started the replacement, not b.
    h.start(c);
    assert_eq!(h.calls(), [Call::SetNext(Some(b))]);
    assert_eq!(h.events(), [started(a), ended(a), started(c)]);
    h.end(c);
    h.start(b);
    assert_eq!(h.events(), [ended(c), started(b)]);
}

#[test]
fn removing_the_handed_over_item_between_items_stops_at_the_end() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.end(a);
    // Whether or not the engine has started b, nothing is left to play.
    h.core.command(Command::Remove(b));
    assert_eq!(h.calls(), [Call::SetNext(None), Call::Clear]);
    assert_eq!(h.events(), [started(a), ended(a), PlayerEvent::Stopped]);
    h.start(b);
    assert_eq!(h.events(), []);
    assert_eq!(h.core.state, State::Stopped);
}

#[test]
fn an_item_that_cannot_be_opened_inserted_between_items_is_reported_and_skipped() {
    let mut h = Harness::new();
    let a = h.play("a");
    let b = h.append("b");
    h.start(a);
    h.calls();
    h.end(a);
    let broken = h.insert_next("broken");
    // The engine must not keep b, and does not move on by itself: the player
    // loads what follows the broken item.
    assert_eq!(h.calls(), [Call::SetNext(None), Call::Load(b), Call::Play]);
    let events = h.events();
    assert_eq!(events[..2], [started(a), ended(a)]);
    assert!(matches!(
        events[2],
        PlayerEvent::Error { item: Some(item), error: PlayerError::Decode(_) } if item == broken
    ));
    assert_eq!(events.len(), 3);
    h.start(b);
    assert_eq!(h.events(), [started(b)]);
}
