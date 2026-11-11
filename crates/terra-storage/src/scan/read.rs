//! Reading one audio file: its tags, its audio properties and its hash.
//!
//! Everything here blocks on the file system; the scanner runs it on Tokio's
//! blocking threads.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::iff::wav::{WavFile, WavFormat};
use lofty::mp4::{AtomData, AtomIdent, Mp4Codec, Mp4File};
use lofty::mpeg::{Layer, MpegFile};
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use terra_core::AudioFormat;

/// The artist of a file without an artist tag.
pub(crate) const UNKNOWN_ARTIST: &str = "Unknown Artist";

/// What the scanner stores of a file it read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileInfo {
    pub(crate) tags: Tags,
    pub(crate) duration: Duration,
    pub(crate) format: AudioFormat,
    pub(crate) has_embedded_art: bool,
    /// The BLAKE3 hash of the whole file, tags included.
    pub(crate) content_hash: [u8; 32],
}

/// A file's tags, with the scanner's fallbacks applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tags {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) artist_sort: String,
    pub(crate) album: Option<String>,
    /// The album artist tag and its sort name, when the file has one.
    pub(crate) album_artist: Option<(String, String)>,
    pub(crate) compilation: bool,
    pub(crate) year: Option<i32>,
    pub(crate) disc_no: Option<u32>,
    pub(crate) track_no: Option<u32>,
}

/// Reads and hashes the file at `path`. The error is a message for the
/// scan report.
pub(crate) fn read_file(path: &Path) -> Result<FileInfo, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|err| err.to_string())?);
    let content_hash = hash(&mut reader).map_err(|err| err.to_string())?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|err| err.to_string())?;

    let probe = Probe::new(reader)
        .guess_file_type()
        .map_err(|err| err.to_string())?;
    let file_type = probe.file_type();
    let mut reader = probe.into_inner();
    let Audio {
        codec,
        lossless,
        file: tagged,
        trimmed_duration,
    } = read_audio(&mut reader, file_type).map_err(|err| err.to_string())?;

    let properties = tagged.properties();
    let sample_rate = properties
        .sample_rate()
        .filter(|&rate| rate > 0)
        .ok_or("the file has no audio stream")?;
    let format = AudioFormat {
        codec: codec.to_owned(),
        lossless,
        sample_rate,
        channels: properties.channels().map_or(0, u16::from),
        // Lofty gives kilobits per second.
        bitrate: properties
            .audio_bitrate()
            .filter(|&kbps| kbps > 0)
            .map(|kbps| kbps.saturating_mul(1000)),
    };
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(FileInfo {
        tags: tags(tag, path),
        duration: trimmed_duration.unwrap_or_else(|| properties.duration()),
        format,
        has_embedded_art: tagged.tags().iter().any(|tag| tag.picture_count() > 0),
        content_hash,
    })
}

fn hash(reader: &mut BufReader<File>) -> std::io::Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(reader)?;
    Ok(*hasher.finalize().as_bytes())
}

/// An audio file as [`read_audio`] read it.
struct Audio {
    codec: &'static str,
    /// Whether the codec is lossless.
    lossless: bool,
    file: TaggedFile,
    /// The duration without the encoder delay and padding, when the file
    /// gives them; Lofty's duration includes them.
    trimmed_duration: Option<Duration>,
}

