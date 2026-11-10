# 0002. Playback engine

## Status

Accepted

## Context

Brief §4.2 and §13 question 1 leave the playback engine open: Rodio, or playback backed by mpv, with gapless playback tested explicitly. The player crate (#7) cannot start until one is chosen. This record answers the question from measurements taken on 2026-11-10 in a one-session spike (#2). Every figure below comes from the prototype in Appendix A, run with the commands under "Reproduce".

### Candidates and versions

| Candidate | What was measured |
| --- | --- |
| Rodio | `rodio` 0.23.0 (released 2026-10-11) with the Symphonia 0.5.5 decoders it pins, `cpal` 0.18.2 for output |
| Rodio, Opus | Symphonia 0.5.5 driven directly, with `symphonia-adapter-libopus` 0.2.9 against the system libopus 1.6.1 (see criterion 1) |
| Rodio, HTTP | `stream-download` 0.24.5, which gives Rodio a `Read + Seek` over HTTP range requests |
| mpv, subprocess | mpv 0.41.0 (Arch `1:0.41.0-6`, FFmpeg n9.0.2), driven over its JSON IPC socket |
| mpv, libmpv | `libmpv2` 7.0.1 (released 2026-11-07) with `libmpv2-sys` 4.0.1, against libmpv 2.5.0 |

The original `libmpv` (2.0.1) and `libmpv-sys` (3.1.0) crates have had no release since 2020, so the binding measured is their maintained fork `libmpv2`, which released 5.0.0 on 2025-07-27 and six versions since. `rsmpv` (first release 2026-09-04) is too new to judge; `mpv-client` and `mpvipc` are GPL-3.0.

Machine: Arch Linux, kernel 7.2.8, rustc 1.99.1, PipeWire 1.6.9 with its PulseAudio and ALSA compatibility layers. Fixtures sat in tmpfs.

### Method

**Fixtures.** `gen-fixtures.sh` (Appendix A) uses ffmpeg to generate everything; all of it is synthetic.

- *Gapless:* a continuous 20 s, 1 kHz sine at 48 kHz, mono, amplitude 0.5, split at sample 480 517, which is not a multiple of any codec's frame size. Each half is encoded as MP3 (LAME, 192 kbit/s CBR), FLAC, Opus in Ogg (libopus, 128 kbit/s), AAC in M4A (FFmpeg's encoder, 192 kbit/s) and ALAC in M4A. `ffmpeg -i a.<fmt> -f s16le -` decodes every half to exactly its source length, 480 517 samples, so all five containers carry correct gapless metadata.
- *Marker:* the same tone, silent before sample 3000 and after sample 50 999, in each format. Its onset shows how many samples an engine adds or drops at the start of a file, which the periodic tone alone can resolve only modulo its 48-sample period.
- *Long:* 60 minutes of seeded pink noise at 48 kHz, mono, as MP3, FLAC, Opus and AAC in M4A; the WAV it was encoded from is the reference.

**Capture without an output device.**

- *Rodio:* both decoders are appended to `rodio::Player::new()`, and the program pulls the returned `SourcesQueueOutput` sample by sample, as the device callback would.
- *mpv:* `--ao=pcm --audio-format=float --audio-samplerate=48000 --audio-channels=mono` writes every sample mpv plays to a WAV file, with both files on one command line.
- *libmpv:* the same options, set through `Mpv::with_initializer`, with both files loaded by `loadfile`.

**Join analysis** (`analysis.rs`). An engine's output is the continuous sine delayed by `lead` samples before the join and by `lead + gap` after it. A least-squares sine fit over 100 periods gives each delay modulo the 48-sample period; the marker's onset (for `lead`) and the signal envelope (for the rest) resolve it. A positive `gap` means samples were inserted at the join and a negative one that samples were lost. The *join residual* is the largest |output − ideal sine| within 2048 samples either side of the join, outside the gap itself; the *steady residual* is the same measure far from the join, that is, the codec's own error.

**Seeking.** After each seek the first 8192 samples are located in the reference by cross-correlation within ±2 s; the error is in samples, positive when late. Rodio's latency is the wall time of `Decoder::try_seek` plus the first sample. mpv's is the wall time from the IPC command `seek <t> absolute+exact` to the `playback-restart` event, with `--ao=null` and paused; libmpv's the same through `Mpv::command`. mpv's accuracy comes from `--start=<t>` captured with `--ao=pcm`.

**HTTP.** The prototype's test server serves the fixtures on 127.0.0.1 with byte ranges, throttled to 4 MB/s (32 Mbit/s) to stand in for a link to the homelab, and logs every request with its `Range` header. Unthrottled, localhost serves the whole 86 MB file before a seek happens, so it does not exercise range requests.

### Results

| Criterion | Rodio 0.23 | mpv 0.41, subprocess (JSON IPC) | mpv 0.41, libmpv (`libmpv2` 7.0.1) |
| --- | --- | --- | --- |
| 1. Formats | MP3, FLAC, AAC/M4A, ALAC/M4A (ALAC needs the non-default feature `symphonia-alac`). **Opus: no.** Works with 125 lines of own code and libopus | All five | All five |
| 2. Gapless, join gap in samples | MP3 0, FLAC 0, ALAC 0, Opus (own code) 0. **AAC/M4A +1787** | MP3 0, FLAC 0, Opus 0, ALAC 0. **AAC/M4A −1024** | Same as the subprocess |
| 3. Seek on the 60-min file | Error 0 or −1 samples in MP3, FLAC, Opus (own code); **AAC −1024**. Latency 0.01–1.94 ms | Error 0 or −1 samples in all four formats. Latency 0.16–9.69 ms | Latency 0.10–9.80 ms; `time-pos` equals the target |
| 4. HTTP, seek to 2400 s at 4 MB/s | MP3 14.5 s; FLAC 600 ms; AAC 80 ms, all accurate (AAC −1024 as locally); Opus not tried (no HTTP glue) | MP3 14.6 s; FLAC 76 ms; Opus 21 ms; AAC 4.2 ms, all accurate | Same core as the subprocess (not run separately) |
| 5. Devices | ALSA host by default, through PipeWire's ALSA plugin; native PipeWire host with `cpal/pipewire`. No device: an error in about 5 ms | Native `ao=pipewire`. No device: `end-file` with reason `error` | As the subprocess, but in-process |
| 5. No device, hangs | Default build: 0 in 60 runs. With `cpal/pipewire`: 4 in 60, in libpipewire | 2 in 21 runs, in libpipewire; the process can be killed | Same library; a hang would block a thread of the application |
| 6. Volume | Linear factor, applied within 237 samples (5 ms) | Cubic: `volume=25` multiplies by 0.0156 | Same as the subprocess |
| 6. Position | Exact to within one 5 ms update | Updates about every 80–100 ms | Same as the subprocess |
| 6. End of track | Callback after exactly 480 517 samples | `end-file` 0.27 s (PipeWire) to 0.47 s (`ao=null`) before the track ends; meanwhile the next track is reported at `time-pos` 0 | Same as the subprocess |
| 7. Build and runtime | ALSA headers to build, `libasound.so.2` to run; libopus for Opus | None to build; the `mpv` binary to run | `libmpv.so.2` to link and run, which pulls 203 shared libraries |
| 7. `unsafe` in dependencies | `alsa` 762, `cpal` ALSA host 10, `rodio` 1, Symphonia ≤ 1 per crate, libopus adapter 6 | None: a process boundary | `libmpv2` 123, `libmpv2-sys` 65 (generated FFI) |
| 7. Licence of the binary | Permissive, plus MPL-2.0 (Symphonia, file-level) | Not affected: mpv runs as a separate program | `libmpv2` is LGPL-2.1; Arch's libmpv is built with `gpl` and links GPL-3.0-only FFmpeg |

The rest of this section gives the measurements behind each row.

#### 1. Formats

Rodio decodes MP3, FLAC, AAC in M4A and ALAC in M4A. It reports Opus in Ogg as `The format of the data has not been recognized.` Rodio's `Decoder` builds its codec from `symphonia::default::get_codecs()` (`rodio-0.23.0/src/decoder/symphonia.rs:105`), so no codec can be added to it, and neither Symphonia 0.5.5 nor 0.6.1 has an Opus decoder. `symphonia-adapter-libopus` 0.2.x (for Symphonia 0.5; 0.3.0 targets 0.6) wraps libopus as a Symphonia decoder, and `opus_source.rs` in Appendix A is a Rodio `Source` that drives Symphonia with a registry holding it: 125 lines including accurate seeking. The adapter's default `bundled` feature builds libopus with CMake; without it, it links the system libopus through pkg-config.

mpv and libmpv decode all five formats.

Commands: `proto gapless-rodio fixtures`, `proto gapless-mpv fixtures yes` and `proto gapless-libmpv fixtures` decode every format; Rodio's line for Opus is its error. `ffmpeg -encoders` lists the encoders used.

#### 2. Gapless playback

Columns: per-file length error of each half decoded alone (A, B), then for the two played back to back the total length error, lead, gap at the join and trail, all in samples, and the join and steady residuals. Command: `proto gapless-rodio`, `proto gapless-mpv fixtures yes`, `proto gapless-libmpv`.

