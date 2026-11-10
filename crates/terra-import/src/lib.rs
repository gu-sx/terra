//! Tracklist detection and normalisation for DJ-set imports (brief §5,
//! pipeline steps 2 and 3).
//!
//! A long recording is split into [`Segment`]s, either from the chapters
//! yt-dlp reports ([`from_chapters`]) or from timestamps in the source's
//! description ([`parse_description`]). Neither function fails: input it
//! cannot use is skipped or reported as a [`Warning`], so the preview step can
//! show what was dropped and why.
//!
//! # Description grammar
//!
//! [`parse_description`] reads the description line by line. Line numbers in
//! warnings count from 1.
//!
//! **Timestamps.** A timestamp has the form `m:ss`, `mm:ss` or `h:mm:ss`
//! (also `hh:mm:ss`). Seconds, and minutes in the hour form, are below 60. A
//! timestamp is a whole token: it is not part of a longer run of letters,
//! digits or colons, so `1:23:456`, `a1:23` and `5:42pm` are not timestamps.
//!
//! **Track lines.** A line is a track line when a timestamp, optionally
//! inside `[]` or `()`, stands
//!
//! - at the start of the line, after optional numbering such as `1.`, `01)`
//!   or `#1` followed by whitespace, or
//! - at the end of the line.
//!
//! When the line starts with a timestamp, that one is used; otherwise the one
//! at the end. Two timestamps joined by `-`, `–`, `—` or `~`, such as
//! `00:00 - 05:42`, count as one range, and the first is the start. A
//! timestamp elsewhere in the line is part of the text. Lines that are not
//! track lines are ignored.
//!
//! **Artist and title.** After the timestamp (or range) and the numbering are
//! removed, whitespace and the separators `-`, `–`, `—`, `~`, `|` and `:` are
//! trimmed from both ends of the remaining text. The text splits into artist
//! and title at the first ` - `, ` – ` or ` — `; without one of those, the
//! whole text is the title and the artist is `None`.
//!
//! **Warnings.** Each of these is a [`WarningKind`]:
//!
//! - the first segment starts after 0:00 ([`WarningKind::FirstStartAfterZero`]);
//! - a start is earlier than the previous segment's start; the line is
//!   dropped ([`WarningKind::NotIncreasing`]);
//! - a start equals the previous segment's start; the later line is dropped
//!   ([`WarningKind::DuplicateTimestamp`]);
//! - a start is at or past the recording's total length; the line is dropped,
//!   since the segment would be empty ([`WarningKind::PastTotal`]).
//!
//! Starts of the returned segments therefore strictly increase.
//!
//! **Ends.** Each segment ends where the next one starts. The last segment
//! ends at the total length, or has no end when the total is unknown.
//!
//! # Example
//!
//! The example from the brief, for a 30-minute recording:
//!
//! ```
//! use std::time::Duration;
//! use terra_import::parse_description;
//!
//! let description = "\
//! 00:00   Artist A - Opening Track
//! 05:42   Artist B - Midnight
//! 11:18   Artist C - After Hours
//! 17:53   Artist D - Sunrise
//! 24:07   Artist E - Closing
//! ";
//! let parsed = parse_description(description, Some(Duration::from_secs(30 * 60)));
//!
//! assert!(parsed.warnings.is_empty());
//! assert_eq!(parsed.segments.len(), 5);
//! let last = &parsed.segments[4];
//! assert_eq!(last.artist.as_deref(), Some("Artist E"));
//! assert_eq!(last.title, "Closing");
//! assert_eq!(last.start, Duration::from_secs(24 * 60 + 7));
//! assert_eq!(last.end, Some(Duration::from_secs(30 * 60)));
//! ```
//!
//! # Chapters
//!
//! [`from_chapters`] takes the `chapters` field of yt-dlp's info JSON. Chapter
//! titles split into artist and title like description text, and the same
//! ordering warnings apply; a warning's line is the chapter's number, from 1.
//! A chapter keeps its own `end_time`, cut back to the next segment's start if
//! it overlaps it; without a usable `end_time` it ends where the next segment
//! starts. A start or end time that is negative or not finite, or an end that
//! is not after the start, gets [`WarningKind::InvalidChapterTime`]: a bad
//! start drops the chapter, a bad end is ignored.

mod chapters;
mod description;

use std::time::Duration;

pub use chapters::{Chapter, from_chapters};
pub use description::parse_description;

/// The segments found in a tracklist, and what was wrong with it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedTracklist {
    /// The segments, ordered by strictly increasing start.
    pub segments: Vec<Segment>,
    /// Warnings, ordered by line.
    pub warnings: Vec<Warning>,
}

/// One track of a long recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Position in [`ParsedTracklist::segments`], from 0.
    pub index: usize,
    /// Offset of the track's start in the recording.
    pub start: Duration,
    /// Offset of the track's end, or `None` for a last segment of a recording
    /// whose length is unknown.
    pub end: Option<Duration>,
    pub artist: Option<String>,
    pub title: String,
    /// The line the segment came from, trimmed, or the chapter's title.
    pub source_line: String,
}

/// Something in the tracklist that was dropped or looks wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Line of the description, or number of the chapter, from 1.
    pub line: usize,
    pub kind: WarningKind,
}

/// The kinds of [`Warning`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningKind {
    /// The first segment starts after 0:00, so the recording's opening is in
    /// no segment.
    FirstStartAfterZero { start: Duration },
    /// The start is earlier than the previous segment's start; the line was
    /// dropped.
    NotIncreasing { start: Duration, previous: Duration },
    /// The start equals the start on `first_line`; the line was dropped.
    DuplicateTimestamp { start: Duration, first_line: usize },
    /// The start is at or past the recording's total length; the line was
    /// dropped.
    PastTotal { start: Duration, total: Duration },
    /// A chapter's start or end time is negative, not finite or out of range,
    /// or its end is not after its start. A bad start drops the chapter; a bad
    /// end is ignored.
    InvalidChapterTime,
}

/// Splits track text into artist and title at the first ` - `, ` – ` or
/// ` — `.
fn split_artist_title(text: &str) -> (Option<String>, String) {
    let split = [" - ", " – ", " — "]
        .iter()
        .filter_map(|separator| text.find(separator).map(|at| (at, separator.len())))
        .min();
    match split {
        Some((at, len)) => {
            let artist = text[..at].trim();
            let title = text[at + len..].trim();
            let artist = (!artist.is_empty()).then(|| artist.to_owned());
            (artist, title.to_owned())
        }
        None => (None, text.trim().to_owned()),
    }
}

/// Checks a candidate start against the previous segment's start and line,
/// and returns the warning that drops it, if any.
fn ordering_warning(start: Duration, previous: Option<(Duration, usize)>) -> Option<WarningKind> {
    let (previous, first_line) = previous?;
    if start == previous {
        Some(WarningKind::DuplicateTimestamp { start, first_line })
    } else if start < previous {
        Some(WarningKind::NotIncreasing { start, previous })
    } else {
        None
    }
}

/// Adds the warning for a first segment that starts after 0:00 and puts the
/// warnings in line order.
fn finish(
    segments: Vec<Segment>,
    mut warnings: Vec<Warning>,
    first_line: Option<usize>,
) -> ParsedTracklist {
    if let (Some(first), Some(line)) = (segments.first(), first_line)
        && !first.start.is_zero()
    {
        warnings.push(Warning {
            line,
            kind: WarningKind::FirstStartAfterZero { start: first.start },
        });
    }
    warnings.sort_by_key(|warning| warning.line);
    ParsedTracklist { segments, warnings }
}
