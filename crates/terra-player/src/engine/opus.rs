//! Opus for Rodio. Rodio's `Decoder` builds its codec from Symphonia's
//! default registry, which has no Opus decoder, so this source drives
//! Symphonia 0.5 directly with a registry that adds libopus through
//! `symphonia-adapter-libopus`. It is the prototype's `opus_source.rs`
//! (`docs/decisions/0002-playback-engine.md`, Appendix A.6), with errors in
//! place of panics.

use std::fs::File;
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecRegistry, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::frames_to_duration;

pub(crate) struct OpusSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    buf: Vec<f32>,
    pos: usize,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    total: Option<Duration>,
    /// Opus pre-skip: Ogg timestamps count it, positions in the audio do not.
    delay: u64,
}

impl OpusSource {
    pub(crate) fn open(path: &Path) -> Result<OpusSource, String> {
        let mut codecs = CodecRegistry::new();
        codecs.register_all::<symphonia_adapter_libopus::OpusDecoder>();
        let file = File::open(path).map_err(|e| e.to_string())?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("opus");
        let options = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &options, &MetadataOptions::default())
            .map_err(|e| e.to_string())?;
        let format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| "no audio track".to_owned())?;
        let params = track.codec_params.clone();
        let decoder = codecs
            .make(&params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let channels = params
            .channels
            .and_then(|c| u16::try_from(c.count()).ok())
            .and_then(NonZero::new)
            .ok_or_else(|| "no channel count".to_owned())?;
        let rate = params
            .sample_rate
            .and_then(NonZero::new)
            .ok_or_else(|| "no sample rate".to_owned())?;
        let delay = u64::from(params.delay.unwrap_or(0));
        // The frame count includes the pre-skip, which is not played.
        let total = params
            .n_frames
            .map(|n| frames_to_duration(n.saturating_sub(delay), rate.get()));
        let mut source = OpusSource {
            track: track.id,
            format,
            decoder,
            buf: Vec::new(),
            pos: 0,
            channels,
            rate,
            total,
            delay,
        };
        // Decode the first packet now, so a file that is not Opus fails here.
        source.refill()?;
        Ok(source)
    }

    /// Decodes the next packet into `buf`. `Ok(false)` at the end.
    fn refill(&mut self) -> Result<bool, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // A damaged packet is skipped, as Rodio's decoder does.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            samples.copy_interleaved_ref(decoded);
            self.buf.clear();
            self.buf.extend_from_slice(samples.samples());
            self.pos = 0;
            if !self.buf.is_empty() {
                return Ok(true);
            }
        }
    }
}

impl Iterator for OpusSource {
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

impl Source for OpusSource {
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
        self.total
    }

    /// Accurate seek, as Rodio's own Symphonia decoder does it: the demuxer
    /// seeks to a packet at or before the target, then decoded samples up to
    /// the target are dropped. Ogg timestamps include the pre-skip, which the
    /// spike measured as 312 samples early without this correction.
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let to = SeekTo::Time {
            time: pos.into(),
            track_id: Some(self.track),
        };
        let seeked = self
            .format
            .seek(SeekMode::Accurate, to)
            .map_err(|e| SeekError::Other(Arc::new(e)))?;
        self.decoder.reset();
        self.buf.clear();
        self.pos = 0;
        let required = seeked.required_ts + self.delay;
        let frames = required.saturating_sub(seeked.actual_ts);
        let mut skip = frames as usize * usize::from(self.channels.get());
        while skip > 0 {
            if self.pos == self.buf.len() && !self.refill().unwrap_or(false) {
                break;
            }
            let n = skip.min(self.buf.len() - self.pos);
            self.pos += n;
            skip -= n;
        }
        Ok(())
    }
}
