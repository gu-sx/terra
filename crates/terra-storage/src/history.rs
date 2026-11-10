//! Play history: which device played which track, and when.

use std::time::SystemTime;

use sqlx::SqlitePool;
use terra_core::{LibraryError, Play, Resource, TrackId};

use crate::backend::{TrackRow, sql_count, track_columns, track_tables};
use crate::time::{from_nanos, to_nanos};

#[derive(sqlx::FromRow)]
struct PlayRow {
    device: String,
    played_at: i64,
    #[sqlx(flatten)]
    track: TrackRow,
}

impl TryFrom<PlayRow> for Play {
    type Error = LibraryError;

    fn try_from(row: PlayRow) -> Result<Self, LibraryError> {
        Ok(Self {
            track: row.track.try_into()?,
            device: row.device,
            at: from_nanos(row.played_at),
        })
    }
}

/// Records a play. The device name is stored as given.
///
/// A time the database cannot store, outside roughly the years 1678 to
/// 2262, is [`LibraryError::InvalidInput`].
pub(crate) async fn record_play(
    pool: &SqlitePool,
    track: TrackId,
    device: &str,
    at: SystemTime,
) -> Result<(), LibraryError> {
    if device.trim().is_empty() {
        return Err(LibraryError::InvalidInput(
            "a device name cannot be empty".to_owned(),
        ));
    }
    let at = to_nanos(at).ok_or_else(|| {
        LibraryError::InvalidInput(format!("the play time {at:?} is out of range"))
    })?;
    // Inserting from the track's row checks that it exists in the same
    // statement.
    let inserted = sqlx::query(
        "INSERT INTO play_history (track_id, device, played_at)
         SELECT id, ?2, ?3 FROM track WHERE id = ?1",
    )
    .bind(track.get())
    .bind(device)
    .bind(at)
    .execute(pool)
    .await
    .map_err(LibraryError::storage)?
    .rows_affected();
    if inserted == 0 {
        return Err(LibraryError::NotFound(Resource::Track(track)));
    }
    Ok(())
}

/// Returns at most `limit` plays, the latest first, and of plays at the same
/// time the last recorded first.
pub(crate) async fn recently_played(
    pool: &SqlitePool,
    limit: usize,
) -> Result<Vec<Play>, LibraryError> {
    let rows: Vec<PlayRow> = sqlx::query_as(concat!(
        "SELECT p.device, p.played_at,",
        track_columns!(),
        track_tables!(),
        "JOIN play_history p ON p.track_id = t.id
         ORDER BY p.played_at DESC, p.id DESC
         LIMIT ?1"
    ))
    .bind(sql_count(limit))
    .fetch_all(pool)
    .await
    .map_err(LibraryError::storage)?;
    rows.into_iter().map(Play::try_from).collect()
}
