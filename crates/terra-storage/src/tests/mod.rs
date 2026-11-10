//! The crate's tests, and the sample library they seed.

mod contract;
mod history;
mod lifecycle;
mod playlists;
mod read;
mod records;
pub(crate) mod scan;
mod search;

use std::path::Path;
use std::time::SystemTime;

use terra_core::testing::sample::{self, SampleLibrary};
use terra_core::{AudioSource, Track};

use crate::LocalLibrary;
use crate::write::{
    NewAlbum, NewFile, NewTrack, RootId, ensure_album, ensure_artist, insert_root, insert_track,
};

/// The directory every sample track's file is in.
pub(crate) const SAMPLE_ROOT: &str = "/music";

/// Creates a library in `dir` holding the [sample library](sample).
pub(crate) async fn sample_library(dir: &Path) -> LocalLibrary {
    let library = LocalLibrary::create(dir).await.unwrap();
    seed(&library, &sample::library()).await;
    library
}

/// Adds the sample's records, in order, to an empty library, so that each
/// gets the identifier it has in the sample.
pub(crate) async fn seed(library: &LocalLibrary, sample: &SampleLibrary) {
    let mut tx = library.pool.begin().await.unwrap();
    let root = insert_root(&mut tx, Path::new(SAMPLE_ROOT), SystemTime::UNIX_EPOCH)
        .await
        .unwrap();
    for artist in &sample.artists {
        let id = ensure_artist(&mut tx, &artist.name, &artist.sort_name)
            .await
            .unwrap();
        assert_eq!(id, artist.id);
    }
    for album in &sample.albums {
        let new = NewAlbum {
            title: &album.title,
            album_artist: Some(album.album_artist.id),
            year: album.year,
            kind: album.kind,
        };
        assert_eq!(ensure_album(&mut tx, &new).await.unwrap(), album.id);
    }
    for (track, source) in &sample.tracks {
        let AudioSource::LocalPath(path) = source else {
            panic!("a sample track's source is not a path");
        };
        let rel_path = path.strip_prefix(SAMPLE_ROOT).unwrap().to_str().unwrap();
        let id = insert_track(&mut tx, &new_track(track, root, rel_path))
            .await
            .unwrap();
        assert_eq!(id, track.id);
    }
    tx.commit().await.unwrap();
}

/// A [`NewTrack`] with the fields of `track`, and a file at `rel_path` with
/// synthetic size, time and hash.
pub(crate) fn new_track<'a>(track: &'a Track, root: RootId, rel_path: &'a str) -> NewTrack<'a> {
    NewTrack {
        title: &track.title,
        artist: track.artist.id,
        album: track.album.as_ref().map(|album| album.id),
        disc_no: track.disc_no,
        track_no: track.track_no,
        duration: track.duration,
        added_at: track.added_at,
        file: NewFile {
            root,
            rel_path,
            size: 1_000,
            mtime: track.added_at,
            content_hash: track.title.as_bytes(),
            format: &track.format,
            has_embedded_art: false,
        },
    }
}
