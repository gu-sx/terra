//! Runs the backend contract suite against the in-memory backend.

use std::path::Path;

use terra_core::testing::{MemoryLibrary, block_on};
use terra_core::{AudioSource, LibraryBackend, Page, TrackFilter};

mod contract {
    terra_core::contract_tests! {
        run: terra_core::testing::block_on,
        backend: terra_core::testing::MemoryLibrary::sample(),
    }
}

/// The trait promises `Send` futures; this fails to compile if
/// `MemoryLibrary`'s are not.
#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(_: T) {}
    let library = MemoryLibrary::sample();
    assert_send(library.search("mid", 1));
    assert_send(library.create_playlist("Driving"));
    assert_send(library.add_to_playlist(1.into(), &[], None));
}

#[test]
fn sample_tracks_resolve_to_their_files() {
    block_on(async {
        let library = MemoryLibrary::sample();
        let tracks = library
            .list_tracks(TrackFilter::default(), Page::ALL)
            .await
            .unwrap();
        let sunrise = tracks.iter().find(|t| t.title == "Sunrise").unwrap();
        assert_eq!(
            library.resolve_audio_source(sunrise.id).await.unwrap(),
            AudioSource::LocalPath(Path::new("/music/artist-d/sunrise.opus").to_path_buf()),
        );
    });
}

#[test]
fn an_empty_library_has_nothing() {
    block_on(async {
        let library = MemoryLibrary::new();
        assert!(library.list_artists().await.unwrap().is_empty());
        assert!(library.recently_added(10).await.unwrap().is_empty());
        assert_eq!(
            library.search("artist", 10).await.unwrap(),
            Default::default()
        );
    });
}
