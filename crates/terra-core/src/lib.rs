//! Domain types and the library backend trait.
//!
//! The terminal UI and the command line talk to a library through
//! [`LibraryBackend`], which has a local implementation (SQLite and files) and
//! a remote one (a homelab server). This crate holds the types they share and
//! the trait itself. It does no I/O and depends on no async runtime, so it
//! knows nothing of the terminal, the database or HTTP.

mod backend;
mod error;
mod id;
mod model;
mod source;

pub use backend::LibraryBackend;
pub use error::{LibraryError, Resource};
pub use id::{AlbumId, ArtistId, ParseIdError, PlaylistEntryId, PlaylistId, TrackId};
pub use model::{
    Album, AlbumFilter, AlbumKind, AlbumRef, Artist, ArtistRef, AudioFormat, Page, Play, Playlist,
    PlaylistContents, PlaylistEntry, SearchResults, Track, TrackFilter,
};
pub use source::AudioSource;