/// Reads the audio file as the type its content shows.
///
/// The codec comes from the audio stream, never from the container or the
/// extension: an M4A file holds AAC or ALAC (brief §5).
fn read_audio(
    reader: &mut BufReader<File>,
    file_type: Option<FileType>,
) -> Result<Audio, Box<dyn std::error::Error>> {
    let options = ParseOptions::new();
    let audio = |codec, lossless, file: TaggedFile| Audio {
        codec,
        lossless,
        file,
        trimmed_duration: None,
    };
    Ok(match file_type {
        Some(FileType::Mpeg) => {
            let file = MpegFile::read_from(reader, options)?;
            let codec = match file.properties().layer() {
                Layer::Layer1 => "mp1",
                Layer::Layer2 => "mp2",
                Layer::Layer3 => "mp3",
            };
            Audio {
                trimmed_duration: mp3_trimmed_duration(reader),
                ..audio(codec, false, file.into())
            }
        }
        Some(FileType::Flac) => audio("flac", true, FlacFile::read_from(reader, options)?.into()),
        Some(FileType::Vorbis) => audio(
            "vorbis",
            false,
            VorbisFile::read_from(reader, options)?.into(),
        ),
        Some(FileType::Opus) => audio("opus", false, OpusFile::read_from(reader, options)?.into()),
        Some(FileType::Mp4) => {
            let file = Mp4File::read_from(reader, options)?;
            let (codec, lossless) = match file.properties().codec() {
                Some(Mp4Codec::AAC) => ("aac", false),
                Some(Mp4Codec::ALAC) => ("alac", true),
                Some(Mp4Codec::MP3) => ("mp3", false),
                Some(Mp4Codec::FLAC) => ("flac", true),
                _ => return Err("the MP4 file's audio codec is not supported".into()),
            };
            let trimmed_duration = mp4_edit_duration(reader).or_else(|| {
                let rate = file.properties().sample_rate()?;
                itunsmpb_duration(file.ilst()?.get(&ITUNSMPB)?, rate)
            });
            Audio {
                trimmed_duration,
                ..audio(codec, lossless, file.into())
            }
        }
        Some(FileType::Wav) => {
            let file = WavFile::read_from(reader, options)?;
            match file.properties().format() {
                WavFormat::PCM | WavFormat::IEEE_FLOAT => {}
                WavFormat::Other(tag) => {
                    return Err(format!("the WAV format 0x{tag:04x} is not supported").into());
                }
            }
            audio("pcm", true, file.into())
        }
        Some(other) => return Err(format!("the format {other:?} is not supported").into()),
        None => return Err("the file is not in a known audio format".into()),
    })
}

/// `samples` at `rate` samples per second, as a duration.
fn samples_to_duration(samples: u64, rate: u64) -> Option<Duration> {
    if rate == 0 {
        return None;
    }
    let nanos = u128::from(samples) * 1_000_000_000 / u128::from(rate);
    Some(Duration::from_nanos(u64::try_from(nanos).ok()?))
}

/// How far into an MP3 file, after its ID3v2 tag, the first frame is
/// looked for.
const MP3_SYNC_WINDOW: u64 = 64 << 10;

/// An MP3 file's duration without the encoder delay and padding, from the
/// LAME tag that follows the Xing/Info header in the first frame:
/// (frames × samples per frame − delay − padding) / sample rate. `None`
/// when the file has no such tag, or it cannot be read.
///
/// The offsets and the encoders whose tag gives the delay and padding
/// (LAME, and Lavf and Lavc for ffmpeg) are those of ffmpeg's MP3 demuxer,
/// `mp3_parse_info_tag` in `libavformat/mp3dec.c`. Both it and the LAME
/// decoder add 529 samples of decoder delay at each end, which cancel out.
fn mp3_trimmed_duration(reader: &mut (impl Read + Seek)) -> Option<Duration> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut id3 = [0; 10];
    reader.read_exact(&mut id3).ok()?;
    // An ID3v2 tag: its size is syncsafe, with 7 bits a byte, and excludes
    // the header and any footer.
    let start = if &id3[..3] == b"ID3" {
        let size = id3[6..10]
            .iter()
            .fold(0, |size, &b| (size << 7) | u64::from(b & 0x7f));
        let footer = if id3[5] & 0x10 != 0 { 10 } else { 0 };
        10 + size + footer
    } else {
        0
    };
    reader.seek(SeekFrom::Start(start)).ok()?;
    let mut window = Vec::new();
    reader.take(MP3_SYNC_WINDOW).read_to_end(&mut window).ok()?;
    let at = window
        .windows(4)
        .position(|bytes| mp3_header(bytes).is_some())?;
    lame_trimmed_duration(&window[at..])
}

