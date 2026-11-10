//! The contract every [`LibraryBackend`] must pass.
//!
//! Each case is an `async fn` that takes a fresh backend holding the
//! [sample library](super::sample) and no playlists or plays, and panics when
//! the backend breaks the contract. The cases find sample records by name, not
//! by identifier, so a backend may assign its own identifiers.
//! [`contract_tests!`](crate::contract_tests) runs them all.

use std::fmt::Debug;
use std::time::{Duration, SystemTime};

use crate::{
    AlbumFilter, AlbumKind, ArtistId, LibraryBackend, LibraryError, Page, PlaylistContents,
    PlaylistEntry, PlaylistEntryId, PlaylistId, Resource, Track, TrackFilter, TrackId,
};

// Helpers.

async fn track<B: LibraryBackend>(backend: &B, title: &str) -> Track {
    let tracks = backend
        .list_tracks(TrackFilter::default(), Page::ALL)
        .await
        .unwrap();
    tracks
        .into_iter()
        .find(|track| track.title == title)
        .unwrap_or_else(|| panic!("the sample library has no track {title:?}"))
}

async fn track_ids<B: LibraryBackend>(backend: &B, titles: &[&str]) -> Vec<TrackId> {
    let mut ids = Vec::with_capacity(titles.len());
    for title in titles {
        ids.push(track(backend, title).await.id);
    }
    ids
}

async fn missing_track<B: LibraryBackend>(backend: &B) -> TrackId {
    let tracks = backend
        .list_tracks(TrackFilter::default(), Page::ALL)
        .await
        .unwrap();
    TrackId::new(tracks.iter().map(|track| track.id.get()).max().unwrap_or(0) + 1)
}

async fn artist_id<B: LibraryBackend>(backend: &B, name: &str) -> ArtistId {
    let artists = backend.list_artists().await.unwrap();
    artists
        .into_iter()
        .find(|artist| artist.name == name)
        .unwrap_or_else(|| panic!("the sample library has no artist {name:?}"))
        .id
}

/// Returns a playlist's contents after checking that its positions run from
/// 0 without gaps and that its count and duration match its entries.
async fn contents<B: LibraryBackend>(backend: &B, id: PlaylistId) -> PlaylistContents {
    let contents = backend.get_playlist(id).await.unwrap();
    let positions: Vec<usize> = contents
        .entries
        .iter()
        .map(|entry| entry.position)
        .collect();
    let expected: Vec<usize> = (0..contents.entries.len()).collect();
    assert_eq!(positions, expected, "positions are not contiguous from 0");
    assert_eq!(contents.playlist.id, id);
    assert_eq!(contents.playlist.entry_count, contents.entries.len());
    assert_eq!(
        contents.playlist.duration,
        contents
            .entries
            .iter()
            .map(|entry| entry.track.duration)
            .sum::<Duration>(),
    );
    let listed = backend.list_playlists().await.unwrap();
    let listed = listed
        .into_iter()
        .find(|playlist| playlist.id == id)
        .expect("list_playlists does not list the playlist");
    assert_eq!(listed, contents.playlist);
    contents
}

fn titles(entries: &[PlaylistEntry]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry.track.title.as_str())
        .collect()
}

fn entry_ids(entries: &[PlaylistEntry]) -> Vec<PlaylistEntryId> {
    entries.iter().map(|entry| entry.id).collect()
}

/// Creates a playlist holding the given tracks, in order.
async fn playlist_of<B: LibraryBackend>(
    backend: &B,
    name: &str,
    titles: &[&str],
) -> (PlaylistId, Vec<PlaylistEntry>) {
    let id = backend.create_playlist(name).await.unwrap().id;
    let tracks = track_ids(backend, titles).await;
    backend.add_to_playlist(id, &tracks, None).await.unwrap();
    let entries = contents(backend, id).await.entries;
    (id, entries)
}

const FIVE: [&str; 5] = [
    "Midnight Drive",
    "After Hours",
    "Into the Deep",
    "Sunrise",
    "Last Light",
];

#[track_caller]
fn assert_not_found<T: Debug>(result: Result<T, LibraryError>, expected: Resource) {
    match result {
        Err(LibraryError::NotFound(resource)) => assert_eq!(resource, expected),
        other => panic!("expected NotFound({expected:?}), got {other:?}"),
    }
}

