//! Table tests for the description grammar.

use std::time::Duration;

use terra_import::{ParsedTracklist, Warning, WarningKind, parse_description};

/// Seconds from `h:mm:ss` parts.
fn hms(hours: u64, minutes: u64, seconds: u64) -> Duration {
    Duration::from_secs(hours * 3600 + minutes * 60 + seconds)
}

/// A segment as the tables give it: start, end, artist and title.
type Expected = (
    Duration,
    Option<Duration>,
    Option<&'static str>,
    &'static str,
);

/// A warning case: name, input, total, segments and warnings.
type WarningCase = (
    &'static str,
    &'static str,
    Option<Duration>,
    Vec<Expected>,
    Vec<Warning>,
);

fn segments(parsed: &ParsedTracklist) -> Vec<(Duration, Option<Duration>, Option<&str>, &str)> {
    parsed
        .segments
        .iter()
        .map(|segment| {
            (
                segment.start,
                segment.end,
                segment.artist.as_deref(),
                segment.title.as_str(),
            )
        })
        .collect()
}

fn assert_indexes(parsed: &ParsedTracklist) {
    for (position, segment) in parsed.segments.iter().enumerate() {
        assert_eq!(segment.index, position);
    }
}

#[test]
fn brief_example_with_total() {
    let description = "\
00:00   Artist A - Opening Track
05:42   Artist B - Midnight
11:18   Artist C - After Hours
17:53   Artist D - Sunrise
24:07   Artist E - Closing
";
    let parsed = parse_description(description, Some(hms(0, 30, 0)));

    assert_eq!(parsed.warnings, []);
    assert_indexes(&parsed);
    assert_eq!(
        segments(&parsed),
        [
            (
                hms(0, 0, 0),
                Some(hms(0, 5, 42)),
                Some("Artist A"),
                "Opening Track"
            ),
            (
                hms(0, 5, 42),
                Some(hms(0, 11, 18)),
                Some("Artist B"),
                "Midnight"
            ),
            (
                hms(0, 11, 18),
                Some(hms(0, 17, 53)),
                Some("Artist C"),
                "After Hours"
            ),
            (
                hms(0, 17, 53),
                Some(hms(0, 24, 7)),
                Some("Artist D"),
                "Sunrise"
            ),
            (
                hms(0, 24, 7),
                Some(hms(0, 30, 0)),
                Some("Artist E"),
                "Closing"
            ),
        ]
    );
    assert_eq!(
        parsed.segments[1].source_line,
        "05:42   Artist B - Midnight"
    );
}

