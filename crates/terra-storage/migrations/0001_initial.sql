-- The Phase 1 library schema. docs/design/library-schema.md explains it.
--
-- Every timestamp is an INTEGER count of nanoseconds since the Unix epoch.
-- Every `*_key` column is the trimmed, Unicode lower-cased form of the text
-- it belongs to, which the storage code computes.

CREATE TABLE library_root (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
);

CREATE TABLE artist (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    sort_name TEXT NOT NULL,
    name_key TEXT NOT NULL UNIQUE
);

CREATE TABLE album (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    title_key TEXT NOT NULL,
    album_artist_id INTEGER REFERENCES artist (id),
    year INTEGER,
    kind TEXT NOT NULL CHECK (kind IN ('album', 'compilation', 'dj_set'))
);

-- SQLite treats NULLs as distinct in a UNIQUE constraint, so a plain
-- UNIQUE (album_artist_id, title_key) would allow any number of albums with
-- the same title and no album artist. Identifiers start at 1, so 0 stands for
-- "no album artist" and makes those albums collide like any other.
CREATE UNIQUE INDEX album_artist_title ON album (IFNULL(album_artist_id, 0), title_key);
CREATE INDEX album_album_artist ON album (album_artist_id);

CREATE TABLE track (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    artist_id INTEGER NOT NULL REFERENCES artist (id),
    album_id INTEGER REFERENCES album (id),
    disc_no INTEGER CHECK (disc_no >= 0),
    track_no INTEGER CHECK (track_no >= 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    added_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX track_artist ON track (artist_id);
CREATE INDEX track_album ON track (album_id);
CREATE INDEX track_added ON track (added_at, id);

CREATE TABLE media_file (
    id INTEGER PRIMARY KEY,
    track_id INTEGER NOT NULL UNIQUE REFERENCES track (id) ON DELETE CASCADE,
    root_id INTEGER NOT NULL REFERENCES library_root (id),
    rel_path TEXT NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    mtime_ns INTEGER NOT NULL,
    content_hash BLOB NOT NULL,
    codec TEXT NOT NULL,
    lossless INTEGER NOT NULL CHECK (lossless IN (0, 1)),
    sample_rate INTEGER NOT NULL CHECK (sample_rate >= 0),
    channels INTEGER NOT NULL CHECK (channels >= 0),
    bitrate INTEGER CHECK (bitrate >= 0),
    has_embedded_art INTEGER NOT NULL CHECK (has_embedded_art IN (0, 1)),
    missing_since INTEGER,
    UNIQUE (root_id, rel_path)
);

CREATE INDEX media_file_content_hash ON media_file (content_hash);

CREATE TABLE playlist (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playlist_entry (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlist (id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0)
);

CREATE INDEX playlist_entry_position ON playlist_entry (playlist_id, position);
CREATE INDEX playlist_entry_track ON playlist_entry (track_id);

CREATE TABLE play_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id INTEGER NOT NULL REFERENCES track (id) ON DELETE CASCADE,
    device TEXT NOT NULL,
    played_at INTEGER NOT NULL
);

CREATE INDEX play_history_played ON play_history (played_at, id);
CREATE INDEX play_history_track ON play_history (track_id);

-- Full-text search. Each table's rowid is the identifier of the record it
-- indexes, and the storage code rewrites a record's row in the transaction
-- that changes the record (src/fts.rs).
CREATE VIRTUAL TABLE track_fts USING fts5 (
    title,
    artist,
    album,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE album_fts USING fts5 (
    title,
    album_artist,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE VIRTUAL TABLE artist_fts USING fts5 (
    name,
    tokenize = 'unicode61 remove_diacritics 2'
);
