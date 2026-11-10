//! Writing library roots, artists, albums and tracks.
//!
//! These helpers are the only code that inserts, changes or deletes library
//! records. Each takes
//! a connection, so the caller decides the transaction: a scan writes each
//! file, its track and its search rows in one. The tests seed their
//! libraries with them.

use std::path::Path;
use std::time::{Duration, SystemTime};

use sqlx::SqliteConnection;
use terra_core::{AlbumId, AlbumKind, ArtistId, AudioFormat, TrackId};

use crate::backend::album_kind_text;
use crate::fts;
use crate::time::to_nanos;

/// Identifies a library root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RootId(pub(crate) i64);

/// An album to find or add.
#[derive(Debug, Clone)]
pub(crate) struct NewAlbum<'a> {
    pub(crate) title: &'a str,
    pub(crate) album_artist: Option<ArtistId>,
    pub(crate) year: Option<i32>,
    pub(crate) kind: AlbumKind,
}

/// A track to add, with its file.
#[derive(Debug, Clone)]
pub(crate) struct NewTrack<'a> {
    pub(crate) title: &'a str,
    pub(crate) artist: ArtistId,
    pub(crate) album: Option<AlbumId>,
    pub(crate) disc_no: Option<u32>,
    pub(crate) track_no: Option<u32>,
    pub(crate) duration: Duration,
    pub(crate) added_at: SystemTime,
    pub(crate) file: NewFile<'a>,
}

/// The audio file of a [`NewTrack`].
#[derive(Debug, Clone)]
pub(crate) struct NewFile<'a> {
    pub(crate) root: RootId,
    /// The path within the root.
    pub(crate) rel_path: &'a str,
    pub(crate) size: u64,
    pub(crate) mtime: SystemTime,
    pub(crate) content_hash: &'a [u8],
    pub(crate) format: &'a AudioFormat,
    pub(crate) has_embedded_art: bool,
}

/// The key a name or title is unique and found by: trimmed and Unicode
/// lower-cased.
pub(crate) fn key(text: &str) -> String {
    text.trim().to_lowercase()
}

fn nanos(time: SystemTime) -> sqlx::Result<i64> {
    to_nanos(time).ok_or_else(|| sqlx::Error::Encode(format!("{time:?} is out of range").into()))
}

fn integer<T: TryInto<i64> + Copy + std::fmt::Display>(value: T, what: &str) -> sqlx::Result<i64> {
    value
        .try_into()
        .map_err(|_| sqlx::Error::Encode(format!("the {what} {value} is out of range").into()))
}

/// Adds a library root.
///
/// The path is stored as text, so it must be valid UTF-8.
pub(crate) async fn insert_root(
    conn: &mut SqliteConnection,
    path: &Path,
    added_at: SystemTime,
) -> sqlx::Result<RootId> {
    let path = path
        .to_str()
        .ok_or_else(|| sqlx::Error::Encode(format!("{path:?} is not valid UTF-8").into()))?;
    let id = sqlx::query_scalar(
        "INSERT INTO library_root (path, added_at) VALUES (?1, ?2) RETURNING id",
    )
    .bind(path)
    .bind(nanos(added_at)?)
    .fetch_one(conn)
    .await?;
    Ok(RootId(id))
}

/// Returns the artist whose name equals `name` ignoring case and surrounding
/// whitespace, adding it if there is none. An existing artist keeps its
/// name and sort name.
pub(crate) async fn ensure_artist(
    conn: &mut SqliteConnection,
    name: &str,
    sort_name: &str,
) -> sqlx::Result<ArtistId> {
    let name_key = key(name);
    let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM artist WHERE name_key = ?1")
        .bind(&name_key)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = existing {
        return Ok(ArtistId::new(id));
    }
    let id = sqlx::query_scalar(
        "INSERT INTO artist (name, sort_name, name_key) VALUES (?1, ?2, ?3) RETURNING id",
    )
    .bind(name.trim())
    .bind(sort_name.trim())
    .bind(&name_key)
    .fetch_one(&mut *conn)
    .await
    .map(ArtistId::new)?;
    fts::sync_artist(conn, id).await?;
    Ok(id)
}

