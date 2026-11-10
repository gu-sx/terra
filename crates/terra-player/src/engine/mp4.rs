//! MP4 audio (AAC and ALAC in M4A) with the track's edit list applied.
//!
//! Symphonia 0.5's MP4 reader parses the edit list but neither applies nor
//! exposes it, so Rodio's decoder plays an AAC file's priming samples and
//! padding: 1024 samples early and with a gap at every join
//! (`docs/decisions/0002-playback-engine.md`, "AAC in M4A"). This source
//! reads the edit list from the file itself, drives Symphonia directly and
//! plays only the samples the edit presents. Packets carry their timestamps,
//! so trimming and seeking both work on the track's own timeline.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::{duration_to_frames, frames_to_duration};

/// Frames decoded and dropped before a seek target. An AAC frame overlaps
/// the one before it, so the first frame decoded after a seek is wrong;
/// 4096 covers that for AAC-LC (1024 frames) and HE-AAC (2048).
const PREROLL_FRAMES: u64 = 4096;

/// The largest `moov` box read. Real ones are kilobytes to a few megabytes.
const MOOV_LIMIT: u64 = 64 << 20;

pub(crate) struct Mp4Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    /// The track's timescale: packet timestamps count in it.
    timescale: u32,
    buf: Vec<f32>,
    pos: usize,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    /// The frames the edit list presents, as decoded frames from the
    /// track's start: `start..end`, or to the last decoded one.
    start: u64,
    end: Option<u64>,
    /// The first decoded frame still to play: `start`, or where a seek
    /// landed.
    from: u64,
}

impl Mp4Source {
    pub(crate) fn open(path: &Path) -> Result<Mp4Source, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        // A file whose boxes cannot be read plays as Rodio's decoder would
        // play it: untrimmed.
        let tracks = read_tracks(&mut file).unwrap_or_default();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| e.to_string())?;
        let format = probed.format;
        // The track Rodio's decoder picks: the first with a known codec.
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "no audio track".to_owned())?;
        let params = track.codec_params.clone();
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        // The MP4 reader leaves AAC's channel count to the decoder: the first
        // packet decoded sets it.
        let channels = params
            .channels
            .and_then(|c| u16::try_from(c.count()).ok())
            .and_then(NonZero::new)
            .unwrap_or(NonZero::<u16>::MIN);
        let rate = params
            .sample_rate
            .and_then(NonZero::new)
            .ok_or_else(|| "no sample rate".to_owned())?;
        let timescale = params
            .time_base
            .filter(|base| base.numer == 1)
            .map(|base| base.denom)
            .ok_or_else(|| "no timescale".to_owned())?;
        // Symphonia numbers the tracks in the order of their `trak` boxes.
        let window = usize::try_from(track.id)
            .ok()
            .and_then(|i| tracks.get(i))
            .and_then(|timing| timing.window(rate.get()));
        // The media's length, which fragmented files give as 0.
        let media_end = params
            .n_frames
            .filter(|&n| n > 0)
            .map(|n| rescale(n, timescale, rate.get()));
        let (start, end) = match window {
            Some((start, length)) => (start, length.map(|l| start + l).or(media_end)),
            None => (0, media_end),
        };
        let mut source = Mp4Source {
            track: track.id,
            format,
            decoder,
            timescale,
            buf: Vec::new(),
            pos: 0,
            channels,
            rate,
            start,
            end,
            from: start,
        };
        // Decode the first packet now, so a file that cannot be decoded
        // fails here.
        source.refill()?;
        Ok(source)
    }

    /// Decodes packets until one has frames to play, and keeps those in
    /// `buf`. `Ok(false)` at the end.
    fn refill(&mut self) -> Result<bool, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                // Symphonia reports the end of the file as an I/O error.
                Err(SymphoniaError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let first = rescale(packet.ts(), self.timescale, self.rate.get());
            if self.end.is_some_and(|end| first >= end) {
                return Ok(false);
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // A damaged packet is skipped, as Rodio's decoder does.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let frames = decoded.frames() as u64;
            self.channels = u16::try_from(decoded.spec().channels.count())
                .ok()
                .and_then(NonZero::new)
                .ok_or_else(|| "no channel count".to_owned())?;
            let channels = usize::from(self.channels.get());
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            samples.copy_interleaved_ref(decoded);
            let from = self.from.max(first);
            let to = self
                .end
                .map_or(first + frames, |end| end.min(first + frames));
            if from >= to {
                continue;
            }
            let range = (from - first) as usize * channels..(to - first) as usize * channels;
            self.buf.clear();
            self.buf.extend_from_slice(&samples.samples()[range]);
            self.pos = 0;
            return Ok(true);
        }
    }
}

