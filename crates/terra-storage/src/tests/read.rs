//! Listing, getting and resolving the sample library's records.

use std::error::Error as _;
use std::path::PathBuf;
use std::time::SystemTime;

use tempfile::TempDir;
use terra_core::testing::sample;
use terra_core::{
    AlbumFilter, AudioSource, LibraryBackend, LibraryError, Page, PlaylistId, TrackFilter,
};

use super::sample_library;
use crate::MissingFile;
use crate::time::to_nanos;

#[tokio::test]
async fn lists_return_the_sample_records() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let sample = sample::library();

    let mut artists = library.list_artists().await.unwrap();
    artists.sort_by_key(|artist| artist.id);
    assert_eq!(artists, sample.artists);

    let mut albums = library.list_albums(AlbumFilter::default()).await.unwrap();
    albums.sort_by_key(|album| album.id);
    assert_eq!(albums, sample.albums);

    let mut tracks = library
        .list_tracks(TrackFilter::default(), Page::ALL)
        .await
        .unwrap();
    tracks.sort_by_key(|track| track.id);
    let expected: Vec<_> = sample
        .tracks
        .iter()
        .map(|(track, _)| track.clone())
        .collect();
    assert_eq!(tracks, expected);
}

#[tokio::test]
async fn get_track_returns_every_field() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    for (track, _) in sample::library().tracks {
        assert_eq!(library.get_track(track.id).await.unwrap(), track);
    }
}

#[tokio::test]
async fn library_survives_closing_and_reopening() {
    let dir = TempDir::new().unwrap();
    sample_library(dir.path()).await.close().await;

    let library = crate::LocalLibrary::open(dir.path()).await.unwrap();
    let (track, _) = &sample::library().tracks[0];
    assert_eq!(&library.get_track(track.id).await.unwrap(), track);
}

#[tokio::test]
async fn resolve_audio_source_joins_the_root_and_the_path() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    for (track, source) in sample::library().tracks {
        assert_eq!(
            library.resolve_audio_source(track.id).await.unwrap(),
            source
        );
    }
}

#[tokio::test]
async fn resolve_audio_source_of_a_missing_file_fails() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let (track, source) = &sample::library().tracks[0];
    sqlx::query("UPDATE media_file SET missing_since = ?1 WHERE track_id = ?2")
        .bind(to_nanos(SystemTime::now()).unwrap())
        .bind(track.id.get())
        .execute(&library.pool)
        .await
        .unwrap();

    let err = library.resolve_audio_source(track.id).await.unwrap_err();
    let LibraryError::Storage(_) = &err else {
        panic!("expected a storage error, got {err:?}");
    };
    let AudioSource::LocalPath(path) = source else {
        unreachable!("sample sources are paths");
    };
    let missing = err.source().unwrap().downcast_ref::<MissingFile>().unwrap();
    assert_eq!(
        missing,
        &MissingFile {
            track: track.id,
            path: PathBuf::from(path),
        }
    );
    // The track itself is still there.
    assert_eq!(&library.get_track(track.id).await.unwrap(), track);

    // The file reappears.
    sqlx::query("UPDATE media_file SET missing_since = NULL WHERE track_id = ?1")
        .bind(track.id.get())
        .execute(&library.pool)
        .await
        .unwrap();
    assert_eq!(
        &library.resolve_audio_source(track.id).await.unwrap(),
        source
    );
}

#[tokio::test]
async fn playlists_and_history_are_unsupported_until_issue_9() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    for result in [
        library.list_playlists().await.map(drop),
        library.get_playlist(PlaylistId::new(1)).await.map(drop),
        library.create_playlist("Mix").await.map(drop),
        library.recently_played(10).await.map(drop),
    ] {
        match result {
            Err(LibraryError::Unsupported(message)) => assert!(message.contains("#9"), "{message}"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}
