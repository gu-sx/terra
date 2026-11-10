//! The engine trait the player drives, and what an engine reports back.

pub(crate) mod deck;
pub(crate) mod opus;
pub(crate) mod rodio;

use std::sync::Arc;
use std::time::Duration;

use terra_core::AudioSource;

use crate::{PlayerError, QueueItemId};

/// One playback engine. The player owns it on its own thread and calls it
/// there; an engine reports through the [`EventSink`] it was built with.
///
/// The shape follows `docs/decisions/0002-playback-engine.md`, with
/// `enqueue` split into [`load`](Engine::load) and
/// [`set_next`](Engine::set_next), so that the player can replace the item it
/// handed over for the gapless join without touching the current one.
pub(crate) trait Engine {
    /// Starts `source` in place of whatever plays, and drops the next item.
    /// The engine reports [`EngineEvent::Started`] when its first sample plays.
    fn load(&mut self, item: QueueItemId, source: &AudioSource) -> Result<(), PlayerError>;

    /// Sets the item that starts the moment the current one ends, replacing
    /// the one set before; `None` drops it. With nothing playing, the engine
    /// starts it at once. On an error the earlier next item is kept.
    fn set_next(&mut self, next: Option<(QueueItemId, &AudioSource)>) -> Result<(), PlayerError>;

    /// Drops the current and the next item.
    fn clear(&mut self);

    /// Plays: the counterpart of [`pause`](Engine::pause).
    fn play(&mut self);

    fn pause(&mut self);

    /// Seeks within the current item. The engine reports a failure as an
    /// [`EngineEvent::Error`], and success with a [`EngineEvent::Position`]
    /// at the new position before any later one.
    fn seek(&mut self, position: Duration);

    /// Linear gain, 0.0 to 1.0. The player maps its volume control onto it.
    fn set_gain(&mut self, gain: f32);
}

/// What an engine reports, in the order it happens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EngineEvent {
    /// The item's first sample has been played.
    Started(QueueItemId),
    /// The item's last sample has been played.
    Ended(QueueItemId),
    Position {
        item: QueueItemId,
        elapsed: Duration,
        total: Option<Duration>,
    },
    Error {
        item: Option<QueueItemId>,
        error: PlayerError,
    },
}

/// Where an engine sends its events. Cloning it gives another handle to the
/// same receiver.
#[derive(Clone)]
pub(crate) struct EventSink(Arc<dyn Fn(EngineEvent) + Send + Sync>);

impl EventSink {
    pub(crate) fn new(send: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(send))
    }

    pub(crate) fn send(&self, event: EngineEvent) {
        (self.0)(event)
    }
}

/// The time `frames` frames last at `rate` frames per second, exact to the
/// nanosecond below.
pub(crate) fn frames_to_duration(frames: u64, rate: u32) -> Duration {
    let rate = u64::from(rate);
    let nanos = (frames % rate) * 1_000_000_000 / rate;
    Duration::new(frames / rate, nanos as u32)
}

/// The frame nearest to `position` at `rate` frames per second. Rounds, so
/// that a position a hair under a frame, such as one made by
/// `Duration::from_secs_f64`, does not land a frame early.
pub(crate) fn duration_to_frames(position: Duration, rate: u32) -> u64 {
    let nanos = position.as_nanos() * u128::from(rate);
    ((nanos + 500_000_000) / 1_000_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_durations_round_trip() {
        assert_eq!(frames_to_duration(72_517, 48_000).as_nanos(), 1_510_770_833);
        assert_eq!(
            duration_to_frames(Duration::from_millis(500), 48_000),
            24_000
        );
        // Duration::from_secs_f64 truncates 1234.567 s to just under the
        // sample; rounding recovers it (the record's seek finding).
        let target = Duration::from_secs_f64(1234.567);
        assert_eq!(duration_to_frames(target, 48_000), 59_259_216);
        for frames in [0, 1, 47_999, 48_000, 72_517, 59_259_216] {
            let d = frames_to_duration(frames, 48_000);
            assert_eq!(duration_to_frames(d, 48_000), frames);
        }
    }
}