impl Iterator for Mp4Source {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        // A read error mid-stream ends the item, as in Rodio's decoder.
        if self.pos == self.buf.len() && !self.refill().unwrap_or(false) {
            return None;
        }
        self.pos += 1;
        Some(self.buf[self.pos - 1])
    }
}

impl Source for Mp4Source {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.buf.len())
    }

    fn channels(&self) -> NonZero<u16> {
        self.channels
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.end
            .map(|end| frames_to_duration(end.saturating_sub(self.start), self.rate.get()))
    }

    /// Seeks to the frame nearest `pos`. The demuxer seeks by timestamp to
    /// [`PREROLL_FRAMES`] before the target, and the frames before the
    /// target are decoded and dropped, so the first frame played is the
    /// target's and was decoded with the frame before it.
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let mut target = self.start + duration_to_frames(pos, self.rate.get());
        if let Some(end) = self.end {
            target = target.min(end);
        }
        let ts = rescale(
            target.saturating_sub(PREROLL_FRAMES),
            self.rate.get(),
            self.timescale,
        );
        let to = SeekTo::TimeStamp {
            ts,
            track_id: self.track,
        };
        self.format
            .seek(SeekMode::Accurate, to)
            .map_err(|e| SeekError::Other(Arc::new(e)))?;
        self.decoder.reset();
        self.buf.clear();
        self.pos = 0;
        self.from = target;
        Ok(())
    }
}

/// `value` counted at `from` units per second, in units of `to` per second,
/// rounded to the nearest.
fn rescale(value: u64, from: u32, to: u32) -> u64 {
    if from == to {
        return value;
    }
    let (from, to) = (u128::from(from), u128::from(to));
    ((u128::from(value) * to + from / 2) / from) as u64
}

/// What the `moov` box says about one track's timing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TrackTiming {
    /// The movie's timescale, in which edit durations count.
    movie_timescale: u32,
    /// The track's own timescale, in which media times count.
    timescale: u32,
    /// The edit list, reduced to the one edit terra applies.
    edits: Edits,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Edits {
    /// No edit list: play the whole media.
    #[default]
    None,
    /// One edit that plays the media at normal rate, after any number of
    /// empty edits.
    One(Edit),
    /// An edit list this source does not apply: several media edits, or one
    /// at another rate.
    Unsupported,
}

/// One edit of the edit list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edit {
    /// Where the edit starts, in the track's timescale.
    media_time: u64,
    /// How long it lasts, in the movie's timescale, or `None` for to the end
    /// of the media (a duration of 0, as fragmented files write).
    duration: Option<u64>,
}

impl TrackTiming {
    /// The decoded frames, at `rate` frames per second, that the edit
    /// presents: where it starts and how many, or `None` for to the end of
    /// the media. `None` when there is no edit to apply.
    fn window(&self, rate: u32) -> Option<(u64, Option<u64>)> {
        let Edits::One(edit) = self.edits else {
            return None;
        };
        if self.timescale == 0 || self.movie_timescale == 0 {
            return None;
        }
        let start = rescale(edit.media_time, self.timescale, rate);
        let length = edit
            .duration
            .map(|d| rescale(d, self.movie_timescale, rate));
        Some((start, length))
    }
}

