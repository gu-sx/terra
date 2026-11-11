//! Playback: the player the TUI drives, the jobs that turn tracks into audio
//! sources for its queue, the state the bottom bar shows, and the queue view.
//!
//! The TUI keeps no copy of the queue, since the player moves through it on
//! its own, for example past an item that fails to open. After each command
//! it sends, and each event the player reports other than a position, it
//! reads the player's [`PlayerStatus`].
//!
//! Playing or queueing tracks first asks the backend where their audio is,
//! with a [`Request::Resolve`]. The jobs run one at a time, in the order they
//! were made, so that tracks reach the queue in the order they were queued.
//! `Enter` on a track replaces the queue: it drops the jobs not yet done and
//! resolves the selected track on its own, so that it starts at once, then
//! the rest of the list; as the rest arrives, a player that has stopped, for
//! example because the first track failed to open, plays on. A track the
//! backend cannot resolve, such as a file a scan found missing, is reported
//! and skipped; a file the player cannot open is reported by the player,
//! which skips it.
//!
//! A play is recorded once each time an item starts, when its position
//! reaches half the track or four minutes, whichever comes first.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime};

use terra_core::{AudioSource, Track};
use terra_player::{PlaybackState, Player, PlayerError, PlayerEvent, PlayerStatus, QueueItemId};

use super::app::{App, Focus, Item, Load, Payload, Request, Status};

/// How far into an item its play is recorded, at most.
pub const PLAY_THRESHOLD: Duration = Duration::from_secs(4 * 60);

/// How much `-` and `+` change the volume, in percent.
const VOLUME_STEP: u8 = 5;

/// The commands the TUI gives a player. [`Player`] is the real one; tests
/// use a fake.
pub trait Playback {
    fn play(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError>;
    fn append(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError>;
    fn insert_next(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError>;
    fn remove(&mut self, item: QueueItemId) -> Result<(), PlayerError>;
    fn move_to(&mut self, item: QueueItemId, index: usize) -> Result<(), PlayerError>;
    /// Empties the queue and stops.
    fn clear(&mut self) -> Result<(), PlayerError>;
    fn toggle(&mut self) -> Result<(), PlayerError>;
    fn resume(&mut self) -> Result<(), PlayerError>;
    fn next(&mut self) -> Result<(), PlayerError>;
    fn previous(&mut self) -> Result<(), PlayerError>;
    fn seek(&mut self, position: Duration) -> Result<(), PlayerError>;
    /// From 0.0 to 1.0.
    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError>;
    fn status(&mut self) -> Result<PlayerStatus, PlayerError>;
}

impl Playback for Player {
    fn play(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        Player::play(self, source)
    }

    fn append(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        Player::append(self, source)
    }

    fn insert_next(&mut self, source: AudioSource) -> Result<QueueItemId, PlayerError> {
        Player::insert_next(self, source)
    }

    fn remove(&mut self, item: QueueItemId) -> Result<(), PlayerError> {
        Player::remove(self, item)
    }

    fn move_to(&mut self, item: QueueItemId, index: usize) -> Result<(), PlayerError> {
        Player::move_to(self, item, index)
    }

    fn clear(&mut self) -> Result<(), PlayerError> {
        Player::clear(self)
    }

    fn toggle(&mut self) -> Result<(), PlayerError> {
        Player::toggle(self)
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        Player::resume(self)
    }

    fn next(&mut self) -> Result<(), PlayerError> {
        Player::next(self)
    }

    fn previous(&mut self) -> Result<(), PlayerError> {
        Player::previous(self)
    }

    fn seek(&mut self, position: Duration) -> Result<(), PlayerError> {
        Player::seek(self, position)
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlayerError> {
        Player::set_volume(self, volume)
    }

    fn status(&mut self) -> Result<PlayerStatus, PlayerError> {
        Player::status(self)
    }
}

/// No player, because opening one failed: every command fails with that
/// error.
pub struct NoPlayer(pub PlayerError);

impl Playback for NoPlayer {
    fn play(&mut self, _: AudioSource) -> Result<QueueItemId, PlayerError> {
        Err(self.0.clone())
    }

    fn append(&mut self, _: AudioSource) -> Result<QueueItemId, PlayerError> {
        Err(self.0.clone())
    }

    fn insert_next(&mut self, _: AudioSource) -> Result<QueueItemId, PlayerError> {
        Err(self.0.clone())
    }

    fn remove(&mut self, _: QueueItemId) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn move_to(&mut self, _: QueueItemId, _: usize) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn clear(&mut self) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn toggle(&mut self) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn resume(&mut self) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn next(&mut self) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn previous(&mut self) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn seek(&mut self, _: Duration) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn set_volume(&mut self, _: f32) -> Result<(), PlayerError> {
        Err(self.0.clone())
    }

    fn status(&mut self) -> Result<PlayerStatus, PlayerError> {
        Err(self.0.clone())
    }
}

/// How a job puts its tracks in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queueing {
    /// Replaces the queue and plays.
    Replace,
    /// Adds at the end.
    Append,
    /// Adds to play after the current item.
    Next,
}

/// Tracks on their way to the queue.
#[derive(Debug)]
pub(super) struct Job {
    id: u64,
    how: Queueing,
    /// The tracks not yet asked for.
    tracks: VecDeque<Track>,
    /// Whether a track of a [`Queueing::Replace`] job has replaced the queue.
    started: bool,
}

/// How far an item has played, as the player last reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub item: QueueItemId,
    pub elapsed: Duration,
    pub total: Option<Duration>,
}

/// The queue view's selection, an index into the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueView {
    pub selected: usize,
}

/// What the TUI knows of the player.
#[derive(Debug)]
pub struct NowPlaying {
    /// The player's status as last read.
    pub status: PlayerStatus,
    /// The track of each item in the queue.
    pub tracks: HashMap<QueueItemId, Track>,
    pub position: Option<Position>,
    /// In percent.
    pub volume: u8,
    /// The item whose start was reported last, and whether its play has
    /// been recorded.
    started: Option<(QueueItemId, bool)>,
}

impl NowPlaying {
    pub(super) fn new() -> Self {
        Self {
            status: PlayerStatus {
                state: PlaybackState::Stopped,
                items: Vec::new(),
                current: None,
            },
            tracks: HashMap::new(),
            position: None,
            volume: 100,
            started: None,
        }
    }

