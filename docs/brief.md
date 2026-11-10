# Rust TUI Music Player — Project Brief

**Status:** Initial architecture and feature planning  
**Project type:** Personal, local-first Rust music platform  
**Primary deployment:** Homelab, with full standalone support  
**Last consolidated:** 2026-11-03

> A terminal-first music player with a proper music library, playlists, DJ-set importing and splitting, and optional private streaming to other devices over Tailscale.

---

## 1. Project vision

Build a fun, useful Rust music application that is more than a terminal audio player: a **local-first music platform** where the library is independent of the user interface and audio output.

It should work equally well when:

- Running entirely on a laptop or desktop without a network connection.
- Connecting a TUI on a laptop or desktop to the central library hosted on the homelab.
- Serving music privately to an Android phone over Tailscale using a compatible streaming client.

A defining feature is the ability to import a long music recording (for example, a DJ set), recognize track timestamps from chapters or a video description, split the recording into individual songs, and store both the original recording and its derived tracks.

### Goals

1. A fully fledged, searchable music library with albums, artists, tracks, playlists, queueing, metadata and playback history.
2. A polished, keyboard-first TUI built in Rust.
3. Audio playback on the **client's** device, regardless of whether the library is local or remote.
4. Integrated imports using mature existing tools rather than rewriting audio codecs and downloading logic.
5. Private mobile streaming through a widely supported protocol, preferably OpenSubsonic.
6. An architecture that supports both standalone and client/server configurations without duplicating domain logic.

### Non-goals for the first version

- Writing an original audio codec or streaming protocol.
- Building a custom mobile app.
- Microservices or distributed databases.
- Full bidirectional offline library synchronization.
- Hosting the SQLite database on a network-mounted filesystem.

---

## 2. Agreed architectural direction

**Decision:** Design one Rust platform that has multiple operating modes. A single installation can own a library, and a remote client talks to that installation through an API.

The library remains the **source of truth**: it assigns stable track IDs and owns playlists, imports, playback history and metadata overrides. Audio files remain the source of encoded audio and embedded tags; the database links everything together.

The application's responsibilities are separated into three layers:

| Layer | Responsibility |
| --- | --- |
| **Library** | Artists, albums, tracks, metadata, file indexing, playlists, history and imports |
| **Playback** | Decoding audio, queue management, audio output and seeking on the playback device |
| **Interfaces** | Ratatui TUI, command line and a server API usable by remote clients/mobile apps |

The core library logic should not depend on a terminal UI or a particular HTTP protocol.

### Conceptual architecture

```text
                     LAPTOP / DESKTOP

               +------------------------+
               | TUI (Ratatui) / CLI    |
               +------------+-----------+
                            |
                 Library backend interface
                            |
                  +---------+---------+
                  |                   |
              Local backend       Remote backend
                  |                   |
          +-------v--------+          | API requests
          | Local library  |          v
          | SQLite + files |    +--------------------+
          +----------------+    | Homelab server     |
                                | Library API        |
                                | OpenSubsonic API   |
                                | SQLite + files     |
                                | Import workers     |
                                +---------+----------+
                                          |
                                     Tailscale
                                          |
                                   Android client
                                   (e.g. Symfonium)

Playback on a laptop is always performed on the laptop: local files
or an HTTP audio stream feed the laptop's local playback engine.
```

A **single Rust binary** with several modes is a good initial user experience; Cargo workspace crates can keep the source organized without creating separate deployed services.

---

## 3. Operating modes

### Mode A — Standalone

Run everything on the local device. No server or internet required.

```text
TUI -> Local library core -> SQLite / audio files
                       \-> Local player -> speakers
```

Expected behavior:

- Locally scanned library with search, metadata and playlists.
- Local playback and queue.
- Local imports and processing.
- Normal operation without Tailscale or a network connection.

### Mode B — Remote library, local playback

The laptop or desktop TUI connects to the homelab's library. The homelab owns the files and database; the client receives audio and plays it locally.

```text
Laptop TUI -> Homelab library API -> metadata, playlists, search
Laptop player <- Homelab audio stream -> laptop speakers
```

Expected behavior:

- The same TUI experience as standalone mode.
- Playlists and library metadata are shared across clients of the same library.
- Playback queues are independent for each playback device.
- Remote track content is streamed, not played through the homelab's speakers.
- Import requests run on the machine that hosts the remote library.
- Optional local track caching can be added later.

### Mode C — Headless homelab, Android streaming

The always-on homelab runs the library as a service and exposes an OpenSubsonic-compatible API on the private tailnet.

```text
Android music app -> Tailscale -> Homelab OpenSubsonic API
                                  -> audio library
```

