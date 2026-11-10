//! The synthetic sample library: the artists and tracks of the brief's §9
//! mock-up, with made-up albums around them.
//!
//! | Track          | Artist   | Album            | Kind        | No. |
//! | -------------- | -------- | ---------------- | ----------- | --- |
//! | Midnight Drive | Artist A | Nocturnes        | album       | 1   |
//! | After Hours    | Artist B | Late Shift       | compilation | 1   |
//! | Into the Deep  | Artist C | Late Shift       | compilation | 2   |
//! | Sunrise        | Artist D | Daybreak Session | DJ set      | 1   |
//! | Last Light     | Artist E | Daybreak Session | DJ set      | 2   |
//! | Demo Tape      | Artist E | none             |             |     |
//!
//! "Late Shift" has the album artist "Various Artists". The tracks were
//! added a day apart, in the order of the table.
//!
//! Two more artists have no tracks:
//!
//! | Artist        | Sort name      | Id |
//! | ------------- | -------------- | -- |
//! | The Aardvarks | Aardvarks, The | 7  |
//! | Café Ensemble | Café Ensemble  | 8  |
//!
//! "The Aardvarks" comes first by sort name but not by name or identifier,
//! so ordering artists by sort name, by name or by identifier gives three
//! different orders. "Café Ensemble" has an accent for search to ignore.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::{
    Album, AlbumId, AlbumKind, AlbumRef, Artist, ArtistId, ArtistRef, AudioFormat, AudioSource,
    Track, TrackId,
};

/// The sample library's records, with the audio source of each track.
#[derive(Debug, Clone)]
pub struct SampleLibrary {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<(Track, AudioSource)>,
}

/// When the first sample track was added: 2024-01-01T00:00:00Z.
pub const FIRST_ADDED: u64 = 1_704_067_200;

const DAY: u64 = 24 * 60 * 60;

/// Builds the sample library.
pub fn library() -> SampleLibrary {
    let artist_names = [
        ("Artist A", "Artist A"),
        ("Artist B", "Artist B"),
        ("Artist C", "Artist C"),
        ("Artist D", "Artist D"),
        ("Artist E", "Artist E"),
        ("Various Artists", "Various Artists"),
        ("The Aardvarks", "Aardvarks, The"),
        ("Café Ensemble", "Café Ensemble"),
    ];
    let artists: Vec<Artist> = artist_names
        .iter()
        .zip(1..)
        .map(|((name, sort_name), id)| Artist {
            id: ArtistId::new(id),
            name: (*name).to_owned(),
            sort_name: (*sort_name).to_owned(),
        })
        .collect();
    let artist = |id: i64| {
        let artist = &artists[usize::try_from(id - 1).unwrap()];
        ArtistRef {
            id: artist.id,
            name: artist.name.clone(),
        }
    };

    let albums = vec![
        album(1, "Nocturnes", artist(1), 2021, AlbumKind::Album),
        album(2, "Late Shift", artist(6), 2022, AlbumKind::Compilation),
        album(3, "Daybreak Session", artist(4), 2023, AlbumKind::DjSet),
    ];
    let album_ref = |id: i64| {
        let album = &albums[usize::try_from(id - 1).unwrap()];
        AlbumRef {
            id: album.id,
            title: album.title.clone(),
        }
    };

    let flac = AudioFormat {
        codec: "flac".to_owned(),
        lossless: true,
        sample_rate: 44_100,
        channels: 2,
        bitrate: None,
    };
    let mp3 = AudioFormat {
        codec: "mp3".to_owned(),
        lossless: false,
        sample_rate: 44_100,
        channels: 2,
        bitrate: Some(320_000),
    };
    let opus = AudioFormat {
        codec: "opus".to_owned(),
        lossless: false,
        sample_rate: 48_000,
        channels: 2,
        bitrate: Some(160_000),
    };

    let rows = [
        ("Midnight Drive", 1, Some((1, 1)), 5 * 60 + 42, &flac),
        ("After Hours", 2, Some((2, 1)), 6 * 60 + 18, &mp3),
        ("Into the Deep", 3, Some((2, 2)), 4 * 60 + 51, &mp3),
        ("Sunrise", 4, Some((3, 1)), 6 * 60 + 14, &opus),
        ("Last Light", 5, Some((3, 2)), 5 * 60 + 6, &opus),
        ("Demo Tape", 5, None, 3 * 60 + 33, &flac),
    ];
    let tracks = rows
        .into_iter()
        .zip(1..)
        .map(|((title, artist_id, place, seconds, format), id)| {
            let track = Track {
                id: TrackId::new(id),
                title: title.to_owned(),
                artist: artist(artist_id),
                album: place.map(|(album_id, _)| album_ref(album_id)),
                disc_no: place.map(|_| 1),
                track_no: place.map(|(_, number)| number),
                duration: Duration::from_secs(seconds),
                format: format.clone(),
                added_at: SystemTime::UNIX_EPOCH
                    + Duration::from_secs(FIRST_ADDED + DAY * (id.unsigned_abs() - 1)),
            };
            let file = format!(
                "/music/{}/{}.{}",
                track.artist.name.to_lowercase().replace(' ', "-"),
                title.to_lowercase().replace(' ', "-"),
                format.codec,
            );
            (track, AudioSource::LocalPath(PathBuf::from(file)))
        })
        .collect();

    SampleLibrary {
        artists,
        albums,
        tracks,
    }
}

fn album(id: i64, title: &str, album_artist: ArtistRef, year: i32, kind: AlbumKind) -> Album {
    Album {
        id: AlbumId::new(id),
        title: title.to_owned(),
        album_artist,
        year: Some(year),
        kind,
    }
}