    /// The current item's track, how far it has played and how long it is.
    pub fn current(&self) -> Option<(&Track, Duration, Duration)> {
        let item = self.status.current?;
        let track = self.tracks.get(&item)?;
        let (elapsed, total) = match self.position {
            Some(position) if position.item == item => (position.elapsed, position.total),
            _ => (Duration::ZERO, None),
        };
        Some((track, elapsed, total.unwrap_or(track.duration)))
    }
}

fn error(text: String) -> Option<Status> {
    Some(Status { text, error: true })
}

fn info(text: String) -> Option<Status> {
    Some(Status { text, error: false })
}

impl App {
    /// Plays the selected track and queues the tracks after it in the list.
    pub(super) fn play_from_selected(&mut self) -> Vec<Request> {
        let Load::Ready(items) = self.rows() else {
            return Vec::new();
        };
        let tracks: VecDeque<Track> = items[self.cursor().selected..]
            .iter()
            .filter_map(Item::track)
            .cloned()
            .collect();
        // The queue is replaced: the jobs before this one no longer matter,
        // and the response of the running one is dropped.
        self.jobs.clear();
        self.start_job(Queueing::Replace, tracks)
    }

    /// Queues the selected track of the main list.
    pub(super) fn queue_selected(&mut self, how: Queueing) -> Vec<Request> {
        let track = match self.focus {
            Focus::List => self.selected_item().and_then(Item::track).cloned(),
            Focus::Sidebar => None,
        };
        match track {
            Some(track) => self.start_job(how, VecDeque::from([track])),
            None => {
                self.status = info("Select a track to queue".to_owned());
                Vec::new()
            }
        }
    }

    fn start_job(&mut self, how: Queueing, tracks: VecDeque<Track>) -> Vec<Request> {
        let id = self.seq();
        self.jobs.push_back(Job {
            id,
            how,
            tracks,
            started: false,
        });
        match self.jobs.len() {
            1 => self.resolve_next(),
            _ => Vec::new(),
        }
    }