Expected behavior:

- Browse artists, albums and songs.
- Search, stream and download tracks, subject to client capabilities.
- Playlist functionality through compatible OpenSubsonic endpoints.
- Phone playback happens on the phone, not on the server.

**Preferred deployment:** the homelab hosts the authoritative everyday library, while standalone installs remain fully supported for users or situations without a server.

### Data ownership and synchronization

A library instance owns its SQLite database. A standalone library and a homelab library are **separate libraries** unless an explicit import/migration or future sync mechanism is developed. This avoids silently promising automatic synchronization between two authoritative stores.

---

## 4. Functional requirements

### 4.1 Music library

**Baseline**

- Scan configured folders for supported audio files.
- Read audio tags and cover art.
- Model tracks, artists, albums and compilations/DJ sets.
- Support browsing and fast searching.
- Create, rename, edit and delete playlists.
- Add and remove playlist entries; retain playlist ordering.
- Manage play queue and current playback position.
- Track recently added and played songs.
- Preserve stable track identifiers when practical, including after file moves/renames.

**Later additions**

- Smart playlists such as *Recently Added*, *Most Played* and *Unplayed*.
- Ratings, favorites and play counts.
- Duplicate detection, missing-file reconciliation and metadata overrides.
- Album art and additional tracklist enrichment.

### 4.2 Playback

- Play common formats such as MP3, FLAC, Opus and M4A, subject to the selected backend/codec support.
- Pause, resume, skip, seek and volume control.
- Queue tracks and playlists.
- Stream remote track audio to the local playback engine.
- Prefer gapless playback, especially for continuous DJ sets.
- Maintain playback state independently for each device.

**Playback-engine choice remains open:**

- **Rodio:** attractive Rust-native integration; fits a learning project.
- **mpv-backed playback:** worth evaluating for robustness, codec breadth, gapless behavior and device handling.

Don't implement a new audio decoder just for this project.

### 4.3 Playlist behavior

| Topic | Behavior |
| --- | --- |
| Standalone playlists | Owned by the local library |
| Homelab playlists | Owned by the server library and exposed to its clients |
| Playback queues | Independent per device/client |
| Listening history | Recorded by the owning library; remote clients report plays when online |
| Offline playback | Optional future cache of selected remote audio |

---

## 5. DJ-set / long-recording importer

This is a signature feature. Given a supported video or audio URL, the importer should fetch the source and its tracklist or chapters, allow review of boundaries, split into tracks and ingest the result into the library.

### Example description

```text
00:00   Artist A - Opening Track
05:42   Artist B - Midnight
11:18   Artist C - After Hours
17:53   Artist D - Sunrise
24:07   Artist E - Closing
```

### Pipeline

1. **Fetch input** — use `yt-dlp` to retrieve source audio and metadata/description where permitted.
2. **Detect chapters/timestamps** — prefer explicit chapter data; otherwise parse timestamps from the description.
3. **Normalize tracklist** — infer title, artist and start/end boundaries, including the final segment's end from the source duration.
4. **Preview and edit** — show proposed cuts and allow corrections to names and times before processing.
5. **Split** — invoke FFmpeg to create individual audio files. Choose stream copying or re-encoding based on input format and need for accurate cuts.
6. **Tag** — write metadata and optional artwork using Lofty or a suitable supported tag writer.
7. **Import** — register individual tracks **and** the full original recording in the library, grouped as a set/compilation.
8. **Record provenance** — keep the input URL, source metadata, original timestamps, import settings and derived-track mapping.

### Why preserve the original recording?

A mixed set has two different listening modes:

- **Full set:** uninterrupted playback of the continuous original mix.
- **Individual tracks:** searchable segments that can be played separately or added to playlists.

The original file also permits re-splitting after correcting boundary times and avoids cumulative quality loss from reprocessing already-derived files.

### Technical details to get right

- `yt-dlp --split-chapters` can help when chapter boundaries are available, but not every description timestamp becomes an actual chapter; a custom parser is needed for arbitrary tracklists.
- Timestamps can have inconsistent formats (`mm:ss`, `hh:mm:ss`), missing entries, non-track text or imperfect start times.
- Music in mixed sets frequently overlaps at transitions; splitting by listed timestamps does **not** recover original unmixed tracks.
- Do not label lossy sourced audio as lossless. Preserve source quality where possible and transcode only when necessary.
- Represent imports as jobs with visible progress, errors and a possible retry/review step.
- Download only material the user is entitled to obtain and handle according to applicable terms and copyright law.

### Related tools to inspect

