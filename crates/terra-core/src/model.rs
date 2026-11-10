//! The library's records, as the interfaces see them.

use std::time::{Duration, SystemTime};

use crate::id::{AlbumId, ArtistId, PlaylistEntryId, PlaylistId, TrackId};

/// An artist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
    /// The name to sort by, such as "Artist, The" for "The Artist".
    pub sort_name: String,
}

/// An artist as another record refers to it: enough to show and to follow.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtistRef {
    pub id: ArtistId,
    pub name: String,
}

/// What kind of release an album is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlbumKind {
    /// A regular album by one album artist.
    Album,
    /// Tracks by several artists, such as a various-artists release.
    Compilation,
    /// A long recording, such as a DJ set, and the tracks split from it.
    DjSet,
}

/// An album.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Album {
    pub id: AlbumId,
    pub title: String,
    pub album_artist: ArtistRef,
    pub year: Option<i32>,
    pub kind: AlbumKind,
}

/// An album as another record refers to it: enough to show and to follow.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlbumRef {
    pub id: AlbumId,
    pub title: String,
}

/// How a track's audio is encoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    /// The codec's short lower-case name, such as `flac`, `mp3` or `opus`.
    pub codec: String,
    pub lossless: bool,
    /// Samples per second, in hertz.
    pub sample_rate: u32,
    pub channels: u16,
    /// The average bitrate in bits per second, when known.
    pub bitrate: Option<u32>,
}

/// A track in the library.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    /// The track's own artist, which on a compilation differs from the album
    /// artist.
    pub artist: ArtistRef,
    /// The album, if the track belongs to one.
    pub album: Option<AlbumRef>,
    pub disc_no: Option<u32>,
    pub track_no: Option<u32>,
    pub duration: Duration,
    pub format: AudioFormat,
    /// When the track entered the library.
    pub added_at: SystemTime,
}

/// A playlist, without its entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub entry_count: usize,
    /// The sum of the entries' durations.
    pub duration: Duration,
}

/// One entry of a playlist.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlaylistEntry {
    pub id: PlaylistEntryId,
    /// The entry's place in the playlist, counted from 0. A playlist's
    /// positions are always 0, 1, 2 and so on, without gaps.
    pub position: usize,
    pub track: Track,
}

/// A playlist and its entries, ordered by position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlaylistContents {
    pub playlist: Playlist,
    pub entries: Vec<PlaylistEntry>,
}

/// One recorded play of a track.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Play {
    pub track: Track,
    /// The device that played it, as the client named it.
    pub device: String,
    pub at: SystemTime,
}

/// What a search found, in each category.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

/// Which albums [`list_albums`](crate::LibraryBackend::list_albums) returns.
/// Every field that is set must match; the default matches every album.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AlbumFilter {
    /// Only albums with this album artist.
    pub album_artist: Option<ArtistId>,
    /// Only albums of this kind.
    pub kind: Option<AlbumKind>,
}

/// Which tracks [`list_tracks`](crate::LibraryBackend::list_tracks) returns.
/// Every field that is set must match; the default matches every track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TrackFilter {
    /// Only tracks by this artist (the track's own artist, not the album
    /// artist).
    pub artist: Option<ArtistId>,
    /// Only tracks on this album.
    pub album: Option<AlbumId>,
}

/// A window into an ordered list: skip `offset` items, then return at most
/// `limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Page {
    pub offset: usize,
    pub limit: usize,
}

impl Page {
    /// Every item.
    pub const ALL: Self = Self {
        offset: 0,
        limit: usize::MAX,
    };

    /// The first `limit` items.
    pub const fn first(limit: usize) -> Self {
        Self { offset: 0, limit }
    }
}
