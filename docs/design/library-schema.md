# Library schema

## Status

Accepted

## Scope

This document settles the brief's §13.2 ("Library model") for Phase 1: the schema of the local library database, the stable-ID rule, the strategy for detecting moved files, metadata precedence and compilation handling. Migration `crates/terra-storage/migrations/0001_initial.sql` implements it, and the two must agree. It follows the brief's §8 ("Database guidelines").

The scanner (issue #8) adds its algorithm to this document ("Scanning"). Playlists and history (issue #9) use the tables below without changing them.

## Files and lifecycle

A library is a directory holding the SQLite database `library.db`, and `scan.lock` once a scan has run ("One scan at a time" below). `terra-storage`'s `LocalLibrary` owns it, and keeps the directory it was created or opened with:

- `LocalLibrary::create(dir)` creates `dir` if needed and `dir/library.db`, and applies every migration. It refuses a directory that already has a `library.db`.
- `LocalLibrary::migrate(dir)` applies the migrations the database lacks.
- `LocalLibrary::open(dir)` first reads, through a read-only connection, which migrations the database records. If one is missing it returns `SchemaOutdated` and leaves `library.db` and `library.db-wal` as they were: it writes to neither, not even the journal mode. A read-only connection to a WAL database would still create its `-wal` and `-shm` files, so when there is no `-wal` file, and so every committed page is in the database file, the check opens the file as immutable and changes and creates no file. When there is one, for example after a writer crashed, the check reads through it, and may create or rewrite `library.db-shm`: SQLite's shared-memory index of the WAL, which reading a WAL needs and which SQLite rebuilds from the WAL. If the database records a migration this build does not have, it returns `SchemaTooNew`; if a recorded migration's checksum differs from this build's, or one did not finish, `SchemaMismatch`.

No migration ever runs as a side effect of opening a library (`AGENTS.md`). The migrations are embedded in the binary with SQLx's `migrate!`, which records them in SQLx's `_sqlx_migrations` table.

Every connection turns foreign keys on and uses the WAL journal, so the TUI can read while a scan writes. The bundled SQLite (`libsqlite3-sys` 0.37.0, through SQLx 0.9.0's `sqlite-bundled`) is built with `SQLITE_ENABLE_FTS5`, and a test checks it.

## Conventions

- **Identifiers** are SQLite `INTEGER PRIMARY KEY`s. Artists, albums, tracks, playlists, playlist entries and plays use `AUTOINCREMENT`, so an identifier is never reused after a delete, and a client that kept an old identifier gets "not found" rather than a different record.
- **Timestamps** are `INTEGER` nanoseconds since the Unix epoch, UTC. Nanoseconds round-trip a Rust `SystemTime` exactly, which the backend contract needs for play times, and cover the years 1678 to 2262.
- **Keys.** A `*_key` column holds the trimmed, Unicode lower-cased text (Rust's `str::trim` and `str::to_lowercase`) that a record is unique and found by. The storage code computes it; SQLite's own `lower()` folds only ASCII.
- **Text order.** The backend contract orders text ignoring case by Unicode lower case. Each connection registers the collation `unicode_nocase`, which compares the lower-cased strings, and the queries use it. The schema never names it, so any SQLite tool can still open the database.
- **Paths** are stored as text, so a path that is not valid UTF-8 cannot be stored; the scanner reports such a file as an error.

## Tables

| Table | Holds | Constraints |
| --- | --- | --- |
| `library_root` | `id`, `path`, `added_at`: a folder the scanner reads | `path` unique |
| `artist` | `id`, `name`, `sort_name`, `name_key` | `name_key` unique |
| `album` | `id`, `title`, `title_key`, `album_artist_id` (null allowed), `year`, `kind` | `kind` in `album`, `compilation`, `dj_set`; unique on (`IFNULL(album_artist_id, 0)`, `title_key`) |
| `track` | `id`, `title`, `artist_id`, `album_id` (null allowed), `disc_no`, `track_no`, `duration_ms`, `added_at`, `updated_at` | the numbers are not negative |
| `media_file` | `id`, `track_id`, `root_id`, `rel_path`, `size`, `mtime_ns`, `content_hash`, `codec`, `lossless`, `sample_rate`, `channels`, `bitrate`, `has_embedded_art`, `missing_since` (null allowed) | `track_id` unique; (`root_id`, `rel_path`) unique |
| `playlist` | `id`, `name`, `name_key`, `created_at`, `updated_at` | `name_key` unique |
| `playlist_entry` | `id`, `playlist_id`, `track_id`, `position` | index on (`playlist_id`, `position`) |
| `play_history` | `id`, `track_id`, `device`, `played_at` | index on (`played_at`, `id`) |
| `track_fts`, `album_fts`, `artist_fts` | FTS5 tables for search (below) | rowid is the record's `id` |

Foreign keys: a track references its artist and album, an album its album artist, a file its track and root, an entry its playlist and track, a play its track. Deleting a track deletes its file, its playlist entries and its plays (`ON DELETE CASCADE`); deleting a playlist deletes its entries. Nothing else cascades: an artist, album or root that something still references cannot be deleted.

`playlist_entry.position` orders a playlist's entries. Writers keep the positions 0 to n-1 (issue #9); a track deletion can leave a gap, so readers number the entries by their order, not by the stored value.

## Stable identity

**A track's identity is `track.id`. It is never derived from the file's path, name or tags**, as §8 asks. The path, the tags and the file's content may all change, and the track keeps its ID as long as the scanner can tell it is the same track. Playlists and history refer to tracks by ID, so they survive renames, moves and retagging.

Phase 1 keeps one file per track: `media_file.track_id` is unique, and every track has a file. That split is what lets Phase 2 give derived tracks a source segment instead of a file of their own without changing the track's identity.

## Scanning

The scanner, `crates/terra-storage/src/scan/`, keeps the library in step with the audio files under its roots. It reads tags and audio properties with Lofty 0.25.5, walks folders with walkdir 2.5.0 and hashes files with BLAKE3 (`blake3` 1.8.7). The file system work runs on Tokio's blocking threads.

### Roots

`LocalLibrary::add_root(path)` stores a folder as a library root and returns the path it stored: `path` made absolute with symbolic links resolved (`std::fs::canonicalize`), so that one folder is never stored twice under two spellings. It refuses a path that is not a directory, is not valid UTF-8, or overlaps a root: is one already, is inside one or contains one. Overlapping roots would see the same file twice, as two different paths, and make it two tracks.

`LocalLibrary::list_roots()` returns the roots ordered by path.

**Removing a root removes its tracks.** `LocalLibrary::remove_root(path)` deletes the root, every track whose file is under it, and so those tracks' playlist entries and plays (`ON DELETE CASCADE`), in one transaction. The alternative, keeping the tracks as missing, would leave files that no root can find again. It returns whether `path` was a root, compared after resolving it as `add_root` does, or as given when that fails, such as for a folder that no longer exists.

### The algorithm

`LocalLibrary::scan(progress)` returns a `ScanReport { added, updated, moved, missing, unchanged, errors }`. Each file the walk found counts once, in `added`, `updated`, `moved`, `unchanged` or `errors`; `missing` counts the known files the walk did not find, except those it leaves as they were (step 1). After each file it calls `progress` with a `ScanProgress { done, total, path }`. A file only counts as moved once the whole walk is done, so that a copy is never mistaken for a move:

1. Walk every root to the end, without following symbolic links and skipping hidden files and folders (whose name starts with `.`; a root's own name does not count). Keep the regular files whose extension is `mp3`, `flac`, `ogg`, `opus`, `m4a` or `wav`, ignoring ASCII case, with their size and modification time. A path that is not valid UTF-8, or that the walk cannot read, is an error and is not seen.
   A root is **unavailable** when the walk cannot read the root itself (it is gone, cannot be listed or is not a directory), or finds no audio file under it while the root has at least one known file whose `missing_since` is NULL, such as an unmounted drive's empty mount point. The scan leaves the known files of an unavailable root as they were, and those at or under any other path the walk could not read, such as a folder it could not list: they are not candidates in step 4, not marked missing in step 5 and not counted in any `ScanReport` field. `errors` gets one entry per unavailable root, `(root path, "the root could not be read or holds no audio files; its <n> files were left as they were")`, where `<n>` counts every known file of the root, instead of the walk's own error for the root; the error for a path below a root is the walk's. When the walk could not read the root itself, the message ends with the cause, `"…; its <n> files were left as they were: <cause>"`: the I/O error reading the root, without the path the entry names already, such as `No such file or directory (os error 2)`, or `not a directory` (the `Display` of `std::io::ErrorKind::NotADirectory`) for a root that is not one. A root the walk read but found no audio file under gets the message without a cause.
2. A file whose (`root`, `rel_path`) is known and whose size and modification time are unchanged is unchanged. It is not read. If it was missing, `missing_since` is cleared and it counts as updated.
3. A known path whose size or modification time changed is read and hashed (BLAKE3 over the whole file, stored in `media_file.content_hash`, which is indexed) and is the same track: its fields are updated in place, and `missing_since` is cleared.
4. Only after the walk, the known files the walk did not see, and those already missing, are the candidates for a move, except those step 1 leaves as they were. Each unknown path is read and hashed, in (`root`, `rel_path`) order with the roots ordered by path:
   - If its hash equals a candidate's, it is that file moved: the file row takes the new path, the track keeps its ID and is updated from the tags, `missing_since` is cleared and the candidate is used up. When several unknown paths share a candidate's hash, the first in that order takes it; when several candidates share a hash, the one stored first is taken.
   - Anything else is a new track.
5. The candidates left are missing. Those without `missing_since` get the scan's time; those that already have it keep it. Their tracks, playlist entries and plays stay; the field is cleared if the file comes back. `resolve_audio_source` refuses a missing file (below).

So a copy whose original is still in place is a new track, whichever of the two the walk reaches first, because a file the walk saw is never a candidate. A known file the walk saw but could not read keeps its row as it was, and is not a candidate either.

Each file is written in one transaction: its artist and album (found or added), its track, its `media_file` row and its search rows. A file that cannot be read, such as one that is not audio, is recorded in `errors` with the reason, and the scan goes on; so is a panic while reading it. A database failure ends the scan with an error; what was written before stays.

The content, not the extension, decides how a file is read: Lofty guesses the format from the file's first bytes. The scanner reads MPEG audio, FLAC, Ogg Vorbis, Opus, MP4 and WAV (PCM and floating point), and reports any other format, or a file without an audio stream, as an error.

### One scan at a time

Two scans of one library, such as two `terra scan` processes or the CLI next to the TUI, would each add the same new path, and one of them would fail on the unique (`root_id`, `rel_path`). So `scan` first takes an exclusive lock on `scan.lock` in the library's directory, creating the file if it is missing, with `std::fs::File::try_lock`. If another scan holds the lock, in this process or another, `scan` returns `Error::ScanInProgress` at once, before it reads the database or a folder; any other failure to open or lock the file is `Error::Io`. The scan holds the lock until it returns, on every path, whether it succeeds, fails or is dropped. The operating system releases the lock when the process ends, so a crashed scan leaves no stale lock; the file it leaves is harmless and is reused.

Only `scan` takes the lock. Reading the library, `add_root` and `remove_root` do not wait for it.

### Tags and properties

Each changed file's tags are mapped as follows, from the file's primary tag as Lofty names it for the format (such as ID3v2 for MP3, Vorbis comments for FLAC and Ogg, and the `ilst` atom for MP4), or its first tag when it has no primary one:

- The title falls back to the file stem, the artist to "Unknown Artist". Empty or blank tags count as absent. The artist's sort name is the artist sort tag, or the name.
- The album, track number, disc number and year are copied. A track without an album tag has no album.
- The album artist is the album artist tag; without one, the track's artist, except on a compilation, which keeps none (below).
- The compilation flag (`TCMP`, `cpil`, `COMPILATION`) makes the album `AlbumKind::Compilation`. It is set by `1`, any other number but `0`, or `true`.
- An album found by its key, rather than added, still takes the file's year, when the file has one, and its kind: `compilation` with the flag, `album` without it. A `dj_set` album stays one, because the importer, not the tags, makes it. Of several files on one album, the last one written wins, so files that disagree leave the album as the last changed one says.
- The codec and `lossless` come from the audio stream, never from the container or the extension (brief §5: do not label lossy audio as lossless): `mp3` (or `mp1`, `mp2`), `vorbis`, `opus` and `aac` are lossy; `flac`, `alac` and `pcm` are lossless. An M4A file is `aac` or `alac`, as its sample description says.
- The sample rate and channels come from the audio properties, the bitrate from Lofty's audio bitrate (kilobits per second, stored in bits per second), and the duration from Lofty, corrected as the next item says. `has_embedded_art` is set when any tag of the file holds a picture.
- Lofty 0.25.5's duration includes an encoder's delay and padding, so the scanner subtracts them where the file gives them. For MP3 the LAME tag in the first frame's Xing/Info header, written by LAME or ffmpeg, gives the delay and padding, and the duration is (frames × samples per frame − delay − padding) / sample rate. For MP4 the first audio track's edit list gives the duration it presents, in the same cases where `terra-player` applies it (`docs/design/player.md`): one edit at normal rate after any empty edits, ending at the end of the media at the latest. An MP4 file without such an edit list takes the sample count of its `iTunSMPB` tag, at the stored sample rate. A file without any of these keeps Lofty's duration.

An artist or album that a scan, or `remove_root`, leaves without tracks is deleted, with its search row, unless something else still refers to it: an album artist keeps its artist. Otherwise retagging every track of an artist would leave the old name in the artist list for good.

### Known limitations

- The hash covers the whole file, tags included, so a file that is both moved and retagged between two scans looks like a new track, and the old one is reported missing.
- A file changed without changing its size or modification time, within the file system's clock resolution, looks unchanged.
- A root that cannot be read, or holds no audio file while some of its tracks are present, is left as it was (step 1), so that an unmounted drive's tracks are neither marked missing nor taken over by a backup copy under another root. The trade-off: a root emptied on purpose keeps its tracks, which never go missing; `remove_root` removes them. A root that still holds an audio file is walked as usual, so one whose other files were deleted marks them missing. A file moved out of a folder the walk cannot read is a new track at its new path, and the old one stays.
- Only `scan` takes the scan lock, so `remove_root` can run during a scan of the same library; the scan then fails with a database error when it writes a file under the removed root.

## Metadata precedence

In Phase 1 the tags are the only source: each scan of a changed file writes its tags into the track's fields, and nothing else edits them. The scanner's fallbacks are under "Tags and properties" above: the title falls back to the file stem, the artist to "Unknown Artist", and the album artist to the artist except on a compilation (below).

Artists and albums are shared records found by key. An artist is the same artist whenever its name is the same ignoring case and surrounding whitespace, and the first spelling stored is kept. An album is the same album whenever its album artist and its title key are the same.

Manual overrides come later (brief §4.1, "Later additions"). They will live in their own table, which a migration adds, and win over the tags, so that a rescan never undoes a user's edit.

## Compilations and DJ sets

`album.kind` says what a release is: `album`, `compilation` (tracks by several artists) or `dj_set` (a long recording and the tracks split from it). Each track keeps its own artist, and the album has an album artist, so a compilation lists under its album artist ("Various Artists" is an ordinary artist) while each track lists under its performer.

An album may have no album artist (`album_artist_id` is null): a compilation whose files have no album-artist tag. Falling back to each track's artist there would split one compilation into one album per artist, so the scanner leaves it null. SQLite treats nulls as distinct in a `UNIQUE` constraint, so the unique index is on `IFNULL(album_artist_id, 0)` instead: identifiers start at 1, and all albums without an album artist share the value 0. Two such albums with the same title are therefore one album, whichever code inserts them.

**How an album without an album artist reads.** `terra_core::Album` requires an album artist, so the backend shows the artist of the album's first track, in `list_tracks` order, and leaves out such an album while it has no tracks. Filtering albums by album artist and searching by album artist use the stored album artist only, so such an album matches neither. The alternative is to make `Album::album_artist` optional in `terra-core`, which every interface would then handle; that is a change to #3's model and best made after the TUI (#6) merges.

## Search

Three FTS5 tables index the fields the backend contract searches:

- `track_fts (title, artist, album)`: a track's title, its artist's name and its album's title;
- `album_fts (title, album_artist)`: an album's title and its album artist's name;
- `artist_fts (name)`: an artist's name.

Each is declared with `tokenize = 'unicode61 remove_diacritics 2'`, so search ignores case and the diacritics on Latin letters (below). Each row's rowid is the record's `id`. The issue proposed only `track_fts`; the other two let the artist and album categories of a search use the index too, including artists without tracks.

The tables store their own copy of the text, and the storage code keeps it current: the helpers `fts::sync_track`, `fts::sync_album` and `fts::sync_artist` rewrite a record's row, and those of the records that show its name, inside the caller's transaction. Every write in `src/write.rs` calls them, and the scanner reuses them. Triggers could do the same, but would hide writes from the code that makes them.

A user's query is split into terms at every character that is not alphanumeric, as the backend contract says, except that a combining mark (general category `Mn`) stays in its word. Each term becomes a quoted FTS5 string with a prefix match, `"term"*`, and FTS5 requires every one of them in some column of the row. So no input is read as FTS5 syntax: `"`, `*`, `-`, `:` and parentheses only separate terms, and `NEAR`, `AND`, `OR` and `NOT` are ordinary words. A query without terms, or whose terms are only marks, finds nothing and runs no SQL.

**Accents.** The backend contract says that search ignores diacritics on Latin letters, and that a precomposed letter (`é`, U+00E9) and the same letter as a base letter and a combining mark (`e` and U+0301) match each other and `e`. The Rust code folds the indexed text and the query terms alike, the same way as the in-memory backend: `fts::fold` decomposes the text (NFD), drops every combining mark (general category `Mn`) and lower-cases what is left. `Café`, `Cafe` followed by U+0301, `CAFÉ` and `ca` followed by U+0305 and `fe` all fold to `cafe`. The `sync_*` functions write the folded text to the FTS tables, and search keeps reading the names it displays from the record tables. `match_query` splits the query into terms first, where a mark does not split a word, then folds each term and drops a term that folding leaves empty. The tokenizer's `remove_diacritics 2` has no marks left to remove; folding in Rust matters because the tokenizer reads some marks, such as the overline U+0305, as separators rather than diacritics, which split `ca` from `fe`. Folding goes beyond the contract's Latin letters: a precomposed Greek `ά` folds to `α` in the index and in the query alike. Letters without a decomposition, such as `ø`, `ł` and `ß`, are not folded. Rows written before folding was added hold the text unfolded; the tokenizer still removes Latin diacritics from them, but a query no longer matches the accented letters it does not fold, such as Greek `ά`, or a word that a mark like U+0305 splits, until the record is written again. No migration rebuilds them.

## Playing a file

`resolve_audio_source` joins the root's `path` and the file's `rel_path`. When `missing_since` is set it fails with `LibraryError::Storage`, whose source is `terra_storage::MissingFile` naming the track and the path, so the interface can tell it from other failures.

## Phase 2

The DJ-set importer (brief §5, §11) adds a migration, `0002`, for import jobs, source recordings and source segments: `import_job` (URL, stage, status, logs, results), `source_recording` (the preserved full-length audio and its original metadata) and `source_segment` (a derived track's recording and start and end times). It is expected to add tables and leave the ones above as they are.
