# Playback in the TUI

## Status

Proposed, with #12. The responsible engineer approves it by merging that pull request.

## Scope

How the TUI drives the player (`docs/design/player.md`): the keys, the bottom bar, the queue view and when a play is recorded. #12 sets the keys and the bar (brief §4.2 and §9); this document records the behaviour a user sees where the issue leaves it open.

## Keys

The playback keys work in the library lists and in the queue view, and act at once, even while a list loads. `e`, `E` and `Enter` act on the selected track of the main list, so they wait for it like the other browsing keys.

| Key | Effect |
| --- | --- |
| `Enter` on a track | Replaces the queue with the track and the tracks after it in the list, and plays. On an artist, an album or a sidebar entry it opens it, as before. In search results, the tracks after it are the track results below it. |
| `e` | Adds the selected track at the end of the queue. Nothing starts playing. |
| `E` | Adds the selected track after the current one. Two `E` in a row put the second before the first. |
| `Space` | Pauses or resumes. Stopped, it plays the current item from its start, or the next one when there is none, as the player's `resume`. |
| `>` `<` | Next or previous track, as the player's `next` and `previous`: `<` at the first track plays it again from its start. |
| `[` `]`, `{` `}` | Back or forward 5 s, or 30 s, from the last position the player reported, or from the last seek when the player has not yet reported it, so that repeated presses add up. Back stops at the start. Forward past the end of the track skips to the next one. |
| `-` `+` | Volume down or up in steps of 5 %, from 100 %. The status line shows the new volume. |
| `Q` | Opens or closes the queue view. |

## Bottom bar

The first line shows the play state (`▶` playing, `‖` paused, `■` stopped), `title — artist` and `elapsed/total`; the second a gauge of the elapsed part. The total is the one the player reports, or the library's duration until it does. With no current item the bar shows "■ Nothing playing" and an empty gauge.

The status line under it shows errors until the next key: a track the backend cannot resolve and a file the player cannot open, each as `Cannot play “<title>”: <reason>`, and a play that could not be recorded.

## Queue

The TUI keeps no copy of the queue. The player moves through it on its own, for example past an item that fails to open, so the TUI reads the player's `status()` after each of its commands and each event other than a position.

Before tracks reach the queue, the TUI asks the backend where their audio is, one job at a time in the order the keys were pressed, so that tracks reach the queue in that order. `Enter` drops the jobs still waiting, since it replaces the queue, and asks for the selected track alone first, so that it starts without waiting for the rest of a long list. A track the backend cannot resolve is reported and skipped; if it was the selected one, the next track that resolves plays. As the rest of the list arrives, a player that has stopped, because the first track failed to open or already ended, plays on.

The queue view covers the sidebar and the list. It lists every item, marks the current one with the play state, and opens on it. `j` `k` `g` `G` move, `x` removes the selected item, `J` and `K` move it down or up, and `Esc` or `Q` close the view. Removing the item that plays moves on to the next one, as the player does.

## Recording plays

A play is recorded once each time an item starts, when the position the player reports reaches half the track or four minutes, whichever comes first. The track's length is the one the player reports, or the library's when it does not; with neither, four minutes. Playing the same item again, by `<` at the first track, is another play. A seek forward past the threshold counts as reaching it. The device is the machine's host name, or `unknown` when it cannot be read.

## Audio output

`TERRA_AUDIO_OUTPUT`, which the help does not list, chooses the output: unset or empty for the default device, `null` for none, which plays in real time and discards the audio, for a headless run or a test. Another value is an error. When the output cannot be opened the TUI still starts, without playback: the status line says why at the start and on each playback key.

## Help

The help overlay has grown taller than an 80×24 terminal, so it scrolls: `j` and `k`, or the arrows, scroll it, as its bottom border says.
