//! Segments from yt-dlp chapters.

use std::time::Duration;

use serde::Deserialize;

use crate::{
    ParsedTracklist, Segment, Warning, WarningKind, finish, ordering_warning, split_artist_title,
};

/// One entry of the `chapters` field in yt-dlp's info JSON (`--dump-json`).
///
/// Checked against yt-dlp 2026.08.19: extractors give `start_time`, and
/// optionally `end_time` and `title`, in seconds; yt-dlp then fills in a
/// missing `end_time` from the next chapter's start or the video's duration,
/// and a missing title as `<Untitled Chapter N>`. `end_time` stays `null` for
/// the last chapter when the duration is unknown. Other fields are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Chapter {
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub title: String,
}

/// Turns yt-dlp chapters into segments.
///
/// The crate documentation describes how ends are chosen and which warnings
/// apply.
pub fn from_chapters(chapters: &[Chapter]) -> ParsedTracklist {
    let mut segments: Vec<Segment> = Vec::new();
    let mut warnings = Vec::new();
    let mut previous: Option<(Duration, usize)> = None;
    let mut first_line = None;

    for (number, chapter) in chapters.iter().enumerate() {
        let line = number + 1;
        let Ok(start) = Duration::try_from_secs_f64(chapter.start_time) else {
            warnings.push(Warning {
                line,
                kind: WarningKind::InvalidChapterTime,
            });
            continue;
        };
        if let Some(kind) = ordering_warning(start, previous) {
            warnings.push(Warning { line, kind });
            continue;
        }
        let end = match chapter.end_time.map(Duration::try_from_secs_f64) {
            None => None,
            Some(Ok(end)) if end > start => Some(end),
            Some(_) => {
                warnings.push(Warning {
                    line,
                    kind: WarningKind::InvalidChapterTime,
                });
                None
            }
        };
        let (artist, title) = split_artist_title(chapter.title.trim());
        segments.push(Segment {
            index: segments.len(),
            start,
            end,
            artist,
            title,
            source_line: chapter.title.clone(),
        });
        previous = Some((start, line));
        first_line.get_or_insert(line);
    }

    let starts: Vec<Duration> = segments.iter().map(|segment| segment.start).collect();
    for (segment, next) in segments.iter_mut().zip(starts.iter().skip(1)) {
        segment.end = Some(segment.end.map_or(*next, |end| end.min(*next)));
    }
    finish(segments, warnings, first_line)
}