/// Returns the album with this album artist (or none) whose title equals
/// the album's ignoring case and surrounding whitespace, adding it if there
/// is none. An existing album keeps its fields.
pub(crate) async fn ensure_album(
    conn: &mut SqliteConnection,
    album: &NewAlbum<'_>,
) -> sqlx::Result<AlbumId> {
    let title_key = key(album.title);
    let artist = album.album_artist.map(ArtistId::get);
    // The same expression as the unique index `album_artist_title`, so that
    // albums without an album artist match each other.
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM album WHERE IFNULL(album_artist_id, 0) = IFNULL(?1, 0) AND title_key = ?2",
    )
    .bind(artist)
    .bind(&title_key)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        return Ok(AlbumId::new(id));
    }
    let id = sqlx::query_scalar(
        "INSERT INTO album (title, title_key, album_artist_id, year, kind)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
    )
    .bind(album.title.trim())
    .bind(&title_key)
    .bind(artist)
    .bind(album.year)
    .bind(album_kind_text(album.kind))
    .fetch_one(&mut *conn)
    .await
    .map(AlbumId::new)?;
    fts::sync_album(conn, id).await?;
    Ok(id)
}

/// Adds a track and its file, and indexes it for search.
pub(crate) async fn insert_track(
    conn: &mut SqliteConnection,
    track: &NewTrack<'_>,
) -> sqlx::Result<TrackId> {
    let added_at = nanos(track.added_at)?;
    let id = sqlx::query_scalar(
        "INSERT INTO track
             (title, artist_id, album_id, disc_no, track_no, duration_ms, added_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) RETURNING id",
    )
    .bind(track.title)
    .bind(track.artist.get())
    .bind(track.album.map(AlbumId::get))
    .bind(track.disc_no)
    .bind(track.track_no)
    .bind(integer(
        track.duration.as_millis(),
        "duration in milliseconds",
    )?)
    .bind(added_at)
    .fetch_one(&mut *conn)
    .await
    .map(TrackId::new)?;

    let file = &track.file;
    sqlx::query(
        "INSERT INTO media_file
             (track_id, root_id, rel_path, size, mtime_ns, content_hash, codec, lossless,
              sample_rate, channels, bitrate, has_embedded_art)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(id.get())
    .bind(file.root.0)
    .bind(file.rel_path)
    .bind(integer(file.size, "file size")?)
    .bind(nanos(file.mtime)?)
    .bind(file.content_hash)
    .bind(&file.format.codec)
    .bind(file.format.lossless)
    .bind(file.format.sample_rate)
    .bind(file.format.channels)
    .bind(file.format.bitrate)
    .bind(file.has_embedded_art)
    .execute(&mut *conn)
    .await?;

    fts::sync_track(conn, id).await?;
    Ok(id)
}

/// Deletes a track with its file, playlist entries and plays, and removes it
/// from search. Returns whether the track existed.
pub(crate) async fn delete_track(conn: &mut SqliteConnection, id: TrackId) -> sqlx::Result<bool> {
    let deleted = sqlx::query("DELETE FROM track WHERE id = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?
        .rows_affected()
        > 0;
    fts::sync_track(conn, id).await?;
    Ok(deleted)
}

/// Rewrites a track's fields and its file's, clears the file's
/// `missing_since`, and reindexes the track. The track keeps its identifier
/// and `added_at` (`track.added_at` is ignored); `updated_at` becomes
/// `updated_at`. An artist or album the track no longer uses is deleted when
/// nothing else uses it either.
pub(crate) async fn update_track(
    conn: &mut SqliteConnection,
    id: TrackId,
    track: &NewTrack<'_>,
    updated_at: SystemTime,
) -> sqlx::Result<()> {
    let (old_artist, old_album): (i64, Option<i64>) =
        sqlx::query_as("SELECT artist_id, album_id FROM track WHERE id = ?1")
            .bind(id.get())
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query(
        "UPDATE track
         SET title = ?2, artist_id = ?3, album_id = ?4, disc_no = ?5, track_no = ?6,
             duration_ms = ?7, updated_at = ?8
         WHERE id = ?1",
    )
    .bind(id.get())
    .bind(track.title)
    .bind(track.artist.get())
    .bind(track.album.map(AlbumId::get))
    .bind(track.disc_no)
    .bind(track.track_no)
    .bind(integer(
        track.duration.as_millis(),
        "duration in milliseconds",
    )?)
    .bind(nanos(updated_at)?)
    .execute(&mut *conn)
    .await?;

    let file = &track.file;
    sqlx::query(
        "UPDATE media_file
         SET root_id = ?2, rel_path = ?3, size = ?4, mtime_ns = ?5, content_hash = ?6,
             codec = ?7, lossless = ?8, sample_rate = ?9, channels = ?10, bitrate = ?11,
             has_embedded_art = ?12, missing_since = NULL
         WHERE track_id = ?1",
    )
    .bind(id.get())
    .bind(file.root.0)
    .bind(file.rel_path)
    .bind(integer(file.size, "file size")?)
    .bind(nanos(file.mtime)?)
    .bind(file.content_hash)
    .bind(&file.format.codec)
    .bind(file.format.lossless)
    .bind(file.format.sample_rate)
    .bind(file.format.channels)
    .bind(file.format.bitrate)
    .bind(file.has_embedded_art)
    .execute(&mut *conn)
    .await?;

    fts::sync_track(conn, id).await?;
    let old_album = old_album.map(AlbumId::new);
    if old_album != track.album {
        delete_unused_album(conn, old_album).await?;
    }
    if old_artist != track.artist.get() {
        delete_unused_artist(conn, ArtistId::new(old_artist)).await?;
    }
    Ok(())
}

/// Sets `missing_since` on a file that does not have it yet. Returns whether
/// it changed.
pub(crate) async fn mark_missing(
    conn: &mut SqliteConnection,
    file_id: i64,
    since: SystemTime,
) -> sqlx::Result<bool> {
    let changed = sqlx::query(
        "UPDATE media_file SET missing_since = ?2 WHERE id = ?1 AND missing_since IS NULL",
    )
    .bind(file_id)
    .bind(nanos(since)?)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(changed > 0)
}

/// Clears `missing_since` on a file.
pub(crate) async fn clear_missing(conn: &mut SqliteConnection, file_id: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE media_file SET missing_since = NULL WHERE id = ?1")
        .bind(file_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Deletes a library root with its files and their tracks (and so their
/// playlist entries and plays), and the artists and albums nothing uses any
/// more. Returns whether the root existed.
pub(crate) async fn delete_root(conn: &mut SqliteConnection, root: RootId) -> sqlx::Result<bool> {
    let tracks: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT t.id, t.artist_id, t.album_id
         FROM media_file f JOIN track t ON t.id = f.track_id
         WHERE f.root_id = ?1
         ORDER BY t.id",
    )
    .bind(root.0)
    .fetch_all(&mut *conn)
    .await?;
    for &(track, _, _) in &tracks {
        delete_track(conn, TrackId::new(track)).await?;
    }
    let mut albums: Vec<i64> = tracks.iter().filter_map(|&(_, _, album)| album).collect();
    albums.sort_unstable();
    albums.dedup();
    for album in albums {
        delete_unused_album(conn, Some(AlbumId::new(album))).await?;
    }
    let mut artists: Vec<i64> = tracks.iter().map(|&(_, artist, _)| artist).collect();
    artists.sort_unstable();
    artists.dedup();
    for artist in artists {
        delete_unused_artist(conn, ArtistId::new(artist)).await?;
    }
    let deleted = sqlx::query("DELETE FROM library_root WHERE id = ?1")
        .bind(root.0)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

/// Deletes an album that no track is on, then its album artist if nothing
/// uses that either.
async fn delete_unused_album(
    conn: &mut SqliteConnection,
    album: Option<AlbumId>,
) -> sqlx::Result<()> {
    let Some(album) = album else {
        return Ok(());
    };
    let deleted: Option<Option<i64>> = sqlx::query_scalar(
        "DELETE FROM album
         WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM track WHERE album_id = ?1)
         RETURNING album_artist_id",
    )
    .bind(album.get())
    .fetch_optional(&mut *conn)
    .await?;
    let Some(album_artist) = deleted else {
        return Ok(());
    };
    fts::sync_album(conn, album).await?;
    if let Some(artist) = album_artist {
        delete_unused_artist(conn, ArtistId::new(artist)).await?;
    }
    Ok(())
}

/// Deletes an artist that no track or album uses.
async fn delete_unused_artist(conn: &mut SqliteConnection, artist: ArtistId) -> sqlx::Result<()> {
    let deleted = sqlx::query(
        "DELETE FROM artist
         WHERE id = ?1
           AND NOT EXISTS (SELECT 1 FROM track WHERE artist_id = ?1)
           AND NOT EXISTS (SELECT 1 FROM album WHERE album_artist_id = ?1)",
    )
    .bind(artist.get())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if deleted > 0 {
        fts::sync_artist(conn, artist).await?;
    }
    Ok(())
}
