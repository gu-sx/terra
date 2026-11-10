//! The read side of [`LibraryBackend`] for [`LocalLibrary`].

use std::path::Path;
use std::time::{Duration, SystemTime};

use terra_core::{
    Album, AlbumFilter, AlbumId, AlbumKind, AlbumRef, Artist, ArtistId, ArtistRef, AudioFormat,
    AudioSource, LibraryBackend, LibraryError, Page, Play, Playlist, PlaylistContents,
    PlaylistEntry, PlaylistEntryId, PlaylistId, Resource, SearchResults, Track, TrackFilter,
    TrackId,
};

use crate::error::MissingFile;
use crate::library::LocalLibrary;
use crate::time::from_nanos;
use crate::{fts, history, playlist};

// The queries below spell out `COLLATE unicode_nocase`, the collation
// `library::NOCASE` names, because `concat!` takes only literals.

/// The columns of a [`TrackRow`], from [`track_tables!`].
macro_rules! track_columns {
    () => {
        " t.id, t.title, t.artist_id, ar.name AS artist_name, t.album_id,
          al.title AS album_title, t.disc_no, t.track_no, t.duration_ms, t.added_at,
          f.codec, f.lossless, f.sample_rate, f.channels, f.bitrate "
    };
}

/// `track t` and the tables a [`TrackRow`] joins to it. A query may join
/// more after them.
macro_rules! track_tables {
    () => {
        " FROM track t
          JOIN artist ar ON ar.id = t.artist_id
          LEFT JOIN album al ON al.id = t.album_id
          JOIN media_file f ON f.track_id = t.id "
    };
}

pub(crate) use {track_columns, track_tables};

macro_rules! select_tracks {
    () => {
        concat!("SELECT", track_columns!(), track_tables!())
    };
}

/// The order of `list_tracks`: album title (tracks without an album last),
/// album, disc and track number (missing numbers last), title, identifier.
macro_rules! order_tracks {
    () => {
        " ORDER BY al.id IS NULL, al.title COLLATE unicode_nocase, al.id,
                   t.disc_no IS NULL, t.disc_no, t.track_no IS NULL, t.track_no,
                   t.title COLLATE unicode_nocase, t.id "
    };
}

/// The columns of an [`AlbumRow`], from `album al` and its album artist.
///
/// An album without an album artist shows the artist of its first track, in
/// the order of `list_tracks`; one without tracks either is left out, because
/// [`Album`] needs an artist. `docs/design/library-schema.md` explains why.
macro_rules! select_albums {
    () => {
        "SELECT al.id, al.title, al.year, al.kind, ar.id AS artist_id, ar.name AS artist_name
         FROM album al
         JOIN artist ar ON ar.id = IFNULL(
             al.album_artist_id,
             (SELECT t.artist_id FROM track t WHERE t.album_id = al.id
              ORDER BY t.disc_no IS NULL, t.disc_no, t.track_no IS NULL, t.track_no,
                       t.title COLLATE unicode_nocase, t.id
              LIMIT 1)
         ) "
    };
}

macro_rules! order_albums {
    () => {
        " ORDER BY al.title COLLATE unicode_nocase, al.id "
    };
}

macro_rules! select_artists {
    () => {
        "SELECT ar.id, ar.name, ar.sort_name FROM artist ar "
    };
}

macro_rules! order_artists {
    () => {
        " ORDER BY ar.sort_name COLLATE unicode_nocase, ar.id "
    };
}

#[derive(sqlx::FromRow)]
struct ArtistRow {
    id: i64,
    name: String,
    sort_name: String,
}