#[track_caller]
fn assert_invalid_input<T: Debug>(result: Result<T, LibraryError>) {
    match result {
        Err(LibraryError::InvalidInput(_)) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

// Playlists.

pub async fn create_playlist_lists_it<B: LibraryBackend>(backend: B) {
    let created = backend.create_playlist("Driving").await.unwrap();
    assert_eq!(created.name, "Driving");
    assert_eq!(created.entry_count, 0);
    assert_eq!(created.duration, Duration::ZERO);
    assert_eq!(
        backend.list_playlists().await.unwrap(),
        std::slice::from_ref(&created)
    );
    let contents = contents(&backend, created.id).await;
    assert_eq!(contents.playlist, created);
    assert!(contents.entries.is_empty());

    let trimmed = backend.create_playlist("  Discoveries\t").await.unwrap();
    assert_eq!(trimmed.name, "Discoveries");
    assert_ne!(trimmed.id, created.id);
}

pub async fn create_playlist_rejects_an_empty_name<B: LibraryBackend>(backend: B) {
    assert_invalid_input(backend.create_playlist("").await);
    assert_invalid_input(backend.create_playlist(" \t\n").await);
    assert!(backend.list_playlists().await.unwrap().is_empty());
}

pub async fn create_playlist_rejects_a_duplicate_name_ignoring_case<B: LibraryBackend>(backend: B) {
    let favorites = backend.create_playlist("Favorites").await.unwrap();
    assert_invalid_input(backend.create_playlist("Favorites").await);
    assert_invalid_input(backend.create_playlist("FAVORITES").await);
    assert_invalid_input(backend.create_playlist(" favorites ").await);
    assert_eq!(backend.list_playlists().await.unwrap(), [favorites]);
}

pub async fn list_playlists_orders_by_name_ignoring_case<B: LibraryBackend>(backend: B) {
    for name in ["driving", "Favorites", "Discoveries"] {
        backend.create_playlist(name).await.unwrap();
    }
    let names: Vec<String> = backend
        .list_playlists()
        .await
        .unwrap()
        .into_iter()
        .map(|playlist| playlist.name)
        .collect();
    assert_eq!(names, ["Discoveries", "driving", "Favorites"]);
}

pub async fn rename_playlist_changes_the_name<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &["Sunrise"]).await;

    let renamed = backend.rename_playlist(id, " Road Trip ").await.unwrap();
    assert_eq!(renamed.id, id);
    assert_eq!(renamed.name, "Road Trip");
    let after = contents(&backend, id).await;
    assert_eq!(after.playlist, renamed);
    assert_eq!(after.entries, entries);

    let recased = backend.rename_playlist(id, "road trip").await.unwrap();
    assert_eq!(recased.name, "road trip");

    // The old name is free again.
    backend.create_playlist("Driving").await.unwrap();
}

pub async fn rename_playlist_rejects_a_duplicate_name<B: LibraryBackend>(backend: B) {
    backend.create_playlist("Favorites").await.unwrap();
    let driving = backend.create_playlist("Driving").await.unwrap();

    assert_invalid_input(backend.rename_playlist(driving.id, "favorites").await);
    assert_invalid_input(backend.rename_playlist(driving.id, "FAVORITES ").await);
    assert_invalid_input(backend.rename_playlist(driving.id, "  ").await);
    assert_eq!(contents(&backend, driving.id).await.playlist, driving);
}

pub async fn rename_playlist_of_a_missing_playlist_is_not_found<B: LibraryBackend>(backend: B) {
    let id = backend.create_playlist("Driving").await.unwrap().id;
    backend.delete_playlist(id).await.unwrap();
    assert_not_found(
        backend.rename_playlist(id, "Road Trip").await,
        Resource::Playlist(id),
    );
}

pub async fn delete_playlist_removes_it_and_keeps_the_tracks<B: LibraryBackend>(backend: B) {
    let (id, _) = playlist_of(&backend, "Driving", &["Sunrise", "Last Light"]).await;
    let kept = backend.create_playlist("Favorites").await.unwrap();

    backend.delete_playlist(id).await.unwrap();
    assert_eq!(backend.list_playlists().await.unwrap(), [kept]);
    assert_not_found(backend.get_playlist(id).await, Resource::Playlist(id));
    assert_not_found(backend.delete_playlist(id).await, Resource::Playlist(id));
    track(&backend, "Sunrise").await;
    track(&backend, "Last Light").await;

    // The name is free again.
    backend.create_playlist("driving").await.unwrap();
}

