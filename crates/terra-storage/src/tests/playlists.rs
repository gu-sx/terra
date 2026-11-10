//! Playlists as the database stores them: positions, `updated_at`, and what
//! survives reopening the library.

use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use terra_core::testing::sample;
use terra_core::{LibraryBackend, LibraryError, PlaylistEntryId, PlaylistId, TrackId};

use super::sample_library;
use crate::LocalLibrary;
use crate::write::delete_track;

/// The sample tracks' identifiers, in the order of the sample's table.
fn sample_tracks() -> Vec<TrackId> {
    sample::library()
        .tracks
        .iter()
        .map(|(track, _)| track.id)
        .collect()
}

/// The positions the database stores for a playlist, in order.
async fn stored_positions(library: &LocalLibrary, id: PlaylistId) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT position FROM playlist_entry WHERE playlist_id = ?1 ORDER BY position, id",
    )
    .bind(id.get())
    .fetch_all(&library.pool)
    .await
    .unwrap()
}

#[track_caller]
fn assert_contiguous(positions: &[i64], len: usize) {
    let expected: Vec<i64> = (0..len as i64).collect();
    assert_eq!(positions, expected, "stored positions are not 0..{len}");
}

async fn entry_ids(library: &LocalLibrary, id: PlaylistId) -> Vec<PlaylistEntryId> {
    let contents = library.get_playlist(id).await.unwrap();
    contents.entries.iter().map(|entry| entry.id).collect()
}

async fn updated_at(library: &LocalLibrary, id: PlaylistId) -> i64 {
    sqlx::query_scalar("SELECT updated_at FROM playlist WHERE id = ?1")
        .bind(id.get())
        .fetch_one(&library.pool)
        .await
        .unwrap()
}

async fn reset_updated_at(library: &LocalLibrary, id: PlaylistId) {
    sqlx::query("UPDATE playlist SET updated_at = 0 WHERE id = ?1")
        .bind(id.get())
        .execute(&library.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn moving_first_to_last_and_last_to_first_keeps_positions_contiguous() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let id = library.create_playlist("Driving").await.unwrap().id;
    let tracks = sample_tracks();
    library.add_to_playlist(id, &tracks, None).await.unwrap();
    let before = entry_ids(&library, id).await;
    let len = before.len();

    library
        .move_playlist_entry(id, before[0], len - 1)
        .await
        .unwrap();
    let mut expected = before.clone();
    expected.rotate_left(1);
    assert_eq!(entry_ids(&library, id).await, expected);
    assert_contiguous(&stored_positions(&library, id).await, len);

    library.move_playlist_entry(id, before[0], 0).await.unwrap();
    assert_eq!(entry_ids(&library, id).await, before);
    assert_contiguous(&stored_positions(&library, id).await, len);

    let last = before[len - 1];
    library.move_playlist_entry(id, last, 0).await.unwrap();
    let mut expected = before.clone();
    expected.rotate_right(1);
    assert_eq!(entry_ids(&library, id).await, expected);
    assert_contiguous(&stored_positions(&library, id).await, len);

    for to in [len, usize::MAX] {
        match library.move_playlist_entry(id, last, to).await {
            Err(LibraryError::InvalidInput(_)) => {}
            other => panic!("expected InvalidInput moving to {to}, got {other:?}"),
        }
        assert_eq!(entry_ids(&library, id).await, expected);
        assert_contiguous(&stored_positions(&library, id).await, len);
    }
}

#[tokio::test]
async fn renaming_to_another_playlists_name_in_another_case_is_invalid() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    library.create_playlist("Favorites").await.unwrap();
    let driving = library.create_playlist("Driving").await.unwrap();

    for name in ["favorites", "FAVORITES", " fAvOrItEs\t"] {
        match library.rename_playlist(driving.id, name).await {
            Err(LibraryError::InvalidInput(_)) => {}
            other => panic!("expected InvalidInput renaming to {name:?}, got {other:?}"),
        }
    }
    assert_eq!(
        library.get_playlist(driving.id).await.unwrap().playlist,
        driving
    );
}

