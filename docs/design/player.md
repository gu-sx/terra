# Player and queue

Status: Proposed, with #7. The responsible engineer approves it by merging that pull request.

`crates/terra-player` plays audio on the machine it runs on (brief §4.2, §14 "Play audio where the user is"). The engine is Rodio, as `docs/decisions/0002-playback-engine.md` decides. This document records the behaviour a user sees, where the brief, the record and #7 leave it open, and the two places where the crate departs from the record's sketch.

## Queue

`Queue<T>` is an ordered list with a cursor and no audio. Every entry gets a new `QueueItemId`, so the same track queued twice is two entries.

The cursor is either **on** an item, the current one, or **between** items, with no current item. A new queue stands before its first item.

| Operation | Effect |
| --- | --- |
| `append` | Adds at the end. Appended to a queue that ran past its end, the item comes next. |
| `insert_next` | Adds after the current item; with none, where `next` would move to. |
| `remove` | Removes an item. Removing the current item leaves the cursor where it was, so `next` moves to the item that followed it. |
| `move_to(item, index)` | Moves an item so it ends up at `index`, or at the end for a larger index. The current item stays current. |
| `clear` | Removes every item. Repeat is kept. |
| `next` | Moves to the next item. At the end: with repeat off, returns nothing and leaves the queue past its end with no current item; with repeat all, moves to the first item. |
| `previous` | Moves to the previous item. At the first item: with repeat off, returns nothing and stays; with repeat all, moves to the last item. Past the end, moves to the last item. |

Shuffle comes later; #7 leaves it out.

## Player

`Player` is a `Clone + Send` handle to a player running on its own thread, which owns a `Queue` of `AudioSource`s and the engine. Each player has its own queue and state, so each device has its own (brief §4.3). The player stops when its last handle is dropped.

Each command returns once the player has applied it; the engine follows within about 10 ms of audio. A command fails only when the player's thread has stopped. Every other failure is a `PlayerEvent::Error`.

| Command | Effect |
| --- | --- |
| `play(source)` | Replaces the queue with `source` and plays it. To play a list, `play` its first track and `append` the rest. |
| `append`, `insert_next`, `remove`, `move_to`, `clear`, `set_repeat` | As on the queue. `clear` also stops. Removing the item that plays moves on to the item that followed it, as if it had ended. |
| `pause`, `resume`, `toggle` | `resume` after `stop` plays the current item from its start, or the next item when there is none. |
| `stop` | Stops playback. The queue and its current item stay. |
| `seek(position)` | Seeks within the current item. |
| `set_volume(v)` | 0.0 to 1.0, clamped; NaN is ignored. The control is perceptual: the gain is v³, as mpv's (the record's "Volume" consequence). The change is a step, without a ramp. |
| `next` | Plays the next item. At the end of the queue, with repeat off, it stops. |
| `previous` | Plays the previous item. At the first item, with repeat off, it plays that item again from its start. |

`next` and `previous` while paused load the item and stay paused; while stopped they only move the cursor.

Events arrive on a channel, in the order things happen:

- `TrackStarted { item }`: the item's first sample has gone to the output.
- `Position { item, elapsed, total }`: about every 100 ms of the item's audio, and at once after a seek. `elapsed` counts the samples played, so it is exact; `total` is `None` when the source does not say.
- `TrackEnded { item }`: the item has played to its end. An item that is skipped, removed or stopped does not end.
- `Stopped`: playback has stopped, by a command or because the queue ran out.
- `Error { item, error }`. An item that cannot be opened, such as a missing file, is reported and skipped.

Positions and events refer to the samples handed to the output, which the device plays one buffer later (42.7 ms in the record's measurement).

### Outputs

`Player::open(Output::Device)` plays on the default device through cpal's ALSA host. `Output::Null` plays in real time to no device, in stereo at 48 kHz, for a headless run such as #12's `TERRA_AUDIO_OUTPUT=null`. `Player::capture(channels, sample_rate)` gives the caller the samples to pull in that format, as a device would, for tests; nothing plays until they are pulled.

Stream URLs (`AudioSource::Url`) are reported as unsupported until Phase 3.

## Gapless playback

While an item plays, the player hands the engine the item that follows it, so the engine moves to it at the exact sample where the current one ends. When the queue changes what comes next, by `append`, `insert_next`, `remove`, `move_to` or `set_repeat`, the player hands over the new one in its place.

### Departures from the record's sketch

The record sketches the engine as an `enqueue` onto Rodio's queue, bracketed by `EmptyCallback` sources. Rodio's queue cannot replace an item once it is appended, and Rodio's `Player::clear` and `Player::try_seek` block until the output has been pulled. So:

1. **The engine trait has `load` and `set_next` in place of `enqueue`.** `load` starts an item now; `set_next` sets, or replaces, the one that follows. `seek` reports its result as an event instead of returning it.
2. **The Rodio engine plays through one source of its own, the deck**, appended once to a Rodio `Player`, which still applies pause and volume. The engine posts operations to the deck, which applies them on the output's thread, in order, every 10 ms of audio. The deck reports start, end and position from the samples it hands out, which gives the exact events the record found in Rodio.

The record's capture method still holds: tests pull the Rodio `Player`'s output without a device, and the gapless test analyses it with the record's `analysis.rs`.

The deck plays in the output's format: the device's channel count and sample rate. It converts each item to that format with Rodio's own converters, one for the whole item, and reports its output to Rodio as one endless span. Rodio's mixer otherwise starts a new sample-rate converter at every span of its input, for example every packet of a decoder, and loses audio at each boundary: in a test, 2 s of 44.1 kHz audio came out of a 48 kHz mixer 202 samples short and drifting in phase. Through the deck the length is right and the phase holds. Because each item has its own converter, an item at another rate than the output starts up to one output sample off the grid of the item before it, within the record's ±1-sample threshold.

## Known limits

- Symphonia 0.5 ignores the MP4 edit list (the record's "AAC in M4A" consequence), so AAC and ALAC in M4A play through a source of the crate's own that applies it (#23): it plays only the samples the edit presents and seeks by timestamp, which makes AAC gapless and its timeline exact. It applies one edit at normal rate, after any empty edits; a file with several edits, or none, plays the whole media. It does not read an `iTunSMPB` tag, so an AAC file that carries one instead of an edit list keeps its priming and padding.
- A seek or a skip while paused takes effect when playback resumes.