| Engine | Format | A | B | Total | Lead | **Gap** | Trail | Join residual | Steady residual |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Rodio | MP3 | 0 | 0 | 0 | 0 | **0** | 0 | 0.0562 | 0.0150 |
| Rodio | FLAC | 0 | 0 | 0 | 0 | **0** | 0 | 0.0000 | 0.0000 |
| Rodio | Opus | cannot decode | | | | | | | |
| Rodio + libopus source | Opus | 0 | 0 | 0 | 0 | **0** | 0 | 0.0444 | 0.0045 |
| Rodio | AAC/M4A | +1787 | +1797 | +3584 | +1024 | **+1787** | +773 | 0.1250 | 0.0004 |
| Rodio | ALAC/M4A | 0 | 0 | 0 | 0 | **0** | 0 | 0.0000 | 0.0000 |
| mpv | MP3 | 0 | 0 | 0 | 0 | **0** | 0 | 0.0562 | 0.0150 |
| mpv | FLAC | 0 | 0 | 0 | 0 | **0** | 0 | 0.0000 | 0.0000 |
| mpv | Opus | 0 | 0 | 0 | 0 | **0** | 0 | 0.0444 | 0.0045 |
| mpv | AAC/M4A | −1024 | −1024 | −2048 | −1024 | **−1024** | 0 | 0.0112 | 0.0004 |
| mpv | ALAC/M4A | 0 | 0 | 0 | 0 | **0** | 0 | 0.0000 | 0.0000 |
| libmpv | all five | | | | | as mpv | | | |

- Both engines join MP3, FLAC, Opus and ALAC with no gap. The join residuals of MP3 (0.056) and Opus (0.044) are the same in both engines, whose decoders are unrelated (Symphonia and libopus against FFmpeg), so they come from encoding the halves separately, not from either engine.
- **AAC in M4A fails in both, differently.** Rodio trims nothing: Symphonia 0.5's MP4 reader ignores the edit list, so each file keeps its 1024 priming samples and its 763 padding samples, and the join has 1787 extra samples (37 ms) in it. mpv trims the priming twice: the marker's onset arrives at sample 1980 instead of 3004, so each file loses its first 1024 samples (21 ms) of music, at the start of the first file and at the join alike. Only FFmpeg-encoded M4A was tested; files with an `iTunSMPB` tag may behave differently.
- mpv's `--gapless-audio` default `weak` gives the same result as `yes`. With `no`, mpv reopens the audio output between files, which truncates the PCM file, so this method cannot measure that mode.
- libmpv: in 1 of 51 runs (the first), the output held only the second file, which means mpv reopened the audio output between the files. The prototype queues the second file microseconds after the first; the 50 later runs and 20 subprocess runs did not repeat it, and the cause was not established.
- Rodio's queue plays about 512 samples of silence when a track ends with nothing queued after it (`rodio-0.23.0/src/queue.rs`, `go_next`), so the next track must be appended before the current one ends. #7's scope already says so.

#### 3. Seeking on a 60-minute file

Five seeks per file, in this order: 600 s, 1234.567 s, 2400.25 s, 3333.333 s, then back to 100 s. Commands: `proto seek-rodio`, `proto seek-mpv`, `proto seek-libmpv`.

| Engine | Format | Error, samples | Latency, ms |
| --- | --- | --- | --- |
| Rodio | MP3 | 0, −1, 0, 0, 0 | 1.02, 1.07, 1.94, 1.57, 0.21 |
| Rodio | FLAC | 0, −1, 0, 0, 0 | 0.07, 0.07, 0.07, 0.06, 0.08 |
| Rodio + libopus source | Opus | 0, −1, 0, 0, 0 | 0.19, 0.18, 0.29, 0.14, 0.18 |
| Rodio | AAC/M4A | −1024, −1025, −1024, −1024, −1024 | 0.01–0.03 |
| mpv (IPC) | MP3 | 0, 0, 0, 0, 0 | 5.32, 5.70, 9.69, 7.94, 0.34 |
| mpv (IPC) | FLAC | 0, 0, 0, 0, −1 | 0.26, 0.39, 0.40, 0.38, 0.66 |
| mpv (IPC) | Opus | 0, 0, 0, 0, 0 | 0.33, 0.98, 0.77, 0.74, 0.43 |
| mpv (IPC) | AAC/M4A | 0, 0, −1, 0, −1 | 0.16, 0.26, 0.29, 0.29, 0.30 |
| libmpv | MP3 | `time-pos` = target | 5.01, 5.36, 9.80, 8.09, 0.20 |
| libmpv | FLAC, Opus, AAC | `time-pos` = target | 0.10–0.81 |

- Both engines seek to the sample. Rodio's −1 at 1234.567 s comes from `Duration::from_secs_f64(1234.567)`, which is a hair under the target and truncates; positions should be converted to samples with rounding.
- Rodio's AAC timeline is 1024 samples early everywhere, including at the start of the file, because the priming is not trimmed. mpv seeks AAC exactly, so its AAC defect is limited to the start of a file.
- The libopus source first landed exactly 312 samples early on every seek: Ogg timestamps include Opus's pre-skip, which Symphonia exposes as `CodecParameters::delay`. The figures above are after correcting for it, which is part of the 125 lines.
- These latencies exclude the output buffer. Rodio's `Player` applies a seek on its next 5 ms control tick (`player.rs`, `periodic_access`).

#### 4. HTTP input

Command: `SPIKE_HTTP_RATE=4000000 SPIKE_HTTP_LOG=1 proto http`. Throttled to 4 MB/s; the seek to 2400.25 s is far beyond what has been downloaded.

