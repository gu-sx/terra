//! The description parser. The crate documentation gives its grammar.

use std::time::Duration;

use crate::{
    ParsedTracklist, Segment, Warning, WarningKind, finish, ordering_warning, split_artist_title,
};

/// Parses timestamped track lines from a description.
///
/// `total` is the recording's length: the last segment ends there, and lines
/// at or past it are dropped. Without it the last segment has no end. The
/// crate documentation gives the grammar and the warnings.
pub fn parse_description(text: &str, total: Option<Duration>) -> ParsedTracklist {
    let mut segments: Vec<Segment> = Vec::new();
    let mut warnings = Vec::new();
    let mut previous: Option<(Duration, usize)> = None;
    let mut first_line = None;

    for (number, raw) in text.lines().enumerate() {
        let line = number + 1;
        let source_line = raw.trim();
        let Some((start, text)) = parse_track_line(source_line) else {
            continue;
        };
        if let Some(total) = total
            && start >= total
        {
            warnings.push(Warning {
                line,
                kind: WarningKind::PastTotal { start, total },
            });
            continue;
        }
        if let Some(kind) = ordering_warning(start, previous) {
            warnings.push(Warning { line, kind });
            continue;
        }
        let (artist, title) = split_artist_title(text);
        segments.push(Segment {
            index: segments.len(),
            start,
            end: None,
            artist,
            title,
            source_line: source_line.to_owned(),
        });
        previous = Some((start, line));
        first_line.get_or_insert(line);
    }

    let starts: Vec<Duration> = segments.iter().map(|segment| segment.start).collect();
    for (segment, next) in segments
        .iter_mut()
        .zip(starts.iter().skip(1).map(Some).chain([None]))
    {
        segment.end = next.copied().or(total);
    }
    finish(segments, warnings, first_line)
}

/// A timestamp's byte range in a line, and its value.
#[derive(Debug, Clone, Copy)]
struct Stamp {
    from: usize,
    to: usize,
    at: Duration,
}

/// Returns the start and the remaining text of a track line, or `None` when
/// the line is not one. `line` is already trimmed.
fn parse_track_line(line: &str) -> Option<(Duration, &str)> {
    let body = strip_numbering(line);
    let stamps = find_stamps(body);
    let (at, text) = leading(body, &stamps).or_else(|| trailing(body, &stamps))?;
    Some((at, trim_separators(text)))
}

/// Removes numbering such as `1.`, `01)` or `#1`, followed by whitespace,
/// from the start of the line.
fn strip_numbering(line: &str) -> &str {
    let bytes = line.as_bytes();
    let hash = usize::from(bytes.first() == Some(&b'#'));
    let digits = bytes[hash..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(1..=3).contains(&digits) {
        return line;
    }
    let mut end = hash + digits;
    if hash == 0 {
        match bytes.get(end) {
            Some(b'.' | b')') => end += 1,
            _ => return line,
        }
    }
    match bytes.get(end) {
        Some(byte) if byte.is_ascii_whitespace() => line[end..].trim_start(),
        _ => line,
    }
}

/// Finds every timestamp token in the line, in order.
fn find_stamps(line: &str) -> Vec<Stamp> {
    let bytes = line.as_bytes();
    let mut stamps = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        if let Some(stamp) = stamp_at(line, at) {
            stamps.push(stamp);
            at = stamp.to;
            continue;
        }
        // Skip the rest of this run of digits and colons.
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b':') {
            at += 1;
        }
    }
    stamps
}

/// Parses a timestamp token that starts at `from`, the first digit of a run.
fn stamp_at(line: &str, from: usize) -> Option<Stamp> {
    if line[..from]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == ':')
    {
        return None;
    }
    let bytes = line.as_bytes();
    let (first, mut to) = number(bytes, from, 1..=2)?;
    if bytes.get(to) != Some(&b':') {
        return None;
    }
    let (second, after) = number(bytes, to + 1, 2..=2)?;
    to = after;
    let seconds =
        if bytes.get(to) == Some(&b':') && bytes.get(to + 1).is_some_and(u8::is_ascii_digit) {
            let (third, after) = number(bytes, to + 1, 2..=2)?;
            to = after;
            if second >= 60 || third >= 60 {
                return None;
            }
            first * 3600 + second * 60 + third
        } else {
            if second >= 60 {
                return None;
            }
            first * 60 + second
        };
    if !ends_token(line, to) {
        return None;
    }
    Some(Stamp {
        from,
        to,
        at: Duration::from_secs(seconds),
    })
}