/// Inputs without warnings, and the segments they give with no total.
#[test]
fn formats() {
    let cases: Vec<(&str, &str, Vec<Expected>)> = vec![
        (
            "h:mm:ss",
            "0:00:00 Artist A - One\n1:02:03 Artist B - Two\n10:00:00 Artist C - Three",
            vec![
                (hms(0, 0, 0), Some(hms(1, 2, 3)), Some("Artist A"), "One"),
                (hms(1, 2, 3), Some(hms(10, 0, 0)), Some("Artist B"), "Two"),
                (hms(10, 0, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "mixed formats",
            "0:00 Artist A - One\n59:59 Artist B - Two\n1:00:00 Artist C - Three\n01:30:15 Artist D - Four",
            vec![
                (hms(0, 0, 0), Some(hms(0, 59, 59)), Some("Artist A"), "One"),
                (hms(0, 59, 59), Some(hms(1, 0, 0)), Some("Artist B"), "Two"),
                (
                    hms(1, 0, 0),
                    Some(hms(1, 30, 15)),
                    Some("Artist C"),
                    "Three",
                ),
                (hms(1, 30, 15), None, Some("Artist D"), "Four"),
            ],
        ),
        (
            "bracketed timestamps",
            "[00:00] Artist A - One\n(04:10) Artist B - Two\n[ 1:08:00 ] Artist C - Three",
            vec![
                (hms(0, 0, 0), Some(hms(0, 4, 10)), Some("Artist A"), "One"),
                (hms(0, 4, 10), Some(hms(1, 8, 0)), Some("Artist B"), "Two"),
                (hms(1, 8, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "numbered lines",
            "1. 00:00 Artist A - One\n02) 03:00 Artist B - Two\n#3 [06:00] Artist C - Three\n4. Artist D - Four 09:00",
            vec![
                (hms(0, 0, 0), Some(hms(0, 3, 0)), Some("Artist A"), "One"),
                (hms(0, 3, 0), Some(hms(0, 6, 0)), Some("Artist B"), "Two"),
                (hms(0, 6, 0), Some(hms(0, 9, 0)), Some("Artist C"), "Three"),
                (hms(0, 9, 0), None, Some("Artist D"), "Four"),
            ],
        ),
        (
            "timestamp at the end of the line",
            "Artist A - One 0:00\nArtist B - Two (3:30)\nArtist C - Three [1:07:00]",
            vec![
                (hms(0, 0, 0), Some(hms(0, 3, 30)), Some("Artist A"), "One"),
                (hms(0, 3, 30), Some(hms(1, 7, 0)), Some("Artist B"), "Two"),
                (hms(1, 7, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "two timestamps: the first is the start",
            "00:00 - 03:10 Artist A - One\n[03:10 – 07:00] Artist B - Two\nArtist C - Three 07:00 - 09:00",
            vec![
                (hms(0, 0, 0), Some(hms(0, 3, 10)), Some("Artist A"), "One"),
                (hms(0, 3, 10), Some(hms(0, 7, 0)), Some("Artist B"), "Two"),
                (hms(0, 7, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "two timestamps without a separator: the first is the start",
            "00:00 03:00 Artist A - One\nArtist B - Two 03:00 05:00\n(05:00 07:00) Artist C - Three",
            vec![
                (hms(0, 0, 0), Some(hms(0, 3, 0)), Some("Artist A"), "One"),
                (hms(0, 3, 0), Some(hms(0, 5, 0)), Some("Artist B"), "Two"),
                (hms(0, 5, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "en and em dashes",
            "00:00 Artist A – One\n02:00 Artist B — Two\n04:00 Artist C - Part – Two",
            vec![
                (hms(0, 0, 0), Some(hms(0, 2, 0)), Some("Artist A"), "One"),
                (hms(0, 2, 0), Some(hms(0, 4, 0)), Some("Artist B"), "Two"),
                (hms(0, 4, 0), None, Some("Artist C"), "Part – Two"),
            ],
        ),
        (
            "separators between the timestamp and the text",
            "00:00 - Artist A - One\n01:00 | Artist B - Two\n02:00: Artist C - Three",
            vec![
                (hms(0, 0, 0), Some(hms(0, 1, 0)), Some("Artist A"), "One"),
                (hms(0, 1, 0), Some(hms(0, 2, 0)), Some("Artist B"), "Two"),
                (hms(0, 2, 0), None, Some("Artist C"), "Three"),
            ],
        ),
        (
            "no artist separator",
            "00:00 Intro\n01:00 Artist-B-Hyphenated\n02:00 Artist C -Three",
            vec![
                (hms(0, 0, 0), Some(hms(0, 1, 0)), None, "Intro"),
                (
                    hms(0, 1, 0),
                    Some(hms(0, 2, 0)),
                    None,
                    "Artist-B-Hyphenated",
                ),
                (hms(0, 2, 0), None, None, "Artist C -Three"),
            ],
        ),
        (
            "a timestamp inside the text stays in the title, and the first is the start",
            "00:00 Artist A - Live at 12:00\nArtist B - Ends 4:00 here 05:00",
            vec![
                (
                    hms(0, 0, 0),
                    Some(hms(0, 4, 0)),
                    Some("Artist A"),
                    "Live at 12:00",
                ),
                (hms(0, 4, 0), None, Some("Artist B"), "Ends 4:00 here"),
            ],
        ),
        (
            "Unicode titles",
            "00:00 Artiste É - Café del Mar\n03:00 Künstler Ö — Straße\n06:00 アーティスト - 夜明け 🌅",
            vec![
                (
                    hms(0, 0, 0),
                    Some(hms(0, 3, 0)),
                    Some("Artiste É"),
                    "Café del Mar",
                ),
                (
                    hms(0, 3, 0),
                    Some(hms(0, 6, 0)),
                    Some("Künstler Ö"),
                    "Straße",
                ),
                (hms(0, 6, 0), None, Some("アーティスト"), "夜明け 🌅"),
            ],
        ),
        (
            "CRLF line endings",
            "00:00 Artist A - One\r\n01:00 Artist B - Two\r\n",
            vec![
                (hms(0, 0, 0), Some(hms(0, 1, 0)), Some("Artist A"), "One"),
                (hms(0, 1, 0), None, Some("Artist B"), "Two"),
            ],
        ),
    ];
    for (name, input, expected) in cases {
        let parsed = parse_description(input, None);
        assert_eq!(parsed.warnings, [], "{name}");
        assert_indexes(&parsed);
        assert_eq!(segments(&parsed), expected, "{name}");
    }
}

#[test]
fn ignored_text() {
    let cases: &[(&str, &str)] = &[
        ("empty", ""),
        ("prose", "Recorded live at Venue A.\nThanks for listening!"),
        (
            "hashtags and links",
            "#set #live\nhttps://example.com/set:8080/a",
        ),
        (
            "timestamp in the middle only",
            "The drop at 12:00 is the best part of the set",
        ),
        (
            "not timestamps",
            "1:2 Artist\n12:345 Artist\n1:60 Artist\n1:00:60 Artist\n123:00 Artist",
        ),
        (
            "glued to letters or digits",
            "a0:00 Artist\n0:00am Artist\n0:00.5 Artist\n0:00:0 Artist",
        ),
        (
            "glued to letters in other scripts",
            "Artist A - Café1:23\n1:23é Artist B\nArtist C - 夜1:23",
        ),
        (
            "unmatched brackets",
            "[00:00) Artist A - One\nArtist B - Two (01:00]",
        ),
        ("numbering without a space", "1.00:00 Artist"),
    ];
    for (name, input) in cases {
        let parsed = parse_description(input, Some(hms(1, 0, 0)));
        assert_eq!(parsed, ParsedTracklist::default(), "{name}");
    }
}

#[test]
fn ignored_lines_among_tracks() {
    let description = "\
Tracklist:

00:00 Artist A - One
(this one is a request)
04:00 Artist B - Two
";
    let parsed = parse_description(description, Some(hms(0, 8, 0)));
    assert_eq!(parsed.warnings, []);
    assert_eq!(
        segments(&parsed),
        [
            (hms(0, 0, 0), Some(hms(0, 4, 0)), Some("Artist A"), "One"),
            (hms(0, 4, 0), Some(hms(0, 8, 0)), Some("Artist B"), "Two"),
        ]
    );
}

#[test]
fn warnings_and_edge_cases() {
    let cases: Vec<WarningCase> = vec![
        (
            "first start after 0:00",
            "Intro text\n00:30 Artist A - One\n02:00 Artist B - Two",
            Some(hms(0, 4, 0)),
            vec![
                (hms(0, 0, 30), Some(hms(0, 2, 0)), Some("Artist A"), "One"),
                (hms(0, 2, 0), Some(hms(0, 4, 0)), Some("Artist B"), "Two"),
            ],
            vec![Warning {
                line: 2,
                kind: WarningKind::FirstStartAfterZero {
                    start: hms(0, 0, 30),
                },
            }],
        ),
        (
            "timestamps don't increase",
            "00:00 Artist A - One\n05:00 Artist B - Two\n03:00 Artist C - Three\n07:00 Artist D - Four",
            Some(hms(0, 9, 0)),
            vec![
                (hms(0, 0, 0), Some(hms(0, 5, 0)), Some("Artist A"), "One"),
                (hms(0, 5, 0), Some(hms(0, 7, 0)), Some("Artist B"), "Two"),
                (hms(0, 7, 0), Some(hms(0, 9, 0)), Some("Artist D"), "Four"),
            ],
            vec![Warning {
                line: 3,
                kind: WarningKind::NotIncreasing {
                    start: hms(0, 3, 0),
                    previous: hms(0, 5, 0),
                },
            }],
        ),
        (
            "two lines have the same timestamp",
            "00:00 Artist A - One\n02:00 Artist B - Two\n\n02:00 Artist C - Three",
            Some(hms(0, 4, 0)),
            vec![
                (hms(0, 0, 0), Some(hms(0, 2, 0)), Some("Artist A"), "One"),
                (hms(0, 2, 0), Some(hms(0, 4, 0)), Some("Artist B"), "Two"),
            ],
            vec![Warning {
                line: 4,
                kind: WarningKind::DuplicateTimestamp {
                    start: hms(0, 2, 0),
                    first_line: 2,
                },
            }],
        ),
        (
            "timestamps at and past total",
            "00:00 Artist A - One\n10:00 Artist B - Two\n12:00 Artist C - Three",
            Some(hms(0, 10, 0)),
            vec![(hms(0, 0, 0), Some(hms(0, 10, 0)), Some("Artist A"), "One")],
            vec![
                Warning {
                    line: 2,
                    kind: WarningKind::PastTotal {
                        start: hms(0, 10, 0),
                        total: hms(0, 10, 0),
                    },
                },
                Warning {
                    line: 3,
                    kind: WarningKind::PastTotal {
                        start: hms(0, 12, 0),
                        total: hms(0, 10, 0),
                    },
                },
            ],
        ),
        (
            "every warning, in line order",
            "05:00 Artist A - One\n01:00 Artist B - Two\n05:00 Artist C - Three\n20:00 Artist D - Four",
            Some(hms(0, 10, 0)),
            vec![(hms(0, 5, 0), Some(hms(0, 10, 0)), Some("Artist A"), "One")],
            vec![
                Warning {
                    line: 1,
                    kind: WarningKind::FirstStartAfterZero {
                        start: hms(0, 5, 0),
                    },
                },
                Warning {
                    line: 2,
                    kind: WarningKind::NotIncreasing {
                        start: hms(0, 1, 0),
                        previous: hms(0, 5, 0),
                    },
                },
                Warning {
                    line: 3,
                    kind: WarningKind::DuplicateTimestamp {
                        start: hms(0, 5, 0),
                        first_line: 1,
                    },
                },
                Warning {
                    line: 4,
                    kind: WarningKind::PastTotal {
                        start: hms(0, 20, 0),
                        total: hms(0, 10, 0),
                    },
                },
            ],
        ),
        (
            "no total given: the last segment has no end",
            "00:00 Artist A - One\n99:59 Artist B - Two",
            None,
            vec![
                (hms(0, 0, 0), Some(hms(0, 99, 59)), Some("Artist A"), "One"),
                (hms(0, 99, 59), None, Some("Artist B"), "Two"),
            ],
            vec![],
        ),
        (
            "a timestamp with no text has an empty title",
            "00:00\n01:00 Artist B - Two",
            Some(hms(0, 2, 0)),
            vec![
                (hms(0, 0, 0), Some(hms(0, 1, 0)), None, ""),
                (hms(0, 1, 0), Some(hms(0, 2, 0)), Some("Artist B"), "Two"),
            ],
            vec![],
        ),
        (
            "Unicode titles keep their warnings' line numbers",
            "00:00 Artiste É - Café\n00:00 Künstler Ö — Straße",
            Some(hms(0, 2, 0)),
            vec![(hms(0, 0, 0), Some(hms(0, 2, 0)), Some("Artiste É"), "Café")],
            vec![Warning {
                line: 2,
                kind: WarningKind::DuplicateTimestamp {
                    start: hms(0, 0, 0),
                    first_line: 1,
                },
            }],
        ),
    ];
    for (name, input, total, expected, warnings) in cases {
        let parsed = parse_description(input, total);
        assert_eq!(parsed.warnings, warnings, "{name}");
        assert_indexes(&parsed);
        assert_eq!(segments(&parsed), expected, "{name}");
    }
}
