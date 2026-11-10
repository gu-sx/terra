//! Play history as the database stores it.

use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use terra_core::testing::sample;
use terra_core::{LibraryBackend, LibraryError, TrackId};

use super::sample_library;
use crate::write::delete_track;

fn track(index: usize) -> TrackId {
    sample::library().tracks[index].0.id
}

fn at(seconds: u64, nanos: u32) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(1_800_000_000 + seconds, nanos)
}

#[tokio::test]
async fn recently_played_orders_plays_and_keeps_each_device() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let plays = [
        (track(0), "Laptop", at(10, 0)),
        (track(1), "phone", at(20, 1)),
        // The same instant as the play before, to the nanosecond.
        (track(2), "Living Room Speaker", at(20, 1)),
        (track(0), "phone", at(30, 0)),
        // Recorded last but played first.
        (track(3), "desktop", at(5, 999_999_999)),
    ];
    for (id, device, time) in plays {
        library.record_play(id, device, time).await.unwrap();
    }

    let played = library.recently_played(10).await.unwrap();
    let summary: Vec<(TrackId, &str, SystemTime)> = played
        .iter()
        .map(|play| (play.track.id, play.device.as_str(), play.at))
        .collect();
    assert_eq!(
        summary,
        [
            (track(0), "phone", at(30, 0)),
            (track(2), "Living Room Speaker", at(20, 1)),
            (track(1), "phone", at(20, 1)),
            (track(0), "Laptop", at(10, 0)),
            (track(3), "desktop", at(5, 999_999_999)),
        ],
        "every play is listed, a track once per play"
    );
    assert_eq!(played[0].track, library.get_track(track(0)).await.unwrap());
    assert_eq!(library.recently_played(3).await.unwrap(), played[..3]);
}

#[tokio::test]
async fn record_play_rejects_a_time_it_cannot_store() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let far = SystemTime::UNIX_EPOCH + Duration::from_secs(300 * 365 * 24 * 60 * 60);
    match library.record_play(track(0), "laptop", far).await {
        Err(LibraryError::InvalidInput(_)) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert!(library.recently_played(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_a_track_deletes_its_plays() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    library
        .record_play(track(0), "laptop", at(10, 0))
        .await
        .unwrap();
    library
        .record_play(track(1), "laptop", at(20, 0))
        .await
        .unwrap();

    let mut conn = library.pool.acquire().await.unwrap();
    assert!(delete_track(&mut conn, track(1)).await.unwrap());
    drop(conn);
    let played = library.recently_played(10).await.unwrap();
    assert_eq!(played.len(), 1);
    assert_eq!(played[0].track.id, track(0));
}