pub async fn add_to_playlist_appends_at_the_end<B: LibraryBackend>(backend: B) {
    let id = backend.create_playlist("Driving").await.unwrap().id;
    let first = track_ids(&backend, &["Midnight Drive", "After Hours"]).await;
    let added = backend.add_to_playlist(id, &first, None).await.unwrap();
    assert_eq!(titles(&added), ["Midnight Drive", "After Hours"]);
    assert_eq!(contents(&backend, id).await.entries, added);

    let second = track_ids(&backend, &["Sunrise"]).await;
    let more = backend.add_to_playlist(id, &second, None).await.unwrap();
    assert_eq!(titles(&more), ["Sunrise"]);
    assert_eq!(more[0].position, 2);
    let entries = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&entries),
        ["Midnight Drive", "After Hours", "Sunrise"]
    );
    assert_eq!(entries[2], more[0]);

    assert!(
        backend
            .add_to_playlist(id, &[], None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(contents(&backend, id).await.entries, entries);
}

pub async fn add_to_playlist_inserts_at_a_position<B: LibraryBackend>(backend: B) {
    let (id, _) = playlist_of(&backend, "Driving", &["Midnight Drive", "After Hours"]).await;

    let middle = track_ids(&backend, &["Sunrise", "Last Light"]).await;
    let added = backend.add_to_playlist(id, &middle, Some(1)).await.unwrap();
    assert_eq!(titles(&added), ["Sunrise", "Last Light"]);
    assert_eq!(
        added.iter().map(|entry| entry.position).collect::<Vec<_>>(),
        [1, 2]
    );
    let entries = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&entries),
        ["Midnight Drive", "Sunrise", "Last Light", "After Hours"]
    );

    let front = track_ids(&backend, &["Into the Deep"]).await;
    backend.add_to_playlist(id, &front, Some(0)).await.unwrap();
    let entries = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&entries),
        [
            "Into the Deep",
            "Midnight Drive",
            "Sunrise",
            "Last Light",
            "After Hours"
        ]
    );

    let end = track_ids(&backend, &["Demo Tape"]).await;
    backend.add_to_playlist(id, &end, Some(5)).await.unwrap();
    let entries = contents(&backend, id).await.entries;
    assert_eq!(entries.len(), 6);
    assert_eq!(entries[5].track.title, "Demo Tape");
}

pub async fn add_to_playlist_rejects_a_position_past_the_end<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &["Sunrise", "Last Light"]).await;
    let tracks = track_ids(&backend, &["Midnight Drive"]).await;
    assert_invalid_input(backend.add_to_playlist(id, &tracks, Some(3)).await);
    assert_invalid_input(backend.add_to_playlist(id, &tracks, Some(usize::MAX)).await);
    assert_eq!(contents(&backend, id).await.entries, entries);
}

pub async fn add_to_playlist_allows_duplicate_entries<B: LibraryBackend>(backend: B) {
    let id = backend.create_playlist("Driving").await.unwrap().id;
    let sunrise = track(&backend, "Sunrise").await.id;
    backend
        .add_to_playlist(id, &[sunrise, sunrise], None)
        .await
        .unwrap();
    backend
        .add_to_playlist(id, &[sunrise], Some(1))
        .await
        .unwrap();

    let entries = contents(&backend, id).await.entries;
    assert_eq!(titles(&entries), ["Sunrise", "Sunrise", "Sunrise"]);
    let mut ids = entry_ids(&entries);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each entry has its own identifier");
}

pub async fn add_to_playlist_with_a_missing_track_changes_nothing<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &["Sunrise"]).await;
    let missing = missing_track(&backend).await;
    let sunrise = track(&backend, "Sunrise").await.id;
    assert_not_found(
        backend.add_to_playlist(id, &[sunrise, missing], None).await,
        Resource::Track(missing),
    );
    assert_eq!(contents(&backend, id).await.entries, entries);
}

