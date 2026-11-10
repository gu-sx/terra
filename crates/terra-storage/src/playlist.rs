//! Playlists and their entries.
//!
//! Every change runs in one `BEGIN IMMEDIATE` transaction, which takes the
//! write lock before the first read, so the checks a change makes still hold
//! when it writes. A change that alters the entries rewrites their stored
//! positions to 0..n-1 and sets the playlist's `updated_at`. A track deletion
//! removes entries without renumbering, so readers number entries by their
//! order rather than trusting the stored position.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

use sqlx::{SqliteConnection, SqlitePool};
use terra_core::{
    LibraryError, Playlist, PlaylistContents, PlaylistEntry, PlaylistEntryId, PlaylistId, Resource,
    Track, TrackId,
};

use crate::backend::{TrackRow, number, track_columns, track_tables};
use crate::time::to_nanos;
use crate::write::key;

/// A playlist's columns, its entry count and its duration, from `playlist p`.
macro_rules! select_playlists {
    () => {
        "SELECT p.id, p.name, COUNT(e.id) AS entry_count,
                IFNULL(SUM(t.duration_ms), 0) AS duration_ms
         FROM playlist p
         LEFT JOIN playlist_entry e ON e.playlist_id = p.id
         LEFT JOIN track t ON t.id = e.track_id "
    };
}

const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

#[derive(sqlx::FromRow)]
struct PlaylistRow {
    id: i64,
    name: String,
    entry_count: i64,
    duration_ms: i64,
}

impl TryFrom<PlaylistRow> for Playlist {
    type Error = LibraryError;

    fn try_from(row: PlaylistRow) -> Result<Self, LibraryError> {
        Ok(Self {
            id: PlaylistId::new(row.id),
            name: row.name,
            entry_count: number(row.entry_count, "entry count")?,
            duration: Duration::from_millis(number(row.duration_ms, "playlist duration")?),
        })
    }
}

#[derive(sqlx::FromRow)]
struct EntryRow {
    entry_id: i64,
    #[sqlx(flatten)]
    track: TrackRow,
}

/// Lists every playlist, ordered by name ignoring case.
pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Playlist>, LibraryError> {
    let rows: Vec<PlaylistRow> = sqlx::query_as(concat!(
        select_playlists!(),
        "GROUP BY p.id ORDER BY p.name COLLATE unicode_nocase, p.id"
    ))
    .fetch_all(pool)
    .await
    .map_err(LibraryError::storage)?;
    rows.into_iter().map(Playlist::try_from).collect()
}

/// Returns a playlist and its entries, read in one transaction so that the
/// count matches the entries.
pub(crate) async fn get(
    pool: &SqlitePool,
    id: PlaylistId,
) -> Result<PlaylistContents, LibraryError> {
    let mut tx = pool.begin().await.map_err(LibraryError::storage)?;
    let playlist = summary(&mut tx, id).await?;
    let rows: Vec<EntryRow> = sqlx::query_as(concat!(
        "SELECT e.id AS entry_id,",
        track_columns!(),
        track_tables!(),
        "JOIN playlist_entry e ON e.track_id = t.id
         WHERE e.playlist_id = ?1
         ORDER BY e.position, e.id"
    ))
    .bind(id.get())
    .fetch_all(&mut *tx)
    .await
    .map_err(LibraryError::storage)?;
    tx.commit().await.map_err(LibraryError::storage)?;
    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(position, row)| {
            Ok(PlaylistEntry {
                id: PlaylistEntryId::new(row.entry_id),
                position,
                track: row.track.try_into()?,
            })
        })
        .collect::<Result<_, LibraryError>>()?;
    Ok(PlaylistContents { playlist, entries })
}

/// Creates an empty playlist.
pub(crate) async fn create(pool: &SqlitePool, name: &str) -> Result<Playlist, LibraryError> {
    let (name, name_key) = valid_name(name)?;
    let now = now()?;
    let mut tx = pool
        .begin_with(BEGIN_IMMEDIATE)
        .await
        .map_err(LibraryError::storage)?;
    ensure_name_free(&mut tx, name, &name_key, None).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO playlist (name, name_key, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3) RETURNING id",
    )
    .bind(name)
    .bind(&name_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(LibraryError::storage)?;
    tx.commit().await.map_err(LibraryError::storage)?;
    Ok(Playlist {
        id: PlaylistId::new(id),
        name: name.to_owned(),
        entry_count: 0,
        duration: Duration::ZERO,
    })
}

