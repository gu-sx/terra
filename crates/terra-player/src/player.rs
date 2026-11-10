//! The player handle and the thread behind it.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread;
use std::time::Duration;

use rodio::Source;
use rodio::queue::SourcesQueueOutput;
use terra_core::AudioSource;

use crate::engine::rodio::RodioEngine;
use crate::engine::{Engine, EngineEvent, EventSink};
use crate::{PlayerError, Queue, QueueItemId, Repeat};

/// What the player reports, in the order it happens.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlayerEvent {
    /// The item's first sample has gone to the output.
    TrackStarted { item: QueueItemId },
    /// How far the current item has played, about every 100 ms of its audio
    /// and at once after a seek. `total` is `None` when the source does not
    /// say.
    Position {
        item: QueueItemId,
        elapsed: Duration,
        total: Option<Duration>,
    },
    /// The item has played to its end. An item that is skipped, removed or
    /// stopped does not end.
    TrackEnded { item: QueueItemId },
    /// Playback has stopped: by a command, or because the queue ran out.
    Stopped,
    /// Something failed. `item` names the queue item it concerns, if any. An
    /// item that cannot be opened is skipped.
    Error {
        item: Option<QueueItemId>,
        error: PlayerError,
    },
}

/// Where a player sends its audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Output {
    /// The system's default output device.
    Device,
    /// No device: the audio is played in real time and discarded.
    Null,
}

/// A handle to a player running on its own thread.
///
/// Clones control the same player. Each command returns once the player has
/// applied it; the engine follows within about 10 ms of audio. The player
/// stops when the last handle is dropped.
#[derive(Clone)]
pub struct Player {
    handle: Arc<Handle>,
}

struct Handle {
    messages: Sender<Message>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Shutdown);
    }
}

enum Message {
    Command(Command, SyncSender<()>),
    Engine(EngineEvent),
    Shutdown,
}

#[derive(Debug)]
enum Command {
    Play(QueueItemId, AudioSource),
    Append(QueueItemId, AudioSource),
    InsertNext(QueueItemId, AudioSource),
    Remove(QueueItemId),
    Move(QueueItemId, usize),
    ClearQueue,
    SetRepeat(Repeat),
    Pause,
    Resume,
    Toggle,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    Next,
    Previous,
}

/// The audio of a player started with [`Player::capture`]: an endless
/// stream of interleaved samples, silent while nothing plays.
pub struct Capture {
    output: SourcesQueueOutput,
}

impl Capture {
    /// The channel count of the samples about to come.
    pub fn channels(&self) -> NonZero<u16> {
        self.output.channels()
    }

    /// The sample rate of the samples about to come.
    pub fn sample_rate(&self) -> NonZero<u32> {
        self.output.sample_rate()
    }
}

impl Iterator for Capture {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.output.next()
    }
}

impl Player {
    /// Starts a player on `output`. Opening the device happens on the
    /// player's thread; this waits for it and returns its error.
    pub fn open(output: Output) -> Result<(Player, Receiver<PlayerEvent>), PlayerError> {
        spawn(move |events| match output {
            Output::Device => RodioEngine::device(events),
            Output::Null => RodioEngine::null(events),
        })
    }

    /// Starts a player with the real engine and no device: nothing plays
    /// until the caller pulls samples from the [`Capture`], as a device
    /// would.
    pub fn capture() -> Result<(Player, Receiver<PlayerEvent>, Capture), PlayerError> {
        let (outputs, output) = mpsc::channel();
        let (player, events) = spawn(move |events| {
            let (engine, output) = RodioEngine::capture(events);
            let _ = outputs.send(output);
            Ok(engine)
        })?;
        let output = output.recv().map_err(|_| PlayerError::Closed)?;
        Ok((player, events, Capture { output }))
    }

    /// Replaces the queue with `source` and plays it.
    pub fn play(&self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        let item = QueueItemId::fresh();
        self.send(Command::Play(item, source)).map(|()| item)
    }

    /// Adds `source` at the end of the queue.
    pub fn append(&self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        let item = QueueItemId::fresh();
        self.send(Command::Append(item, source)).map(|()| item)
    }

    /// Adds `source` to play after the current item.
    pub fn insert_next(&self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        let item = QueueItemId::fresh();
        self.send(Command::InsertNext(item, source)).map(|()| item)
    }

    /// Removes an item from the queue. Removing the current item moves on to
    /// the item that followed it, as if it had ended.
    pub fn remove(&self, item: QueueItemId) -> Result<(), PlayerError> {
        self.send(Command::Remove(item))
    }

    /// Moves an item to `index` in the queue (see [`Queue::move_to`]).
    pub fn move_to(&self, item: QueueItemId, index: usize) -> Result<(), PlayerError> {
        self.send(Command::Move(item, index))
    }

    /// Empties the queue and stops.
    pub fn clear(&self) -> Result<(), PlayerError> {
        self.send(Command::ClearQueue)
    }

