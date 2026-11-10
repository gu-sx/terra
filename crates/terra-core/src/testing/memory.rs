use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use super::sample::{self, SampleLibrary};
use crate::{
    Album, AlbumFilter, AlbumId, Artist, ArtistId, AudioSource, LibraryBackend, LibraryError, Page,
    Play, Playlist, PlaylistContents, PlaylistEntry, PlaylistEntryId, PlaylistId, Resource,
    SearchResults, Track, TrackFilter, TrackId,
};

/// A [`LibraryBackend`] that keeps everything in memory.
///
/// It is the reference implementation of the trait's contract and a stand-in
/// backend for tests of the interfaces. Nothing it holds outlives it.
#[derive(Debug, Default)]
pub struct MemoryLibrary {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    artists: BTreeMap<ArtistId, Artist>,
    albums: BTreeMap<AlbumId, Album>,
    tracks: BTreeMap<TrackId, (Track, AudioSource)>,
    playlists: BTreeMap<PlaylistId, StoredPlaylist>,
    plays: Vec<(TrackId, String, SystemTime)>,
    last_playlist_id: i64,
    last_entry_id: i64,
}

#[derive(Debug)]
struct StoredPlaylist {
    name: String,
    entries: Vec<(PlaylistEntryId, TrackId)>,
}

impl MemoryLibrary {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// A library holding the [sample library](super::sample), with no
    /// playlists and no plays.
    pub fn sample() -> Self {
        let SampleLibrary {
            artists,
            albums,
            tracks,
        } = sample::library();
        let mut library = Self::new();
        for artist in artists {
            library.insert_artist(artist);
        }
        for album in albums {
            library.insert_album(album);
        }
        for (track, source) in tracks {
            library.insert_track(track, source);
        }
        library
    }

    /// Adds an artist, or replaces the one with the same identifier.
    pub fn insert_artist(&mut self, artist: Artist) {
        self.state_mut().artists.insert(artist.id, artist);
    }

    /// Adds an album, or replaces the one with the same identifier.
    pub fn insert_album(&mut self, album: Album) {
        self.state_mut().albums.insert(album.id, album);
    }

    /// Adds a track with the source its audio resolves to, or replaces the
    /// one with the same identifier.
    pub fn insert_track(&mut self, track: Track, source: AudioSource) {
        self.state_mut().tracks.insert(track.id, (track, source));
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("a MemoryLibrary method panicked")
    }

    fn state_mut(&mut self) -> &mut State {
        self.state
            .get_mut()
            .expect("a MemoryLibrary method panicked")
    }
}

impl State {
    fn track(&self, id: TrackId) -> Result<&Track, LibraryError> {
        self.tracks
            .get(&id)
            .map(|(track, _)| track)
            .ok_or(LibraryError::NotFound(Resource::Track(id)))
    }

    fn playlist(&self, id: PlaylistId) -> Result<&StoredPlaylist, LibraryError> {
        self.playlists
            .get(&id)
            .ok_or(LibraryError::NotFound(Resource::Playlist(id)))
    }

    fn playlist_mut(&mut self, id: PlaylistId) -> Result<&mut StoredPlaylist, LibraryError> {
        self.playlists
            .get_mut(&id)
            .ok_or(LibraryError::NotFound(Resource::Playlist(id)))
    }

    fn summary(&self, id: PlaylistId, playlist: &StoredPlaylist) -> Playlist {
        Playlist {
            id,
            name: playlist.name.clone(),
            entry_count: playlist.entries.len(),
            duration: playlist
                .entries
                .iter()
                .filter_map(|(_, track)| self.track(*track).ok())
                .map(|track| track.duration)
                .sum::<Duration>(),
        }
    }

