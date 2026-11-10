use std::error::Error;
use std::fmt;

use crate::id::{AlbumId, ArtistId, PlaylistEntryId, PlaylistId, TrackId};

/// A library object, named by its identifier, as reported by
/// [`LibraryError::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    Track(TrackId),
    Album(AlbumId),
    Artist(ArtistId),
    Playlist(PlaylistId),
    PlaylistEntry(PlaylistEntryId),
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Track(id) => write!(f, "track {id}"),
            Self::Album(id) => write!(f, "album {id}"),
            Self::Artist(id) => write!(f, "artist {id}"),
            Self::Playlist(id) => write!(f, "playlist {id}"),
            Self::PlaylistEntry(id) => write!(f, "playlist entry {id}"),
        }
    }
}

/// The error every [`LibraryBackend`](crate::LibraryBackend) method returns.
#[derive(Debug)]
#[non_exhaustive]
pub enum LibraryError {
    /// The object does not exist in this library.
    NotFound(Resource),
    /// The arguments break the method's contract, for example a duplicate
    /// playlist name or a position past the end of a playlist. The message
    /// says which.
    InvalidInput(String),
    /// This backend cannot perform the operation. The message says which.
    Unsupported(String),
    /// The backend's storage or transport failed.
    Storage(Box<dyn Error + Send + Sync + 'static>),
}

impl LibraryError {
    /// Wraps a storage or transport error.
    pub fn storage(source: impl Into<Box<dyn Error + Send + Sync + 'static>>) -> Self {
        Self::Storage(source.into())
    }
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(resource) => write!(f, "{resource} not found"),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Storage(_) => f.write_str("library storage failed"),
        }
    }
}

impl Error for LibraryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_resource() {
        let err = LibraryError::NotFound(Resource::PlaylistEntry(PlaylistEntryId::new(3)));
        assert_eq!(err.to_string(), "playlist entry 3 not found");
    }

    #[test]
    fn storage_keeps_its_source() {
        let err = LibraryError::storage("disk on fire");
        assert_eq!(err.to_string(), "library storage failed");
        assert_eq!(err.source().unwrap().to_string(), "disk on fire");
    }
}