pub async fn add_to_playlist_of_a_missing_playlist_is_not_found<B: LibraryBackend>(backend: B) {
    let id = backend.create_playlist("Driving").await.unwrap().id;
    backend.delete_playlist(id).await.unwrap();
    let sunrise = track(&backend, "Sunrise").await.id;
    assert_not_found(
        backend.add_to_playlist(id, &[sunrise], None).await,
        Resource::Playlist(id),
    );
}

pub async fn remove_from_playlist_closes_the_gaps<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &FIVE).await;

    backend
        .remove_from_playlist(id, &[entries[1].id, entries[3].id, entries[1].id])
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&after),
        ["Midnight Drive", "Into the Deep", "Last Light"]
    );
    assert_eq!(
        entry_ids(&after),
        [entries[0].id, entries[2].id, entries[4].id]
    );

    backend.remove_from_playlist(id, &[]).await.unwrap();
    assert_eq!(contents(&backend, id).await.entries, after);

    // Removing one copy of a duplicated track keeps the other.
    let sunrise = track(&backend, "Sunrise").await.id;
    let copies = backend
        .add_to_playlist(id, &[sunrise, sunrise], Some(0))
        .await
        .unwrap();
    backend
        .remove_from_playlist(id, &[copies[0].id])
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(after[0].id, copies[1].id);
    assert_eq!(
        titles(&after),
        ["Sunrise", "Midnight Drive", "Into the Deep", "Last Light"]
    );

    let all = entry_ids(&after);
    backend.remove_from_playlist(id, &all).await.unwrap();
    assert!(contents(&backend, id).await.entries.is_empty());
}

pub async fn remove_from_playlist_with_a_missing_entry_changes_nothing<B: LibraryBackend>(
    backend: B,
) {
    let (id, entries) = playlist_of(&backend, "Driving", &["Sunrise", "Last Light"]).await;
    let (_, other) = playlist_of(&backend, "Favorites", &["Sunrise"]).await;

    assert_not_found(
        backend
            .remove_from_playlist(id, &[entries[0].id, other[0].id])
            .await,
        Resource::PlaylistEntry(other[0].id),
    );
    assert_eq!(contents(&backend, id).await.entries, entries);

    backend.delete_playlist(id).await.unwrap();
    assert_not_found(
        backend.remove_from_playlist(id, &[entries[0].id]).await,
        Resource::Playlist(id),
    );
}

pub async fn move_playlist_entry_down_shifts_the_entries_between_up<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &FIVE).await;

    backend
        .move_playlist_entry(id, entries[1].id, 3)
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&after),
        [
            "Midnight Drive",
            "Into the Deep",
            "Sunrise",
            "After Hours",
            "Last Light"
        ]
    );
    assert_eq!(after[3].id, entries[1].id, "the entry keeps its identifier");

    backend
        .move_playlist_entry(id, entries[0].id, 4)
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&after),
        [
            "Into the Deep",
            "Sunrise",
            "After Hours",
            "Last Light",
            "Midnight Drive"
        ]
    );
}

pub async fn move_playlist_entry_up_shifts_the_entries_between_down<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &FIVE).await;

    backend
        .move_playlist_entry(id, entries[3].id, 1)
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&after),
        [
            "Midnight Drive",
            "Sunrise",
            "After Hours",
            "Into the Deep",
            "Last Light"
        ]
    );
    assert_eq!(after[1].id, entries[3].id, "the entry keeps its identifier");

    backend
        .move_playlist_entry(id, entries[4].id, 0)
        .await
        .unwrap();
    let after = contents(&backend, id).await.entries;
    assert_eq!(
        titles(&after),
        [
            "Last Light",
            "Midnight Drive",
            "Sunrise",
            "After Hours",
            "Into the Deep"
        ]
    );
}

pub async fn move_playlist_entry_to_its_own_position_changes_nothing<B: LibraryBackend>(
    backend: B,
) {
    let (id, entries) = playlist_of(&backend, "Driving", &FIVE).await;
    backend
        .move_playlist_entry(id, entries[2].id, 2)
        .await
        .unwrap();
    assert_eq!(contents(&backend, id).await.entries, entries);
}