/// Reads the timing of every track, in the order of the `trak` boxes in the
/// file's `moov` box.
fn read_tracks(file: &mut File) -> io::Result<Vec<TrackTiming>> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let len = file.metadata()?.len();
    let mut at = 0;
    while at < len {
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0; 16];
        file.read_exact(&mut header[..8])?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let mut header_len = 8;
        if size == 1 {
            file.read_exact(&mut header[8..])?;
            size = u64::from_be_bytes(header[8..].try_into().unwrap());
            header_len = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < header_len {
            return Err(invalid("box shorter than its header"));
        }
        if &header[4..8] == b"moov" {
            let body_len = size - header_len;
            if body_len > MOOV_LIMIT {
                return Err(invalid("moov box too large"));
            }
            let mut body = vec![0; body_len as usize];
            file.read_exact(&mut body)?;
            return Ok(parse_moov(&body));
        }
        at = at.checked_add(size).ok_or_else(|| invalid("box size"))?;
    }
    Err(invalid("no moov box"))
}

fn parse_moov(moov: &[u8]) -> Vec<TrackTiming> {
    let movie_timescale = child(moov, b"mvhd").and_then(timescale).unwrap_or(0);
    boxes(moov)
        .filter(|(kind, _)| kind == b"trak")
        .map(|(_, trak)| TrackTiming {
            movie_timescale,
            timescale: child(trak, b"mdia")
                .and_then(|mdia| child(mdia, b"mdhd"))
                .and_then(timescale)
                .unwrap_or(0),
            edits: child(trak, b"edts")
                .and_then(|edts| child(edts, b"elst"))
                .map_or(Edits::None, parse_elst),
        })
        .collect()
}

/// The timescale of an `mvhd` or `mdhd` box, which share its place.
fn timescale(body: &[u8]) -> Option<u32> {
    let at = match body.first()? {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    body.get(at..at + 4)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
}

fn parse_elst(body: &[u8]) -> Edits {
    let parse = || -> Option<Edits> {
        let version = *body.first()?;
        let count = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?);
        let entry_len = match version {
            0 => 12,
            1 => 20,
            _ => return None,
        };
        let mut found = None;
        for i in 0..count as usize {
            let entry = body.get(8 + i * entry_len..8 + (i + 1) * entry_len)?;
            let (duration, media_time, rate) = if version == 0 {
                (
                    u64::from(u32::from_be_bytes(entry[..4].try_into().ok()?)),
                    i64::from(i32::from_be_bytes(entry[4..8].try_into().ok()?)),
                    &entry[8..],
                )
            } else {
                (
                    u64::from_be_bytes(entry[..8].try_into().ok()?),
                    i64::from_be_bytes(entry[8..16].try_into().ok()?),
                    &entry[16..],
                )
            };
            // An empty edit delays the media; terra starts it at once.
            if media_time == -1 {
                continue;
            }
            // A media rate of 1.0, as 16.16 fixed point.
            if found.is_some() || media_time < 0 || rate != [0, 1, 0, 0] {
                return Some(Edits::Unsupported);
            }
            found = Some(Edit {
                media_time: media_time as u64,
                duration: (duration > 0).then_some(duration),
            });
        }
        Some(found.map_or(Edits::None, Edits::One))
    };
    parse().unwrap_or(Edits::Unsupported)
}

/// The body of the first child box of `kind`.
fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(body).find(|(k, _)| k == kind).map(|(_, b)| b)
}

/// The boxes in `body`, as their type and body. Stops at a malformed one.
fn boxes(mut body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(body.get(..4)?.try_into().unwrap());
        let kind: [u8; 4] = body.get(4..8)?.try_into().unwrap();
        let (header, size) = match size {
            0 => (8, body.len()),
            1 => (
                16,
                usize::try_from(u64::from_be_bytes(body.get(8..16)?.try_into().unwrap())).ok()?,
            ),
            n => (8, n as usize),
        };
        let inner = body.get(header..size)?;
        body = &body[size..];
        Some((kind, inner))
    })
}

#[cfg(test)]
mod tests;
