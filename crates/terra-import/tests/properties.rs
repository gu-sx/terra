//! Property tests: the parsers never panic, and the segments they return are
//! ordered and contiguous, for arbitrary input.
//!
//! The seed is fixed so that every run checks the same cases; change it to
//! explore others.

use std::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use terra_import::{Chapter, ParsedTracklist, from_chapters, parse_description};

const SEED: u64 = 0x7e44_0004;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 2048,
        rng_seed: RngSeed::Fixed(SEED),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Lines that look like tracklist lines, broken ones and noise.
fn line() -> impl Strategy<Value = String> {
    let timestamp = prop_oneof![
        // A few values, so that lines repeat and reorder starts.
        (0u64..3, prop::sample::select(vec![0u64, 30])).prop_map(|(m, s)| format!("{m}:{s:02}")),
        (0u64..100, 0u64..100).prop_map(|(m, s)| format!("{m}:{s:02}")),
        (0u64..30, 0u64..100, 0u64..100).prop_map(|(h, m, s)| format!("{h}:{m:02}:{s:02}")),
        "[0-9:]{1,9}",
    ];
    let decoration = prop_oneof![
        Just(String::new()),
        Just("1. ".to_owned()),
        Just("#2 ".to_owned()),
        Just("03) ".to_owned()),
        Just("[".to_owned()),
        Just("(".to_owned()),
        Just("]".to_owned()),
        Just(" - ".to_owned()),
        Just(" – ".to_owned()),
        Just(" — ".to_owned()),
        Just(" ~ ".to_owned()),
    ];
    let text = prop_oneof![
        Just("Artist A - Track".to_owned()),
        Just("Artiste É — Café 🌅".to_owned()),
        "\\PC{0,20}",
        any::<String>(),
    ];
    prop_oneof![
        (
            decoration.clone(),
            timestamp.clone(),
            decoration.clone(),
            text.clone()
        )
            .prop_map(|(a, t, b, x)| format!("{a}{t}{b}{x}")),
        (
            text.clone(),
            decoration.clone(),
            timestamp.clone(),
            decoration.clone()
        )
            .prop_map(|(x, a, t, b)| format!("{x}{a}{t}{b}")),
        (timestamp.clone(), decoration, timestamp, text.clone())
            .prop_map(|(t, d, u, x)| format!("{t}{d}{u} {x}")),
        text,
    ]
}

fn description() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::collection::vec(line(), 0..40).prop_map(|lines| lines.join("\n")),
        any::<String>(),
    ]
}

fn total() -> impl Strategy<Value = Option<Duration>> {
    prop::option::of((0u64..200_000).prop_map(Duration::from_secs))
}

fn time() -> impl Strategy<Value = f64> {
    prop_oneof![
        (0u32..20_000).prop_map(f64::from),
        -1e3..2e4f64,
        any::<f64>(),
    ]
}

fn chapters() -> impl Strategy<Value = Vec<Chapter>> {
    let chapter = (time(), prop::option::of(time()), "\\PC{0,20}").prop_map(
        |(start_time, end_time, title)| Chapter {
            start_time,
            end_time,
            title,
        },
    );
    prop::collection::vec(chapter, 0..30)
}

/// Checks what holds for every tracklist: indexes match positions, starts
/// strictly increase, and no segment ends before it starts or after the next
/// one starts.
fn check_ordered(parsed: &ParsedTracklist) -> Result<(), TestCaseError> {
    for (position, segment) in parsed.segments.iter().enumerate() {
        prop_assert_eq!(segment.index, position);
        if let Some(end) = segment.end {
            prop_assert!(
                end > segment.start,
                "segment {position} ends before it starts"
            );
        }
    }
    for pair in parsed.segments.windows(2) {
        prop_assert!(
            pair[0].start < pair[1].start,
            "starts do not strictly increase"
        );
        prop_assert!(pair[0].end.is_some_and(|end| end <= pair[1].start));
    }
    for pair in parsed.warnings.windows(2) {
        prop_assert!(
            pair[0].line <= pair[1].line,
            "warnings are not in line order"
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn description_segments_are_ordered_and_contiguous(text in description(), total in total()) {
        let parsed = parse_description(&text, total);
        check_ordered(&parsed)?;
        for pair in parsed.segments.windows(2) {
            prop_assert_eq!(pair[0].end, Some(pair[1].start));
        }
        if let Some(last) = parsed.segments.last() {
            prop_assert_eq!(last.end, total);
        }
    }

    #[test]
    fn chapter_segments_are_ordered(chapters in chapters()) {
        check_ordered(&from_chapters(&chapters))?;
    }
}