pub async fn move_playlist_entry_out_of_range_changes_nothing<B: LibraryBackend>(backend: B) {
    let (id, entries) = playlist_of(&backend, "Driving", &FIVE).await;
    let (_, other) = playlist_of(&backend, "Favorites", &["Sunrise"]).await;

    assert_invalid_input(backend.move_playlist_entry(id, entries[0].id, 5).await);
    assert_invalid_input(
        backend
            .move_playlist_entry(id, entries[0].id, usize::MAX)
            .await,
    );
    assert_not_found(
        backend.move_playlist_entry(id, other[0].id, 0).await,
        Resource::PlaylistEntry(other[0].id),
    );
    assert_eq!(contents(&backend, id).await.entries, entries);

    backend.delete_playlist(id).await.unwrap();
    assert_not_found(
        backend.move_playlist_entry(id, entries[0].id, 0).await,
        Resource::Playlist(id),
    );
}

// Search.

async fn search_names<B: LibraryBackend>(
    backend: &B,
    query: &str,
    limit: usize,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let results = backend.search(query, limit).await.unwrap();
    (
        results.artists.into_iter().map(|a| a.name).collect(),
        results.albums.into_iter().map(|a| a.title).collect(),
        results.tracks.into_iter().map(|t| t.title).collect(),
    )
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub async fn search_matches_a_prefix_of_a_title_word<B: LibraryBackend>(backend: B) {
    let midnight = (names(&[]), names(&[]), names(&["Midnight Drive"]));
    assert_eq!(search_names(&backend, "mid", 10).await, midnight);
    assert_eq!(search_names(&backend, "midnight", 10).await, midnight);
    assert_eq!(search_names(&backend, "dri", 10).await, midnight);

    let none = (names(&[]), names(&[]), names(&[]));
    assert_eq!(search_names(&backend, "idnight", 10).await, none);
    assert_eq!(search_names(&backend, "midnightdrive", 10).await, none);
}

pub async fn search_ignores_case<B: LibraryBackend>(backend: B) {
    let midnight = (names(&[]), names(&[]), names(&["Midnight Drive"]));
    for query in ["MIDNIGHT", "mIdNiGhT dRiVe", "Midnight Drive"] {
        assert_eq!(search_names(&backend, query, 10).await, midnight, "{query}");
    }
    let various = search_names(&backend, "VARIOUS", 10).await;
    assert_eq!(various.0, names(&["Various Artists"]));
}

pub async fn search_matches_the_artist<B: LibraryBackend>(backend: B) {
    assert_eq!(
        search_names(&backend, "artist c", 10).await,
        (names(&["Artist C"]), names(&[]), names(&["Into the Deep"])),
    );
    // An album matches its album artist; a track does not.
    assert_eq!(
        search_names(&backend, "various", 10).await,
        (
            names(&["Various Artists"]),
            names(&["Late Shift"]),
            names(&[])
        ),
    );
}

pub async fn search_matches_the_album<B: LibraryBackend>(backend: B) {
    assert_eq!(
        search_names(&backend, "daybreak", 10).await,
        (
            names(&[]),
            names(&["Daybreak Session"]),
            names(&["Sunrise", "Last Light"])
        ),
    );
    assert_eq!(
        search_names(&backend, "shift", 10).await,
        (
            names(&[]),
            names(&["Late Shift"]),
            names(&["After Hours", "Into the Deep"])
        ),
    );
}

pub async fn search_requires_every_term<B: LibraryBackend>(backend: B) {
    assert_eq!(
        search_names(&backend, "deep artist", 10).await,
        (names(&[]), names(&[]), names(&["Into the Deep"])),
    );
    assert_eq!(
        search_names(&backend, "midnight sunrise", 10).await,
        (names(&[]), names(&[]), names(&[])),
    );
}

pub async fn search_returns_at_most_limit_per_category<B: LibraryBackend>(backend: B) {
    assert_eq!(
        search_names(&backend, "artist", 2).await,
        (
            names(&["Artist A", "Artist B"]),
            names(&["Daybreak Session", "Late Shift"]),
            names(&["Sunrise", "Last Light"]),
        ),
    );
    let all = search_names(&backend, "artist", 100).await;
    assert_eq!((all.0.len(), all.1.len(), all.2.len()), (6, 3, 6));
    assert_eq!(
        search_names(&backend, "artist", 0).await,
        (names(&[]), names(&[]), names(&[])),
    );
}

pub async fn search_without_terms_finds_nothing<B: LibraryBackend>(backend: B) {
    for query in ["", "   ", "-- !"] {
        assert_eq!(
            search_names(&backend, query, 10).await,
            (names(&[]), names(&[]), names(&[])),
            "{query:?}"
        );
    }
}

// Browsing.

pub async fn list_artists_orders_by_sort_name<B: LibraryBackend>(backend: B) {
    let artists = backend.list_artists().await.unwrap();
    let names: Vec<&str> = artists.iter().map(|artist| artist.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Artist A",
            "Artist B",
            "Artist C",
            "Artist D",
            "Artist E",
            "Various Artists"
        ]
    );
}

