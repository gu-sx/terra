//! The library the command line and the TUI work on.

use std::time::SystemTime;

use terra_core::{
    Album, AlbumFilter, Artist, AudioSource, LibraryBackend, LibraryError, Page, Play, Playlist,
    PlaylistContents, PlaylistEntry, PlaylistEntryId, PlaylistId, SearchResults, Track,
    TrackFilter, TrackId,
};
use terra_storage::LocalLibrary;

/// A library backend chosen at run time. Phase 1 has only the local
/// library; a remote one joins it in Phase 3.
#[derive(Debug, Clone)]
pub enum Backend {
    /// A library on this machine.
    Local(LocalLibrary),
}

impl Backend {
    /// Closes the library's connections.
    pub async fn close(self) {
        match self {
            Self::Local(library) => library.close().await,
        }
    }
}

impl LibraryBackend for Backend {
    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults, LibraryError> {
        match self {
            Self::Local(library) => library.search(query, limit).await,
        }
    }

    async fn list_artists(&self) -> Result<Vec<Artist>, LibraryError> {
        match self {
            Self::Local(library) => library.list_artists().await,
        }
    }

    async fn list_albums(&self, filter: AlbumFilter) -> Result<Vec<Album>, LibraryError> {
        match self {
            Self::Local(library) => library.list_albums(filter).await,
        }
    }

    async fn list_tracks(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> Result<Vec<Track>, LibraryError> {
        match self {
            Self::Local(library) => library.list_tracks(filter, page).await,
        }
    }

    async fn get_track(&self, id: TrackId) -> Result<Track, LibraryError> {
        match self {
            Self::Local(library) => library.get_track(id).await,
        }
    }

    async fn recently_added(&self, limit: usize) -> Result<Vec<Track>, LibraryError> {
        match self {
            Self::Local(library) => library.recently_added(limit).await,
        }
    }

    async fn list_playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        match self {
            Self::Local(library) => library.list_playlists().await,
        }
    }

    async fn get_playlist(&self, id: PlaylistId) -> Result<PlaylistContents, LibraryError> {
        match self {
            Self::Local(library) => library.get_playlist(id).await,
        }
    }

    async fn create_playlist(&self, name: &str) -> Result<Playlist, LibraryError> {
        match self {
            Self::Local(library) => library.create_playlist(name).await,
        }
    }

    async fn rename_playlist(&self, id: PlaylistId, name: &str) -> Result<Playlist, LibraryError> {
        match self {
            Self::Local(library) => library.rename_playlist(id, name).await,
        }
    }

    async fn delete_playlist(&self, id: PlaylistId) -> Result<(), LibraryError> {
        match self {
            Self::Local(library) => library.delete_playlist(id).await,
        }
    }

    async fn add_to_playlist(
        &self,
        id: PlaylistId,
        tracks: &[TrackId],
        at: Option<usize>,
    ) -> Result<Vec<PlaylistEntry>, LibraryError> {
        match self {
            Self::Local(library) => library.add_to_playlist(id, tracks, at).await,
        }
    }

    async fn remove_from_playlist(
        &self,
        id: PlaylistId,
        entries: &[PlaylistEntryId],
    ) -> Result<(), LibraryError> {
        match self {
            Self::Local(library) => library.remove_from_playlist(id, entries).await,
        }
    }

    async fn move_playlist_entry(
        &self,
        id: PlaylistId,
        entry: PlaylistEntryId,
        to: usize,
    ) -> Result<(), LibraryError> {
        match self {
            Self::Local(library) => library.move_playlist_entry(id, entry, to).await,
        }
    }

    async fn resolve_audio_source(&self, track: TrackId) -> Result<AudioSource, LibraryError> {
        match self {
            Self::Local(library) => library.resolve_audio_source(track).await,
        }
    }

    async fn record_play(
        &self,
        track: TrackId,
        device: &str,
        at: SystemTime,
    ) -> Result<(), LibraryError> {
        match self {
            Self::Local(library) => library.record_play(track, device, at).await,
        }
    }

    async fn recently_played(&self, limit: usize) -> Result<Vec<Play>, LibraryError> {
        match self {
            Self::Local(library) => library.recently_played(limit).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn local_delegates_to_the_library() {
        let dir = TempDir::new().unwrap();
        let library = LocalLibrary::create(dir.path()).await.unwrap();
        let backend = Backend::Local(library.clone());

        let playlist = backend.create_playlist("Mix A").await.unwrap();
        assert_eq!(
            library.list_playlists().await.unwrap(),
            vec![playlist.clone()]
        );
        assert_eq!(backend.list_playlists().await.unwrap(), vec![playlist]);
        assert!(backend.list_artists().await.unwrap().is_empty());
        backend.close().await;
    }
}