/// What an MPEG Layer III frame header says: the sample rate, the samples
/// a frame holds and the length of the side information that follows the
/// header. `None` for anything that is not a valid Layer III header.
fn mp3_header(bytes: &[u8]) -> Option<(u32, u64, usize)> {
    let header = u32::from_be_bytes(bytes.try_into().ok()?);
    let version = (header >> 19) & 3;
    let layer = (header >> 17) & 3;
    let bitrate = (header >> 12) & 15;
    let rate_index = (header >> 10) & 3;
    let mono = (header >> 6) & 3 == 3;
    if header >> 21 != 0x7ff || version == 1 || layer != 1 || bitrate == 0 || bitrate == 15 {
        return None;
    }
    let base = [44_100, 48_000, 32_000].get(rate_index as usize).copied()?;
    // MPEG-1, MPEG-2 and MPEG-2.5.
    Some(match version {
        3 => (base, 1152, if mono { 17 } else { 32 }),
        2 => (base / 2, 576, if mono { 9 } else { 17 }),
        _ => (base / 4, 576, if mono { 9 } else { 17 }),
    })
}

/// The duration a LAME tag gives, from the MPEG frame that starts `frame`.
fn lame_trimmed_duration(frame: &[u8]) -> Option<Duration> {
    let (rate, samples_per_frame, side_info) = mp3_header(frame.get(..4)?)?;
    let xing = frame.get(4 + side_info..)?;
    if !matches!(xing.get(..4)?, b"Xing" | b"Info") {
        return None;
    }
    let flags = u32::from_be_bytes(xing.get(4..8)?.try_into().ok()?);
    // The frame count, then the byte count, the table of contents and the
    // quality, each there when its flag is set.
    if flags & 1 == 0 {
        return None;
    }
    let frames = u64::from(u32::from_be_bytes(xing.get(8..12)?.try_into().ok()?));
    let mut at = 12;
    for (flag, len) in [(2, 4), (4, 100), (8, 4)] {
        if flags & flag != 0 {
            at += len;
        }
    }
    let lame = xing.get(at..at + 24)?;
    if !matches!(&lame[..4], b"LAME" | b"Lavf" | b"Lavc") {
        return None;
    }
    // The version string (9 bytes), the revision, the lowpass, the peak
    // (4), two ReplayGains (2 each), the flags and the bitrate; then the
    // delay and the padding, 12 bits each.
    let delays = u32::from_be_bytes([0, lame[21], lame[22], lame[23]]);
    let (delay, padding) = (u64::from(delays >> 12), u64::from(delays & 0xfff));
    let samples = (frames * samples_per_frame).checked_sub(delay + padding)?;
    if samples == 0 {
        return None;
    }
    samples_to_duration(samples, u64::from(rate))
}

/// The largest `moov` box read. Real ones are kilobytes to a few megabytes.
const MOOV_LIMIT: u64 = 64 << 20;

/// An MP4 file's duration as its audio track's edit list presents it,
/// which leaves out an AAC encoder's priming and padding. `None` when the
/// track has no edit list, or one with several edits or another rate than
/// 1.0, as `terra-player` then plays the whole media too.
fn mp4_edit_duration(reader: &mut (impl Read + Seek)) -> Option<Duration> {
    let len = reader.seek(SeekFrom::End(0)).ok()?;
    let mut at = 0;
    while at < len {
        reader.seek(SeekFrom::Start(at)).ok()?;
        let mut header = [0; 16];
        reader.read_exact(&mut header[..8]).ok()?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().ok()?));
        let mut header_len = 8;
        if size == 1 {
            reader.read_exact(&mut header[8..]).ok()?;
            size = u64::from_be_bytes(header[8..].try_into().ok()?);
            header_len = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < header_len {
            return None;
        }
        if &header[4..8] == b"moov" {
            let body_len = size - header_len;
            if body_len > MOOV_LIMIT {
                return None;
            }
            let mut body = vec![0; usize::try_from(body_len).ok()?];
            reader.read_exact(&mut body).ok()?;
            return moov_edit_duration(&body);
        }
        at = at.checked_add(size)?;
    }
    None
}