#[tokio::test]
async fn names_are_unique_ignoring_case_beyond_ascii() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    library.create_playlist("Café Mix").await.unwrap();
    match library.create_playlist("CAFÉ MIX").await {
        Err(LibraryError::InvalidInput(_)) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn writes_close_the_gap_a_track_deletion_leaves() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let id = library.create_playlist("Driving").await.unwrap().id;
    let tracks = sample_tracks();
    library
        .add_to_playlist(id, &tracks[..3], None)
        .await
        .unwrap();

    let mut conn = library.pool.acquire().await.unwrap();
    assert!(delete_track(&mut conn, tracks[1]).await.unwrap());
    drop(conn);
    assert_eq!(stored_positions(&library, id).await, [0, 2]);
    let contents = library.get_playlist(id).await.unwrap();
    let positions: Vec<usize> = contents.entries.iter().map(|e| e.position).collect();
    assert_eq!(positions, [0, 1], "readers number entries by their order");
    assert_eq!(contents.playlist.entry_count, 2);

    let added = library
        .add_to_playlist(id, &tracks[3..4], Some(2))
        .await
        .unwrap();
    assert_eq!(added[0].position, 2);
    assert_contiguous(&stored_positions(&library, id).await, 3);
}

#[tokio::test]
async fn each_change_sets_updated_at() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let id = library.create_playlist("Driving").await.unwrap().id;
    assert!(updated_at(&library, id).await > 0);
    let tracks = sample_tracks();

    reset_updated_at(&library, id).await;
    library.add_to_playlist(id, &tracks, None).await.unwrap();
    assert!(updated_at(&library, id).await > 0, "add");

    let entries = entry_ids(&library, id).await;
    reset_updated_at(&library, id).await;
    library
        .move_playlist_entry(id, entries[0], 2)
        .await
        .unwrap();
    assert!(updated_at(&library, id).await > 0, "move");

    reset_updated_at(&library, id).await;
    library
        .remove_from_playlist(id, &entries[..1])
        .await
        .unwrap();
    assert!(updated_at(&library, id).await > 0, "remove");

    reset_updated_at(&library, id).await;
    library.rename_playlist(id, "Road Trip").await.unwrap();
    assert!(updated_at(&library, id).await > 0, "rename");

    // Changes that change nothing leave it.
    reset_updated_at(&library, id).await;
    library.add_to_playlist(id, &[], None).await.unwrap();
    library.remove_from_playlist(id, &[]).await.unwrap();
    library
        .move_playlist_entry(id, entries[1], 0)
        .await
        .unwrap();
    assert_eq!(updated_at(&library, id).await, 0);
}

#[tokio::test]
async fn playlists_and_plays_survive_closing_and_reopening() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let tracks = sample_tracks();
    let driving = library.create_playlist("Driving").await.unwrap().id;
    library
        .add_to_playlist(driving, &[tracks[3], tracks[0], tracks[3]], None)
        .await
        .unwrap();
    let favorites = library.create_playlist("Favorites").await.unwrap().id;
    library
        .add_to_playlist(favorites, &tracks[1..2], None)
        .await
        .unwrap();
    let at = SystemTime::UNIX_EPOCH + Duration::new(1_800_000_000, 123_456_789);
    library.record_play(tracks[0], "laptop", at).await.unwrap();
    library
        .record_play(tracks[3], "phone", at + Duration::from_secs(1))
        .await
        .unwrap();

    let playlists = library.list_playlists().await.unwrap();
    let contents = [
        library.get_playlist(driving).await.unwrap(),
        library.get_playlist(favorites).await.unwrap(),
    ];
    let plays = library.recently_played(10).await.unwrap();
    library.close().await;

    let reopened = LocalLibrary::open(dir.path()).await.unwrap();
    assert_eq!(reopened.list_playlists().await.unwrap(), playlists);
    assert_eq!(reopened.get_playlist(driving).await.unwrap(), contents[0]);
    assert_eq!(reopened.get_playlist(favorites).await.unwrap(), contents[1]);
    assert_eq!(reopened.recently_played(10).await.unwrap(), plays);
    assert_eq!(plays.len(), 2);
}