    fn entries(&self, playlist: &StoredPlaylist) -> Vec<PlaylistEntry> {
        playlist
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, (id, track))| {
                Some(PlaylistEntry {
                    id: *id,
                    position,
                    track: self.track(*track).ok()?.clone(),
                })
            })
            .collect()
    }

    /// Checks a playlist name and returns it trimmed.
    fn playlist_name<'a>(
        &self,
        name: &'a str,
        renaming: Option<PlaylistId>,
    ) -> Result<&'a str, LibraryError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LibraryError::InvalidInput(
                "a playlist name cannot be empty".to_owned(),
            ));
        }
        let folded = name.to_lowercase();
        let taken = self
            .playlists
            .iter()
            .any(|(id, other)| Some(*id) != renaming && other.name.to_lowercase() == folded);
        if taken {
            return Err(LibraryError::InvalidInput(format!(
                "a playlist named {name:?} already exists"
            )));
        }
        Ok(name)
    }

    fn sorted_albums(&self, filter: AlbumFilter) -> Vec<Album> {
        let mut albums: Vec<Album> = self
            .albums
            .values()
            .filter(|album| {
                filter
                    .album_artist
                    .is_none_or(|artist| album.album_artist.id == artist)
                    && filter.kind.is_none_or(|kind| album.kind == kind)
            })
            .cloned()
            .collect();
        albums.sort_by_cached_key(|album| (album.title.to_lowercase(), album.id));
        albums
    }

    fn sorted_tracks(&self, filter: TrackFilter) -> Vec<Track> {
        let mut tracks: Vec<Track> = self
            .tracks
            .values()
            .map(|(track, _)| track)
            .filter(|track| {
                filter.artist.is_none_or(|artist| track.artist.id == artist)
                    && filter
                        .album
                        .is_none_or(|album| track.album.as_ref().is_some_and(|a| a.id == album))
            })
            .cloned()
            .collect();
        tracks.sort_by_cached_key(|track| {
            (
                track
                    .album
                    .as_ref()
                    .map(|album| (album.title.to_lowercase(), album.id))
                    .ok_or(()),
                track.disc_no.ok_or(()),
                track.track_no.ok_or(()),
                track.title.to_lowercase(),
                track.id,
            )
        });
        tracks
    }

    fn sorted_artists(&self) -> Vec<Artist> {
        let mut artists: Vec<Artist> = self.artists.values().cloned().collect();
        artists.sort_by_cached_key(|artist| (artist.sort_name.to_lowercase(), artist.id));
        artists
    }
}

/// Splits text into lower-case words at every character that is not
/// alphanumeric.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Whether every term is a prefix of a word in one of the fields.
fn matches(terms: &[String], fields: &[&str]) -> bool {
    let words: Vec<String> = fields.iter().flat_map(|field| words(field)).collect();
    terms
        .iter()
        .all(|term| words.iter().any(|word| word.starts_with(term.as_str())))
}

fn invalid_position(what: &str, position: usize, len: usize) -> LibraryError {
    LibraryError::InvalidInput(format!(
        "{what} {position} is out of range for a playlist of {len} entries"
    ))
}

impl LibraryBackend for MemoryLibrary {
    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults, LibraryError> {
        let terms: Vec<String> = words(query).collect();
        if terms.is_empty() {
            return Ok(SearchResults::default());
        }
        let state = self.state();
        Ok(SearchResults {
            artists: state
                .sorted_artists()
                .into_iter()
                .filter(|artist| matches(&terms, &[&artist.name]))
                .take(limit)
                .collect(),
            albums: state
                .sorted_albums(AlbumFilter::default())
                .into_iter()
                .filter(|album| matches(&terms, &[&album.title, &album.album_artist.name]))
                .take(limit)
                .collect(),
            tracks: state
                .sorted_tracks(TrackFilter::default())
                .into_iter()
                .filter(|track| {
                    let album = track.album.as_ref().map_or("", |album| &album.title);
                    matches(&terms, &[&track.title, &track.artist.name, album])
                })
                .take(limit)
                .collect(),
        })
    }

    async fn list_artists(&self) -> Result<Vec<Artist>, LibraryError> {
        Ok(self.state().sorted_artists())
    }

    async fn list_albums(&self, filter: AlbumFilter) -> Result<Vec<Album>, LibraryError> {
        Ok(self.state().sorted_albums(filter))
    }

    async fn list_tracks(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> Result<Vec<Track>, LibraryError> {
        Ok(self
            .state()
            .sorted_tracks(filter)
            .into_iter()
            .skip(page.offset)
            .take(page.limit)
            .collect())
    }

    async fn get_track(&self, id: TrackId) -> Result<Track, LibraryError> {
        self.state().track(id).cloned()
    }

    async fn recently_added(&self, limit: usize) -> Result<Vec<Track>, LibraryError> {
        let mut tracks: Vec<Track> = self
            .state()
            .tracks
            .values()
            .map(|(track, _)| track.clone())
            .collect();
        tracks.sort_by_key(|track| Reverse((track.added_at, track.id)));
        tracks.truncate(limit);
        Ok(tracks)
    }

    async fn list_playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        let state = self.state();
        let mut playlists: Vec<Playlist> = state
            .playlists
            .iter()
            .map(|(id, playlist)| state.summary(*id, playlist))
            .collect();
        playlists.sort_by_cached_key(|playlist| (playlist.name.to_lowercase(), playlist.id));
        Ok(playlists)
    }

    async fn get_playlist(&self, id: PlaylistId) -> Result<PlaylistContents, LibraryError> {
        let state = self.state();
        let playlist = state.playlist(id)?;
        Ok(PlaylistContents {
            playlist: state.summary(id, playlist),
            entries: state.entries(playlist),
        })
    }