/// Renames a playlist. Its own name in another case is free.
pub(crate) async fn rename(
    pool: &SqlitePool,
    id: PlaylistId,
    name: &str,
) -> Result<Playlist, LibraryError> {
    let mut tx = pool
        .begin_with(BEGIN_IMMEDIATE)
        .await
        .map_err(LibraryError::storage)?;
    ensure_exists(&mut tx, id).await?;
    let (name, name_key) = valid_name(name)?;
    ensure_name_free(&mut tx, name, &name_key, Some(id)).await?;
    sqlx::query("UPDATE playlist SET name = ?1, name_key = ?2, updated_at = ?3 WHERE id = ?4")
        .bind(name)
        .bind(&name_key)
        .bind(now()?)
        .bind(id.get())
        .execute(&mut *tx)
        .await
        .map_err(LibraryError::storage)?;
    let playlist = summary(&mut tx, id).await?;
    tx.commit().await.map_err(LibraryError::storage)?;
    Ok(playlist)
}

/// Deletes a playlist; its entries go with it by cascade.
pub(crate) async fn delete(pool: &SqlitePool, id: PlaylistId) -> Result<(), LibraryError> {
    let deleted = sqlx::query("DELETE FROM playlist WHERE id = ?1")
        .bind(id.get())
        .execute(pool)
        .await
        .map_err(LibraryError::storage)?
        .rows_affected();
    if deleted == 0 {
        return Err(LibraryError::NotFound(Resource::Playlist(id)));
    }
    Ok(())
}

/// Adds tracks at `at`, or at the end, and returns the new entries.
pub(crate) async fn add(
    pool: &SqlitePool,
    id: PlaylistId,
    tracks: &[TrackId],
    at: Option<usize>,
) -> Result<Vec<PlaylistEntry>, LibraryError> {
    let mut tx = pool
        .begin_with(BEGIN_IMMEDIATE)
        .await
        .map_err(LibraryError::storage)?;
    ensure_exists(&mut tx, id).await?;
    let mut order = entry_order(&mut tx, id).await?;
    let at = at.unwrap_or(order.len());
    if at > order.len() {
        return Err(out_of_range("position", at, order.len()));
    }
    let mut found: HashMap<TrackId, Track> = HashMap::new();
    for &track in tracks {
        if let Entry::Vacant(slot) = found.entry(track) {
            slot.insert(find_track(&mut tx, track).await?);
        }
    }
    if tracks.is_empty() {
        return Ok(Vec::new());
    }

    let mut added = Vec::with_capacity(tracks.len());
    for (offset, track) in tracks.iter().enumerate() {
        let position = at + offset;
        let entry: i64 = sqlx::query_scalar(
            "INSERT INTO playlist_entry (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3) RETURNING id",
        )
        .bind(id.get())
        .bind(track.get())
        .bind(sql_position(position)?)
        .fetch_one(&mut *tx)
        .await
        .map_err(LibraryError::storage)?;
        added.push(PlaylistEntry {
            id: PlaylistEntryId::new(entry),
            position,
            track: found[track].clone(),
        });
    }
    order.splice(at..at, added.iter().map(|entry| entry.id.get()));
    renumber(&mut tx, id, &order).await?;
    tx.commit().await.map_err(LibraryError::storage)?;
    Ok(added)
}

/// Removes entries and closes the gaps. Fails without removing any if one
/// of them is not in the playlist.
pub(crate) async fn remove(
    pool: &SqlitePool,
    id: PlaylistId,
    entries: &[PlaylistEntryId],
) -> Result<(), LibraryError> {
    let mut tx = pool
        .begin_with(BEGIN_IMMEDIATE)
        .await
        .map_err(LibraryError::storage)?;
    ensure_exists(&mut tx, id).await?;
    let mut order = entry_order(&mut tx, id).await?;
    let present: BTreeSet<i64> = order.iter().copied().collect();
    if let Some(missing) = entries.iter().find(|entry| !present.contains(&entry.get())) {
        return Err(LibraryError::NotFound(Resource::PlaylistEntry(*missing)));
    }
    if entries.is_empty() {
        return Ok(());
    }
    let remove: BTreeSet<i64> = entries.iter().map(|entry| entry.get()).collect();
    for entry in &remove {
        sqlx::query("DELETE FROM playlist_entry WHERE id = ?1")
            .bind(entry)
            .execute(&mut *tx)
            .await
            .map_err(LibraryError::storage)?;
    }
    order.retain(|entry| !remove.contains(entry));
    renumber(&mut tx, id, &order).await?;
    tx.commit().await.map_err(LibraryError::storage)
}