    pub fn set_repeat(&self, repeat: Repeat) -> Result<(), PlayerError> {
        self.send(Command::SetRepeat(repeat))
    }

    pub fn pause(&self) -> Result<(), PlayerError> {
        self.send(Command::Pause)
    }

    /// Resumes after a pause. When stopped, plays the current item from its
    /// start, or the next one when there is no current item.
    pub fn resume(&self) -> Result<(), PlayerError> {
        self.send(Command::Resume)
    }

    /// Pauses when playing, resumes otherwise.
    pub fn toggle(&self) -> Result<(), PlayerError> {
        self.send(Command::Toggle)
    }

    /// Stops playback. The queue and its current item stay.
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.send(Command::Stop)
    }

    /// Seeks within the current item.
    pub fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        self.send(Command::Seek(position))
    }

    /// Sets the volume, from 0.0 (silent) to 1.0 (full); values outside are
    /// clamped and NaN is ignored. The control is perceptual: the gain is
    /// its cube, as in mpv.
    pub fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        self.send(Command::SetVolume(volume))
    }

    /// Skips to the next item. At the end of the queue, with repeat off, it
    /// stops.
    pub fn next(&self) -> Result<(), PlayerError> {
        self.send(Command::Next)
    }

    /// Goes back to the previous item. At the first item, with repeat off,
    /// it plays that item again from its start.
    pub fn previous(&self) -> Result<(), PlayerError> {
        self.send(Command::Previous)
    }

    fn send(&self, command: Command) -> Result<(), PlayerError> {
        let (done, applied) = mpsc::sync_channel(1);
        self.handle
            .messages
            .send(Message::Command(command, done))
            .map_err(|_| PlayerError::Closed)?;
        applied.recv().map_err(|_| PlayerError::Closed)
    }
}

/// Starts the player's thread with the engine `make` builds there, and waits
/// until it is built.
fn spawn<E, F>(make: F) -> Result<(Player, Receiver<PlayerEvent>), PlayerError>
where
    E: Engine,
    F: FnOnce(EventSink) -> Result<E, PlayerError> + Send + 'static,
{
    let (messages, inbox) = mpsc::channel();
    let (events, outbox) = mpsc::channel();
    let (built, ready) = mpsc::sync_channel(1);
    let engine_messages = messages.clone();
    thread::Builder::new()
        .name("terra-player".to_owned())
        .spawn(move || {
            let sink = EventSink::new(move |event| {
                let _ = engine_messages.send(Message::Engine(event));
            });
            match make(sink) {
                Ok(engine) => {
                    let _ = built.send(Ok(()));
                    Core::new(engine, events).run(&inbox);
                }
                Err(error) => {
                    let _ = built.send(Err(error));
                }
            }
        })
        .map_err(|e| PlayerError::Output(e.to_string()))?;
    ready.recv().map_err(|_| PlayerError::Closed)??;
    let player = Player {
        handle: Arc::new(Handle { messages }),
    };
    Ok((player, outbox))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Stopped,
    Playing,
    Paused,
}

/// The player itself: the queue, the engine and what the engine was told.
struct Core<E> {
    engine: E,
    events: Sender<PlayerEvent>,
    queue: Queue<AudioSource>,
    state: State,
    /// The item the engine plays.
    current: Option<QueueItemId>,
    /// An item just loaded whose start has not been reported: until it is,
    /// events about other items are stale.
    loading: Option<QueueItemId>,
    /// The item handed to the engine to follow the current one.
    next: Option<QueueItemId>,
    /// An item the engine could not open as the next one. It is not handed
    /// over again; loading it reports the error when its turn comes.
    next_failed: Option<QueueItemId>,
}

impl<E: Engine> Core<E> {
    fn new(engine: E, events: Sender<PlayerEvent>) -> Self {
        Self {
            engine,
            events,
            queue: Queue::new(),
            state: State::Stopped,
            current: None,
            loading: None,
            next: None,
            next_failed: None,
        }
    }

    fn run(mut self, inbox: &Receiver<Message>) {
        while let Ok(message) = inbox.recv() {
            match message {
                Message::Command(command, done) => {
                    self.command(command);
                    let _ = done.send(());
                }
                Message::Engine(event) => self.engine_event(event),
                Message::Shutdown => break,
            }
        }
    }

