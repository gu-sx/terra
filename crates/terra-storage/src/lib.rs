//! The local library: an SQLite database of tracks, albums, artists,
//! playlists and plays, in a directory on this machine.
//!
//! [`LocalLibrary`] implements [`LibraryBackend`](terra_core::LibraryBackend).
//! Its lifecycle is explicit: [`LocalLibrary::create`] makes a library,
//! [`LocalLibrary::migrate`] brings an older one up to date, and
//! [`LocalLibrary::open`] opens one that is, without changing its schema.
//! [`LocalLibrary::add_root`] adds a folder of audio files, and
//! [`LocalLibrary::scan`] reads the folders into the library.
//!
//! `docs/design/library-schema.md` describes the schema.

mod backend;
mod error;
mod fts;
mod history;
mod library;
mod playlist;
mod scan;
mod time;
mod write;

#[cfg(test)]
mod tests;

pub use error::{Error, MissingFile};
pub use library::{DATABASE_FILE, LocalLibrary};
pub use scan::{ScanProgress, ScanReport};
