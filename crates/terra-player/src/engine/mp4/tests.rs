use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

use super::*;

fn fixture(name: &str) -> File {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    File::open(path).unwrap()
}

/// An `elst` box body: version, flags, then `entries` as (duration,
/// media time, media rate in 16.16).
fn elst(version: u8, entries: &[(u64, i64, u32)]) -> Vec<u8> {
    let mut body = vec![version, 0, 0, 0];
    body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for &(duration, media_time, rate) in entries {
        if version == 0 {
            body.extend_from_slice(&(duration as u32).to_be_bytes());
            body.extend_from_slice(&(media_time as i32).to_be_bytes());
        } else {
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&media_time.to_be_bytes());
        }
        body.extend_from_slice(&rate.to_be_bytes());
    }
    body
}

const RATE_1: u32 = 0x0001_0000;

#[test]
fn reads_the_edit_list_ffmpeg_writes_for_aac() {
    // gen.sh's a.aac.m4a: 1024 priming frames, then the 72 517 of the tone.
    let tracks = read_tracks(&mut fixture("a.aac.m4a")).unwrap();
    assert_eq!(
        tracks,
        [TrackTiming {
            movie_timescale: 48_000,
            timescale: 48_000,
            edits: Edits::One(Edit {
                media_time: 1024,
                duration: Some(72_517),
            }),
        }]
    );
    assert_eq!(tracks[0].window(48_000), Some((1024, Some(72_517))));
}

#[test]
fn reads_the_edit_list_ffmpeg_writes_for_alac() {
    // ALAC has no priming: the edit presents the media as it is.
    let tracks = read_tracks(&mut fixture("a.alac.m4a")).unwrap();
    assert_eq!(tracks[0].window(48_000), Some((0, Some(72_517))));
}

#[test]
fn a_file_that_is_not_mp4_has_no_tracks() {
    assert!(read_tracks(&mut fixture("a.flac")).is_err());
}

#[test]
fn reads_both_versions_of_the_edit_list() {
    let edit = Edits::One(Edit {
        media_time: 2112,
        duration: Some(441_000),
    });
    assert_eq!(parse_elst(&elst(0, &[(441_000, 2112, RATE_1)])), edit);
    assert_eq!(parse_elst(&elst(1, &[(441_000, 2112, RATE_1)])), edit);
}

#[test]
fn an_empty_edit_before_the_media_is_skipped() {
    assert_eq!(
        parse_elst(&elst(0, &[(480, -1, RATE_1), (48_000, 1024, RATE_1)])),
        Edits::One(Edit {
            media_time: 1024,
            duration: Some(48_000),
        })
    );
}

#[test]
fn a_duration_of_zero_runs_to_the_end_of_the_media() {
    assert_eq!(
        parse_elst(&elst(0, &[(0, 1024, RATE_1)])),
        Edits::One(Edit {
            media_time: 1024,
            duration: None,
        })
    );
}

#[test]
fn edit_lists_this_source_cannot_apply_are_marked() {
    assert_eq!(parse_elst(&elst(0, &[])), Edits::None);
    // Two media edits, another rate, a version it does not know, and a
    // count larger than the entries.
    let two = elst(0, &[(1000, 0, RATE_1), (1000, 5000, RATE_1)]);
    assert_eq!(parse_elst(&two), Edits::Unsupported);
    assert_eq!(
        parse_elst(&elst(0, &[(1000, 0, 2 * RATE_1)])),
        Edits::Unsupported
    );
    let mut unknown = elst(0, &[(1000, 0, RATE_1)]);
    unknown[0] = 2;
    assert_eq!(parse_elst(&unknown), Edits::Unsupported);
    let mut short = elst(0, &[(1000, 0, RATE_1)]);
    short[7] = 2;
    assert_eq!(parse_elst(&short), Edits::Unsupported);
    let timing = TrackTiming {
        movie_timescale: 48_000,
        timescale: 48_000,
        edits: Edits::Unsupported,
    };
    assert_eq!(timing.window(48_000), None);
}

#[test]
fn the_window_is_rescaled_to_the_sample_rate() {
    // A movie timescale of 600 and a media timescale of 90 kHz, as some
    // muxers write, for 44.1 kHz audio.
    let timing = TrackTiming {
        movie_timescale: 600,
        timescale: 90_000,
        edits: Edits::One(Edit {
            media_time: 4320,
            duration: Some(600),
        }),
    };
    assert_eq!(timing.window(44_100), Some((2117, Some(44_100))));
}

#[test]
fn malformed_boxes_end_the_list() {
    // A box claiming more bytes than there are, after a whole one.
    let mut data = Vec::new();
    data.extend_from_slice(&12u32.to_be_bytes());
    data.extend_from_slice(b"free");
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&100u32.to_be_bytes());
    data.extend_from_slice(b"trak");
    let kinds: Vec<[u8; 4]> = boxes(&data).map(|(kind, _)| kind).collect();
    assert_eq!(kinds, [*b"free"]);
}

/// `a.aac.m4a`, with `patch` applied to its bytes.
fn patched(patch: impl FnOnce(&mut Vec<u8>)) -> Mp4Source {
    let mut data = Vec::new();
    fixture("a.aac.m4a").read_to_end(&mut data).unwrap();
    patch(&mut data);
    Mp4Source::new(Box::new(Cursor::new(data))).unwrap()
}

/// Where the first `kind` box starts in `data`.
fn find_box(data: &[u8], kind: &[u8; 4]) -> usize {
    data.windows(4).position(|w| w == kind).unwrap() - 4
}

fn set_u32(data: &mut [u8], at: usize, expected: u32, value: u32) {
    assert_eq!(
        u32::from_be_bytes(data[at..at + 4].try_into().unwrap()),
        expected
    );
    data[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn the_sample_rate_is_the_decoders() {
    // The sample entry's rate is 16.16 fixed point and cannot hold rates
    // above 65 535 Hz; a muxer may leave it 0. The decoder reads the rate
    // from the AudioSpecificConfig.
    let source = patched(|data| {
        let mp4a = find_box(data, b"mp4a");
        set_u32(data, mp4a + 32, 48_000 << 16, 0);
    });
    assert_eq!(source.sample_rate().get(), 48_000);
    assert_eq!(source.count(), 72_517);
}
