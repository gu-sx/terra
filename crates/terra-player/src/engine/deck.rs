//! The deck: one endless Rodio [`Source`] that plays the current item and
//! moves to the next one at the exact sample where the current one ends.
//!
//! Rodio's own queue cannot replace an item once it is appended, and its
//! `Player::clear` and `Player::try_seek` block until the output has been
//! pulled. The deck takes their place: the engine posts operations to it, and
//! the deck applies them on the output's thread every [`TICK_FRAMES`]
//! frames, in order. Because it is a single source, the item after the
//! current one can be swapped at any time before the join, and the join
//! stays gapless.
//!
//! The deck plays in one fixed format, the output's, and converts each item
//! to it with one converter for the whole item. Rodio's mixer starts a new
//! sample-rate converter for every span of its input and loses audio at each
//! boundary; a source in the mixer's own format passes through it
//! unconverted, so the deck reports a single endless span and Rodio never
//! converts again.

use std::mem;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source;
use rodio::conversions::{ChannelCountConverter, SampleRateConverter};

use super::{EngineEvent, EventSink, duration_to_frames, frames_to_duration};
use crate::{PlayerError, QueueItemId};

/// Frames between two applications of the posted operations: 10 ms at
/// 48 kHz. Operations wait at most this long.
const TICK_FRAMES: usize = 480;

/// How often the deck reports the position of the current item, in
/// milliseconds of its audio.
const POSITION_EVERY_MS: u64 = 100;

/// A decoded item, ready to play.
pub(crate) struct Track {
    pub(crate) item: QueueItemId,
    pub(crate) source: Box<dyn Source + Send>,
}

enum Op {
    Load(Track),
    SetNext(Option<Track>),
    Clear,
    Seek(Duration),
}

#[derive(Default)]
struct Shared {
    ops: Mutex<Vec<Op>>,
    pending: AtomicBool,
}

/// Posts operations to a [`Deck`] from another thread.
pub(crate) struct DeckControl {
    shared: Arc<Shared>,
}

impl DeckControl {
    pub(crate) fn load(&self, track: Track) {
        self.post(Op::Load(track));
    }

    pub(crate) fn set_next(&self, track: Option<Track>) {
        self.post(Op::SetNext(track));
    }

    pub(crate) fn clear(&self) {
        self.post(Op::Clear);
    }

    pub(crate) fn seek(&self, position: Duration) {
        self.post(Op::Seek(position));
    }

    fn post(&self, op: Op) {
        self.shared.ops.lock().unwrap().push(op);
        self.shared.pending.store(true, Ordering::Release);
    }
}

/// The output's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Format {
    pub(crate) channels: NonZero<u16>,
    pub(crate) rate: NonZero<u32>,
}

type Converted = ChannelCountConverter<SampleRateConverter<Box<dyn Source + Send>>>;

/// Converts `source` to `to`, with one converter for all of it.
fn convert(source: Box<dyn Source + Send>, to: Format) -> Converted {
    let (channels, rate) = (source.channels(), source.sample_rate());
    let resampled = SampleRateConverter::new(source, rate, to.rate, channels);
    ChannelCountConverter::new(resampled, channels, to.channels)
}

/// The current item and how far it has played.
struct Playing {
    item: QueueItemId,
    /// The item, converted to the output's format. `None` only during a seek.
    output: Option<Converted>,
    /// The item's own sample rate.
    rate: NonZero<u32>,
    total: Option<Duration>,
    /// Where `samples` was last zero: the start, or where a seek landed.
    base: Duration,
    /// Output samples played since `base`.
    samples: u64,
    /// Whether [`EngineEvent::Started`] has been sent.
    started: bool,
    /// `samples` when the position was last reported, or `None` to report
    /// it at the next tick.
    reported_at: Option<u64>,
}

impl Playing {
    fn new(track: Track, format: Format) -> Self {
        Self {
            item: track.item,
            rate: track.source.sample_rate(),
            total: track.source.total_duration(),
            output: Some(convert(track.source, format)),
            base: Duration::ZERO,
            samples: 0,
            started: false,
            reported_at: Some(0),
        }
    }

    fn elapsed(&self, format: Format) -> Duration {
        let frames = self.samples / u64::from(format.channels.get());
        self.base + frames_to_duration(frames, format.rate.get())
    }
}

/// The deck itself: append it once to a Rodio `Player` whose output has
/// the deck's format.
pub(crate) struct Deck {
    shared: Arc<Shared>,
    events: EventSink,
    format: Format,
    current: Option<Playing>,
    next: Option<Track>,
    /// Samples left until the posted operations are applied again.
    tick_left: usize,
    /// Output samples played since the start, to keep frames whole.
    played: u64,
    /// Silent samples that finish a frame an item left incomplete.
    padding: usize,
}

