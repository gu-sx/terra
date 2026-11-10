//! The deck: one endless Rodio [`Source`] that plays the current item and
//! moves to the next one at the exact sample where the current one ends.
//!
//! Rodio's own queue cannot replace an item once it is appended, and its
//! `Player::clear` and `Player::try_seek` block until the output has been
//! pulled. The deck takes their place: the engine posts operations to it, and
//! the deck applies them on the output's thread, between spans, in order.
//! Because it is a single source, the item after the current one can be
//! swapped at any time before the join, and the join stays gapless.
//!
//! The deck splits its output into spans of [`SPAN_FRAMES`] frames, after
//! each of which Rodio rereads the channel count and sample rate. A track
//! starts within a span when its format matches the span's; otherwise the
//! deck pads the span with silence, which only happens where gapless playback
//! is impossible anyway.

use std::cell::RefCell;
use std::mem;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source;

use super::{EngineEvent, EventSink, duration_to_frames, frames_to_duration};
use crate::{PlayerError, QueueItemId};

/// Frames per span: 10 ms at 48 kHz. Operations wait at most this long.
const SPAN_FRAMES: usize = 480;

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

/// The current item and how far it has played.
struct Playing {
    item: QueueItemId,
    source: Box<dyn Source + Send>,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    total: Option<Duration>,
    /// The frame the source stood at when `samples` was last zero: zero, or
    /// where a seek landed.
    base_frames: u64,
    /// Samples played since `base_frames`.
    samples: u64,
    /// Whether [`EngineEvent::Started`] has been sent.
    started: bool,
    /// `samples` when the position was last reported, or `None` to report
    /// it at the next span.
    reported_at: Option<u64>,
}

impl Playing {
    fn new(track: Track) -> Self {
        Self {
            item: track.item,
            channels: track.source.channels(),
            rate: track.source.sample_rate(),
            total: track.source.total_duration(),
            source: track.source,
            base_frames: 0,
            samples: 0,
            started: false,
            reported_at: Some(0),
        }
    }

    fn elapsed(&self) -> Duration {
        let frames = self.base_frames + self.samples / u64::from(self.channels.get());
        frames_to_duration(frames, self.rate.get())
    }
}

struct State {
    shared: Arc<Shared>,
    events: EventSink,
    current: Option<Playing>,
    next: Option<Track>,
    /// The format of the span being played.
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    /// The span's length, and the samples left in it. A new span begins when
    /// none are left and the output asks for a sample or for the span.
    span_len: usize,
    span_left: usize,
    /// Silent samples to play before the current item, to finish a span in
    /// the format of the item before it.
    padding: usize,
}

/// The deck itself: append it once to a Rodio `Player`.
pub(crate) struct Deck {
    state: RefCell<State>,
}

impl Deck {
    pub(crate) fn new(events: EventSink) -> (Deck, DeckControl) {
        let shared = Arc::new(Shared::default());
        let state = State {
            shared: shared.clone(),
            events,
            current: None,
            next: None,
            channels: NonZero::new(2).unwrap(),
            rate: NonZero::new(48_000).unwrap(),
            span_len: 0,
            span_left: 0,
            padding: 0,
        };
        let deck = Deck {
            state: RefCell::new(state),
        };
        (deck, DeckControl { shared })
    }
}

impl State {
    /// Applies the posted operations, in order.
    fn apply_ops(&mut self) {
        if !self.shared.pending.swap(false, Ordering::Acquire) {
            return;
        }
        let ops = mem::take(&mut *self.shared.ops.lock().unwrap());
        for op in ops {
            match op {
                Op::Load(track) => {
                    self.current = Some(Playing::new(track));
                    self.next = None;
                }
                Op::SetNext(track) => {
                    self.next = track;
                    if self.current.is_none() {
                        self.current = self.next.take().map(Playing::new);
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

    fn seek(&mut self, position: Duration) {
        let Some(playing) = self.current.as_mut() else {
            return;
        };
        match playing.source.try_seek(position) {
            Ok(()) => {
                playing.base_frames = duration_to_frames(position, playing.rate.get());
                playing.samples = 0;
                playing.reported_at = None;
            }
            Err(error) => self.events.send(EngineEvent::Error {
                item: Some(playing.item),
                error: PlayerError::Seek(error.to_string()),
            }),
        }
    }

    /// Starts a span: applies the posted operations, reports the position
    /// when it is due and takes the format of the current item.
    fn begin_span(&mut self) {
        self.apply_ops();
        if let Some(playing) = self.current.as_mut() {
            let every = u64::from(playing.rate.get())
                * u64::from(playing.channels.get())
                * POSITION_EVERY_MS
                / 1000;
            let due = match playing.reported_at {
                None => true,
                Some(at) => playing.samples - at >= every,
            };
            if playing.started && due {
                playing.reported_at = Some(playing.samples);
                self.events.send(EngineEvent::Position {
                    item: playing.item,
                    elapsed: playing.elapsed(),
                    total: playing.total,
                });
            }
            self.channels = playing.channels;
            self.rate = playing.rate;
        }
        self.span_len = SPAN_FRAMES * usize::from(self.channels.get());
        self.span_left = self.span_len;
    }

    fn ensure_span(&mut self) {
        if self.span_left == 0 {
            self.begin_span();
        }
    }

    /// The current item has played its last sample: report it and move to
    /// the next one, within the span when the format allows.
    fn end_current(&mut self) {
        let ended = self.current.take().expect("an item is playing");
        self.events.send(EngineEvent::Ended(ended.item));
        // A load or next item posted just now still makes the join.
        self.apply_ops();
        if self.current.is_none() {
            self.current = self.next.take().map(Playing::new);
        }
        let played = self.span_len - self.span_left;
        let channels = usize::from(self.channels.get());
        self.padding = match &self.current {
            Some(next) if (next.channels, next.rate) != (self.channels, self.rate) => {
                self.span_left
            }
            // A source that stopped within a frame: finish the frame.
            _ => (channels - played % channels) % channels,
        };
    }

    fn next_sample(&mut self) -> f32 {
        loop {
            self.ensure_span();
            if self.padding > 0 {
                self.padding -= 1;
                self.span_left -= 1;
                return 0.0;
            }
            let Some(playing) = self.current.as_mut() else {
                self.span_left -= 1;
                return 0.0;
            };
            if !playing.started {
                playing.started = true;
                self.events.send(EngineEvent::Started(playing.item));
            }
            match playing.source.next() {
                Some(sample) => {
                    playing.samples += 1;
                    self.span_left -= 1;
                    return sample;
                }
                None => self.end_current(),
            }
        }
    }
}

impl Iterator for Deck {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        Some(self.state.get_mut().next_sample())
    }
}

impl Source for Deck {
    // Rodio asks for the span's format between spans; the deck begins the
    // span then, so the answer holds for every sample of it.
    fn current_span_len(&self) -> Option<usize> {
        let mut state = self.state.borrow_mut();
        state.ensure_span();
        Some(state.span_len)
    }

    fn channels(&self) -> NonZero<u16> {
        let mut state = self.state.borrow_mut();
        state.ensure_span();
        state.channels
    }

    fn sample_rate(&self) -> NonZero<u32> {
        let mut state = self.state.borrow_mut();
        state.ensure_span();
        state.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
