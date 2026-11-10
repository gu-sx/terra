//! Reading one audio file: its tags, its audio properties and its hash.
//!
//! Everything here blocks on the file system; the scanner runs it on Tokio's
//! blocking threads.

use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::iff::wav::{WavFile, WavFormat};
use lofty::mp4::{Mp4Codec, Mp4File};
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
    pub(crate) duration: std::time::Duration,
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
    let (codec, lossless, tagged) =
        read_audio(&mut reader, file_type).map_err(|err| err.to_string())?;

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
        duration: properties.duration(),
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

/// Reads the audio file as the type its content shows, and returns its
/// codec, whether that codec is lossless, and the file.
///
/// The codec comes from the audio stream, never from the container or the
/// extension: an M4A file holds AAC or ALAC (brief §5).
fn read_audio(
    reader: &mut BufReader<File>,
    file_type: Option<FileType>,
) -> Result<(&'static str, bool, TaggedFile), Box<dyn std::error::Error>> {
    let options = ParseOptions::new();
    Ok(match file_type {
        Some(FileType::Mpeg) => {
            let file = MpegFile::read_from(reader, options)?;
            let codec = match file.properties().layer() {
                Layer::Layer1 => "mp1",
                Layer::Layer2 => "mp2",
                Layer::Layer3 => "mp3",
            };
            (codec, false, file.into())
        }
        Some(FileType::Flac) => ("flac", true, FlacFile::read_from(reader, options)?.into()),
        Some(FileType::Vorbis) => (
            "vorbis",
            false,
            VorbisFile::read_from(reader, options)?.into(),
        ),
        Some(FileType::Opus) => ("opus", false, OpusFile::read_from(reader, options)?.into()),
        Some(FileType::Mp4) => {
            let file = Mp4File::read_from(reader, options)?;
            let (codec, lossless) = match file.properties().codec() {
                Some(Mp4Codec::AAC) => ("aac", false),
                Some(Mp4Codec::ALAC) => ("alac", true),
                Some(Mp4Codec::MP3) => ("mp3", false),
                Some(Mp4Codec::FLAC) => ("flac", true),
                _ => return Err("the MP4 file's audio codec is not supported".into()),
            };
            (codec, lossless, file.into())
        }
        Some(FileType::Wav) => {
            let file = WavFile::read_from(reader, options)?;
            match file.properties().format() {
                WavFormat::PCM | WavFormat::IEEE_FLOAT => {}
                WavFormat::Other(tag) => {
                    return Err(format!("the WAV format 0x{tag:04x} is not supported").into());
                }
            }
            ("pcm", true, file.into())
        }
        Some(other) => return Err(format!("the format {other:?} is not supported").into()),
        None => return Err("the file is not in a known audio format".into()),
    })
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
    use super::is_set;

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