/// Reads a run of ASCII digits at `from` whose length is in `len`, and
/// returns its value and the index after it. A longer run is not a number.
fn number(bytes: &[u8], from: usize, len: std::ops::RangeInclusive<usize>) -> Option<(u64, usize)> {
    let digits = bytes
        .get(from..)?
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !len.contains(&digits) {
        return None;
    }
    let value = bytes[from..from + digits]
        .iter()
        .fold(0, |value, digit| value * 10 + u64::from(digit - b'0'));
    Some((value, from + digits))
}

/// Whether a token may end before index `at`: the next character is not a
/// letter or digit, in any script, nor a `:` or `.` followed by a digit.
fn ends_token(line: &str, at: usize) -> bool {
    let mut rest = line[at..].chars();
    match rest.next() {
        None => true,
        Some(c) if c.is_alphanumeric() => false,
        Some(':' | '.') => !rest.next().is_some_and(|c| c.is_ascii_digit()),
        Some(_) => true,
    }
}

/// The separators that join two timestamps into a range.
const RANGE_SEPARATORS: [&str; 4] = ["-", "–", "—", "~"];

/// Matches a timestamp or range, optionally bracketed, at the start of the
/// line, and returns its start and the text after it.
fn leading<'a>(line: &'a str, stamps: &[Stamp]) -> Option<(Duration, &'a str)> {
    let first = stamps.first()?;
    let (closer, mut at) = match line.as_bytes().first() {
        Some(b'[') => (Some(']'), 1),
        Some(b'(') => (Some(')'), 1),
        _ => (None, 0),
    };
    at = skip_space(line, at);
    if first.from != at {
        return None;
    }
    at = first.to;

    let mut after = skip_space(line, at);
    if let Some(separator) = RANGE_SEPARATORS
        .iter()
        .find(|separator| line[after..].starts_with(**separator))
    {
        after = skip_space(line, after + separator.len());
        if let Some(second) = stamps.get(1)
            && second.from == after
        {
            at = second.to;
        }
    }

    if let Some(closer) = closer {
        at = skip_space(line, at);
        if !line[at..].starts_with(closer) {
            return None;
        }
        at += closer.len_utf8();
    }
    Some((first.at, &line[at..]))
}

/// Matches a timestamp or range, optionally bracketed, at the end of the
/// line, and returns its start and the text before it.
fn trailing<'a>(line: &'a str, stamps: &[Stamp]) -> Option<(Duration, &'a str)> {
    let last = stamps.last()?;
    let (opener, mut at) = match line.as_bytes().last() {
        Some(b']') => (Some('['), line.len() - 1),
        Some(b')') => (Some('('), line.len() - 1),
        _ => (None, line.len()),
    };
    at = skip_space_back(line, at);
    if last.to != at {
        return None;
    }
    at = last.from;
    let mut start = last.at;

    let before = skip_space_back(line, at);
    if let Some(separator) = RANGE_SEPARATORS
        .iter()
        .find(|separator| line[..before].ends_with(**separator))
    {
        let before = skip_space_back(line, before - separator.len());
        if let [.., first, _] = stamps
            && first.to == before
        {
            at = first.from;
            start = first.at;
        }
    }

    if let Some(opener) = opener {
        at = skip_space_back(line, at);
        if !line[..at].ends_with(opener) {
            return None;
        }
        at -= opener.len_utf8();
    }
    Some((start, &line[..at]))
}

/// Returns the index after any whitespace at `at`.
fn skip_space(line: &str, at: usize) -> usize {
    line.len() - line[at..].trim_start().len()
}

/// Returns the index before any whitespace that ends at `at`.
fn skip_space_back(line: &str, at: usize) -> usize {
    line[..at].trim_end().len()
}

/// Trims whitespace and the separators left between a timestamp and the text.
fn trim_separators(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '~' | '|' | ':'))
}