    async fn create_playlist(&self, name: &str) -> Result<Playlist, LibraryError> {
        let mut state = self.state();
        let name = state.playlist_name(name, None)?.to_owned();
        state.last_playlist_id += 1;
        let id = PlaylistId::new(state.last_playlist_id);
        let playlist = StoredPlaylist {
            name,
            entries: Vec::new(),
        };
        let summary = state.summary(id, &playlist);
        state.playlists.insert(id, playlist);
        Ok(summary)
    }

    async fn rename_playlist(&self, id: PlaylistId, name: &str) -> Result<Playlist, LibraryError> {
        let mut state = self.state();
        state.playlist(id)?;
        let name = state.playlist_name(name, Some(id))?.to_owned();
        state.playlist_mut(id)?.name = name;
        Ok(state.summary(id, state.playlist(id)?))
    }

    async fn delete_playlist(&self, id: PlaylistId) -> Result<(), LibraryError> {
        self.state()
            .playlists
            .remove(&id)
            .map(drop)
            .ok_or(LibraryError::NotFound(Resource::Playlist(id)))
    }

    async fn add_to_playlist(
        &self,
        id: PlaylistId,
        tracks: &[TrackId],
        at: Option<usize>,
    ) -> Result<Vec<PlaylistEntry>, LibraryError> {
        let mut state = self.state();
        let len = state.playlist(id)?.entries.len();
        let at = at.unwrap_or(len);
        if at > len {
            return Err(invalid_position("position", at, len));
        }
        let tracks = tracks
            .iter()
            .map(|track| state.track(*track).cloned())
            .collect::<Result<Vec<Track>, LibraryError>>()?;
        let mut added = Vec::with_capacity(tracks.len());
        for (offset, track) in tracks.into_iter().enumerate() {
            state.last_entry_id += 1;
            added.push(PlaylistEntry {
                id: PlaylistEntryId::new(state.last_entry_id),
                position: at + offset,
                track,
            });
        }
        let new = added.iter().map(|entry| (entry.id, entry.track.id));
        state.playlist_mut(id)?.entries.splice(at..at, new);
        Ok(added)
    }

    async fn remove_from_playlist(
        &self,
        id: PlaylistId,
        entries: &[PlaylistEntryId],
    ) -> Result<(), LibraryError> {
        let mut state = self.state();
        let playlist = state.playlist_mut(id)?;
        let remove: BTreeSet<PlaylistEntryId> = entries.iter().copied().collect();
        if let Some(missing) = remove
            .iter()
            .find(|entry| !playlist.entries.iter().any(|(id, _)| id == *entry))
        {
            return Err(LibraryError::NotFound(Resource::PlaylistEntry(*missing)));
        }
        playlist.entries.retain(|(id, _)| !remove.contains(id));
        Ok(())
    }

    async fn move_playlist_entry(
        &self,
        id: PlaylistId,
        entry: PlaylistEntryId,
        to: usize,
    ) -> Result<(), LibraryError> {
        let mut state = self.state();
        let entries = &mut state.playlist_mut(id)?.entries;
        let from = entries
            .iter()
            .position(|(id, _)| *id == entry)
            .ok_or(LibraryError::NotFound(Resource::PlaylistEntry(entry)))?;
        if to >= entries.len() {
            return Err(invalid_position("target position", to, entries.len()));
        }
        let moved = entries.remove(from);
        entries.insert(to, moved);
        Ok(())
    }

    async fn resolve_audio_source(&self, track: TrackId) -> Result<AudioSource, LibraryError> {
        self.state()
            .tracks
            .get(&track)
            .map(|(_, source)| source.clone())
            .ok_or(LibraryError::NotFound(Resource::Track(track)))
    }

    async fn record_play(
        &self,
        track: TrackId,
        device: &str,
        at: SystemTime,
    ) -> Result<(), LibraryError> {
        let mut state = self.state();
        state.track(track)?;
        if device.trim().is_empty() {
            return Err(LibraryError::InvalidInput(
                "a device name cannot be empty".to_owned(),
            ));
        }
        state.plays.push((track, device.to_owned(), at));
        Ok(())
    }

    async fn recently_played(&self, limit: usize) -> Result<Vec<Play>, LibraryError> {
        let state = self.state();
        let mut plays: Vec<(usize, Play)> = state
            .plays
            .iter()
            .enumerate()
            .filter_map(|(order, (track, device, at))| {
                Some((
                    order,
                    Play {
                        track: state.track(*track).ok()?.clone(),
                        device: device.clone(),
                        at: *at,
                    },
                ))
            })
            .collect();
        plays.sort_by_key(|(order, play)| Reverse((play.at, *order)));
        Ok(plays
            .into_iter()
            .take(limit)
            .map(|(_, play)| play)
            .collect())
    }
}
