//! Runs the state's requests against a [`LibraryBackend`].

use std::collections::HashSet;

use terra_core::{AlbumFilter, LibraryBackend, LibraryError, Page, TrackFilter};

use super::app::{Item, Payload, Request, Response, View};
use super::playlist::Edit;

/// How many results a search asks for in each category.
pub const SEARCH_LIMIT: usize = 200;

/// Runs a request and returns its response. A backend error becomes the
/// response's error text.
pub async fn fetch<B: LibraryBackend>(backend: &B, request: Request) -> Response {
    let result = load(backend, &request)
        .await
        .map_err(|error| error.to_string());
    Response { request, result }
}

async fn load<B: LibraryBackend>(backend: &B, request: &Request) -> Result<Payload, LibraryError> {
    let items = match request {
        Request::Playlists { .. } => {
            return backend.list_playlists().await.map(Payload::Playlists);
        }
        Request::Edit(edit) => return run_edit(backend, edit).await,
        Request::Resolve { tracks, .. } => {
            let mut sources = Vec::with_capacity(tracks.len());
            for track in tracks {
                let source = backend.resolve_audio_source(track.id).await;
                sources.push(source.map_err(|error| error.to_string()));
            }
            return Ok(Payload::Sources(sources));
        }
        Request::RecordPlay { track, device, at } => {
            backend.record_play(track.id, device, *at).await?;
            return Ok(Payload::Done);
        }
        Request::Screen { view, .. } => load_view(backend, *view).await?,
        Request::Search { query, .. } => {
            let results = backend.search(query, SEARCH_LIMIT).await?;
            let artists = results.artists.into_iter().map(Item::Artist);
            let albums = results.albums.into_iter().map(Item::Album);
            let tracks = results.tracks.into_iter().map(Item::Track);
            artists.chain(albums).chain(tracks).collect()
        }
    };
    Ok(Payload::Items(items))
}

async fn run_edit<B: LibraryBackend>(backend: &B, edit: &Edit) -> Result<Payload, LibraryError> {
    match edit {
        Edit::Create(name) => return backend.create_playlist(name).await.map(Payload::Playlist),
        Edit::Rename(id, name) => {
            return backend
                .rename_playlist(*id, name)
                .await
                .map(Payload::Playlist);
        }
        Edit::Delete(playlist) => backend.delete_playlist(playlist.id).await?,
        Edit::Add { playlist, track } => {
            backend
                .add_to_playlist(playlist.id, &[track.id], None)
                .await?;
        }
        Edit::Remove { playlist, entry } => {
            backend.remove_from_playlist(*playlist, &[*entry]).await?;
        }
        Edit::Move {
            playlist,
            entry,
            to,
        } => backend.move_playlist_entry(*playlist, *entry, *to).await?,
    }
    Ok(Payload::Done)
}

async fn load_view<B: LibraryBackend>(backend: &B, view: View) -> Result<Vec<Item>, LibraryError> {
    Ok(match view {
        View::Tracks(filter) => {
            let tracks = backend.list_tracks(filter, Page::ALL).await?;
            tracks.into_iter().map(Item::Track).collect()
        }
        View::Artists => {
            let artists = backend.list_artists().await?;
            artists.into_iter().map(Item::Artist).collect()
        }
        View::Albums(filter) => {
            let albums = backend.list_albums(filter).await?;
            albums.into_iter().map(Item::Album).collect()
        }
        View::ArtistAlbums(artist) => {
            let filter = TrackFilter {
                artist: Some(artist),
                ..TrackFilter::default()
            };
            let appears_on: HashSet<_> = backend
                .list_tracks(filter, Page::ALL)
                .await?
                .into_iter()
                .filter_map(|track| track.album.map(|album| album.id))
                .collect();
            let albums = backend.list_albums(AlbumFilter::default()).await?;
            albums
                .into_iter()
                .filter(|album| album.album_artist.id == artist || appears_on.contains(&album.id))
                .map(Item::Album)
                .collect()
        }
        View::Playlist(id) => {
            let contents = backend.get_playlist(id).await?;
            contents.entries.into_iter().map(Item::Entry).collect()
        }
    })
}