/// The duration the edit list of the first audio track in a `moov` box's
/// body presents.
fn moov_edit_duration(moov: &[u8]) -> Option<Duration> {
    let movie_timescale = mp4_child(moov, b"mvhd").and_then(mp4_timescale)?;
    // The track Lofty reads: the first whose handler is `soun`.
    let (trak, mdia) = mp4_boxes(moov)
        .filter(|(kind, _)| kind == b"trak")
        .filter_map(|(_, trak)| Some((trak, mp4_child(trak, b"mdia")?)))
        .find(|(_, mdia)| {
            mp4_child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) == Some(b"soun")
        })?;
    let mdhd = mp4_child(mdia, b"mdhd")?;
    let timescale = mp4_timescale(mdhd)?;
    let media_duration = match mdhd.first()? {
        0 => u64::from(u32::from_be_bytes(mdhd.get(16..20)?.try_into().ok()?)),
        _ => u64::from_be_bytes(mdhd.get(24..32)?.try_into().ok()?),
    };
    let elst = mp4_child(mp4_child(trak, b"edts")?, b"elst")?;
    let (media_time, duration) = one_edit(elst)?;
    // The media left after the edit's start, in the track's timescale; a
    // fragmented file gives the media's duration as 0.
    let rest = media_duration
        .checked_sub(media_time)
        .filter(|_| media_duration > 0);
    // The edit ends at the end of the media, as in `terra-player`.
    let rest = rest.and_then(|rest| samples_to_duration(rest, u64::from(timescale)));
    let edit = duration.and_then(|d| samples_to_duration(d, u64::from(movie_timescale)));
    match (edit, rest) {
        (Some(edit), Some(rest)) => Some(edit.min(rest)),
        (edit, rest) => edit.or(rest),
    }
    .filter(|duration| !duration.is_zero())
}

/// The one edit of an `elst` box's body that plays the media at normal
/// rate, after any empty edits: where it starts, in the track's timescale,
/// and how long it lasts, in the movie's, or `None` for to the end of the
/// media (a duration of 0). `None` for no such edit, or several.
fn one_edit(elst: &[u8]) -> Option<(u64, Option<u64>)> {
    let version = *elst.first()?;
    let count = u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?);
    let entry_len = match version {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    let mut found = None;
    for i in 0..count as usize {
        let entry = elst.get(8 + i * entry_len..8 + (i + 1) * entry_len)?;
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
        if found.is_some() || rate != [0, 1, 0, 0] {
            return None;
        }
        found = Some((
            u64::try_from(media_time).ok()?,
            (duration > 0).then_some(duration),
        ));
    }
    found
}

/// The timescale of an `mvhd` or `mdhd` box, which share its place.
fn mp4_timescale(body: &[u8]) -> Option<u32> {
    let at = match body.first()? {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    let timescale = u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?);
    (timescale > 0).then_some(timescale)
}

/// The body of the first child box of `kind`.
fn mp4_child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    mp4_boxes(body).find(|(k, _)| k == kind).map(|(_, b)| b)
}

/// The boxes in `body`, as their type and body. Stops at a malformed one.
fn mp4_boxes(mut body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(body.get(..4)?.try_into().ok()?);
        let kind: [u8; 4] = body.get(4..8)?.try_into().ok()?;
        let (header, size) = match size {
            0 => (8, body.len()),
            1 => (
                16,
                usize::try_from(u64::from_be_bytes(body.get(8..16)?.try_into().ok()?)).ok()?,
            ),
            n => (8, n as usize),
        };
        let inner = body.get(header..size)?;
        body = &body[size..];
        Some((kind, inner))
    })
}

/// The iTunes tag that gives an AAC file's priming, padding and length
/// when it has no edit list.
const ITUNSMPB: AtomIdent<'static> = AtomIdent::Freeform {
    mean: std::borrow::Cow::Borrowed("com.apple.iTunes"),
    name: std::borrow::Cow::Borrowed("iTunSMPB"),
};

/// The duration an `iTunSMPB` tag gives: its fourth field, the samples
/// without priming and padding, in hexadecimal, at `rate`.
fn itunsmpb_duration(atom: &lofty::mp4::Atom<'_>, rate: u32) -> Option<Duration> {
    let Some(AtomData::UTF8(text) | AtomData::UTF16(text)) = atom.data().next() else {
        return None;
    };
    itunsmpb_samples(text).and_then(|samples| samples_to_duration(samples, u64::from(rate)))
}

/// The sample count in an `iTunSMPB` value, such as
/// ` 00000000 00000840 000001C0 000000000000BB80 …`.
fn itunsmpb_samples(text: &str) -> Option<u64> {
    let samples = u64::from_str_radix(text.split_whitespace().nth(3)?, 16).ok()?;
    (samples > 0).then_some(samples)
}