    /// Asks for the next tracks of the first job: one at a time until a
    /// replacing job has started, then all the rest. A replacing job none of
    /// whose tracks could be queued still replaces the queue: it empties it.
    fn resolve_next(&mut self) -> Vec<Request> {
        while let Some(job) = self.jobs.front_mut() {
            if job.tracks.is_empty() {
                let unstarted = job.how == Queueing::Replace && !job.started;
                self.jobs.pop_front();
                if unstarted {
                    self.command(|player| player.clear());
                }
                continue;
            }
            let count = match (job.how, job.started) {
                (Queueing::Replace, false) => 1,
                _ => job.tracks.len(),
            };
            let tracks = job.tracks.drain(..count).collect();
            return vec![Request::Resolve {
                job: job.id,
                tracks,
            }];
        }
        Vec::new()
    }

    /// Queues the tracks a job's request resolved, reports those that did
    /// not, and asks for the next ones.
    pub(super) fn finish_resolve(
        &mut self,
        job: u64,
        tracks: Vec<Track>,
        result: Result<Payload, String>,
    ) -> Vec<Request> {
        if self.jobs.front().map(|job| job.id) != Some(job) {
            return Vec::new();
        }
        let sources = match result {
            Ok(Payload::Sources(sources)) => sources,
            Ok(_) => vec![Err("unexpected response".to_owned()); tracks.len()],
            Err(error) => vec![Err(error); tracks.len()],
        };
        let mut failure = None;
        for (track, source) in tracks.into_iter().zip(sources) {
            match source {
                Ok(source) => {
                    if let Err(error) = self.hand_over(track, source) {
                        failure = Some(error.to_string());
                    }
                }
                Err(reason) => failure = Some(format!("Cannot play “{}”: {reason}", track.title)),
            }
        }
        let job = self.jobs.front().expect("the job is running");
        if job.how == Queueing::Replace && job.started {
            self.refresh();
            if self.now.status.state == PlaybackState::Stopped {
                self.command(|player| player.resume());
            }
        } else {
            self.refresh();
        }
        if let Some(text) = failure {
            self.status = error(text);
        }
        self.resolve_next()
    }

    /// Hands a resolved track to the player as the first job says.
    fn hand_over(&mut self, track: Track, source: AudioSource) -> Result<(), PlayerError> {
        let job = self.jobs.front_mut().expect("the job is running");
        let title = track.title.clone();
        let item = match (job.how, job.started) {
            (Queueing::Replace, false) => {
                let item = self.player.play(source)?;
                job.started = true;
                item
            }
            (Queueing::Replace, true) => self.player.append(source)?,
            (Queueing::Append, _) => {
                let item = self.player.append(source)?;
                self.status = info(format!("Queued “{title}”"));
                item
            }
            (Queueing::Next, _) => {
                let item = self.player.insert_next(source)?;
                self.status = info(format!("“{title}” plays next"));
                item
            }
        };
        self.now.tracks.insert(item, track);
        Ok(())
    }

    /// Reads the player's status, and forgets the tracks of the items no
    /// longer in its queue.
    pub(super) fn refresh(&mut self) {
        match self.player.status() {
            Ok(status) => {
                let items: HashSet<QueueItemId> = status.items.iter().copied().collect();
                self.now.tracks.retain(|item, _| items.contains(item));
                if let Some(view) = &mut self.queue_view {
                    view.selected = view.selected.min(status.items.len().saturating_sub(1));
                }
                self.now.status = status;
            }
            Err(failed) => self.status = error(failed.to_string()),
        }
    }

    /// Gives the player a command, then reads its status.
    fn command(&mut self, command: impl FnOnce(&mut dyn Playback) -> Result<(), PlayerError>) {
        match command(self.player.as_mut()) {
            Ok(()) => self.refresh(),
            Err(failed) => self.status = error(failed.to_string()),
        }
    }

    pub(super) fn toggle_pause(&mut self) {
        self.command(|player| player.toggle());
    }

    pub(super) fn skip(&mut self, forward: bool) {
        if forward {
            self.command(|player| player.next());
        } else {
            self.command(|player| player.previous());
        }
    }

    /// Seeks `by` forward or back in the current item. Seeking past its end
    /// skips to the next item.
    pub(super) fn seek_by(&mut self, by: Duration, forward: bool) {
        if self.now.status.state == PlaybackState::Stopped {
            return;
        }
        let (Some(item), Some((_, elapsed, total))) = (self.now.status.current, self.now.current())
        else {
            return;
        };
        let target = if forward {
            elapsed + by
        } else {
            elapsed.saturating_sub(by)
        };
        if forward && !total.is_zero() && target >= total {
            self.skip(true);
            return;
        }
        // Keys pressed before the player reports the new position count
        // from it.
        let position = self.now.position.filter(|position| position.item == item);
        self.now.position = Some(Position {
            item,
            elapsed: target,
            total: position.and_then(|position| position.total),
        });
        self.command(|player| player.seek(target));
    }

