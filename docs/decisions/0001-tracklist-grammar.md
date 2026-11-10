# 0001. Tracklist grammar for DJ-set imports

## Status

Proposed

## Context

The importer splits a long recording into tracks (brief §5). Pipeline steps 2 and 3 take the boundaries from yt-dlp's chapters when there are any, and otherwise from timestamps in the source's description. Brief §13.6 leaves the tracklist grammar open, along with inaccurate timestamps, boundary review and cancel/retry behaviour.

Descriptions are written by hand and vary: timestamps come as `m:ss`, `mm:ss` or `h:mm:ss`, sometimes in brackets, numbered, at the end of the line, or as a range. Artist and title are separated by a hyphen, an en dash or an em dash. Prose, links and hashtags sit between the track lines. Some lists repeat a timestamp, go backwards, start after 0:00 or run past the end of the recording.

Work package #4 proposed a grammar and the responsible engineer approved it with the package. This record writes it down, together with the points the proposal left open, which pull request #14 settled. The crate documentation of `terra-import` carries the same grammar, with examples.

yt-dlp 2026.08.19 gives each chapter a `start_time` and, optionally, an `end_time` and `title`. It fills in a missing end from the next chapter's start or the video's duration, and a missing title as `<Untitled Chapter N>`. The last chapter's `end_time` stays `null` when the duration is unknown.

## Decision

Parsing never fails. Lines it cannot use are ignored, and lines it drops or doubts are reported as warnings with their line number, so the review step can show them.

**Timestamps.** A timestamp has the form `m:ss`, `mm:ss`, `h:mm:ss` or `hh:mm:ss`. Seconds, and minutes in the hour form, are below 60. A timestamp is a whole token: it does not touch a letter or digit in any script, or a colon, so `1:23:456`, `Café1:23` and `5:42pm` are not timestamps.

**Track lines.** A line is a track line when a timestamp, optionally inside `[]` or `()`, stands at the start of the line or at the end of it. At the start it may follow numbering such as `1.`, `01)` or `#1` and whitespace. Other lines are ignored.

**Start.** The first timestamp of a track line is its start, wherever it stands.

**Text.** The timestamp at the start of the line, or else the one at the end, is removed from the text, together with a second timestamp next to it. The two may be separated by whitespace or by `-`, `–`, `—` or `~`, as in `00:00 - 05:42`. Other timestamps stay in the text. Numbering is removed too. Whitespace and the separators `-`, `–`, `—`, `~`, `|` and `:` are trimmed from both ends of what remains.

**Artist and title.** The text splits into artist and title at the first ` - `, ` – ` or ` — `. Without one of those, the whole text is the title and there is no artist. A line holding only a timestamp gives a segment with an empty title.

**Warnings.**

- The first segment starts after 0:00. The segment is kept.
- A start is earlier than the previous segment's start. The line is dropped.
- A start equals the previous segment's start. The later line is dropped.
- A start is at or past the recording's total length. The line is dropped, because the segment would be empty.

The starts of the segments therefore strictly increase.

**Ends.** Each segment ends where the next one starts. The last segment ends at the total length, or has no end when the length is unknown.

**Chapters.** Chapter titles split into artist and title like description text. The same ordering warnings apply, numbered by chapter. A chapter keeps its own end, cut back to the next segment's start when they overlap. Without a usable end, the chapter ends where the next segment starts. A start or end that is negative or not finite, or an end that is not after the start, gets an invalid-time warning. A bad start drops the chapter, and a bad end is ignored.

## Consequences

- Every description has exactly one reading, which table tests and property tests in `crates/terra-import` pin down. Changing the grammar means superseding this record and changing those tests together.
- A track whose title contains a time, such as `Artist A - Live at 12:00`, keeps that time in its title only when the line's own timestamp comes first. In `Artist B - At 4:00 here 05:00`, the start is 4:00. The review step is where a user corrects such lines.
- Lines with a timestamp only in the middle are ignored, so a tracklist in that form yields no segments rather than wrong ones.
- Dropped lines are not lost: each one is a warning with its line number, which the review step can offer to restore or edit.
- Still open under §13.6, for Phase 2: correcting inaccurate timestamps, the interactive boundary review, and cancelling and retrying imports.