/// Maps a file's tags to the library's fields, with the fallbacks
/// `docs/design/library-schema.md` gives.
fn tags(tag: Option<&Tag>, path: &Path) -> Tags {
    let text = |key: ItemKey| {
        tag.and_then(|tag| tag.get_string(key))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let title = text(ItemKey::TrackTitle).unwrap_or_else(|| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let artist = text(ItemKey::TrackArtist).unwrap_or_else(|| UNKNOWN_ARTIST.to_owned());
    let artist_sort = text(ItemKey::TrackArtistSortOrder).unwrap_or_else(|| artist.clone());
    let album_artist = text(ItemKey::AlbumArtist).map(|name| {
        let sort = text(ItemKey::AlbumArtistSortOrder).unwrap_or_else(|| name.clone());
        (name, sort)
    });
    Tags {
        title,
        artist,
        artist_sort,
        album: text(ItemKey::AlbumTitle),
        album_artist,
        compilation: text(ItemKey::FlagCompilation).is_some_and(|flag| is_set(&flag)),
        year: tag
            .and_then(Accessor::date)
            .map(|date| i32::from(date.year)),
        disc_no: tag.and_then(Accessor::disk),
        track_no: tag.and_then(Accessor::track),
    }
}

/// Whether a flag tag is set. Lofty writes `1` and `0`; other taggers write
/// `true` or another number.
fn is_set(flag: &str) -> bool {
    flag.eq_ignore_ascii_case("true") || flag.parse::<u64>().is_ok_and(|n| n != 0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{is_set, itunsmpb_samples, lame_trimmed_duration, moov_edit_duration, one_edit};

    /// A media rate of 1.0, as 16.16 fixed point.
    const RATE_1: u32 = 0x0001_0000;

    /// A box of `kind` around `body`.
    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (8 + body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// An `elst` box body, version 0: `entries` as (duration, media time,
    /// rate).
    fn elst(entries: &[(u32, i32, u32)]) -> Vec<u8> {
        let mut body = vec![0; 4];
        body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for &(duration, media_time, rate) in entries {
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&media_time.to_be_bytes());
            body.extend_from_slice(&rate.to_be_bytes());
        }
        body
    }

    /// A version 0 `mvhd` or `mdhd` body: timescale and duration.
    fn header(timescale: u32, duration: u32) -> Vec<u8> {
        let mut body = vec![0; 12];
        body.extend_from_slice(&timescale.to_be_bytes());
        body.extend_from_slice(&duration.to_be_bytes());
        body
    }

    /// A `moov` body with a 1000 Hz movie timescale and one track of
    /// `handler` at 48 kHz, lasting `media` samples, with `edits`.
    fn moov(handler: &[u8; 4], media: u32, edits: Option<&[(u32, i32, u32)]>) -> Vec<u8> {
        let mut hdlr = vec![0; 8];
        hdlr.extend_from_slice(handler);
        let mut mdia = mp4_box(b"mdhd", &header(48_000, media));
        mdia.extend(mp4_box(b"hdlr", &hdlr));
        let mut trak = mp4_box(b"tkhd", &[0; 84]);
        if let Some(edits) = edits {
            trak.extend(mp4_box(b"edts", &mp4_box(b"elst", &elst(edits))));
        }
        trak.extend(mp4_box(b"mdia", &mdia));
        let mut moov = mp4_box(b"mvhd", &header(1000, 1000));
        moov.extend(mp4_box(b"trak", &trak));
        moov
    }

    #[test]
    fn the_edit_list_gives_the_duration_it_presents() {
        let one = [(1000, 1024, RATE_1)];
        let duration = moov_edit_duration(&moov(b"soun", 49_984, Some(&one)));
        assert_eq!(duration, Some(Duration::from_secs(1)));
        // Empty edits before it are skipped, as `terra-player` does.
        let delayed = [(500, -1, RATE_1), (1000, 1024, RATE_1)];
        let duration = moov_edit_duration(&moov(b"soun", 49_984, Some(&delayed)));
        assert_eq!(duration, Some(Duration::from_secs(1)));
    }

    #[test]
    fn an_edit_ends_at_the_end_of_the_media() {
        // A duration of 0 runs to the end of the media.
        let open = [(0, 1024, RATE_1)];
        let duration = moov_edit_duration(&moov(b"soun", 49_024, Some(&open)));
        assert_eq!(duration, Some(Duration::from_secs(1)));
        // So does an edit longer than the media.
        let long = [(2000, 1024, RATE_1)];
        let duration = moov_edit_duration(&moov(b"soun", 49_024, Some(&long)));
        assert_eq!(duration, Some(Duration::from_secs(1)));
        // A fragmented file's media duration of 0 does not end it.
        let duration = moov_edit_duration(&moov(b"soun", 0, Some(&long)));
        assert_eq!(duration, Some(Duration::from_secs(2)));
        assert_eq!(moov_edit_duration(&moov(b"soun", 0, Some(&open))), None);
    }

    #[test]
    fn an_edit_list_terra_does_not_apply_gives_no_duration() {
        assert_eq!(moov_edit_duration(&moov(b"soun", 49_984, None)), None);
        let one = [(1000, 1024, RATE_1)];
        assert_eq!(moov_edit_duration(&moov(b"vide", 49_984, Some(&one))), None);
        let two = [(500, 0, RATE_1), (500, 30_000, RATE_1)];
        assert_eq!(one_edit(&elst(&two)), None);
        assert_eq!(one_edit(&elst(&[(1000, 0, 2 * RATE_1)])), None);
        assert_eq!(one_edit(&elst(&[])), None);
        let mut short = elst(&one);
        short.pop();
        assert_eq!(one_edit(&short), None);
    }

    /// The first frame of a 48 kHz mono MPEG-1 Layer III file: a header, the
    /// side information, then an Info header with the frame count and a
    /// LAME tag from `encoder` with `delay` and `padding`.
    fn info_frame(encoder: &[u8; 4], frames: u32, delay: u32, padding: u32) -> Vec<u8> {
        let mut frame = vec![0xff, 0xfb, 0x44, 0xc0];
        frame.extend_from_slice(&[0; 17]);
        frame.extend_from_slice(b"Info");
        frame.extend_from_slice(&1u32.to_be_bytes());
        frame.extend_from_slice(&frames.to_be_bytes());
        frame.extend_from_slice(encoder);
        frame.extend_from_slice(&[0; 17]);
        frame.extend_from_slice(&((delay << 12) | padding).to_be_bytes()[1..]);
        frame.resize(192, 0);
        frame
    }

    #[test]
    fn a_lame_tag_gives_the_duration_without_delay_and_padding() {
        // 43 frames of 1152 samples, less 1536.
        for encoder in [b"LAME", b"Lavf", b"Lavc"] {
            let frame = info_frame(encoder, 43, 576, 960);
            assert_eq!(lame_trimmed_duration(&frame), Some(Duration::from_secs(1)));
        }
        // Another encoder's tag, or none, gives no delay and padding.
        assert_eq!(
            lame_trimmed_duration(&info_frame(b"GOGO", 43, 576, 960)),
            None
        );
        assert_eq!(
            lame_trimmed_duration(&info_frame(&[0; 4], 43, 576, 960)),
            None
        );
        // Nor does a delay and padding longer than the frames.
        assert_eq!(
            lame_trimmed_duration(&info_frame(b"LAME", 1, 576, 960)),
            None
        );
        // Nor a frame without the frame count.
        let mut frame = info_frame(b"LAME", 43, 576, 960);
        frame[28] = 0;
        assert_eq!(lame_trimmed_duration(&frame), None);
    }

    #[test]
    fn itunsmpb_gives_the_samples_without_priming_and_padding() {
        let value = " 00000000 00000840 000001C0 0000000000ACBEB0 00000000 00000000";
        assert_eq!(itunsmpb_samples(value), Some(0xAC_BEB0));
        assert_eq!(itunsmpb_samples(" 00000000 00000840 000001C0"), None);
        assert_eq!(itunsmpb_samples(" 00000000 00000840 000001C0 0"), None);
        assert_eq!(itunsmpb_samples(" 00000000 00000840 000001C0 xyz"), None);
    }

    #[test]
    fn flags_are_set_by_true_or_a_number_other_than_zero() {
        for flag in ["1", "01", "2", "true", "TRUE"] {
            assert!(is_set(flag), "{flag}");
        }
        for flag in ["0", "00", "false", "no", "", "-1"] {
            assert!(!is_set(flag), "{flag}");
        }
    }
}