| Engine | Format | Open to first sample, ms | Seek to 2400.25 s, ms | Seek back to 100 s, ms | Error, samples | Range requests |
| --- | --- | --- | --- | --- | --- | --- |
| Rodio + `stream-download` | MP3 | 68.2 | **14 549.8** | 0.2 | 0, 0 | none after the first request |
| Rodio + `stream-download` | FLAC | 66.8 | 601.1 | 533.8 | 0, 0 | 20 requests in all, a binary search per seek |
| Rodio + `stream-download` | AAC/M4A | 262.5 | 80.2 | 66.9 | −1024, −1024 | 5 requests in all |
| Rodio | Opus | cannot decode (Rodio's `Decoder`) | | | | |
| mpv (IPC) | MP3 | 5.1 | **14 611.3** | 0.6 | 0, 0 | none for the far seek |
| mpv (IPC) | FLAC | 17.1 | 76.0 | 199.6 | 0, −1 | several per seek |
| mpv (IPC) | Opus | 16.6 | 21.1 | 10.5 | 0, 0 | several per seek |
| mpv (IPC) | AAC/M4A | 180.9 | 4.2 | 0.5 | −1, −1 | one per seek |

- MP3 needs a sequential read to the target in both engines: the request log shows no range request for that seek, and 57.6 MB at 4 MB/s is the 14.5 s measured. Neither engine finds the byte position of a far target in this MP3 without reading up to it. FLAC, AAC and Opus seek by range request.
- Rodio needs `stream-download` (or its own range reader) for HTTP, and Opus over HTTP would need the libopus source to take that reader; it was not tried. mpv plays URLs itself.

#### 5. Output devices

Commands: `proto device-rodio` (the default build) and `proto-pw device-rodio` (built with `--features pw`, which enables `cpal/pipewire`). The device test plays one second of digital silence, so nothing is heard.

- **Rodio, default build.** cpal's default host on Linux is ALSA. The default device, ALSA's `default`, is routed into PipeWire by PipeWire's ALSA plugin. `DeviceSinkBuilder::open_default_sink()` opened it in 6.0 ms: 2 channels, 48 kHz, f32, a fixed buffer of 2048 frames (42.7 ms). `sleep_until_end` returned 942 ms into one second of silence, a buffer early. When the sink is dropped, Rodio prints a message on stderr unless `log_on_drop(false)` is set.
- **Rodio with `cpal/pipewire`.** cpal 0.18.2 prefers its native PipeWire host when PipeWire is available. It opened `default_output` in 1.8 ms with the same configuration. The feature needs libpipewire's headers and bindgen to build, and links `libpipewire-0.3.so.0`.
- **mpv.** It uses its native `ao=pipewire`. The Arch build also has PulseAudio, ALSA, JACK, OpenAL, sndio and SDL2 outputs.
- **No device**, as on a CI runner. `bwrap --dev-bind / / --tmpfs /dev/snd --tmpfs /run/user/$UID` hides the sound devices and the PipeWire and PulseAudio sockets:
  - Rodio, default build: `open_default_sink()` returned `Err` ("Failed to get the config for the given device") after about 5 ms in 60 of 60 runs, after ALSA printed errors on stderr. It never panicked.
  - Rodio with `cpal/pipewire`: it falls back to the ALSA host, and hung in 4 of 60 runs. The main thread waits in `pw_thread_loop_stop`, called from `_snd_pcm_pipewire_open` under `snd_pcm_open`, under cpal's `default_output_config`.
  - mpv on the command line: it exited with status 2 ("Could not open/initialize audio device -> no sound") but hung in 2 of 21 runs; with `--ao=pipewire` alone it hung in 1 of 15. Its main thread waits in the same `pw_thread_loop_stop`. SIGTERM did not end it; SIGKILL did.
  - mpv over IPC: `end-file` with reason `error` and `file_error` "audio output initialization failed", then `idle`.

  The hang is in the PipeWire client library (1.6.9) when its socket is missing, not in either engine; any client that loads it is exposed. A CI runner without PipeWire's ALSA plugin should not reach that code, which was not verified. Neither engine needs a device for tests: Rodio's `Player::new()` returns its output for the test to pull, and mpv has `--ao=null` and `--ao=pcm`.

#### 6. Player features

Commands: `proto features-rodio`, `proto features-mpv`, `proto features-libmpv` and `proto volume-mpv`. Rodio is measured by pulling samples without a device; mpv in real time with `--ao=null`, except `volume-mpv`, which captures PCM.

- **Volume.** Rodio's `Player::set_volume(0.25)` scales samples linearly (peak 0.5 became 0.1250), and the last full-volume sample came 237 samples after the call: the factor is applied on the next 5 ms control tick, as a step without a ramp. mpv's `volume` property is cubic: `proto volume-mpv` plays the tone with `--volume=25` through `--ao=pcm`, and the peak fell from 0.5 to 0.00781, a factor of 0.01562 = 0.25³.
- **Position.** Rodio's `get_pos()` matched the samples pulled to within 4 ms, the phase of its 5 ms update tick. mpv's `time-pos`, polled every 10 ms, changed 251 times in 19.6 s, about every 80 ms, and lagged the wall clock by a median of 38 ms.
- **End of track.** In Rodio, an `EmptyCallback` source appended after a track ran after exactly 480 517 samples had been pulled, the length of the track. This is when the device pulls the last sample, one device buffer before it is heard (2048 frames, 42.7 ms here). mpv sent `end-file` (reason `eof`) for the first track, `start-file` for the second and the `playlist-pos` change all at 9.540 s, while the first track lasts 10.011 s. From 9.553 s to 10.047 s it reported the second track at `time-pos` 0.000, while the last 0.47 s of the first track was still playing. With gapless playback, mpv moves to the next file when the decoder reaches the end, not when the audio does. On the real PipeWire device at volume 0 (`proto features-mpv fixtures pipewire`), `end-file` came at 9.745 s, the second track was reported at `time-pos` 0 from 9.758 s to 10.086 s, and `time-pos` lagged the wall clock by a median of 88 ms. libmpv gave the same times as the subprocess, to the millisecond.

#### 7. Cost

Commands: the block "Gapless metadata and cost" under "Reproduce".

- **Runtime and build.** Rodio's `playback` feature uses `alsa-sys`, whose build script requires pkg-config `alsa`: CI needs `libasound2-dev`, and the binary needs `libasound.so.2`. The native PipeWire host needs libpipewire's headers and bindgen as well. Opus needs libopus (`libopus.so.0`), or CMake to build the bundled one. `libmpv2-sys` links `-lmpv`, so CI needs `libmpv-dev`; `libmpv.so.2` pulls in 203 shared libraries (`ldd`). The subprocess needs nothing to build and the `mpv` binary to run, and a CI test of it needs mpv installed.
- **Dependencies** (`cargo tree -e normal`, each crate's subtree): `rodio` with the features used here 42 crates; `symphonia-adapter-libopus` 8; `libmpv2` 2; `serde_json`, for IPC, 5; `stream-download` with its defaults 114, which include `reqwest`, `rustls`, `aws-lc-rs` and `tokio`.
- **FFI and `unsafe`.** Occurrences of the keyword: `alsa` 762, `cpal`'s ALSA host 10, `rodio` 1, Symphonia's crates 0 or 1 each, `symphonia-adapter-libopus` 6; `libmpv2` 123, `libmpv2-sys` 65. In `libmpv2`, `Mpv::create_client` passes the result of `mpv_create_client` to `NonNull::new_unchecked` without checking it for null (`libmpv2-7.0.1/src/mpv.rs`). The subprocess puts a process boundary where FFI would be, and a hung mpv can be killed (criterion 5); a hang in libmpv blocks a thread of the application.
- **Maintenance** (crates.io): `rodio` 0.23.0 2026-10-11, four releases in 2026; `cpal` 0.18.2 2026-08-16; `symphonia` 0.6.1 2026-08-13 while Rodio still pins 0.5; `libmpv2` 7.0.1 2026-11-07; `stream-download` 0.24.5 2026-10-28; `symphonia-adapter-libopus` 0.3.0 2026-05-16; `libmpv`/`libmpv-sys` 2020.
- **Licence.** Rodio's tree is MIT or Apache-2.0 apart from 13 MPL-2.0 crates (Symphonia), whose copyleft covers only changes to their own files, and libopus is BSD-3-Clause. `libmpv2` is LGPL-2.1, and statically linking it means offering object files for relinking. Arch's mpv is built with the `gpl` feature (licences `GPL-2.0-or-later AND LGPL-2.1-or-later`) and its FFmpeg is GPL-3.0-only, so a terra binary linked to libmpv would have to be distributed under GPL-3.0-compatible terms. Running mpv as a separate program puts no such terms on terra.

### Not measured

- A device that disappears or a default sink that changes during playback: testing it would mean changing the engineer's audio setup.
- Tracks with different sample rates in a row, where mpv reopens its audio output and Rodio resamples.
- Rodio with a real device beyond opening it and playing silence.
- AAC files that carry `iTunSMPB` instead of an edit list, and anything other than mono 48 kHz.

## Decision

We recommend Rodio 0.23 as terra's playback engine, behind a small engine trait in `terra-player` (#7), with:

- Rodio's own `Decoder` for MP3, FLAC, ALAC (the `symphonia-alac` feature) and AAC in M4A, with AAC's known gap (below);
- Opus through terra's own `Source` over Symphonia 0.5 and `symphonia-adapter-libopus` 0.2.x against the system libopus, as prototyped in `opus_source.rs`, including the pre-skip correction for seeking;
- cpal's default ALSA host, not the `cpal/pipewire` feature, which adds build dependencies and was the only Rodio configuration that hung without a device;
- `stream-download` 0.24, or a range reader of terra's own, for HTTP when Phase 3 needs it.

The reasons, from the measurements:

- **Gapless playback is a tie.** Both engines join MP3, FLAC, Opus and ALAC with a gap of 0 samples and the same residual, and both fail AAC in M4A, in opposite directions.
- **Rodio gives #7's queue exact events.** Its end-of-track callback fires at the exact sample, and its position is exact to within one 5 ms tick. mpv moves to the next file when the decoder reaches the end: `end-file`, `start-file` and `playlist-pos` arrive 0.27 s (PipeWire) to 0.47 s (`--ao=null`) before the audible end, and the next track is reported at `time-pos` 0 meanwhile. Its position changes only about every 80 ms. A queue on mpv would have to correct for this, and a test of it would depend on timing.
- **Rodio's tests are headless and need no other program.** The real-engine tests #7 asks for can pull samples from `Player::new()` deterministically. Testing mpv needs mpv installed in CI and runs in real time or through `--ao=pcm`.
- **Rodio keeps the binary's licence permissive**, with Symphonia's file-level MPL-2.0, and it is the Rust-native option the brief (§4.2) prefers for a learning project.

The costs accepted are measured and bounded: about 125 lines of Opus glue, which this spike showed to be gapless and sample-accurate; ALSA and libopus headers on CI; slower HTTP seeks in FLAC (601 ms against mpv's 76 ms) and AAC (80 ms against 4 ms) at 32 Mbit/s; and an AAC-in-M4A gap that mpv does not fix either.

mpv over JSON IPC remains the alternative engine behind the same trait. It wins on codec breadth (Opus built in), HTTP seek speed, build dependencies (none) and isolation (a hung mpv can be killed), and `mpv_ipc.rs` is a starting point. libmpv is ruled out: it has the same audio behaviour as the subprocess, it puts GPL terms on the binary through Arch's libmpv and FFmpeg, it links 203 shared libraries, its binding has 188 `unsafe` sites including an unchecked null pointer, and a hang in it blocks the application.

**Gapless threshold for #7's test.** Measure with this record's capture method: decoders appended to `Player::new()`, its output pulled without a device, analysed as `analysis.rs` does, with the lead taken from the marker fixture. The test fails unless, for each of MP3, FLAC, Opus and ALAC:

- the gap at the join is within ±1 sample (measured: 0);
- the total length of the joined output is within ±1 sample of the source (measured: 0);
- the join residual is at most 0.1 for a tone of amplitude 0.5 (measured: at most 0.0562, from MP3).

AAC in M4A does not meet it (gap +1787 samples). #7 should keep that case as a skipped test that names the follow-up issue.

## Consequences

- **#7** adds `crates/terra-player` with `rodio = "=0.23.0"`, default features off, and `playback`, `mp3`, `flac`, `mp4` and `symphonia-alac` on; with `symphonia = "=0.5.5"` (feature `ogg`) and `symphonia-adapter-libopus` 0.2 (`default-features = false`) for Opus. CI installs `libasound2-dev` and `libopus-dev`, which #7 already expects to do.
- **Shape of the player crate.** The `Player` handle in #7 drives an internal trait that a fake engine can implement in tests:

  ```rust
  /// One playback engine. Runs on the player's own thread.
  pub(crate) trait Engine: Send + 'static {
      /// Queues `source` to start the moment the previous item ends.
      /// The player calls it before the current item ends; Rodio would
      /// otherwise insert about 512 samples of silence.
      fn enqueue(&mut self, item: QueueItemId, source: AudioSource) -> Result<(), EngineError>;
      /// Drops everything queued, including the current item.
      fn clear(&mut self);
      fn play(&mut self);
      fn pause(&mut self);
      /// Seeks within the current item.
      fn seek(&mut self, position: Duration) -> Result<(), EngineError>;
      /// Linear gain, 0.0 to 1.0. The player maps its volume control onto it.
      fn set_gain(&mut self, gain: f32);
      /// Started, ended and position events, tagged with the item.
      fn events(&self) -> Receiver<EngineEvent>;
  }
  ```

  The Rodio engine opens the device sink, or for tests hands `Player::new()`'s output to the test. It brackets each item with `EmptyCallback` sources that send `Started` and `Ended`, and derives `Position` from the samples played or from `get_pos()`. It picks the libopus source for Opus and Rodio's `Decoder` otherwise. An mpv engine would implement the same trait over JSON IPC and would have to correct for mpv's early `end-file`.
- **Volume.** Rodio's gain is linear and changes in one step. The player should map its 0.0 to 1.0 volume onto a perceptual curve (mpv uses a cube), and may ramp changes; clicks from the step were not measured.
- **Positions in samples.** Convert durations to samples with rounding: `Duration::from_secs_f64` truncated 1234.567 s to one sample early.
- **AAC in M4A is not gapless** and its timeline is 1024 samples early, because Symphonia 0.5 ignores the MP4 edit list. A follow-up issue should evaluate a Rodio release on Symphonia 0.6, or applying the edit list in terra. Until then, the DJ-set importer (Phase 2) should write segments in a format that measured gapless here, such as FLAC or Opus; that choice belongs to the importer's design.
- **Far seeks in MP3 over HTTP read sequentially in both engines**: 14.5 s to reach 40 minutes into a 60-minute, 192 kbit/s file at 32 Mbit/s. Phase 3 should stream long recordings in a format that seeks by range request (Opus, FLAC, M4A), or seek on the server (brief §13 question 5).
- **Tests never open an audio device.** With no device, Rodio returns an error rather than panicking. terra should not enable `cpal/pipewire` unless a later measurement shows the hang is fixed.
- **Ruled out:** libmpv, for this record's licence, linking and `unsafe` findings. Choosing mpv later means a new record that supersedes this one.

## Reproduce

Requirements: Linux, ffmpeg with `libmp3lame` and `libopus`, mpv 0.41 with libmpv, the ALSA and libopus headers, bubblewrap, and Rust 1.99. The prototype writes its temporary files to `/tmp/terra-spike`, so use that directory. Write out the files in Appendix A with this layout:

```text
/tmp/terra-spike/
├── gen-fixtures.sh
├── run-all.sh
└── proto/
    ├── Cargo.toml
    └── src/{main,analysis,http,mpv_ipc,opus_source}.rs
```

```sh
cd /tmp/terra-spike
sh gen-fixtures.sh fixtures      # about 830 MB, mostly the 60-minute files
(cd proto && cargo build --release --features pw) && cp proto/target/release/proto proto-pw
(cd proto && cargo build --release)
sh run-all.sh                    # criteria 2, 3, 4, 5 and 6; one file each in results/
```

Each line of `run-all.sh` is one measurement, for example `./proto/target/release/proto gapless-rodio fixtures`. The device test and `features-mpv fixtures pipewire` open the real default device and play only digital silence.

No device (criterion 5):

```sh
B="bwrap --dev-bind / / --tmpfs /dev/snd --tmpfs /run/user/$(id -u) --setenv XDG_RUNTIME_DIR /run/user/$(id -u)"
for i in $(seq 20); do timeout -s KILL 5 $B ./proto/target/release/proto device-rodio; echo "exit $?"; done
for i in $(seq 20); do timeout -s KILL 5 $B ./proto-pw device-rodio; echo "exit $?"; done
for i in $(seq 20); do timeout -s KILL 10 $B mpv --no-config --really-quiet --length=1 fixtures/a.flac; echo "exit $?"; done
# Exit 137 is a hang killed by the timeout. While one hangs: eu-stack -p <pid>
```

Gapless metadata and cost (criteria 2 and 7):

```sh
for f in fixtures/a.*; do echo "$f $(ffmpeg -v error -i $f -f s16le -ac 1 - | wc -c)"; done  # bytes / 2 = 480517
cd proto
cargo tree -p rodio -e normal --prefix none | sed 's/ (\*)//' | sort -u | wc -l   # likewise for the other crates
grep -rwo unsafe ~/.cargo/registry/src/*/alsa-0.11.0 --include=*.rs | wc -l       # likewise for the other crates
readelf -d target/release/proto | grep NEEDED
ldd /usr/lib/libmpv.so.2 | wc -l
mpv -v --version | grep 'List of enabled features'
pacman -Qi mpv ffmpeg | grep -E '^(Name|Licenses)'
curl -s https://crates.io/api/v1/crates/libmpv2/versions   # release dates; likewise for the other crates
```

## Appendix A: prototype

The prototype as it produced the figures above. It is measurement code, not a crate of this workspace, and is not built by `make check`. Its `Cargo.lock` is not reproduced; the crates measured are pinned with `=`.

### A.1 `gen-fixtures.sh`

Generates every fixture.

```sh
#!/bin/sh
# Generates the synthetic fixtures for the playback-engine spike.
# Usage: gen-fixtures.sh <output dir>
set -eu
out=${1:?output dir}
mkdir -p "$out"
cd "$out"
ff="ffmpeg -nostdin -hide_banner -loglevel error -y"

# Gapless: a continuous 20 s, 1 kHz sine at 48 kHz, mono, -6 dBFS, split at
# sample 480517, which is not a multiple of any codec's frame size
# (MP3 1152, AAC 1024, Opus 960, FLAC 4096, ALAC 4096).
split=480517
$ff -f lavfi -i "aevalsrc=0.5*sin(2*PI*1000*t):s=48000:d=20" -c:a pcm_s16le sine.wav
$ff -i sine.wav -af "atrim=end_sample=$split" -c:a pcm_s16le a.wav
$ff -i sine.wav -af "atrim=start_sample=$split,asetpts=PTS-STARTPTS" -c:a pcm_s16le b.wav
for p in a b; do
  $ff -i $p.wav -c:a libmp3lame -b:a 192k $p.mp3
  $ff -i $p.wav -c:a flac $p.flac
  $ff -i $p.wav -c:a libopus -b:a 128k $p.opus
  $ff -i $p.wav -c:a aac -b:a 192k $p.aac.m4a
  $ff -i $p.wav -c:a alac $p.alac.m4a
done

# Lead: the same tone, but silent before sample 3000 and after 50999, so the
# onset shows how many samples an engine adds or drops at the start of a file.
$ff -f lavfi -i "aevalsrc=if(between(n\\,3000\\,50999)\\,0.5*sin(2*PI*1000*t)\\,0):s=48000:d=1.125" \
  -c:a pcm_s16le marker.wav
$ff -i marker.wav -c:a libmp3lame -b:a 192k marker.mp3
$ff -i marker.wav -c:a flac marker.flac
$ff -i marker.wav -c:a libopus -b:a 128k marker.opus
$ff -i marker.wav -c:a aac -b:a 192k marker.aac.m4a
$ff -i marker.wav -c:a alac marker.alac.m4a

# Seeking: 60 minutes of seeded pink noise, so a window after a seek matches
# exactly one position in the reference. 48 kHz mono.
$ff -f lavfi -i "anoisesrc=color=pink:seed=42:amplitude=0.25:sample_rate=48000:duration=3600" \
  -c:a pcm_s16le long.wav
$ff -i long.wav -c:a libmp3lame -b:a 192k long.mp3
$ff -i long.wav -c:a flac long.flac
$ff -i long.wav -c:a libopus -b:a 128k long.opus
$ff -i long.wav -c:a aac -b:a 192k long.aac.m4a
```

### A.2 `run-all.sh`

Runs every measurement after the builds.

```sh
#!/bin/sh
# Runs every measurement of the playback-engine spike. Run from /tmp/terra-spike
# after gen-fixtures.sh and both builds (see the record's "Reproduce" section).
set -u
p=./proto/target/release/proto
mkdir -p results
$p gapless-rodio fixtures > results/gapless-rodio.txt 2>&1
$p gapless-mpv fixtures yes > results/gapless-mpv-yes.txt 2>&1
$p gapless-mpv fixtures weak > results/gapless-mpv-weak.txt 2>&1
$p gapless-mpv fixtures no > results/gapless-mpv-no.txt 2>&1
$p gapless-libmpv fixtures > results/gapless-libmpv.txt 2>&1
$p seek-rodio fixtures > results/seek-rodio.txt 2>&1
$p seek-mpv fixtures > results/seek-mpv.txt 2>&1
$p seek-libmpv fixtures > results/seek-libmpv.txt 2>&1
$p features-rodio fixtures > results/features-rodio.txt 2>&1
$p features-mpv fixtures > results/features-mpv.txt 2>&1
$p features-libmpv fixtures > results/features-libmpv.txt 2>&1
$p volume-mpv fixtures > results/volume-mpv.txt 2>&1
SPIKE_HTTP_RATE=4000000 SPIKE_HTTP_LOG=1 $p http fixtures > results/http.txt 2>&1
$p device-rodio > results/device-rodio.txt 2>&1
./proto-pw device-rodio > results/device-rodio-pw.txt 2>&1
# A real device: PipeWire, at volume 0, so nothing is heard.
$p features-mpv fixtures pipewire > results/features-mpv-pipewire.txt 2>&1
```

### A.3 `proto/Cargo.toml`

The prototype crate. `pw` enables cpal's native PipeWire host.

```toml
[package]
name = "proto"
version = "0.1.0"
edition = "2024"

[dependencies]
cpal = { version = "=0.18.2", optional = true }
libmpv2 = "=7.0.1"
rodio = { version = "=0.23.0", default-features = false, features = ["playback", "flac", "mp3", "mp4", "vorbis", "symphonia-alac", "wav"] }
serde_json = "1"
stream-download = "=0.24.5"
symphonia = { version = "=0.5.5", default-features = false, features = ["ogg"] }
symphonia-adapter-libopus = { version = "=0.2.9", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread"] }

[features]
pw = ["dep:cpal", "cpal/pipewire"]
```

### A.4 `proto/src/main.rs`

The commands: one per measurement.

```rust
//! Playback-engine spike: Rodio 0.23 versus mpv 0.41 (subprocess over JSON IPC,
//! and libmpv through libmpv2 7.0.1). Measurement code, not production code.

mod analysis;
mod http;
mod mpv_ipc;
mod opus_source;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rodio::{Decoder, Player, Source};

const FORMATS: [&str; 5] = ["mp3", "flac", "opus", "aac.m4a", "alac.m4a"];
const LONG: [&str; 4] = ["mp3", "flac", "opus", "aac.m4a"];
const SEEKS: [f64; 5] = [600.0, 1234.567, 2400.25, 3333.333, 100.0];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(2).map(String::as_str).unwrap_or("fixtures");
    match args.get(1).map(String::as_str) {
        Some("gapless-rodio") => {
            gapless_rodio(dir);
            gapless_rodio_opus(dir);
        }
        Some("gapless-mpv") => gapless_mpv(dir, &args[3]),
        Some("gapless-libmpv") => gapless_libmpv(dir),
        Some("seek-rodio") => {
            seek_rodio(dir);
            seek_rodio_opus(dir);
        }
        Some("seek-mpv") => seek_mpv(dir),
        Some("seek-libmpv") => seek_libmpv(dir),
        Some("http") => http_input(dir),
        Some("features-rodio") => features_rodio(dir),
        Some("features-mpv") => features_mpv(dir, args.get(3).map(String::as_str)),
        Some("features-libmpv") => features_libmpv(dir),
        Some("volume-mpv") => volume_mpv(dir),
        Some("device-rodio") => device_rodio(),
        _ => eprintln!("usage: proto <command> <fixtures dir> [args]"),
    }
}

fn path(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

fn open_rodio(p: &str) -> Result<Decoder<BufReader<File>>, rodio::decoder::DecoderError> {
    // TryFrom<File> sets byte_len and seekable; gapless is on by default.
    Decoder::try_from(File::open(p).unwrap())
}

fn report(engine: &str, fmt: &str, la: i64, lb: i64, marker: &[f32], y: &[f32]) {
    let j = analysis::join(y, analysis::marker_lead(marker));
    println!(
        "{engine:10} {fmt:8} A {la:+5} B {lb:+5} | total {:+6} lead {:+5} gap {:+6} trail {:+5} | residual join {:.4} at {:+5}, steady {:.4}",
        j.len_error, j.lead, j.gap, j.trail, j.max_residual, j.max_residual_at, j.steady_residual
    );
}

// --- Gapless -----------------------------------------------------------------

fn gapless_rodio(dir: &str) {
    for fmt in FORMATS {
        let (a, b) = (
            path(dir, &format!("a.{fmt}")),
            path(dir, &format!("b.{fmt}")),
        );
        let da = match open_rodio(&a) {
            Ok(d) => d,
            Err(e) => {
                println!("rodio      {fmt:8} cannot decode: {e}");
                continue;
            }
        };
        assert_eq!((da.sample_rate().get(), da.channels().get()), (48_000, 1));
        let la = da.count() as i64 - analysis::SPLIT as i64;
        let lb =
            open_rodio(&b).unwrap().count() as i64 - (analysis::TOTAL - analysis::SPLIT) as i64;

        // Queue both on a Player with no device and pull its output, as the
        // audio callback would. The player's queue never ends (it pads with
        // silence), so stop when it reports that both sounds are done.
        let (player, mut out) = Player::new();
        player.append(open_rodio(&a).unwrap());
        player.append(open_rodio(&b).unwrap());
        let mut y = Vec::new();
        loop {
            let s = out.next().unwrap();
            if player.empty() {
                break;
            }
            y.push(s);
        }
        let marker: Vec<f32> = open_rodio(&path(dir, &format!("marker.{fmt}")))
            .unwrap()
            .collect();
        report("rodio", fmt, la, lb, &marker, &y);
    }
}

fn gapless_rodio_opus(dir: &str) {
    use opus_source::OpusSource;
    let (a, b) = (path(dir, "a.opus"), path(dir, "b.opus"));
    let la = OpusSource::open(&a).count() as i64 - analysis::SPLIT as i64;
    let lb = OpusSource::open(&b).count() as i64 - (analysis::TOTAL - analysis::SPLIT) as i64;
    let (player, mut out) = Player::new();
    player.append(OpusSource::open(&a));
    player.append(OpusSource::open(&b));
    let mut y = Vec::new();
    loop {
        let s = out.next().unwrap();
        if player.empty() {
            break;
        }
        y.push(s);
    }
    let marker: Vec<f32> = OpusSource::open(&path(dir, "marker.opus")).collect();
    report("rodio+lib", "opus", la, lb, &marker, &y);
}

fn mpv_pcm(files: &[String], extra: &[&str]) -> Vec<f32> {
    let out = format!("/tmp/terra-spike/mpv-{}.wav", std::process::id());
    let status = std::process::Command::new("mpv")
        .args([
            "--no-config",
            "--really-quiet",
            "--ao=pcm",
            "--audio-format=float",
        ])
        .args(["--audio-samplerate=48000", "--audio-channels=mono"])
        .arg(format!("--ao-pcm-file={out}"))
        .args(extra)
        .args(files)
        .status()
        .unwrap();
    assert!(status.success());
    let y = analysis::read_wav(&out);
    std::fs::remove_file(&out).unwrap();
    y
}

fn gapless_mpv(dir: &str, gapless: &str) {
    let opt = format!("--gapless-audio={gapless}");
    for fmt in FORMATS {
        let (a, b) = (
            path(dir, &format!("a.{fmt}")),
            path(dir, &format!("b.{fmt}")),
        );
        let la = mpv_pcm(std::slice::from_ref(&a), &[&opt]).len() as i64 - analysis::SPLIT as i64;
        let lb = mpv_pcm(std::slice::from_ref(&b), &[&opt]).len() as i64
            - (analysis::TOTAL - analysis::SPLIT) as i64;
        let y = mpv_pcm(&[a, b], &[&opt]);
        if y.len() < analysis::TOTAL - 10_000 {
            // ao_pcm truncates its file when the AO is reopened between files.
            println!(
                "mpv {gapless:6} {fmt:8} output holds {} samples: AO reopened between files",
                y.len()
            );
            continue;
        }
        let marker = mpv_pcm(&[path(dir, &format!("marker.{fmt}"))], &[]);
        report(&format!("mpv {gapless}"), fmt, la, lb, &marker, &y);
    }
}

fn libmpv(opts: &[(&str, &str)]) -> libmpv2::Mpv {
    libmpv2::Mpv::with_initializer(|init| {
        for (k, v) in opts {
            init.set_option(k, *v)?;
        }
        Ok(())
    })
    .unwrap()
}

fn wait_for(mpv: &libmpv2::Mpv, want: fn(&libmpv2::events::Event) -> bool) {
    loop {
        match mpv.wait_event(10.0) {
            Some(Ok(ev)) => {
                if std::env::var_os("SPIKE_DEBUG").is_some() {
                    eprintln!("event {ev:?}");
                }
                if want(&ev) {
                    return;
                }
            }
            Some(Err(e)) => eprintln!("event error {e:?}"),
            None => {}
        }
    }
}

fn gapless_libmpv(dir: &str) {
    // Same player core as the subprocess: confirms the binding changes nothing.
    use libmpv2::events::Event;
    for fmt in FORMATS {
        let out = format!("/tmp/terra-spike/libmpv-{}.wav", std::process::id());
        let mpv = libmpv(&[
            ("ao", "pcm"),
            ("ao-pcm-file", &out),
            ("audio-format", "float"),
            ("audio-samplerate", "48000"),
            ("audio-channels", "mono"),
            ("gapless-audio", "yes"),
        ]);
        mpv.command("loadfile", &[&path(dir, &format!("a.{fmt}"))])
            .unwrap();
        mpv.command("loadfile", &[&path(dir, &format!("b.{fmt}")), "append"])
            .unwrap();
        // libmpv runs in idle mode: the core stays up after the playlist ends.
        wait_for(&mpv, |ev| matches!(ev, Event::EndFile(_)));
        wait_for(&mpv, |ev| matches!(ev, Event::EndFile(_)));
        drop(mpv); // closes the PCM file
        let y = analysis::read_wav(&out);
        std::fs::remove_file(&out).unwrap();
        if y.len() < analysis::TOTAL - 10_000 {
            // ao_pcm truncates its file when the AO is reopened, which
            // mpv does between files that it does not play gaplessly.
            println!(
                "libmpv yes {fmt:8} output holds {} samples: AO reopened between files",
                y.len()
            );
            continue;
        }
        let marker = mpv_pcm(&[path(dir, &format!("marker.{fmt}"))], &[]);
        report("libmpv yes", fmt, 0, 0, &marker, &y);
    }
}

// --- Seeking -----------------------------------------------------------------

/// The 60-minute pink-noise reference, as written before encoding.
fn reference(dir: &str) -> Vec<f32> {
    analysis::read_wav(&path(dir, "long.wav"))
}

/// Where `y` (the first samples after a seek) sits in the reference, in
/// samples relative to `expected`, searched within 2 s of it. It matches
/// y[2048..6144], so an engine that lands early is found too.
fn locate(reference: &[f32], y: &[f32], expected: usize) -> i64 {
    let (skip, k) = (2048, 4096);
    let y = &y[skip..skip + k];
    let centre = expected + skip;
    let lo = centre.saturating_sub(96_000);
    let hi = (centre + 96_000).min(reference.len() - k);
    let mut best = (f32::MIN, 0);
    for o in lo..hi {
        let dot: f32 = reference[o..o + k].iter().zip(y).map(|(r, v)| r * v).sum();
        if dot > best.0 {
            best = (dot, o);
        }
    }
    best.1 as i64 - centre as i64
}

fn seek_rodio(dir: &str) {
    let r = reference(dir);
    for fmt in LONG {
        let p = path(dir, &format!("long.{fmt}"));
        let t = Instant::now();
        let mut d = match open_rodio(&p) {
            Ok(d) => d,
            Err(e) => {
                println!("rodio   {fmt:8} cannot decode: {e}");
                continue;
            }
        };
        let open_ms = t.elapsed().as_secs_f64() * 1e3;
        let start: Vec<f32> = d.by_ref().take(8192).collect();
        println!(
            "rodio   {fmt:8} open {open_ms:.1} ms, error at start {:+}",
            locate(&r, &start, 0)
        );
        for target in SEEKS {
            let t = Instant::now();
            let res = d.try_seek(Duration::from_secs_f64(target));
            let first = d.next();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            if let Err(e) = res {
                println!("rodio   {fmt:8} seek {target:9.3} s failed: {e}");
                continue;
            }
            let mut y = vec![first.unwrap()];
            y.extend(d.by_ref().take(8191));
            let err = locate(&r, &y, (target * 48_000.0).round() as usize);
            println!(
                "rodio   {fmt:8} seek {target:9.3} s  latency {ms:7.2} ms  error {err:+6} samples"
            );
        }
    }
}

fn seek_rodio_opus(dir: &str) {
    let r = reference(dir);
    let mut d = opus_source::OpusSource::open(&path(dir, "long.opus"));
    let start: Vec<f32> = d.by_ref().take(8192).collect();
    println!(
        "rodio+lib opus     error at start {:+}",
        locate(&r, &start, 0)
    );
    for target in SEEKS {
        let t = Instant::now();
        let res = d.try_seek(Duration::from_secs_f64(target));
        let first = d.next();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if let Err(e) = res {
            println!("rodio+lib opus     seek {target:9.3} s failed: {e}");
            continue;
        }
        let mut y = vec![first.unwrap()];
        y.extend(d.by_ref().take(8191));
        let err = locate(&r, &y, (target * 48_000.0).round() as usize);
        println!(
            "rodio+lib opus     seek {target:9.3} s  latency {ms:7.2} ms  error {err:+6} samples"
        );
    }
}

fn seek_mpv(dir: &str) {
    let r = reference(dir);
    for fmt in LONG {
        let p = path(dir, &format!("long.{fmt}"));
        // Latency: an IPC seek on a running player, timed to the
        // playback-restart event, which mpv sends once audio resumes.
        let mut ipc = mpv_ipc::Mpv::spawn(&["--ao=null", "--pause"]);
        ipc.command(&["loadfile", &p]);
        ipc.wait_event("playback-restart");
        for target in SEEKS {
            let t = Instant::now();
            ipc.command(&["seek", &target.to_string(), "absolute+exact"]);
            ipc.wait_event("playback-restart");
            let ms = t.elapsed().as_secs_f64() * 1e3;
            // Accuracy: --start performs the same seek, and the PCM output
            // holds exactly what mpv plays from there.
            let y = mpv_pcm(
                std::slice::from_ref(&p),
                &[&format!("--start={target}"), "--length=0.5"],
            );
            let err = locate(&r, &y, (target * 48_000.0).round() as usize);
            println!(
                "mpv     {fmt:8} seek {target:9.3} s  latency {ms:7.2} ms  error {err:+6} samples"
            );
        }
    }
}

fn seek_libmpv(dir: &str) {
    use libmpv2::events::Event;
    for fmt in LONG {
        let mpv = libmpv(&[("ao", "null"), ("pause", "yes")]);
        mpv.command("loadfile", &[&path(dir, &format!("long.{fmt}"))])
            .unwrap();
        wait_for(&mpv, |ev| matches!(ev, Event::PlaybackRestart));
        for target in SEEKS {
            let t = Instant::now();
            mpv.command("seek", &[&target.to_string(), "absolute+exact"])
                .unwrap();
            wait_for(&mpv, |ev| matches!(ev, Event::PlaybackRestart));
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let pos: f64 = mpv.get_property("time-pos").unwrap();
            println!("libmpv  {fmt:8} seek {target:9.3} s  latency {ms:7.2} ms  time-pos {pos:.4}");
        }
    }
}

// --- HTTP input --------------------------------------------------------------

fn http_input(dir: &str) {
    let server = http::serve(dir);
    let r = reference(dir);
    let rt = tokio::runtime::Runtime::new().unwrap();
    for fmt in LONG {
        let url = format!("{}/long.{fmt}", server.base);
        let ext = fmt.rsplit('.').next().unwrap();
        server.requests.store(0, Ordering::SeqCst);
        let t = Instant::now();
        let reader = rt
            .block_on(stream_download::StreamDownload::new_http(
                url.parse().unwrap(),
                stream_download::storage::temp::TempStorageProvider::new(),
                stream_download::Settings::default(),
            ))
            .unwrap();
        let len = reader.content_length().unwrap();
        let built = Decoder::builder()
            .with_data(reader)
            .with_byte_len(len)
            .with_seekable(true)
            .with_hint(ext)
            .build();
        match built {
            Err(e) => println!("rodio   {fmt:8} http cannot decode: {e}"),
            Ok(mut d) => {
                d.next().unwrap();
                println!(
                    "rodio   {fmt:8} http open+first sample {:.1} ms",
                    t.elapsed().as_secs_f64() * 1e3
                );
                for target in [2400.25_f64, 100.0] {
                    let t = Instant::now();
                    d.try_seek(Duration::from_secs_f64(target)).unwrap();
                    let mut y = vec![d.next().unwrap()];
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    y.extend(d.by_ref().take(8191));
                    let err = locate(&r, &y, (target * 48_000.0).round() as usize);
                    println!(
                        "rodio   {fmt:8} http seek {target:8.2} s  latency {ms:7.1} ms  error {err:+} samples"
                    );
                }
                println!(
                    "rodio   {fmt:8} http requests {}",
                    server.requests.load(Ordering::SeqCst)
                );
            }
        }

        server.requests.store(0, Ordering::SeqCst);
        let mut ipc = mpv_ipc::Mpv::spawn(&["--ao=null", "--pause"]);
        let t = Instant::now();
        ipc.command(&["loadfile", &url]);
        ipc.wait_event("playback-restart");
        println!(
            "mpv     {fmt:8} http open {:.1} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        for target in [2400.25_f64, 100.0] {
            let t = Instant::now();
            ipc.command(&["seek", &target.to_string(), "absolute+exact"]);
            ipc.wait_event("playback-restart");
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let y = mpv_pcm(
                std::slice::from_ref(&url),
                &[&format!("--start={target}"), "--length=0.5"],
            );
            let err = locate(&r, &y, (target * 48_000.0).round() as usize);
            println!(
                "mpv     {fmt:8} http seek {target:8.2} s  latency {ms:7.1} ms  error {err:+} samples"
            );
        }
        drop(ipc);
        println!(
            "mpv     {fmt:8} http requests {}",
            server.requests.load(Ordering::SeqCst)
        );
    }
}

// --- Player features ---------------------------------------------------------

fn features_rodio(dir: &str) {
    let (player, mut out) = Player::new();
    let pulled = Arc::new(AtomicU64::new(0));
    let ended_at = Arc::new(AtomicU64::new(0));
    player.append(open_rodio(&path(dir, "a.flac")).unwrap());
    // End of track: a callback source queued after the track runs when the
    // queue reaches it, that is, once the track has been pulled to its end.
    let (p, e) = (pulled.clone(), ended_at.clone());
    player.append(rodio::source::EmptyCallback::new(Box::new(move || {
        e.store(p.load(Ordering::SeqCst), Ordering::SeqCst)
    })));
    player.append(open_rodio(&path(dir, "b.flac")).unwrap());

    let mut max_abs_after_volume: f32 = 0.0;
    let mut volume_applied_after = None;
    let mut pos_err_ms: Vec<i64> = Vec::new();
    for i in 0..(analysis::TOTAL as u64) {
        let s = out.next().unwrap();
        pulled.store(i + 1, Ordering::SeqCst);
        if i == 240_000 {
            player.set_volume(0.25);
        }
        if i > 240_000 && i < 300_000 {
            // Above 0.13 means the 0.25 factor (peak 0.125) is not applied yet.
            if s.abs() > 0.13 {
                volume_applied_after = Some(i - 240_000);
            }
            if i > 241_000 {
                max_abs_after_volume = max_abs_after_volume.max(s.abs());
            }
        }
        if i == 300_000 {
            player.set_volume(1.0);
        }
        if i % 48_000 == 0 && i > 0 {
            let since_track_start = if i as usize > analysis::SPLIT {
                i - analysis::SPLIT as u64
            } else {
                i
            };
            pos_err_ms.push(
                (player.get_pos().as_secs_f64() * 1e3 - since_track_start as f64 / 48.0).round()
                    as i64,
            );
        }
    }
    println!(
        "rodio volume 0.25: max |sample| {max_abs_after_volume:.4} (was 0.5); last full-volume sample {:?} samples after set_volume",
        volume_applied_after
    );
    println!("rodio get_pos - samples pulled (ms), once per second: {pos_err_ms:?}");
    println!(
        "rodio end-of-track callback after {} samples pulled (track length {})",
        ended_at.load(Ordering::SeqCst),
        analysis::SPLIT
    );
}

/// mpv's volume curve: the peak of the 0.5-amplitude tone at --volume=25,
/// captured as PCM, skipping the first 0.1 s.
fn volume_mpv(dir: &str) {
    let y = mpv_pcm(&[path(dir, "a.flac")], &["--volume=25", "--length=1"]);
    let peak = y[4800..].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
    println!(
        "mpv --volume=25: peak {peak:.5} of 0.5, factor {:.5} (0.25^3 = {:.5})",
        peak / 0.5,
        0.25_f32.powi(3)
    );
}

/// Position error statistics: (wall seconds since playback started, reported
/// position, playlist index). Prints the median and largest error and how
/// often the reported value changes.
fn position_report(engine: &str, samples: &[(f64, f64, i64)]) {
    let a_len = analysis::SPLIT as f64 / 48_000.0;
    let mut errs: Vec<f64> = samples
        .iter()
        .map(|(wall, pos, pl)| (pos - if *pl == 0 { *wall } else { wall - a_len }) * 1e3)
        .collect();
    let changes = samples.windows(2).filter(|w| w[0].1 != w[1].1).count();
    // Around the switch from track A to track B.
    for w in samples
        .windows(2)
        .filter(|w| w[0].1 != w[1].1 || w[0].2 != w[1].2)
    {
        if (9.3..10.3).contains(&w[1].0) {
            println!(
                "{engine} at {:.3} s: playlist-pos {} time-pos {:.3}",
                w[1].0, w[1].2, w[1].1
            );
        }
    }
    errs.sort_by(f64::total_cmp);
    println!(
        "{engine} time-pos minus wall clock: median {:+.1} ms, range {:+.1} to {:+.1} ms; {} changes in {} polls over {:.1} s",
        errs[errs.len() / 2],
        errs[0],
        errs[errs.len() - 1],
        changes,
        samples.len(),
        samples.last().unwrap().0
    );
}

/// With `ao` (a real output such as `pipewire`), volume is 0, so nothing is heard.
fn features_mpv(dir: &str, ao: Option<&str>) {
    let ao_opt = format!("--ao={}", ao.unwrap_or("null"));
    let volume = if ao.is_some() { "0" } else { "25" };
    let mut ipc = mpv_ipc::Mpv::spawn(&[&ao_opt, "--pause", &format!("--volume={volume}")]);
    ipc.command_json(serde_json::json!(["observe_property", 1, "playlist-pos"]));
    ipc.command(&["loadfile", &path(dir, "a.flac")]);
    ipc.command(&["loadfile", &path(dir, "b.flac"), "append"]);
    ipc.wait_event("playback-restart");
    ipc.command(&["set", "volume", volume]);
    let t0 = Instant::now();
    ipc.command(&["set", "pause", "no"]);
    let mut samples = Vec::new();
    let mut ended = false;
    while !ended {
        while let Some(ev) = ipc.poll_event(Duration::from_millis(10)) {
            let name = ev["event"].as_str().unwrap_or("");
            if matches!(name, "end-file" | "start-file" | "property-change") {
                println!("mpv event at {:.3} s: {ev}", t0.elapsed().as_secs_f64());
            }
            ended |= name == "end-file" && ev["playlist_entry_id"] == 2;
        }
        let wall = t0.elapsed().as_secs_f64();
        if let (Some(pos), Some(pl)) = (
            ipc.get("time-pos").as_f64(),
            ipc.get("playlist-pos").as_i64(),
        ) {
            samples.push((wall, pos, pl));
        }
    }
    println!(
        "mpv volume property: {} (expected {volume})",
        ipc.get("volume")
    );
    println!(
        "mpv track A lasts {:.3} s",
        analysis::SPLIT as f64 / 48_000.0
    );
    position_report("mpv", &samples);
}

fn features_libmpv(dir: &str) {
    use libmpv2::events::Event;
    let mpv = libmpv(&[("ao", "null"), ("pause", "yes")]);
    mpv.observe_property("playlist-pos", libmpv2::Format::Int64, 1)
        .unwrap();
    mpv.command("loadfile", &[&path(dir, "a.flac")]).unwrap();
    mpv.command("loadfile", &[&path(dir, "b.flac"), "append"])
        .unwrap();
    wait_for(&mpv, |ev| matches!(ev, Event::PlaybackRestart));
    mpv.set_property("volume", 25.0).unwrap();
    let t0 = Instant::now();
    mpv.set_property("pause", false).unwrap();
    let mut samples = Vec::new();
    let mut ends = 0;
    while ends < 2 {
        while let Some(ev) = mpv.wait_event(0.01) {
            let at = t0.elapsed().as_secs_f64();
            match ev {
                Ok(Event::EndFile(r)) => {
                    ends += 1;
                    println!("libmpv event at {at:.3} s: end-file reason {r:?}");
                }
                Ok(Event::StartFile) => println!("libmpv event at {at:.3} s: start-file"),
                Ok(Event::PropertyChange { name, change, .. }) => {
                    println!("libmpv event at {at:.3} s: {name} = {change:?}")
                }
                _ => {}
            }
        }
        let wall = t0.elapsed().as_secs_f64();
        if let (Ok(pos), Ok(pl)) = (
            mpv.get_property::<f64>("time-pos"),
            mpv.get_property::<i64>("playlist-pos"),
        ) {
            samples.push((wall, pos, pl));
        }
    }
    let vol: f64 = mpv.get_property("volume").unwrap();
    println!("libmpv volume property: {vol} (expected 25)");
    position_report("libmpv", &samples);
}

// --- Output device -----------------------------------------------------------

fn device_rodio() {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    println!("cpal default host: {:?}", host.id());
    match host.default_output_device() {
        Some(d) => println!(
            "default output device: {:?}",
            d.description().map(|d| d.name().to_string())
        ),
        None => println!("default output device: none"),
    }
    let t = Instant::now();
    match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => {
            println!(
                "opened default sink in {:.1} ms: {:?}",
                t.elapsed().as_secs_f64() * 1e3,
                sink.config()
            );
            // Digital silence, so nothing is heard.
            let player = Player::connect_new(sink.mixer());
            let silence =
                rodio::source::Zero::new(1.try_into().unwrap(), 48_000.try_into().unwrap());
            player.append(silence.take_duration(Duration::from_secs(1)));
            let t = Instant::now();
            player.sleep_until_end();
            println!(
                "played 1 s of silence in {:.1} ms",
                t.elapsed().as_secs_f64() * 1e3
            );
        }
        Err(e) => println!(
            "open_default_sink failed after {:.1} ms: {e}",
            t.elapsed().as_secs_f64() * 1e3
        ),
    }
}
```

### A.5 `proto/src/analysis.rs`

Join analysis and WAV reading.

```rust
//! Measures the join between two halves of a continuous sine.
//!
//! The fixture is x[m] = 0.5 sin(2π·1000·m / 48000), m = 0..960000, split at
//! m = 480517. An engine's output y[n] is x delayed by `lead` samples before
//! the join and by `lead + gap` after it. A sine fit over a whole number of
//! periods gives each delay modulo the 48-sample period. The coarse value
//! that resolves it comes from an envelope (where |y| first or last exceeds
//! 0.2): for `lead`, the onset of the marker fixture, which is the same tone
//! after 3000 samples of silence, decoded by the same engine.

use std::f64::consts::PI;

pub const PERIOD: usize = 48; // 1 kHz at 48 kHz
pub const SPLIT: usize = 480_517;
pub const TOTAL: usize = 960_000;
const AMP: f64 = 0.5;
const WINDOW: usize = PERIOD * 100;

fn omega() -> f64 {
    2.0 * PI / PERIOD as f64
}

fn ideal(m: i64) -> f64 {
    AMP * (omega() * m as f64).sin()
}

/// Delay modulo the period, from a least-squares sine fit over `y[from..]`.
fn delay_mod(y: &[f32], from: usize) -> f64 {
    let w = omega();
    let (mut s, mut c) = (0.0, 0.0);
    for (i, v) in y[from..from + WINDOW].iter().enumerate() {
        let n = (from + i) as f64;
        s += *v as f64 * (w * n).sin();
        c += *v as f64 * (w * n).cos();
    }
    // y[n] = A sin(w n + phi) with phi = -w d.
    let phi = c.atan2(s);
    (-phi / w).rem_euclid(PERIOD as f64)
}

/// The integer delay closest to `coarse` that matches the fit at `from`.
fn resolve(y: &[f32], from: usize, coarse: i64) -> i64 {
    let fine = delay_mod(y, from);
    let p = PERIOD as f64;
    let k = ((coarse as f64 - fine) / p).round();
    (fine + k * p).round() as i64
}

fn first_loud(y: &[f32]) -> Option<usize> {
    y.iter().position(|v| v.abs() > 0.2)
}

fn last_loud(y: &[f32]) -> Option<usize> {
    y.iter().rposition(|v| v.abs() > 0.2)
}

#[derive(Debug)]
pub struct Join {
    pub len_error: i64,
    pub lead: i64,
    pub gap: i64,
    pub trail: i64,
    /// Largest |y - x| within 2048 samples either side of the join, outside
    /// the gap itself: a click even when the gap is zero.
    pub max_residual: f64,
    /// Where that maximum is, relative to the join.
    pub max_residual_at: i64,
    /// Largest |y - x| over 4096 samples far from the join.
    pub steady_residual: f64,
}

const MARKER_ONSET: i64 = 3000;

/// Samples an engine adds (positive) or drops (negative) at the start of a
/// file, from its output for the marker fixture.
pub fn marker_lead(y: &[f32]) -> i64 {
    let x: Vec<f32> = (0..MARKER_ONSET + 1000)
        .map(|m| {
            if m < MARKER_ONSET {
                0.0
            } else {
                ideal(m) as f32
            }
        })
        .collect();
    let coarse = first_loud(y).unwrap() as i64 - first_loud(&x).unwrap() as i64;
    resolve(y, (MARKER_ONSET + coarse + 480) as usize, coarse)
}

/// Analyses y, the output of playing the two halves back to back, given the
/// engine's lead from `marker_lead`.
pub fn join(y: &[f32], marker_lead: i64) -> Join {
    let len = y.len() as i64;
    let x: Vec<f32> = (0..TOTAL as i64).map(|m| ideal(m) as f32).collect();
    let lead = resolve(y, (marker_lead.max(0) as usize) + 10 * WINDOW, marker_lead);
    let tail_y = len - 1 - last_loud(y).unwrap() as i64;
    let tail_x = TOTAL as i64 - 1 - last_loud(&x).unwrap() as i64;
    let trail_coarse = tail_y - tail_x;
    let gap_coarse = (len - TOTAL as i64) - lead - trail_coarse;
    let after = (SPLIT as i64 + lead + gap_coarse.max(0) + 10 * WINDOW as i64) as usize;
    let gap = resolve(y, after, lead + gap_coarse) - lead;
    let trail = len - TOTAL as i64 - lead - gap;

    let join_at = SPLIT as i64 + lead; // first output index of the gap
    let mut max_residual = (0.0, 0);
    let mut track = |n: i64, delay: i64| {
        let r = (y[n as usize] as f64 - ideal(n - delay)).abs();
        if r > max_residual.0 {
            max_residual = (r, n - join_at);
        }
    };
    for n in (join_at - 2048)..join_at {
        track(n, lead);
    }
    for n in (join_at + gap.max(0))..(join_at + gap.max(0) + 2048) {
        track(n, lead + gap);
    }
    // The same measure far from the join: the codec's own error.
    let from = lead + 100_000;
    let steady_residual = (from..from + 4096)
        .map(|n| (y[n as usize] as f64 - ideal(n - lead)).abs())
        .fold(0.0, f64::max);
    Join {
        len_error: len - TOTAL as i64,
        lead,
        gap,
        trail,
        max_residual: max_residual.0,
        max_residual_at: max_residual.1,
        steady_residual,
    }
}

/// Reads the samples of a mono WAV that mpv's PCM output wrote (float or s16).
pub fn read_wav(path: &str) -> Vec<f32> {
    let b = std::fs::read(path).unwrap();
    let mut i = 12;
    let mut format = 0u16;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let size = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &b[i + 8..(i + 8 + size).min(b.len())];
        if id == b"fmt " {
            format = u16::from_le_bytes(body[14..16].try_into().unwrap()); // bits
        } else if id == b"data" {
            return match format {
                32 => body
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
                16 => body
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes(c.try_into().unwrap()) as f32 / 32768.0)
                    .collect(),
                f => panic!("unsupported bits per sample {f}"),
            };
        }
        i += 8 + size + (size & 1);
    }
    panic!("no data chunk in {path}");
}
```

### A.6 `proto/src/opus_source.rs`

Opus for Rodio through Symphonia and libopus.

```rust
//! Opus for Rodio: Rodio's Decoder uses Symphonia's default codec registry,
//! which has no Opus decoder, so this Source drives Symphonia 0.5 directly
//! with a registry that adds the libopus adapter.

use std::fs::File;
use std::num::NonZero;
use std::time::Duration;

use rodio::Source;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecRegistry, Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct OpusSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    buf: Vec<f32>,
    pos: usize,
    channels: u16,
    rate: u32,
    /// Opus pre-skip: Ogg timestamps count it, positions in the audio do not.
    delay: u64,
}

impl OpusSource {
    pub fn open(path: &str) -> OpusSource {
        let mut codecs = CodecRegistry::new();
        codecs.register_all::<symphonia_adapter_libopus::OpusDecoder>();
        let mss = MediaSourceStream::new(Box::new(File::open(path).unwrap()), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("opus");
        let opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &opts, &MetadataOptions::default())
            .unwrap();
        let format = probed.format;
        let track = format.default_track().unwrap();
        let params = track.codec_params.clone();
        let decoder = codecs.make(&params, &DecoderOptions::default()).unwrap();
        OpusSource {
            track: track.id,
            format,
            decoder,
            buf: Vec::new(),
            pos: 0,
            channels: params.channels.unwrap().count() as u16,
            rate: params.sample_rate.unwrap(),
            delay: params.delay.unwrap_or(0) as u64,
        }
    }

    fn refill(&mut self) -> bool {
        loop {
            let Ok(packet) = self.format.next_packet() else {
                return false;
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = self.decoder.decode(&packet).unwrap();
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            samples.copy_interleaved_ref(decoded);
            self.buf.clear();
            self.buf.extend_from_slice(samples.samples());
            self.pos = 0;
            if !self.buf.is_empty() {
                return true;
            }
        }
    }
}

impl Iterator for OpusSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos == self.buf.len() && !self.refill() {
            return None;
        }
        self.pos += 1;
        Some(self.buf[self.pos - 1])
    }
}

impl Source for OpusSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(self.channels).unwrap()
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(self.rate).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }

    /// Accurate seek, as Rodio's own Symphonia decoder does it: the demuxer
    /// seeks to a packet at or before the target, then decoded samples up to
    /// the target are dropped. Ogg timestamps are in samples at 48 kHz and
    /// include the pre-skip, which the first seek measured as 312 samples early.
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let to = SeekTo::Time {
            time: pos.into(),
            track_id: Some(self.track),
        };
        let seeked = self
            .format
            .seek(SeekMode::Accurate, to)
            .map_err(|e| rodio::source::SeekError::Other(std::sync::Arc::new(e)))?;
        self.decoder.reset();
        self.buf.clear();
        self.pos = 0;
        let required = seeked.required_ts + self.delay;
        let mut skip = (required - seeked.actual_ts) as usize * self.channels as usize;
        while skip > 0 {
            if self.pos == self.buf.len() && !self.refill() {
                break;
            }
            let n = skip.min(self.buf.len() - self.pos);
            self.pos += n;
            skip -= n;
        }
        Ok(())
    }
}
```

### A.7 `proto/src/mpv_ipc.rs`

mpv as a subprocess over JSON IPC.

```rust
//! mpv as a subprocess, driven over its JSON IPC socket (one JSON object per line).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub struct Mpv {
    child: Child,
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    events: VecDeque<Value>,
    next_id: u64,
    partial: String,
}

impl Mpv {
    pub fn spawn(extra: &[&str]) -> Mpv {
        let sock = format!(
            "/tmp/terra-spike/mpv-{}-{}.sock",
            std::process::id(),
            rand_suffix()
        );
        let child = Command::new("mpv")
            .args(["--no-config", "--idle=yes", "--really-quiet"])
            .arg(format!("--input-ipc-server={sock}"))
            .args(extra)
            .spawn()
            .unwrap();
        // mpv creates the socket shortly after it starts.
        let started = Instant::now();
        let stream = loop {
            match UnixStream::connect(&sock) {
                Ok(s) => break s,
                Err(_) if started.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("mpv IPC socket {sock}: {e}"),
            }
        };
        Mpv {
            child,
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
            events: VecDeque::new(),
            next_id: 1,
            partial: String::new(),
        }
    }

    fn read(&mut self) -> Value {
        self.reader.get_ref().set_read_timeout(None).unwrap();
        self.reader.read_line(&mut self.partial).unwrap();
        let v = serde_json::from_str(&self.partial).unwrap();
        self.partial.clear();
        v
    }

    /// The next event, waiting at most `timeout`.
    pub fn poll_event(&mut self, timeout: Duration) -> Option<Value> {
        if let Some(v) = self.events.pop_front() {
            return Some(v);
        }
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .unwrap();
        match self.reader.read_line(&mut self.partial) {
            Ok(_) if self.partial.ends_with('\n') => {
                let v: Value = serde_json::from_str(&self.partial).unwrap();
                self.partial.clear();
                v.get("event").is_some().then_some(v)
            }
            _ => None, // timed out; a partial line stays in `partial`
        }
    }

    /// Sends a command and returns its reply; events read meanwhile are kept.
    pub fn command(&mut self, args: &[&str]) -> Value {
        self.command_json(json!(args))
    }

    pub fn command_json(&mut self, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "command": args, "request_id": id });
        writeln!(self.writer, "{msg}").unwrap();
        loop {
            let v = self.read();
            if v.get("event").is_some() {
                self.events.push_back(v);
            } else if v["request_id"] == id {
                return v;
            }
        }
    }

    pub fn get(&mut self, property: &str) -> Value {
        self.command(&["get_property", property])["data"].clone()
    }

    pub fn wait_event(&mut self, name: &str) -> Value {
        while let Some(v) = self.events.pop_front() {
            if v["event"] == name {
                return v;
            }
        }
        loop {
            let v = self.read();
            if v["event"] == name {
                return v;
            }
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = writeln!(self.writer, "{}", json!({ "command": ["quit"] }));
        let _ = self.child.wait();
    }
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
```

### A.8 `proto/src/http.rs`

The range-capable test server.

```rust
//! A minimal HTTP/1.1 file server with byte-range support, counting requests.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Server {
    pub base: String,
    pub requests: Arc<AtomicU64>,
}

pub fn serve(dir: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicU64::new(0));
    let (root, count) = (PathBuf::from(dir), requests.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (root, count) = (root.clone(), count.clone());
            std::thread::spawn(move || {
                let _ = connection(stream, &root, &count);
            });
        }
    });
    Server { base, requests }
}

fn connection(stream: TcpStream, root: &std::path::Path, count: &AtomicU64) -> std::io::Result<()> {
    // SPIKE_HTTP_RATE caps each response at that many bytes per second;
    // SPIKE_HTTP_LOG prints every request with its Range header.
    let rate: Option<f64> = std::env::var("SPIKE_HTTP_RATE")
        .ok()
        .map(|v| v.parse().unwrap());
    let log = std::env::var("SPIKE_HTTP_LOG")
        .ok()
        .map(|_| "http: ".to_string());
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("/").to_string(),
        );
        let mut range = None;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("range") {
                    range = v.trim().strip_prefix("bytes=").map(str::to_string);
                }
            }
        }
        count.fetch_add(1, Ordering::SeqCst);
        let Ok(mut file) = File::open(root.join(target.trim_start_matches('/'))) else {
            write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")?;
            continue;
        };
        let size = file.metadata()?.len();
        let (start, end, status) = match range.as_deref().and_then(|r| r.split_once('-')) {
            Some((a, b)) => {
                let a: u64 = a.parse().unwrap_or(0);
                let b: u64 = b.parse().unwrap_or(size - 1).min(size - 1);
                (a, b, "206 Partial Content")
            }
            None => (0, size - 1, "200 OK"),
        };
        let len = end + 1 - start;
        write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\n\
             Content-Length: {len}\r\nContent-Range: bytes {start}-{end}/{size}\r\n\r\n"
        )?;
        if let Some(log) = &log {
            eprintln!(
                "{log}{method} {target} range {}",
                range.as_deref().unwrap_or("-")
            );
        }
        if method == "GET" {
            file.seek(SeekFrom::Start(start))?;
            let mut body = (&mut file).take(len);
            match rate {
                // Throttled: 16 KiB chunks at `rate` bytes per second.
                Some(rate) => {
                    let mut chunk = vec![0; 16 * 1024];
                    loop {
                        let n = body.read(&mut chunk)?;
                        if n == 0 {
                            break;
                        }
                        out.write_all(&chunk[..n])?;
                        std::thread::sleep(std::time::Duration::from_secs_f64(n as f64 / rate));
                    }
                }
                None => {
                    std::io::copy(&mut body, &mut out)?;
                }
            }
        }
    }
}
```
