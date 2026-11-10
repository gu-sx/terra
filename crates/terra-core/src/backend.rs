use std::future::Future;
use std::time::SystemTime;

use crate::error::LibraryError;
use crate::id::{PlaylistEntryId, PlaylistId, TrackId};
use crate::model::{
    Album, AlbumFilter, Artist, Page, Play, Playlist, PlaylistContents, PlaylistEntry,
    SearchResults, Track, TrackFilter,
};
use crate::source::AudioSource;

/// The operations an interface performs on a library, whether the library is
/// local or on a remote server.
///
/// The trait covers browsing, playlists and play history. It does not own
/// playback: [`resolve_audio_source`](Self::resolve_audio_source) says where
/// the audio is, and the player on this machine plays it. Nor does it assume
/// how a remote backend talks to its server.
///
/// # `Send` futures
///
/// Every method is declared as `fn …(…) -> impl Future<Output = …> + Send`,
/// the desugared form of `async fn`, so that callers can drive the futures on
/// a multi-threaded executor without naming each method's future type.
/// Implementations write the methods as plain `async fn`, and the compiler
/// rejects one whose future is not `Send`. A bare `async fn` in the trait
/// could not promise this. The trait requires `Send + Sync` so that the
/// futures, which borrow `self`, can be `Send`.
///
/// # Common rules
///
/// - **Errors.** A method that names an object by its identifier returns
///   [`LibraryError::NotFound`] when the object does not exist. Arguments that
///   break the method's contract give [`LibraryError::InvalidInput`]. A
///   failure of the backend's storage or transport gives
///   [`LibraryError::Storage`]. A method that fails changes nothing.
/// - **Text order.** Where a list is ordered by a text field, the comparison
///   ignores case (Unicode lower-case), and records with equal text are
///   ordered by identifier, ascending.
/// - **Limits.** A `limit` of 0 returns an empty list.
/// - **Search.** A query is split into terms at every character that is not
///   alphanumeric. A record matches when every term is, ignoring case, a
///   prefix of a word in one of the record's searched fields, where the
///   fields are split into words the same way. A query without terms matches
///   nothing.
pub trait LibraryBackend: Send + Sync {
    /// Searches artists, albums and tracks.
    ///
    /// Artists are searched by name; albums by title and album artist; tracks
    /// by title, artist and album title. See the common rules for how a query
    /// matches. Each category returns at most `limit` records, in the order
    /// [`list_artists`](Self::list_artists),
    /// [`list_albums`](Self::list_albums) and
    /// [`list_tracks`](Self::list_tracks) give.
    ///
    /// A query without terms returns empty results, not an error.
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<SearchResults, LibraryError>> + Send;

    /// Lists every artist, ordered by sort name.
    fn list_artists(&self) -> impl Future<Output = Result<Vec<Artist>, LibraryError>> + Send;

    /// Lists the albums that match `filter`, ordered by title.
    ///
    /// A filter naming an artist that does not exist matches nothing; it is
    /// not an error.
    fn list_albums(
        &self,
        filter: AlbumFilter,
    ) -> impl Future<Output = Result<Vec<Album>, LibraryError>> + Send;

    /// Lists the tracks that match `filter`, then returns the `page` of them.
    ///
    /// Tracks are ordered by album title (tracks without an album last), then
    /// album identifier, disc number and track number (a missing number after
    /// every present one), title, and identifier. A filter naming an artist or
    /// album that does not exist matches nothing; it is not an error. A page
    /// past the end is empty.
    fn list_tracks(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> impl Future<Output = Result<Vec<Track>, LibraryError>> + Send;

    /// Returns one track.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the track does not exist.
    fn get_track(&self, id: TrackId) -> impl Future<Output = Result<Track, LibraryError>> + Send;

    /// Returns at most `limit` tracks, the most recently added first. Tracks
    /// added at the same time are ordered by identifier, highest first.
    fn recently_added(
        &self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Track>, LibraryError>> + Send;

    /// Lists every playlist, without entries, ordered by name.
    fn list_playlists(&self) -> impl Future<Output = Result<Vec<Playlist>, LibraryError>> + Send;

    /// Returns a playlist and its entries, ordered by position.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist does not exist.
    fn get_playlist(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<PlaylistContents, LibraryError>> + Send;

    /// Creates an empty playlist and returns it.
    ///
    /// The name is stored without leading or trailing whitespace. Playlist
    /// names are unique ignoring case.
    ///
    /// # Errors
    ///
    /// [`LibraryError::InvalidInput`] if the name is empty or only whitespace,
    /// or if another playlist has the same name ignoring case.
    fn create_playlist(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Playlist, LibraryError>> + Send;

    /// Renames a playlist and returns it.
    ///
    /// The name follows the rules of
    /// [`create_playlist`](Self::create_playlist). A playlist may be renamed
    /// to its own name in a different case.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist does not exist, and
    /// [`LibraryError::InvalidInput`] if the name is empty or only
    /// whitespace, or if another playlist has the same name ignoring case.
    fn rename_playlist(
        &self,
        id: PlaylistId,
        name: &str,
    ) -> impl Future<Output = Result<Playlist, LibraryError>> + Send;

    /// Deletes a playlist and its entries. The tracks stay in the library.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist does not exist.
    fn delete_playlist(
        &self,
        id: PlaylistId,
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;

    /// Adds `tracks` to a playlist, in the order given, and returns the new
    /// entries.
    ///
    /// With `at` set to `None` the tracks go at the end. With `Some(p)` the
    /// first new entry takes position `p` and the entries from `p` on move
    /// down; `p` may equal the number of entries, which adds at the end. A
    /// track may appear in a playlist any number of times, and each addition
    /// gets a new entry identifier. An empty `tracks` changes nothing.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist or any of the tracks does
    /// not exist, and [`LibraryError::InvalidInput`] if `at` is greater than
    /// the number of entries.
    fn add_to_playlist(
        &self,
        id: PlaylistId,
        tracks: &[TrackId],
        at: Option<usize>,
    ) -> impl Future<Output = Result<Vec<PlaylistEntry>, LibraryError>> + Send;

    /// Removes entries from a playlist. The remaining entries keep their
    /// order and close the gaps. Naming an entry twice removes it once.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist does not exist, or if any
    /// of the entries is not in it.
    fn remove_from_playlist(
        &self,
        id: PlaylistId,
        entries: &[PlaylistEntryId],
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;

    /// Moves an entry to position `to`. The entries between its old and new
    /// position shift by one to make room; the others keep their positions.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the playlist does not exist or the entry
    /// is not in it, and [`LibraryError::InvalidInput`] if `to` is not less
    /// than the number of entries.
    fn move_playlist_entry(
        &self,
        id: PlaylistId,
        entry: PlaylistEntryId,
        to: usize,
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;

    /// Says where to read a track's audio from. The caller plays it.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the track does not exist.
    fn resolve_audio_source(
        &self,
        track: TrackId,
    ) -> impl Future<Output = Result<AudioSource, LibraryError>> + Send;

    /// Records that `device` played a track at `at`.
    ///
    /// # Errors
    ///
    /// [`LibraryError::NotFound`] if the track does not exist, and
    /// [`LibraryError::InvalidInput`] if `device` is empty or only
    /// whitespace.
    fn record_play(
        &self,
        track: TrackId,
        device: &str,
        at: SystemTime,
    ) -> impl Future<Output = Result<(), LibraryError>> + Send;

    /// Returns at most `limit` plays, the latest first. Plays at the same
    /// time are ordered with the last recorded first.
    fn recently_played(
        &self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Play>, LibraryError>> + Send;
}