async fn album_titles<B: LibraryBackend>(backend: &B, filter: AlbumFilter) -> Vec<String> {
    let albums = backend.list_albums(filter).await.unwrap();
    albums.into_iter().map(|album| album.title).collect()
}

pub async fn list_albums_orders_by_title_and_filters<B: LibraryBackend>(backend: B) {
    assert_eq!(
        album_titles(&backend, AlbumFilter::default()).await,
        ["Daybreak Session", "Late Shift", "Nocturnes"]
    );

    let by_kind = |kind| AlbumFilter {
        kind: Some(kind),
        ..AlbumFilter::default()
    };
    assert_eq!(
        album_titles(&backend, by_kind(AlbumKind::DjSet)).await,
        ["Daybreak Session"]
    );
    assert_eq!(
        album_titles(&backend, by_kind(AlbumKind::Compilation)).await,
        ["Late Shift"]
    );

    let artist_a = artist_id(&backend, "Artist A").await;
    let by_artist = |artist| AlbumFilter {
        album_artist: Some(artist),
        ..AlbumFilter::default()
    };
    assert_eq!(
        album_titles(&backend, by_artist(artist_a)).await,
        ["Nocturnes"]
    );
    // Artist C has tracks on a compilation but is not its album artist.
    let artist_c = artist_id(&backend, "Artist C").await;
    assert!(album_titles(&backend, by_artist(artist_c)).await.is_empty());

    let both = AlbumFilter {
        album_artist: Some(artist_a),
        kind: Some(AlbumKind::DjSet),
    };
    assert!(album_titles(&backend, both).await.is_empty());

    let artists = backend.list_artists().await.unwrap();
    let missing = ArtistId::new(artists.iter().map(|a| a.id.get()).max().unwrap() + 1);
    assert!(album_titles(&backend, by_artist(missing)).await.is_empty());
}

async fn track_titles<B: LibraryBackend>(
    backend: &B,
    filter: TrackFilter,
    page: Page,
) -> Vec<String> {
    let tracks = backend.list_tracks(filter, page).await.unwrap();
    tracks.into_iter().map(|track| track.title).collect()
}

pub async fn list_tracks_orders_filters_and_pages<B: LibraryBackend>(backend: B) {
    let all = TrackFilter::default();
    assert_eq!(
        track_titles(&backend, all, Page::ALL).await,
        [
            "Sunrise",
            "Last Light",
            "After Hours",
            "Into the Deep",
            "Midnight Drive",
            "Demo Tape"
        ]
    );
    let page = |offset, limit| Page { offset, limit };
    assert_eq!(
        track_titles(&backend, all, page(1, 2)).await,
        ["Last Light", "After Hours"]
    );
    assert_eq!(
        track_titles(&backend, all, page(5, 10)).await,
        ["Demo Tape"]
    );
    assert!(track_titles(&backend, all, page(6, 10)).await.is_empty());
    assert!(
        track_titles(&backend, all, page(usize::MAX, usize::MAX))
            .await
            .is_empty()
    );
    assert!(track_titles(&backend, all, page(0, 0)).await.is_empty());

    let artist_e = artist_id(&backend, "Artist E").await;
    let by_artist = TrackFilter {
        artist: Some(artist_e),
        ..TrackFilter::default()
    };
    assert_eq!(
        track_titles(&backend, by_artist, Page::ALL).await,
        ["Last Light", "Demo Tape"]
    );

    let albums = backend.list_albums(AlbumFilter::default()).await.unwrap();
    let album = |title: &str| albums.iter().find(|a| a.title == title).unwrap().id;
    let by_album = TrackFilter {
        album: Some(album("Late Shift")),
        ..TrackFilter::default()
    };
    assert_eq!(
        track_titles(&backend, by_album, Page::ALL).await,
        ["After Hours", "Into the Deep"]
    );
    let both = TrackFilter {
        artist: Some(artist_e),
        album: Some(album("Daybreak Session")),
    };
    assert_eq!(
        track_titles(&backend, both, Page::ALL).await,
        ["Last Light"]
    );
    // Various Artists is an album artist with no tracks of its own.
    let various = TrackFilter {
        artist: Some(artist_id(&backend, "Various Artists").await),
        ..TrackFilter::default()
    };
    assert!(track_titles(&backend, various, Page::ALL).await.is_empty());
}