/// Moves an entry to position `to`, shifting the entries between.
pub(crate) async fn move_entry(
    pool: &SqlitePool,
    id: PlaylistId,
    entry: PlaylistEntryId,
    to: usize,
) -> Result<(), LibraryError> {
    let mut tx = pool
        .begin_with(BEGIN_IMMEDIATE)
        .await
        .map_err(LibraryError::storage)?;
    ensure_exists(&mut tx, id).await?;
    let mut order = entry_order(&mut tx, id).await?;
    let from = order
        .iter()
        .position(|&other| other == entry.get())
        .ok_or(LibraryError::NotFound(Resource::PlaylistEntry(entry)))?;
    if to >= order.len() {
        return Err(out_of_range("target position", to, order.len()));
    }
    if from == to {
        return Ok(());
    }
    let moved = order.remove(from);
    order.insert(to, moved);
    renumber(&mut tx, id, &order).await?;
    tx.commit().await.map_err(LibraryError::storage)
}

/// Checks a playlist name and returns it trimmed, with its key.
fn valid_name(name: &str) -> Result<(&str, String), LibraryError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(LibraryError::InvalidInput(
            "a playlist name cannot be empty".to_owned(),
        ));
    }
    Ok((name, key(name)))
}

/// Fails if a playlist other than `renaming` has the name key.
async fn ensure_name_free(
    conn: &mut SqliteConnection,
    name: &str,
    name_key: &str,
    renaming: Option<PlaylistId>,
) -> Result<(), LibraryError> {
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM playlist WHERE name_key = ?1 AND id IS NOT ?2)",
    )
    .bind(name_key)
    .bind(renaming.map(PlaylistId::get))
    .fetch_one(conn)
    .await
    .map_err(LibraryError::storage)?;
    if taken {
        return Err(LibraryError::InvalidInput(format!(
            "a playlist named {name:?} already exists"
        )));
    }
    Ok(())
}

async fn ensure_exists(conn: &mut SqliteConnection, id: PlaylistId) -> Result<(), LibraryError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM playlist WHERE id = ?1)")
        .bind(id.get())
        .fetch_one(conn)
        .await
        .map_err(LibraryError::storage)?;
    if !exists {
        return Err(LibraryError::NotFound(Resource::Playlist(id)));
    }
    Ok(())
}

async fn summary(conn: &mut SqliteConnection, id: PlaylistId) -> Result<Playlist, LibraryError> {
    let row: Option<PlaylistRow> = sqlx::query_as(concat!(
        select_playlists!(),
        "WHERE p.id = ?1 GROUP BY p.id"
    ))
    .bind(id.get())
    .fetch_optional(conn)
    .await
    .map_err(LibraryError::storage)?;
    row.ok_or(LibraryError::NotFound(Resource::Playlist(id)))?
        .try_into()
}

async fn find_track(conn: &mut SqliteConnection, id: TrackId) -> Result<Track, LibraryError> {
    let row: Option<TrackRow> = sqlx::query_as(concat!(
        "SELECT",
        track_columns!(),
        track_tables!(),
        "WHERE t.id = ?1"
    ))
    .bind(id.get())
    .fetch_optional(conn)
    .await
    .map_err(LibraryError::storage)?;
    row.ok_or(LibraryError::NotFound(Resource::Track(id)))?
        .try_into()
}

/// The identifiers of a playlist's entries, in order.
async fn entry_order(
    conn: &mut SqliteConnection,
    id: PlaylistId,
) -> Result<Vec<i64>, LibraryError> {
    sqlx::query_scalar("SELECT id FROM playlist_entry WHERE playlist_id = ?1 ORDER BY position, id")
        .bind(id.get())
        .fetch_all(conn)
        .await
        .map_err(LibraryError::storage)
}

/// Stores the positions 0..n-1 in the order of `order`, writing only the
/// entries whose position changes, and sets the playlist's `updated_at`.
async fn renumber(
    conn: &mut SqliteConnection,
    id: PlaylistId,
    order: &[i64],
) -> Result<(), LibraryError> {
    for (position, entry) in order.iter().enumerate() {
        let position = sql_position(position)?;
        sqlx::query("UPDATE playlist_entry SET position = ?1 WHERE id = ?2 AND position IS NOT ?1")
            .bind(position)
            .bind(entry)
            .execute(&mut *conn)
            .await
            .map_err(LibraryError::storage)?;
    }
    sqlx::query("UPDATE playlist SET updated_at = ?1 WHERE id = ?2")
        .bind(now()?)
        .bind(id.get())
        .execute(&mut *conn)
        .await
        .map_err(LibraryError::storage)?;
    Ok(())
}

fn sql_position(position: usize) -> Result<i64, LibraryError> {
    i64::try_from(position)
        .map_err(|_| LibraryError::storage(format!("the position {position} is out of range")))
}

fn out_of_range(what: &str, position: usize, len: usize) -> LibraryError {
    LibraryError::InvalidInput(format!(
        "the {what} {position} is out of range for a playlist of {len} entries"
    ))
}

/// The current time in nanoseconds since the Unix epoch.
fn now() -> Result<i64, LibraryError> {
    to_nanos(SystemTime::now())
        .ok_or_else(|| LibraryError::storage("the system clock is out of range"))
}
