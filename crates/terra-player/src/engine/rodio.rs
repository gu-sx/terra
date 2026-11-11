//! The Rodio engine, as `docs/decisions/0002-playback-engine.md` recommends:
//! Rodio's decoders for MP3 and FLAC, [`OpusSource`] for Opus, and cpal's
//! default ALSA output. AAC and ALAC in M4A play through [`Mp4Source`],
//! which applies the edit list that Rodio's decoder ignores.

use std::fs::File;
use std::io::{Read, Seek};
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rodio::queue::SourcesQueueOutput;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source};
use terra_core::AudioSource;

use super::deck::{Deck, DeckControl, Format, Track};
use super::mp4::Mp4Source;
use super::opus::OpusSource;
use super::{Engine, EventSink};
use crate::{PlayerError, QueueItemId};

pub(crate) struct RodioEngine {
    /// Rodio's player, with the deck as its only source: it applies pause
    /// and gain.
    player: rodio::Player,
    deck: DeckControl,
    /// Whatever pulls the output; dropped with the engine.
    _output: OutputGuard,
}

enum OutputGuard {
    /// The caller pulls the output.
    None,
    /// Kept for its stream: dropping it stops the device.
    Device { _sink: MixerDeviceSink },
    /// A thread that pulls the output in real time and discards it.
    Null(Arc<AtomicBool>),
}

impl Drop for OutputGuard {
    fn drop(&mut self) {
        if let OutputGuard::Null(stop) = self {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl RodioEngine {
    /// An engine whose output, in `format`, the caller pulls as the device
    /// would: the spike's capture method.
    pub(crate) fn capture(events: EventSink, format: Format) -> (RodioEngine, SourcesQueueOutput) {
        let (player, output) = rodio::Player::new();
        let engine = Self::with_player(player, events, format, OutputGuard::None);
        (engine, output)
    }

    /// An engine that plays on the default output device.
    pub(crate) fn device(events: EventSink) -> Result<RodioEngine, PlayerError> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| PlayerError::Output(e.to_string()))?;
        sink.log_on_drop(false);
        let format = Format {
            channels: sink.config().channel_count(),
            rate: sink.config().sample_rate(),
        };
        let player = rodio::Player::connect_new(sink.mixer());
        let output = OutputGuard::Device { _sink: sink };
        Ok(Self::with_player(player, events, format, output))
    }

    /// An engine that plays in real time to no device.
    pub(crate) fn null(events: EventSink) -> Result<RodioEngine, PlayerError> {
        let (player, output) = rodio::Player::new();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("terra-player-null".to_owned())
            .spawn(move || pull_in_real_time(output, &stopped))
            .map_err(|e| PlayerError::Output(e.to_string()))?;
        let format = Format {
            channels: NonZero::new(2).unwrap(),
            rate: NonZero::new(48_000).unwrap(),
        };
        Ok(Self::with_player(
            player,
            events,
            format,
            OutputGuard::Null(stop),
        ))
    }

    fn with_player(
        player: rodio::Player,
        events: EventSink,
        format: Format,
        output: OutputGuard,
    ) -> RodioEngine {
        let (deck, control) = Deck::new(events, format);
        player.append(deck);
        RodioEngine {
            player,
            deck: control,
            _output: output,
        }
    }
}

/// Pulls `output` at the pace a device would, 10 ms at a time, until `stop`.
fn pull_in_real_time(mut output: SourcesQueueOutput, stop: &AtomicBool) {
    let start = Instant::now();
    let mut played = Duration::ZERO;
    while !stop.load(Ordering::Relaxed) {
        let rate = output.sample_rate().get();
        let frames = rate / 100;
        let samples = frames as usize * usize::from(output.channels().get());
        output.by_ref().take(samples).for_each(drop);
        played += super::frames_to_duration(u64::from(frames), rate);
        if let Some(ahead) = played.checked_sub(start.elapsed()) {
            thread::sleep(ahead);
        }
    }
}

impl Engine for RodioEngine {
    fn load(&mut self, item: QueueItemId, source: &AudioSource) -> Result<(), PlayerError> {
        let source = open(source)?;
        self.deck.load(Track { item, source });
        Ok(())
    }

    fn set_next(&mut self, next: Option<(QueueItemId, &AudioSource)>) -> Result<(), PlayerError> {
        let track = match next {
            Some((item, source)) => Some(Track {
                item,
                source: open(source)?,
            }),
            None => None,
        };
        self.deck.set_next(track);
        Ok(())
    }

    fn clear(&mut self) {
        self.deck.clear();
    }

    fn play(&mut self) {
        self.player.play();
    }

    fn pause(&mut self) {
        self.player.pause();
    }

    fn seek(&mut self, position: Duration) {
        self.deck.seek(position);
    }

    fn set_gain(&mut self, gain: f32) {
        self.player.set_volume(gain);
    }
}

/// Opens a source for playback: Opus through [`OpusSource`], MP4 through
/// [`Mp4Source`], everything else through Rodio's decoder.
fn open(source: &AudioSource) -> Result<Box<dyn Source + Send>, PlayerError> {
    let path = match source {
        AudioSource::LocalPath(path) => path,
        // The URL itself stays out of the message: it can carry credentials.
        AudioSource::Url(_) => {
            return Err(PlayerError::Unsupported(
                "stream URLs cannot be played yet".to_owned(),
            ));
        }
    };
    let failed = |message: String| PlayerError::Decode(format!("{}: {message}", path.display()));
    if has_extension(path, "opus") {
        return OpusSource::open(path)
            .map(|s| Box::new(s) as Box<dyn Source + Send>)
            .map_err(failed);
    }
    let mut file = File::open(path).map_err(|e| failed(e.to_string()))?;
    if is_mp4(&mut file).map_err(|e| failed(e.to_string()))? {
        return Mp4Source::open(path)
            .map(|s| Box::new(s) as Box<dyn Source + Send>)
            .map_err(failed);
    }
    match Decoder::try_from(file) {
        Ok(decoder) => Ok(Box::new(decoder)),
        // An Ogg file with another extension may hold Opus, which Rodio's
        // decoder cannot read.
        Err(error) => match OpusSource::open(path) {
            Ok(opus) => Ok(Box::new(opus)),
            Err(_) => Err(failed(error.to_string())),
        },
    }
}

/// Whether `file` is an ISO media file, such as M4A: its first box is
/// `ftyp`. Leaves `file` at its start.
fn is_mp4(file: &mut File) -> std::io::Result<bool> {
    let mut header = [0; 8];
    let read = file.read_exact(&mut header);
    file.rewind()?;
    Ok(read.is_ok() && &header[4..] == b"ftyp")
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests;