    fn emit(&self, event: PlayerEvent) {
        let _ = self.events.send(event);
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Play(item, source) => {
                self.queue.clear();
                self.queue.push(item, source);
                self.queue.select(item);
                self.state = State::Playing;
                self.start_current();
            }
            Command::Append(item, source) => {
                self.queue.push(item, source);
                self.sync_next();
            }
            Command::InsertNext(item, source) => {
                self.queue.push_next(item, source);
                self.sync_next();
            }
            Command::Remove(item) => {
                let was_current = self.queue.current_id() == Some(item);
                if self.queue.remove(item).is_none() {
                    return;
                }
                if was_current && self.state != State::Stopped {
                    self.advance();
                } else {
                    self.sync_next();
                }
            }
            Command::Move(item, index) => {
                self.queue.move_to(item, index);
                self.sync_next();
            }
            Command::ClearQueue => {
                self.queue.clear();
                self.stop();
            }
            Command::SetRepeat(repeat) => {
                self.queue.set_repeat(repeat);
                self.sync_next();
            }
            Command::Pause => self.pause(),
            Command::Resume => self.resume(),
            Command::Toggle => match self.state {
                State::Playing => self.pause(),
                State::Paused | State::Stopped => self.resume(),
            },
            Command::Stop => self.stop(),
            Command::Seek(position) => {
                if self.current.is_some() {
                    self.engine.seek(position);
                }
            }
            Command::SetVolume(volume) => {
                if !volume.is_nan() {
                    self.engine.set_gain(volume.clamp(0.0, 1.0).powi(3));
                }
            }
            Command::Next => {
                if self.state == State::Stopped {
                    self.queue.next();
                } else {
                    self.advance();
                }
            }
            Command::Previous => {
                self.queue.previous();
                if self.state != State::Stopped {
                    self.start_current();
                }
            }
        }
    }

    fn engine_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Started(item) => self.started(item),
            EngineEvent::Ended(item) => {
                if self.loading.is_some() || self.current != Some(item) {
                    return;
                }
                self.emit(PlayerEvent::TrackEnded { item });
                self.current = None;
                // With a next item handed over, the engine starts it itself.
                if self.next.is_none() {
                    self.advance();
                }
            }
            EngineEvent::Position {
                item,
                elapsed,
                total,
            } => {
                if self.loading.is_none() && self.current == Some(item) {
                    self.emit(PlayerEvent::Position {
                        item,
                        elapsed,
                        total,
                    });
                }
            }
            EngineEvent::Error { item, error } => self.emit(PlayerEvent::Error { item, error }),
        }
    }

    fn started(&mut self, item: QueueItemId) {
        if let Some(loading) = self.loading {
            if loading != item {
                return;
            }
            self.loading = None;
        } else if self.next == Some(item) && self.current.is_none() {
            self.next = None;
            self.current = Some(item);
            if !self.queue.select(item) {
                // Removed while it was handed over: play what follows
                // instead of it.
                self.advance();
                return;
            }
        } else {
            return;
        }
        self.emit(PlayerEvent::TrackStarted { item });
        self.sync_next();
    }

    fn pause(&mut self) {
        if self.state == State::Playing {
            self.engine.pause();
            self.state = State::Paused;
        }
    }

    fn resume(&mut self) {
        match self.state {
            State::Playing => {}
            State::Paused => {
                self.engine.play();
                self.state = State::Playing;
            }
            State::Stopped => {
                if self.queue.current().is_none() {
                    self.queue.next();
                }
                if self.queue.current().is_some() {
                    self.state = State::Playing;
                    self.start_current();
                }
            }
        }
    }

    /// Moves to the next item and plays it, or stops at the end.
    fn advance(&mut self) {
        if self.queue.next().is_some() {
            self.start_current();
        } else {
            self.stop();
        }
    }

    /// Loads the queue's current item, keeping the play or pause state. An
    /// item that fails to open is reported and skipped.
    fn start_current(&mut self) {
        // Each item is tried at most once, even when repeat wraps around.
        for _ in 0..self.queue.len() {
            let Some((item, source)) = self.queue.current() else {
                break;
            };
            match self.engine.load(item, source) {
                Ok(()) => {
                    self.current = Some(item);
                    self.loading = Some(item);
                    self.next = None;
                    self.next_failed = None;
                    if self.state == State::Paused {
                        self.engine.pause();
                    } else {
                        self.state = State::Playing;
                        self.engine.play();
                    }
                    self.sync_next();
                    return;
                }
                Err(error) => {
                    self.emit(PlayerEvent::Error {
                        item: Some(item),
                        error,
                    });
                    if self.queue.next().is_none() {
                        break;
                    }
                }
            }
        }
        self.stop();
    }

    fn stop(&mut self) {
        if self.current.is_some() || self.next.is_some() {
            self.engine.clear();
        }
        self.current = None;
        self.loading = None;
        self.next = None;
        self.next_failed = None;
        if self.state != State::Stopped {
            self.state = State::Stopped;
            self.emit(PlayerEvent::Stopped);
        }
    }

    /// Hands the engine the item that follows the current one, when it has
    /// changed.
    fn sync_next(&mut self) {
        if self.current.is_none() {
            return;
        }
        let want = self.queue.peek_next();
        if want == self.next || (want.is_some() && want == self.next_failed) {
            return;
        }
        let source = want.and_then(|item| self.queue.get(item).map(|source| (item, source)));
        match self.engine.set_next(source) {
            Ok(()) => {
                self.next = want;
                self.next_failed = None;
            }
            Err(_) => {
                // Loading it later reports the error; meanwhile the engine
                // must not keep the item it had.
                let _ = self.engine.set_next(None);
                self.next = None;
                self.next_failed = want;
            }
        }
    }
}

#[cfg(test)]
mod tests;
