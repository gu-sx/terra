# Library schema

## Status

Proposed

## Scope

This document settles the brief's §13.2 ("Library model") for Phase 1: the schema of the local library database, the stable-ID rule, the strategy for detecting moved files, metadata precedence and compilation handling. Migration `crates/terra-storage/migrations/0001_initial.sql` implements it, and the two must agree. It follows the brief's §8 ("Database guidelines").

The scanner (issue #8) adds its algorithm to this document. Playlists and history (issue #9) use the tables below without changing them.

## Files and lifecycle

A library is a directory holding the SQLite database `library.db`. `terra-storage`'s `LocalLibrary` owns it:

- `LocalLibrary::create(dir)` creates `dir` if needed and `dir/library.db`, and applies every migration. It refuses a directory that already has a `library.db`.
- `LocalLibrary::migrate(dir)` applies the migrations the database lacks.
- `LocalLibrary::open(dir)` first reads, through a read-only connection, which migrations the database records. If one is missing it returns `SchemaOutdated` and writes nothing, not even the journal mode, and creates no file. A read-only connection to a WAL database would still create its `-wal` and `-shm` files, so when there is no `-wal` file, and so every committed page is in the database file, the check opens the file as immutable. When there is one, the files already exist and the check reads through them. If the database records a migration this build does not have, it returns `SchemaTooNew`; if a recorded migration's checksum differs from this build's, or one did not finish, `SchemaMismatch`.

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

## Detecting moved files

The scanner (issue #8) implements this, and documents the full algorithm here. A file only counts as moved once the whole walk is done, so that a copy is never mistaken for a move:

1. Walk every root to the end and record each path seen.
2. A file whose (`root`, `rel_path`) is known and whose size and modification time are unchanged is unchanged. It is not read.
3. A known path whose size or modification time changed is read and hashed (BLAKE3, stored in `media_file.content_hash`, which is indexed) and is the same track: its fields are updated in place.
4. Only after the walk, the known files the walk did not see, and those already missing, are the candidates for a move. Each unknown path is read and hashed:
   - If its hash equals a candidate's, it is that file moved: the file row takes the new path, the track keeps its ID, `missing_since` is cleared and the candidate is used up. When several unknown paths share a candidate's hash, the first in (`root`, `rel_path`) order takes it.
   - Anything else is a new track.
5. The candidates left get `missing_since`. Their tracks, playlist entries and plays stay; the field is cleared if the file comes back. `resolve_audio_source` refuses a missing file (below).

So a copy whose original is still in place is a new track, whichever of the two the walk reaches first, because a file the walk saw is never a candidate.

The known limitation: the hash covers the whole file, tags included, so a file that is both moved and retagged between two scans looks like a new track, and the old one is reported missing.

## Metadata precedence

In Phase 1 the tags are the only source: each scan of a changed file writes its tags into the track's fields, and nothing else edits them. The scanner's fallbacks (issue #8): the title falls back to the file stem, the artist to "Unknown Artist", and the album artist to the artist except on a compilation (below).

Artists and albums are shared records found by key. An artist is the same artist whenever its name is the same ignoring case and surrounding whitespace, and the first spelling stored is kept. An album is the same album whenever its album artist and its title key are the same.

Manual overrides come later (brief §4.1, "Later additions"). They will live in their own table, which a migration adds, and win over the tags, so that a rescan never undoes a user's edit.

## Compilations and DJ sets

`album.kind` says what a release is: `album`, `compilation` (tracks by several artists) or `dj_set` (a long recording and the tracks split from it). Each track keeps its own artist, and the album has an album artist, so a compilation lists under its album artist ("Various Artists" is an ordinary artist) while each track lists under its performer.

An album may have no album artist (`album_artist_id` is null): a compilation whose files have no album-artist tag. Falling back to each track's artist there would split one compilation into one album per artist, so the scanner leaves it null. SQLite treats nulls as distinct in a `UNIQUE` constraint, so the unique index is on `IFNULL(album_artist_id, 0)` instead: identifiers start at 1, and all albums without an album artist share the value 0. Two such albums with the same title are therefore one album, whichever code inserts them.

**Proposed: how an album without an album artist reads.** `terra_core::Album` requires an album artist, so the backend shows the artist of the album's first track, in `list_tracks` order, and leaves out such an album while it has no tracks. Filtering albums by album artist and searching by album artist use the stored album artist only, so such an album matches neither. The alternative is to make `Album::album_artist` optional in `terra-core`, which every interface would then handle; that is a change to #3's model and best made after the TUI (#6) merges. The responsible engineer decides.

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
