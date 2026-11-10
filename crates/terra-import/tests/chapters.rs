//! Segments from yt-dlp chapters, read from a synthetic info JSON fixture.

use std::time::Duration;

use serde::Deserialize;
use terra_import::{Chapter, Warning, WarningKind, from_chapters};

/// The part of yt-dlp's info JSON the importer reads.
#[derive(Deserialize)]
struct Info {
    chapters: Vec<Chapter>,
}

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

fn chapter(start_time: f64, end_time: Option<f64>, title: &str) -> Chapter {
    Chapter {
        start_time,
        end_time,
        title: title.to_owned(),
    }
}

#[test]
fn from_info_json_fixture() {
    let json = include_str!("fixtures/info.json");
    let info: Info = serde_json::from_str(json).expect("the fixture is valid info JSON");
    let parsed = from_chapters(&info.chapters);

    assert_eq!(parsed.warnings, []);
    let got: Vec<_> = parsed
        .segments
        .iter()
        .map(|segment| {
            (
                segment.index,
                segment.start,
                segment.end,
                segment.artist.as_deref(),
                segment.title.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                0,
                ms(0),
                Some(ms(342_000)),
                Some("Artist A"),
                "Opening Track"
            ),
            (
                1,
                ms(342_000),
                Some(ms(678_000)),
                Some("Artist B"),
                "Midnight"
            ),
            (
                2,
                ms(678_000),
                Some(ms(1_073_500)),
                Some("Artist C"),
                "After Hours"
            ),
            (
                3,
                ms(1_073_500),
                Some(ms(1_447_000)),
                None,
                "<Untitled Chapter 4>"
            ),
            (
                4,
                ms(1_447_000),
                Some(ms(1_800_000)),
                Some("Artist E"),
                "Closing"
            ),
        ]
    );
    assert_eq!(parsed.segments[1].source_line, "Artist B – Midnight");
}

#[test]
fn null_end_time_falls_back_to_the_next_start() {
    let json = r#"{"chapters": [
        {"start_time": 0, "end_time": null, "title": "Artist A - One"},
        {"start_time": 60, "title": "Artist B - Two"}
    ]}"#;
    let info: Info = serde_json::from_str(json).expect("valid JSON");
    let parsed = from_chapters(&info.chapters);

    assert_eq!(parsed.warnings, []);
    let ends: Vec<_> = parsed.segments.iter().map(|segment| segment.end).collect();
    assert_eq!(ends, [Some(ms(60_000)), None]);
}

#[test]
fn overlapping_end_is_cut_back_to_the_next_start() {
    let parsed = from_chapters(&[
        chapter(0.0, Some(90.0), "Artist A - One"),
        chapter(60.0, Some(120.0), "Artist B - Two"),
    ]);
    let ends: Vec<_> = parsed.segments.iter().map(|segment| segment.end).collect();
    assert_eq!(ends, [Some(ms(60_000)), Some(ms(120_000))]);
}

#[test]
fn a_gap_before_the_next_chapter_is_kept() {
    let parsed = from_chapters(&[
        chapter(0.0, Some(50.0), "Artist A - One"),
        chapter(60.0, Some(120.0), "Artist B - Two"),
    ]);
    let ends: Vec<_> = parsed.segments.iter().map(|segment| segment.end).collect();
    assert_eq!(ends, [Some(ms(50_000)), Some(ms(120_000))]);
}

#[test]
fn warnings() {
    let parsed = from_chapters(&[
        chapter(5.0, Some(60.0), "Artist A - One"),
        chapter(f64::NAN, Some(70.0), "Artist B - Two"),
        chapter(60.0, Some(60.0), "Artist C - Three"),
        chapter(30.0, Some(90.0), "Artist D - Four"),
        chapter(60.0, Some(100.0), "Artist E - Five"),
        chapter(-1.0, None, "Artist F - Six"),
        chapter(100.0, Some(f64::INFINITY), "Artist G - Seven"),
    ]);

    let starts: Vec<_> = parsed
        .segments
        .iter()
        .map(|segment| (segment.start, segment.end))
        .collect();
    assert_eq!(
        starts,
        [
            (ms(5_000), Some(ms(60_000))),
            (ms(60_000), Some(ms(100_000))),
            (ms(100_000), None),
        ]
    );
    assert_eq!(
        parsed.warnings,
        [
            Warning {
                line: 1,
                kind: WarningKind::FirstStartAfterZero { start: ms(5_000) },
            },
            Warning {
                line: 2,
                kind: WarningKind::InvalidChapterTime,
            },
            Warning {
                line: 3,
                kind: WarningKind::InvalidChapterTime,
            },
            Warning {
                line: 4,
                kind: WarningKind::NotIncreasing {
                    start: ms(30_000),
                    previous: ms(60_000),
                },
            },
            Warning {
                line: 5,
                kind: WarningKind::DuplicateTimestamp {
                    start: ms(60_000),
                    first_line: 3,
                },
            },
            Warning {
                line: 6,
                kind: WarningKind::InvalidChapterTime,
            },
            Warning {
                line: 7,
                kind: WarningKind::InvalidChapterTime,
            },
        ]
    );
}
