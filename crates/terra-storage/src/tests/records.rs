//! Identifiers and uniqueness of the stored records.

use std::path::Path;
use std::time::SystemTime;

use tempfile::TempDir;
use terra_core::testing::sample;
use terra_core::{AlbumFilter, AlbumKind, LibraryBackend, LibraryError, Resource};

use super::{new_track, sample_library};
use crate::LocalLibrary;
use crate::write::{
    NewAlbum, delete_track, ensure_album, ensure_artist, insert_root, insert_track,
};

#[tokio::test]
async fn a_deleted_track_id_is_not_reused() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    let sample = sample::library();
    let (last, _) = sample.tracks.last().unwrap();

    let mut conn = library.pool.acquire().await.unwrap();
    assert!(delete_track(&mut conn, last.id).await.unwrap());
    let root = crate::write::RootId(1);
    let again = insert_track(&mut conn, &new_track(last, root, "again.flac"))
        .await
        .unwrap();
    drop(conn);

    assert!(again > last.id, "{again} reuses or precedes {}", last.id);
    let result = library.get_track(last.id).await;
    assert!(
        matches!(result, Err(LibraryError::NotFound(Resource::Track(id))) if id == last.id),
        "{result:?}"
    );
    // The search row went with the track and came back with the new one.
    let found = library.search("demo tape", 10).await.unwrap().tracks;
    let ids: Vec<_> = found.iter().map(|track| track.id).collect();
    assert_eq!(ids, [again]);
}

#[tokio::test]
async fn albums_without_an_album_artist_are_not_duplicated() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    let mut conn = library.pool.acquire().await.unwrap();
    let album = |title| NewAlbum {
        title,
        album_artist: None,
        year: None,
        kind: AlbumKind::Compilation,
    };

    let first = ensure_album(&mut conn, &album("Night Mix")).await.unwrap();
    let again = ensure_album(&mut conn, &album(" night MIX "))
        .await
        .unwrap();
    assert_eq!(first, again);

    // The unique index holds even for an insert that bypasses the helper.
    let duplicate = sqlx::query(
        "INSERT INTO album (title, title_key, album_artist_id, year, kind)
         VALUES ('Night Mix', 'night mix', NULL, NULL, 'compilation')",
    )
    .execute(&mut *conn)
    .await;
    let err = duplicate.expect_err("a second album without an album artist was inserted");
    assert!(
        err.as_database_error()
            .is_some_and(|err| err.is_unique_violation()),
        "{err:?}"
    );

    // Albums with the same title but an album artist are different albums.
    let artist = ensure_artist(&mut conn, "Artist A", "Artist A")
        .await
        .unwrap();
    let with_artist = NewAlbum {
        album_artist: Some(artist),
        ..album("Night Mix")
    };
    let other = ensure_album(&mut conn, &with_artist).await.unwrap();
    assert_ne!(other, first);
    assert_eq!(ensure_album(&mut conn, &with_artist).await.unwrap(), other);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM album")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn an_album_without_an_album_artist_shows_its_first_tracks_artist() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    let sample = sample::library();
    let mut tx = library.pool.begin().await.unwrap();
    let root = insert_root(&mut tx, Path::new("/music"), SystemTime::UNIX_EPOCH)
        .await
        .unwrap();
    for artist in &sample.artists {
        ensure_artist(&mut tx, &artist.name, &artist.sort_name)
            .await
            .unwrap();
    }
    let untitled = NewAlbum {
        title: "Untitled",
        album_artist: None,
        year: None,
        kind: AlbumKind::Compilation,
    };
    let album = ensure_album(&mut tx, &untitled).await.unwrap();
    let empty = NewAlbum {
        title: "Empty",
        ..untitled
    };
    ensure_album(&mut tx, &empty).await.unwrap();
    // "Into the Deep" (Artist C) is track 2 and "After Hours" (Artist B)
    // track 1, so Artist B is the album's artist although inserted second.
    for (index, number) in [(2, 2), (1, 1)] {
        let (track, _) = &sample.tracks[index];
        let mut new = new_track(track, root, &track.title);
        new.album = Some(album);
        new.track_no = Some(number);
        insert_track(&mut tx, &new).await.unwrap();
    }
    tx.commit().await.unwrap();

    let albums = library.list_albums(AlbumFilter::default()).await.unwrap();
    assert_eq!(albums.len(), 1, "the album without tracks is listed");
    assert_eq!(albums[0].id, album);
    assert_eq!(albums[0].album_artist.name, "Artist B");
    // A filter matches the stored album artist only.
    let by_artist = AlbumFilter {
        album_artist: Some(albums[0].album_artist.id),
        kind: None,
    };
    assert!(library.list_albums(by_artist).await.unwrap().is_empty());
}