impl Deck {
    pub(crate) fn new(events: EventSink, format: Format) -> (Deck, DeckControl) {
        let shared = Arc::new(Shared::default());
        let deck = Deck {
            shared: shared.clone(),
            events,
            format,
            current: None,
            next: None,
            tick_left: 0,
            played: 0,
            padding: 0,
        };
        (deck, DeckControl { shared })
    }

    /// Applies the posted operations, in order.
    fn apply_ops(&mut self) {
        if !self.shared.pending.swap(false, Ordering::Acquire) {
            return;
        }
        let ops = mem::take(&mut *self.shared.ops.lock().unwrap());
        for op in ops {
            match op {
                Op::Load(track) => {
                    self.current = Some(Playing::new(track, self.format));
                    self.next = None;
                }
                Op::SetNext(track) => {
                    self.next = track;
                    if self.current.is_none() {
                        self.start_next();
                    }
                }
                Op::Clear => {
                    self.current = None;
                    self.next = None;
                }
                Op::Seek(position) => self.seek(position),
            }
        }
    }

    fn start_next(&mut self) {
        let format = self.format;
        self.current = self.next.take().map(|track| Playing::new(track, format));
    }

    fn seek(&mut self, position: Duration) {
        let format = self.format;
        let Some(playing) = self.current.as_mut() else {
            return;
        };
        // Rodio's decoders stop at the end; so does the reported position.
        let position = playing.total.map_or(position, |total| position.min(total));
        // The converters hold samples from before the seek: start new ones.
        let mut source = playing
            .output
            .take()
            .expect("an item has its output outside a seek")
            .into_inner()
            .into_inner();
        let result = source.try_seek(position);
        playing.output = Some(convert(source, format));
        match result {
            Ok(()) => {
                let frames = duration_to_frames(position, playing.rate.get());
                playing.base = frames_to_duration(frames, playing.rate.get());
                playing.samples = 0;
                playing.reported_at = None;
            }
            Err(error) => self.events.send(EngineEvent::Error {
                item: Some(playing.item),
                error: PlayerError::Seek(error.to_string()),
            }),
        }
    }

    /// Reports the position of the current item when it is due.
    fn report_position(&mut self) {
        let format = self.format;
        let Some(playing) = self.current.as_mut() else {
            return;
        };
        let every =
            u64::from(format.rate.get()) * u64::from(format.channels.get()) * POSITION_EVERY_MS
                / 1000;
        let due = match playing.reported_at {
            None => true,
            Some(at) => playing.samples - at >= every,
        };
        if playing.started && due {
            playing.reported_at = Some(playing.samples);
            self.events.send(EngineEvent::Position {
                item: playing.item,
                elapsed: playing.elapsed(format),
                total: playing.total,
            });
        }
    }

    /// Applies the posted operations and reports the position.
    fn tick(&mut self) {
        self.tick_left = TICK_FRAMES * usize::from(self.format.channels.get());
        self.apply_ops();
        self.report_position();
    }

    /// The current item has played its last sample: report it and move to
    /// the next one.
    fn end_current(&mut self) {
        let ended = self.current.take().expect("an item is playing");
        self.events.send(EngineEvent::Ended(ended.item));
        // A load or next item posted just now still makes the join.
        self.apply_ops();
        if self.current.is_none() {
            self.start_next();
        }
        // An item that stopped within a frame: finish the frame.
        let channels = u64::from(self.format.channels.get());
        self.padding = ((channels - self.played % channels) % channels) as usize;
    }

    fn next_sample(&mut self) -> f32 {
        loop {
            if self.tick_left == 0 {
                self.tick();
            }
            if self.padding > 0 {
                self.padding -= 1;
                return self.emit(0.0);
            }
            let Some(playing) = self.current.as_mut() else {
                return self.emit(0.0);
            };
            if !playing.started {
                playing.started = true;
                self.events.send(EngineEvent::Started(playing.item));
            }
            let output = playing
                .output
                .as_mut()
                .expect("an item has its output outside a seek");
            match output.next() {
                Some(sample) => {
                    playing.samples += 1;
                    return self.emit(sample);
                }
                None => self.end_current(),
            }
        }
    }

    fn emit(&mut self, sample: f32) -> f32 {
        self.tick_left -= 1;
        self.played += 1;
        sample
    }
}

impl Iterator for Deck {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        Some(self.next_sample())
    }
}

impl Source for Deck {
    /// One endless span: the format never changes.
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZero<u16> {
        self.format.channels
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.format.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests;