    pub(super) fn change_volume(&mut self, up: bool) {
        let volume = if up {
            self.now.volume.saturating_add(VOLUME_STEP).min(100)
        } else {
            self.now.volume.saturating_sub(VOLUME_STEP)
        };
        match self.player.set_volume(f32::from(volume) / 100.0) {
            Ok(()) => {
                self.now.volume = volume;
                self.status = info(format!("Volume {volume}%"));
            }
            Err(failed) => self.status = error(failed.to_string()),
        }
    }

    /// Opens the queue view on the current item, or closes it.
    pub(super) fn toggle_queue(&mut self) {
        if self.queue_view.take().is_some() {
            return;
        }
        self.refresh();
        let status = &self.now.status;
        let selected = status
            .current
            .and_then(|current| status.items.iter().position(|item| *item == current))
            .unwrap_or(0);
        self.queue_view = Some(QueueView { selected });
    }

    /// The item selected in the queue view.
    fn selected_queued(&self) -> Option<(usize, QueueItemId)> {
        let selected = self.queue_view?.selected;
        let item = *self.now.status.items.get(selected)?;
        Some((selected, item))
    }

    /// Moves the queue view's selection with `to`, which gets the current
    /// selection and the length of the queue.
    pub(super) fn move_queue_selection(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        let len = self.now.status.items.len();
        if let Some(view) = &mut self.queue_view {
            view.selected = to(view.selected, len).min(len.saturating_sub(1));
        }
    }

    pub(super) fn remove_queued(&mut self) {
        if let Some((_, item)) = self.selected_queued() {
            self.command(|player| player.remove(item));
        }
    }

    /// Moves the selected item one place down or up. At the end it moves
    /// towards, it stays.
    pub(super) fn move_queued(&mut self, down: bool) {
        let Some((from, item)) = self.selected_queued() else {
            return;
        };
        let to = if down {
            from.checked_add(1)
        } else {
            from.checked_sub(1)
        };
        let Some(to) = to.filter(|&to| to < self.now.status.items.len()) else {
            return;
        };
        self.command(|player| player.move_to(item, to));
        if let Some(view) = &mut self.queue_view {
            view.selected = to;
        }
    }

    /// Records the play of the item that started last once `elapsed`
    /// reaches half of it or [`PLAY_THRESHOLD`].
    fn check_play(
        &mut self,
        item: QueueItemId,
        elapsed: Duration,
        total: Option<Duration>,
    ) -> Vec<Request> {
        let Some((started, recorded)) = &mut self.now.started else {
            return Vec::new();
        };
        if *started != item || *recorded {
            return Vec::new();
        }
        let Some(track) = self.now.tracks.get(&item) else {
            return Vec::new();
        };
        let total = total.or(Some(track.duration).filter(|total| !total.is_zero()));
        let threshold = total.map_or(PLAY_THRESHOLD, |total| (total / 2).min(PLAY_THRESHOLD));
        if elapsed < threshold {
            return Vec::new();
        }
        *recorded = true;
        self.recording += 1;
        vec![Request::RecordPlay {
            track: track.clone(),
            device: self.device.clone(),
            at: SystemTime::now(),
        }]
    }
}

/// Applies what the player reported, and returns the play to record, if
/// any.
pub fn player_event(app: &mut App, event: PlayerEvent) -> Vec<Request> {
    match event {
        PlayerEvent::TrackStarted { item } => {
            app.now.started = Some((item, false));
            app.now.position = Some(Position {
                item,
                elapsed: Duration::ZERO,
                total: None,
            });
            app.refresh();
        }
        PlayerEvent::Position {
            item,
            elapsed,
            total,
        } => {
            app.now.position = Some(Position {
                item,
                elapsed,
                total,
            });
            return app.check_play(item, elapsed, total);
        }
        PlayerEvent::Error {
            item,
            error: failed,
        } => {
            let track = item.and_then(|item| app.now.tracks.get(&item));
            app.status = error(match track {
                Some(track) => format!("Cannot play “{}”: {failed}", track.title),
                None => failed.to_string(),
            });
            app.refresh();
        }
        _ => app.refresh(),
    }
    Vec::new()
}