- [yt-dlp](https://github.com/yt-dlp/yt-dlp)
- [FFmpeg](https://ffmpeg.org/)
- [youtube-chapter-splitter](https://github.com/all3f0r1/youtube-chapter-splitter) — reference for description-based splitting and tagging

---

## 6. Streaming and remote access

### Protocol choice: OpenSubsonic

Rather than inventing a streaming protocol, implement a useful **subset** of [OpenSubsonic](https://opensubsonic.netlify.app/), the ecosystem based on Subsonic-compatible APIs.

Start with the endpoints required for one mobile client to:

- Authenticate and discover the server.
- Browse artists and albums.
- Search the music library.
- Retrieve cover art.
- Stream tracks and access audio file metadata.
- Read playlists.

After that, implement playlist modifications, downloads and playback-history/scrobble operations as needed for client compatibility. Verify actual endpoints and payloads against the OpenSubsonic specification during implementation rather than treating this list as a complete protocol contract.

### Android client

[Symfonium](https://symfonium.app/) is a promising Android app with support for Subsonic-compatible servers, offline caching, playlists and Android Auto. Using an existing mobile client avoids building an Android app as part of this personal project.

### Tailscale deployment

Run the server on the homelab and expose it privately through Tailscale. An illustrative command for a service listening on port `4533` is:

```bash
tailscale serve 4533
```

This is a **deployment example**, not a final configuration. Confirm the exact `tailscale serve` syntax, target address, permissions and desired TLS behavior against the installed Tailscale version. Keep the service private, implement authentication, and avoid exposing raw library endpoints publicly by accident.

### Additional API

OpenSubsonic covers normal music-client behavior; a project-specific API can handle features such as:

- Starting, monitoring and cancelling import jobs.
- Editing DJ-set track boundaries.
- Import provenance and original-set metadata.
- Administrative scans and maintenance.

Do not require a custom mobile client for normal streaming and playlist use.

---

## 7. Proposed Rust technology stack

| Component | Proposed choice | Notes |
| --- | --- | --- |
| TUI | [Ratatui](https://ratatui.rs/) + Crossterm | Keyboard-first terminal UI |
| Playback | [Rodio](https://github.com/RustAudio/rodio) | Evaluate against mpv before committing |
| Audio metadata | [Lofty](https://docs.rs/lofty/latest/lofty/) | Read/write tags and artwork where supported |
| Library database | SQLite + SQLx | Single-file library database; server owns its DB |
| Full-text search | SQLite FTS5 | Artist, album and track search |
| Source extraction | [yt-dlp](https://github.com/yt-dlp/yt-dlp) | External process rather than custom downloader |
| Splitting/transcoding | [FFmpeg](https://ffmpeg.org/) | External process with explicit job progress/errors |
| HTTP API | [Axum](https://github.com/tokio-rs/axum) | Library endpoints and OpenSubsonic layer |
| Async runtime | Tokio | API requests, imports and file operations |
| Mobile protocol | OpenSubsonic | Compatible app ecosystem |
| Networking | Tailscale | Private access to homelab |

**Principle:** Use the Rust ecosystem for the app, not as an excuse to reimplement every external audio/media tool.

---

## 8. Suggested workspace organization

```text
music/
├── Cargo.toml
├── crates/
│   ├── music-core/       # Domain models, library operations, backend traits
│   ├── music-storage/    # SQLite, file scanning, metadata, migrations
│   ├── music-player/     # Playback, local output, queue, streaming inputs
│   ├── music-import/     # yt-dlp, FFmpeg, timestamps, import jobs
│   └── music-server/     # Axum, OpenSubsonic, custom import API
├── src/
│   ├── main.rs           # CLI dispatch: tui / serve / import
│   └── tui/              # Ratatui screens and interactions
└── README.md
```

These are **logical package boundaries**, not a requirement to publish or deploy multiple binaries/services. It is also reasonable to start with fewer crates and split them only as code grows.

### Backend abstraction

The TUI speaks to a common library interface, with local and remote implementations:

```rust
// Illustrative sketch, not production-ready code.
enum Backend {
    Local(LocalBackend),
    Remote(RemoteBackend),
}
```

Prefer an explicit trait/interface for operations such as `search`, `list_albums`, `get_track`, `list_playlists`, `update_playlist` and `resolve_audio_source`. Keep ownership of playback separate from the library backend.

A remote backend may return a stream URL or authorized stream handle, while a local backend returns a path or other local source. Both can feed the same local player abstraction.

### Database guidelines

Store at least these conceptual entities:

- **Library / settings** — root folders, scan state, preferences.
- **Track** — stable ID, title, duration, source relationship, file path reference and codec info.
- **Artist** and **Album** — normalized library browsing fields.
- **Playlist** and **PlaylistEntry** — ordered collections of track IDs.
- **PlaybackHistory** — track, client/device and timestamp.
- **ImportJob** — URL, stage, status, logs and results.
- **SourceRecording** — preserved full-length audio and original metadata.
- **SourceSegment** — links derived tracks to their original recording and start/end times.

This is a conceptual model; schema and constraints still need design. Do not conflate the audio file's filename or ID3 tags with the stable identity of a library track.

---

## 9. TUI experience

A three-column feel can be achieved with a sidebar plus a main track list, and a playback bar along the bottom.

```text
┌────────────────────────────────────────────────────────────────┐
│ ◉ music                                  LIBRARY / ALL TRACKS  │
├──────────────────┬─────────────────────────────────────────────┤
│ BROWSE           │ TITLE / ARTIST                      TIME   │
│ > All tracks     │ > Midnight Drive / Artist A         05:42  │
│   Artists        │   After Hours / Artist B            06:18  │
│   Albums         │   Into the Deep / Artist C          04:51  │
│   DJ Sets        │   Sunrise / Artist D                06:14  │
│                  │   Last Light / Artist E             05:06  │
│ PLAYLISTS        │                                             │
│   Favorites      │                                             │
│   Driving        │                                             │
│   Discoveries    │                                             │
├──────────────────┴─────────────────────────────────────────────┤
│ ▶ Midnight Drive — Artist A                        02:13/05:42 │
│ ━━━━━━━━━━━━━░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │
│ / search     a add     i import                 ? help         │
└────────────────────────────────────────────────────────────────┘
```

### Interaction principles

- Keyboard-first, potentially Vim-like navigation.
- Incremental search and obvious selection/focus state.
- Easy add-to-queue and add-to-playlist actions.
- A dedicated import view with progress and a track-boundary editor.
- The same TUI whether the backend is local or remote.
- Clear indication of the currently connected library and playback device.

---

## 10. Proposed CLI

These are **illustrative commands**, not an existing CLI contract.

```bash
# Run a local library and local playback
music tui --local

# Connect the TUI to the homelab; play audio on this machine
music tui --server https://music.your-tailnet.ts.net

# Run the homelab server against a library directory
music serve --library /srv/music

# Import a set into the selected local library
music import "https://youtube.com/watch?v=..."

# Submit an import job to the homelab library
music import "https://youtube.com/watch?v=..." \
  --server https://music.your-tailnet.ts.net
```

Configuration should eventually support sensible defaults so `music` alone can launch the normal TUI, perhaps with the last-used backend.

---

## 11. Implementation phases

### Phase 1 — Enjoyable standalone player

**Objective:** a player worth using before building a server.

- [ ] Cargo workspace/binary structure.
- [ ] Initial SQLite schema and migrations.
- [ ] Folder scanning and metadata reading.
- [ ] Track/album/artist views and FTS search.
- [ ] Basic playlist CRUD and ordered entries.
- [ ] Local audio playback, queue and player controls.
- [ ] Ratatui library views, search and now-playing area.
- [ ] Establish the library backend abstraction early.

**Milestone:** play music from a local library and manage playlists entirely from the TUI.

### Phase 2 — DJ-set importer

- [ ] URL/source validation and extractor integration.
- [ ] Chapter and description-timestamp parsing.
- [ ] Track-boundary preview/correction screen.
- [ ] FFmpeg split pipeline with job state.
- [ ] Tagging and library ingestion.
- [ ] Preserve the original set and link derived segments.
- [ ] Import history, retries and failure handling.

**Milestone:** import one full set and choose between listening to the complete mix or individual segments.

### Phase 3 — Central library / remote TUI

- [ ] Headless `serve` mode.
- [ ] Axum library API and authentication.
- [ ] Remote backend for the existing TUI.
- [ ] Remote audio streaming into local playback.
- [ ] Remote imports and job progress.
- [ ] Run the authoritative daily library on the homelab.

**Milestone:** laptop TUI controls music stored on the homelab, with audio coming from laptop speakers.

### Phase 4 — Phone streaming

- [ ] Minimum compatible OpenSubsonic endpoints.
- [ ] Tailscale private deployment.
- [ ] Verify browse/search/stream behavior with Symfonium.
- [ ] Playlist reads and then writes/sync as supported.
- [ ] Optional remote transcoding and offline-download compatibility.

**Milestone:** browse and stream the homelab library on Android without writing a mobile app.

### Later / optional

- Smart playlists and advanced history.
- Track cache for remote TUI and offline operation.
- Device-specific output and multi-device control.
- Library export/migration between local and remote installations.
- Duplicate detection, metadata enrichment and sophisticated artwork handling.

**Order note:** The earlier discussion proposed local player → importer → server, but the server can be brought forward if remote TUI access becomes the priority. Maintaining the backend abstraction makes both sequences workable.

---

## 12. Existing projects and research references

The following projects were identified as useful architectural or implementation references, not dependencies that must be adopted.

| Project | Relevance |
| --- | --- |
| [Termusic](https://github.com/tramhao/termusic) | Rust terminal music player; library and client/server design ideas |
| [Orbit](https://github.com/sihooleebd/orbit) | Ratatui/Rodio-style local music player reference |
| [youtube-chapter-splitter](https://github.com/all3f0r1/youtube-chapter-splitter) | DJ-set timestamps, splitting and tagging |
| [WaveFlow Server](https://github.com/InstaZDLL/waveflow-server) | Rust/OpenSubsonic server architecture reference |
| [Ratune](https://docs.rs/crate/ratune/latest) | Rust Subsonic client, playback and caching ideas |
| [Ratatui](https://ratatui.rs/) | TUI framework |
| [OpenSubsonic](https://opensubsonic.netlify.app/) | Protocol specification and compatibility guidance |
| [Symfonium](https://symfonium.app/) | Android playback client |
| [Tailscale Serve](https://tailscale.com/kb/1312/serve) | Private HTTPS access to the server |

Project APIs, maintenance status and compatibility must be checked when implementation starts.

---

## 13. Decisions made vs. questions still open

### Agreed direction

- [x] Rust is the implementation language.
- [x] The UI is terminal-first, using Ratatui as the leading choice.
- [x] Both standalone and homelab-backed modes are required.
- [x] The homelab is the preferred everyday library host.
- [x] A remote TUI plays audio through its **own** local device.
- [x] Library metadata/playlists are owned by the selected library backend.
- [x] SQLite is local to the library host; don't network-share the DB.
- [x] Use external tools for source extraction and FFmpeg processing.
- [x] Preserve the original DJ set and derived per-track segments.
- [x] OpenSubsonic is the preferred mobile streaming protocol.
- [x] Tailscale is the preferred private remote-access layer.
- [x] Start small with a usable player and grow progressively.

### To evaluate or specify

1. **Playback engine:** Rodio versus an mpv-backed integration, with gapless playback tested explicitly.
2. **Library model:** exact schema, stable ID and file-move detection strategy, compilation handling, metadata precedence.
3. **Remote API:** use OpenSubsonic for TUI operations too, or a separate first-party API, or a hybrid.
4. **Server authentication:** token/session handling for TUI and protocol-compatible authentication for mobile clients.
5. **Transcoding:** which formats/devices require server-side conversion and how it is configured.
6. **Import ergonomics:** tracklist grammar, inaccurate timestamps, interactive boundary review and cancel/retry behavior.
7. **Cache strategy:** when to introduce local caching and what offline behavior is expected.
8. **Playlist and history semantics:** how remote updates, shared lists and scrobbling interact.
9. **Configuration:** profile selection, default library, file locations and launch commands.
10. **Packaging:** one binary with subcommands versus a split daemon binary later, and installation/service setup on Arch Linux.

---

## 14. Guiding design principles

- **Make the standalone product useful early.** The architecture should permit a server, but not force one for local listening.
- **Play audio where the user is.** A remote library does not imply remote speakers.
- **Keep one authoritative database per library.** Do not assume local and homelab databases are automatically synced.
- **Reuse protocols and media tools.** Distinguish creative project work from commodity plumbing.
- **Don't throw away information.** Retain original recordings, track boundary metadata and import provenance.
- **Prioritize good continuous playback.** DJ sets are a first-class use case, so gapless behavior matters.
- **Keep the system comprehensible.** An opinionated single-binary application is preferable to unnecessary service decomposition.

---

## 15. One-paragraph project summary

Build a Rust-based, terminal-first music platform with a full SQLite-backed library, playlists, metadata, search and local playback. The same TUI works in standalone mode against local files or in remote mode against an authoritative music library on the homelab, while audio always plays on the local client device. A distinctive importer uses yt-dlp, timestamp/chapter parsing and FFmpeg to split long recordings such as DJ sets into tagged, searchable tracks while retaining the original recording. A headless Axum server will expose library functions and an OpenSubsonic-compatible streaming API, allowing an Android client such as Symfonium to access the library privately through Tailscale. The project should be built incrementally, with a usable local player first and remote/mobile functionality added without rewriting the core.