impl From<ArtistRow> for Artist {
    fn from(row: ArtistRow) -> Self {
        Self {
            id: ArtistId::new(row.id),
            name: row.name,
            sort_name: row.sort_name,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AlbumRow {
    id: i64,
    title: String,
    year: Option<i64>,
    kind: String,
    artist_id: i64,
    artist_name: String,
}

impl TryFrom<AlbumRow> for Album {
    type Error = LibraryError;

    fn try_from(row: AlbumRow) -> Result<Self, LibraryError> {
        Ok(Self {
            id: AlbumId::new(row.id),
            title: row.title,
            album_artist: ArtistRef {
                id: ArtistId::new(row.artist_id),
                name: row.artist_name,
            },
            year: row.year.map(|year| number(year, "year")).transpose()?,
            kind: album_kind(&row.kind)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct TrackRow {
    id: i64,
    title: String,
    artist_id: i64,
    artist_name: String,
    album_id: Option<i64>,
    album_title: Option<String>,
    disc_no: Option<i64>,
    track_no: Option<i64>,
    duration_ms: i64,
    added_at: i64,
    codec: String,
    lossless: bool,
    sample_rate: i64,
    channels: i64,
    bitrate: Option<i64>,
}

impl TryFrom<TrackRow> for Track {
    type Error = LibraryError;

    fn try_from(row: TrackRow) -> Result<Self, LibraryError> {
        let album = match (row.album_id, row.album_title) {
            (Some(id), Some(title)) => Some(AlbumRef {
                id: AlbumId::new(id),
                title,
            }),
            _ => None,
        };
        Ok(Self {
            id: TrackId::new(row.id),
            title: row.title,
            artist: ArtistRef {
                id: ArtistId::new(row.artist_id),
                name: row.artist_name,
            },
            album,
            disc_no: row.disc_no.map(|n| number(n, "disc number")).transpose()?,
            track_no: row
                .track_no
                .map(|n| number(n, "track number"))
                .transpose()?,
            duration: Duration::from_millis(number(row.duration_ms, "duration")?),
            format: AudioFormat {
                codec: row.codec,
                lossless: row.lossless,
                sample_rate: number(row.sample_rate, "sample rate")?,
                channels: number(row.channels, "channel count")?,
                bitrate: row.bitrate.map(|n| number(n, "bitrate")).transpose()?,
            },
            added_at: from_nanos(row.added_at),
        })
    }
}

/// Converts a stored integer, which the schema's checks should keep in range.
pub(crate) fn number<T: TryFrom<i64>>(value: i64, what: &str) -> Result<T, LibraryError> {
    T::try_from(value)
        .map_err(|_| LibraryError::storage(format!("the stored {what} {value} is out of range")))
}

fn album_kind(kind: &str) -> Result<AlbumKind, LibraryError> {
    match kind {
        "album" => Ok(AlbumKind::Album),
        "compilation" => Ok(AlbumKind::Compilation),
        "dj_set" => Ok(AlbumKind::DjSet),
        other => Err(LibraryError::storage(format!(
            "the stored album kind {other:?} is unknown"
        ))),
    }
}

/// The text `album.kind` stores for a kind.
pub(crate) fn album_kind_text(kind: AlbumKind) -> &'static str {
    match kind {
        AlbumKind::Album => "album",
        AlbumKind::Compilation => "compilation",
        AlbumKind::DjSet => "dj_set",
    }
}

/// A `LIMIT` or `OFFSET` value. SQLite takes an `i64`; anything larger means
/// "no limit" in practice.
pub(crate) fn sql_count(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

pub(crate) fn tracks(rows: Vec<TrackRow>) -> Result<Vec<Track>, LibraryError> {
    rows.into_iter().map(Track::try_from).collect()
}

fn albums(rows: Vec<AlbumRow>) -> Result<Vec<Album>, LibraryError> {
    rows.into_iter().map(Album::try_from).collect()
}

impl LibraryBackend for LocalLibrary {
    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults, LibraryError> {
        let Some(query) = fts::match_query(query) else {
            return Ok(SearchResults::default());
        };
        let limit = sql_count(limit);
        let artists: Vec<ArtistRow> = sqlx::query_as(concat!(
            select_artists!(),
            "WHERE ar.id IN (SELECT rowid FROM artist_fts WHERE artist_fts MATCH ?1)",
            order_artists!(),
            "LIMIT ?2"
        ))
        .bind(&query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        let album_rows: Vec<AlbumRow> = sqlx::query_as(concat!(
            select_albums!(),
            "WHERE al.id IN (SELECT rowid FROM album_fts WHERE album_fts MATCH ?1)",
            order_albums!(),
            "LIMIT ?2"
        ))
        .bind(&query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        let track_rows: Vec<TrackRow> = sqlx::query_as(concat!(
            select_tracks!(),
            "WHERE t.id IN (SELECT rowid FROM track_fts WHERE track_fts MATCH ?1)",
            order_tracks!(),
            "LIMIT ?2"
        ))
        .bind(&query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        Ok(SearchResults {
            artists: artists.into_iter().map(Artist::from).collect(),
            albums: albums(album_rows)?,
            tracks: tracks(track_rows)?,
        })
    }

    async fn list_artists(&self) -> Result<Vec<Artist>, LibraryError> {
        let rows: Vec<ArtistRow> = sqlx::query_as(concat!(select_artists!(), order_artists!()))
            .fetch_all(&self.pool)
            .await
            .map_err(LibraryError::storage)?;
        Ok(rows.into_iter().map(Artist::from).collect())
    }

    async fn list_albums(&self, filter: AlbumFilter) -> Result<Vec<Album>, LibraryError> {
        let rows: Vec<AlbumRow> = sqlx::query_as(concat!(
            select_albums!(),
            "WHERE (?1 IS NULL OR al.album_artist_id = ?1) AND (?2 IS NULL OR al.kind = ?2)",
            order_albums!()
        ))
        .bind(filter.album_artist.map(ArtistId::get))
        .bind(filter.kind.map(album_kind_text))
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        albums(rows)
    }

    async fn list_tracks(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> Result<Vec<Track>, LibraryError> {
        let rows: Vec<TrackRow> = sqlx::query_as(concat!(
            select_tracks!(),
            "WHERE (?1 IS NULL OR t.artist_id = ?1) AND (?2 IS NULL OR t.album_id = ?2)",
            order_tracks!(),
            "LIMIT ?3 OFFSET ?4"
        ))
        .bind(filter.artist.map(ArtistId::get))
        .bind(filter.album.map(AlbumId::get))
        .bind(sql_count(page.limit))
        .bind(sql_count(page.offset))
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        tracks(rows)
    }

    async fn get_track(&self, id: TrackId) -> Result<Track, LibraryError> {
        let row: Option<TrackRow> = sqlx::query_as(concat!(select_tracks!(), "WHERE t.id = ?1"))
            .bind(id.get())
            .fetch_optional(&self.pool)
            .await
            .map_err(LibraryError::storage)?;
        row.ok_or(LibraryError::NotFound(Resource::Track(id)))?
            .try_into()
    }

    async fn recently_added(&self, limit: usize) -> Result<Vec<Track>, LibraryError> {
        let rows: Vec<TrackRow> = sqlx::query_as(concat!(
            select_tracks!(),
            "ORDER BY t.added_at DESC, t.id DESC LIMIT ?1"
        ))
        .bind(sql_count(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        tracks(rows)
    }

    async fn list_playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        playlist::list(&self.pool).await
    }

    async fn get_playlist(&self, id: PlaylistId) -> Result<PlaylistContents, LibraryError> {
        playlist::get(&self.pool, id).await
    }

    async fn create_playlist(&self, name: &str) -> Result<Playlist, LibraryError> {
        playlist::create(&self.pool, name).await
    }

    async fn rename_playlist(&self, id: PlaylistId, name: &str) -> Result<Playlist, LibraryError> {
        playlist::rename(&self.pool, id, name).await
    }

    async fn delete_playlist(&self, id: PlaylistId) -> Result<(), LibraryError> {
        playlist::delete(&self.pool, id).await
    }

    async fn add_to_playlist(
        &self,
        id: PlaylistId,
        tracks: &[TrackId],
        at: Option<usize>,
    ) -> Result<Vec<PlaylistEntry>, LibraryError> {
        playlist::add(&self.pool, id, tracks, at).await
    }

    async fn remove_from_playlist(
        &self,
        id: PlaylistId,
        entries: &[PlaylistEntryId],
    ) -> Result<(), LibraryError> {
        playlist::remove(&self.pool, id, entries).await
    }

    async fn move_playlist_entry(
        &self,
        id: PlaylistId,
        entry: PlaylistEntryId,
        to: usize,
    ) -> Result<(), LibraryError> {
        playlist::move_entry(&self.pool, id, entry, to).await
    }

    /// Joins the file's library root and its path within the root.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the track does not exist, and
    /// [`LibraryError::Storage`] with a [`MissingFile`] as its source if the
    /// last scan did not find the file.
    async fn resolve_audio_source(&self, track: TrackId) -> Result<AudioSource, LibraryError> {
        let row: Option<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT r.path, f.rel_path, f.missing_since
             FROM track t
             LEFT JOIN media_file f ON f.track_id = t.id
             LEFT JOIN library_root r ON r.id = f.root_id
             WHERE t.id = ?1",
        )
        .bind(track.get())
        .fetch_optional(&self.pool)
        .await
        .map_err(LibraryError::storage)?;
        let (root, rel_path, missing_since) =
            row.ok_or(LibraryError::NotFound(Resource::Track(track)))?;
        let (Some(root), Some(rel_path)) = (root, rel_path) else {
            return Err(LibraryError::storage(format!("track {track} has no file")));
        };
        let path = Path::new(&root).join(rel_path);
        if missing_since.is_some() {
            return Err(LibraryError::storage(MissingFile { track, path }));
        }
        Ok(AudioSource::LocalPath(path))
    }

    async fn record_play(
        &self,
        track: TrackId,
        device: &str,
        at: SystemTime,
    ) -> Result<(), LibraryError> {
        history::record_play(&self.pool, track, device, at).await
    }

    async fn recently_played(&self, limit: usize) -> Result<Vec<Play>, LibraryError> {
        history::recently_played(&self.pool, limit).await
    }
}