pub async fn get_track_returns_the_track_or_not_found<B: LibraryBackend>(backend: B) {
    let tracks = backend
        .list_tracks(TrackFilter::default(), Page::ALL)
        .await
        .unwrap();
    for track in &tracks {
        assert_eq!(&backend.get_track(track.id).await.unwrap(), track);
    }
    let missing = missing_track(&backend).await;
    assert_not_found(backend.get_track(missing).await, Resource::Track(missing));
}

pub async fn recently_added_returns_the_newest_first<B: LibraryBackend>(backend: B) {
    let tracks = backend.recently_added(3).await.unwrap();
    let titles: Vec<&str> = tracks.iter().map(|track| track.title.as_str()).collect();
    assert_eq!(titles, ["Demo Tape", "Last Light", "Sunrise"]);
    assert!(
        tracks
            .windows(2)
            .all(|pair| pair[0].added_at > pair[1].added_at)
    );
    assert_eq!(backend.recently_added(100).await.unwrap().len(), 6);
    assert!(backend.recently_added(0).await.unwrap().is_empty());
}

// Playback.

pub async fn resolve_audio_source_of_a_missing_track_is_not_found<B: LibraryBackend>(backend: B) {
    let tracks = backend
        .list_tracks(TrackFilter::default(), Page::ALL)
        .await
        .unwrap();
    for track in &tracks {
        backend.resolve_audio_source(track.id).await.unwrap();
    }
    let missing = missing_track(&backend).await;
    assert_not_found(
        backend.resolve_audio_source(missing).await,
        Resource::Track(missing),
    );
}

pub async fn record_play_then_recently_played_returns_the_latest_first<B: LibraryBackend>(
    backend: B,
) {
    let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds);
    let plays = [
        ("Sunrise", "laptop", 10),
        ("Last Light", "desktop", 30),
        ("Midnight Drive", "laptop", 20),
        ("After Hours", "phone", 30),
        ("Sunrise", "phone", 5),
    ];
    assert!(backend.recently_played(10).await.unwrap().is_empty());
    for (title, device, seconds) in plays {
        let id = track(&backend, title).await.id;
        backend.record_play(id, device, at(seconds)).await.unwrap();
    }

    let played = backend.recently_played(10).await.unwrap();
    let summary: Vec<(&str, &str, SystemTime)> = played
        .iter()
        .map(|play| (play.track.title.as_str(), play.device.as_str(), play.at))
        .collect();
    assert_eq!(
        summary,
        [
            ("After Hours", "phone", at(30)),
            ("Last Light", "desktop", at(30)),
            ("Midnight Drive", "laptop", at(20)),
            ("Sunrise", "laptop", at(10)),
            ("Sunrise", "phone", at(5)),
        ]
    );
    assert_eq!(played[0].track, track(&backend, "After Hours").await);

    assert_eq!(backend.recently_played(2).await.unwrap(), played[..2]);
    assert!(backend.recently_played(0).await.unwrap().is_empty());
}

pub async fn record_play_rejects_a_missing_track_or_an_empty_device<B: LibraryBackend>(backend: B) {
    let missing = missing_track(&backend).await;
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    assert_not_found(
        backend.record_play(missing, "laptop", now).await,
        Resource::Track(missing),
    );
    let sunrise = track(&backend, "Sunrise").await.id;
    assert_invalid_input(backend.record_play(sunrise, "", now).await);
    assert_invalid_input(backend.record_play(sunrise, "  ", now).await);
    assert!(backend.recently_played(10).await.unwrap().is_empty());
}
