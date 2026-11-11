//! The scanner, against copies of the audio fixtures in
//! `tests/fixtures/`, whose tags the tests change with Lofty.

use std::collections::HashMap;
use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::mp4::{Atom, AtomData, AtomIdent, Mp4File};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag};
use tempfile::TempDir;
use terra_core::{
    AlbumFilter, AlbumKind, AudioSource, LibraryBackend, LibraryError, Page, Track, TrackFilter,
};

use crate::{Error, LocalLibrary, MissingFile, ScanProgress, ScanReport};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// The fixtures and where each is copied in the music folder.
const FILES: &[(&str, &str)] = &[
    ("tone.mp3", "Album A/01 tone.mp3"),
    ("tone.flac", "Album A/02 tone.flac"),
    ("tone.opus", "Album B/tone.opus"),
    ("tone-aac.m4a", "Album B/tone-aac.m4a"),
    ("tone-alac.m4a", "tone-alac.m4a"),
];

/// The modification time every copied file gets, so that a test controls
/// whether a file looks changed rather than the file system's clock.
const MTIME: Duration = Duration::from_secs(1_700_000_000);

/// A library in one temporary folder and a music folder, its only root, in
/// another, holding a copy of every fixture.
pub(crate) struct Setup {
    _dir: TempDir,
    music: TempDir,
    pub(crate) library: LocalLibrary,
}

impl Setup {
    pub(crate) async fn new() -> Self {
        Self::with(FILES).await
    }

    /// A library whose music folder holds a copy of each of `files`, a
    /// fixture and where it goes.
    async fn with(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let music = TempDir::new().unwrap();
        for (fixture, rel_path) in files {
            let to = music.path().join(rel_path);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(Path::new(FIXTURES).join(fixture), &to).unwrap();
            set_mtime(&to, 0);
        }
        let library = LocalLibrary::create(dir.path().join("library"))
            .await
            .unwrap();
        library.add_root(music.path()).await.unwrap();
        Self {
            _dir: dir,
            music,
            library,
        }
    }

    pub(crate) fn path(&self, rel_path: &str) -> PathBuf {
        self.music.path().join(rel_path)
    }

    pub(crate) async fn scan(&self) -> ScanReport {
        self.library.scan(|_| {}).await.unwrap()
    }

    async fn tracks(&self) -> HashMap<String, Track> {
        let tracks = self
            .library
            .list_tracks(TrackFilter::default(), Page::ALL)
            .await
            .unwrap();
        tracks
            .into_iter()
            .map(|track| (track.title.clone(), track))
            .collect()
    }

    pub(crate) async fn track(&self, title: &str) -> Track {
        let mut tracks = self.tracks().await;
        tracks
            .remove(title)
            .unwrap_or_else(|| panic!("no track {title:?} in {:?}", tracks.keys()))
    }

    pub(crate) async fn missing_since(&self, track: &Track) -> Option<i64> {
        sqlx::query_scalar("SELECT missing_since FROM media_file WHERE track_id = ?1")
            .bind(track.id.get())
            .fetch_one(&self.library.pool)
            .await
            .unwrap()
    }
}

/// Sets a file's modification time to [`MTIME`] plus `step` seconds.
fn set_mtime(path: &Path, step: u64) {
    let time = SystemTime::UNIX_EPOCH + MTIME + Duration::from_secs(step);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// Changes a file's primary tag, creating it if needed, and moves its
/// modification time on by `step` seconds.
fn retag(path: &Path, step: u64, change: impl FnOnce(&mut Tag)) {
    let mut file = lofty::read_from_path(path).unwrap();
    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(file.primary_tag_type()));
    }
    change(file.primary_tag_mut().unwrap());
    file.save_to_path(path, WriteOptions::default()).unwrap();
    set_mtime(path, step);
}

pub(crate) fn report(
    added: usize,
    updated: usize,
    moved: usize,
    missing: usize,
    unchanged: usize,
) -> ScanReport {
    ScanReport {
        added,
        updated,
        moved,
        missing,
        unchanged,
        errors: Vec::new(),
    }
}

#[tokio::test]
async fn scanning_the_fixtures_adds_every_file() {
    let setup = Setup::new().await;
    // Hidden entries and files without an audio extension are skipped.
    fs::create_dir(setup.path(".hidden")).unwrap();
    fs::copy(
        Path::new(FIXTURES).join("tone.mp3"),
        setup.path(".hidden/tone.mp3"),
    )
    .unwrap();
    fs::copy(
        Path::new(FIXTURES).join("tone.mp3"),
        setup.path(".tone.mp3"),
    )
    .unwrap();
    fs::write(setup.path("notes.txt"), "not audio").unwrap();

    let mut progress = Vec::new();
    let report = setup
        .library
        .scan(|step: ScanProgress| progress.push((step.done, step.total)))
        .await
        .unwrap();
    assert_eq!(report, self::report(5, 0, 0, 0, 0));
    assert_eq!(progress, (1..=5).map(|done| (done, 5)).collect::<Vec<_>>());

    let tracks = setup.tracks().await;
    let expected = [
        ("Tone MP3", "Artist A", "Album A", "mp3", false),
        ("Tone FLAC", "Artist A", "Album A", "flac", true),
        ("Tone Opus", "Artist B", "Album B", "opus", false),
        ("Tone AAC", "Artist B", "Album B", "aac", false),
        ("Tone ALAC", "Artist C", "Album C", "alac", true),
    ];
    assert_eq!(tracks.len(), expected.len(), "{:?}", tracks.keys());
    for (title, artist, album, codec, lossless) in expected {
        let track = &tracks[title];
        assert_eq!(track.artist.name, artist, "{title}");
        assert_eq!(track.album.as_ref().unwrap().title, album, "{title}");
        assert_eq!(track.format.codec, codec, "{title}");
        assert_eq!(track.format.lossless, lossless, "{title}");
        assert_eq!(track.format.channels, 1, "{title}");
        // The MP3 and AAC durations leave out the encoder delay and
        // padding.
        assert_one_second(track);
    }
    assert_eq!(tracks["Tone FLAC"].track_no, Some(2));
    assert_eq!(tracks["Tone ALAC"].disc_no, Some(1));

    let albums = setup
        .library
        .list_albums(AlbumFilter::default())
        .await
        .unwrap();
    let album_a = albums
        .iter()
        .find(|album| album.title == "Album A")
        .unwrap();
    assert_eq!(album_a.album_artist.name, "Artist A");
    assert_eq!(album_a.year, Some(2001));
    assert_eq!(album_a.kind, AlbumKind::Album);

    let source = setup
        .library
        .resolve_audio_source(tracks["Tone FLAC"].id)
        .await
        .unwrap();
    assert_eq!(
        source,
        AudioSource::LocalPath(setup.path("Album A/02 tone.flac"))
    );
}

#[tokio::test]
async fn rescanning_an_unchanged_tree_changes_nothing() {
    let setup = Setup::new().await;
    setup.scan().await;
    let before = setup.tracks().await;

    assert_eq!(setup.scan().await, report(0, 0, 0, 0, 5));
    assert_eq!(setup.tracks().await, before);
}

#[tokio::test]
async fn a_moved_file_keeps_its_track_and_playlist_entries() {
    let setup = Setup::new().await;
    setup.scan().await;
    let track = setup.track("Tone FLAC").await;
    sqlx::query("INSERT INTO playlist (id, name, name_key, created_at, updated_at) VALUES (1, 'Mix', 'mix', 0, 0)")
        .execute(&setup.library.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO playlist_entry (playlist_id, track_id, position) VALUES (1, ?1, 0)")
        .bind(track.id.get())
        .execute(&setup.library.pool)
        .await
        .unwrap();

    fs::create_dir(setup.path("Moved")).unwrap();
    fs::rename(
        setup.path("Album A/02 tone.flac"),
        setup.path("Moved/renamed.flac"),
    )
    .unwrap();
    assert_eq!(setup.scan().await, report(0, 0, 1, 0, 4));

    let moved = setup.track("Tone FLAC").await;
    assert_eq!(moved, track);
    let entry: i64 =
        sqlx::query_scalar("SELECT track_id FROM playlist_entry WHERE playlist_id = 1")
            .fetch_one(&setup.library.pool)
            .await
            .unwrap();
    assert_eq!(entry, track.id.get());
    let source = setup
        .library
        .resolve_audio_source(terra_core::TrackId::new(entry))
        .await
        .unwrap();
    assert_eq!(
        source,
        AudioSource::LocalPath(setup.path("Moved/renamed.flac"))
    );
}

#[tokio::test]
async fn a_copy_is_a_new_track() {
    let setup = Setup::new().await;
    setup.scan().await;
    let original = setup.track("Tone MP3").await;

    // The copy sorts before the original, and still is not taken for it.
    fs::copy(
        setup.path("Album A/01 tone.mp3"),
        setup.path("Album A/00 copy.mp3"),
    )
    .unwrap();
    assert_eq!(setup.scan().await, report(1, 0, 0, 0, 5));

    let copies = setup.library.search("Tone MP3", 10).await.unwrap().tracks;
    assert_eq!(copies.len(), 2);
    let source = setup
        .library
        .resolve_audio_source(original.id)
        .await
        .unwrap();
    assert_eq!(
        source,
        AudioSource::LocalPath(setup.path("Album A/01 tone.mp3"))
    );
}

#[tokio::test]
async fn retagging_a_file_updates_its_track_in_place() {
    let setup = Setup::new().await;
    setup.scan().await;
    let track = setup.track("Tone MP3").await;

    retag(&setup.path("Album A/01 tone.mp3"), 1, |tag| {
        tag.set_title("Renamed Song".to_owned());
        tag.set_track(7);
    });
    assert_eq!(setup.scan().await, report(0, 1, 0, 0, 4));

    let retagged = setup.track("Renamed Song").await;
    assert_eq!(retagged.id, track.id);
    assert_eq!(retagged.track_no, Some(7));
    assert_eq!(retagged.added_at, track.added_at);
    let found = setup.library.search("renamed", 10).await.unwrap().tracks;
    assert_eq!(found.iter().map(|t| t.id).collect::<Vec<_>>(), [track.id]);
    let old = setup.library.search("Tone MP3", 10).await.unwrap().tracks;
    assert!(old.is_empty(), "{old:?}");
}

#[tokio::test]
async fn an_artist_and_album_no_track_uses_are_removed() {
    let setup = Setup::new().await;
    setup.scan().await;
    let track = setup.track("Tone ALAC").await;

    retag(&setup.path("tone-alac.m4a"), 1, |tag| {
        tag.set_artist("Artist E".to_owned());
        tag.set_album("Album E".to_owned());
    });
    assert_eq!(setup.scan().await, report(0, 1, 0, 0, 4));

    let retagged = setup.track("Tone ALAC").await;
    assert_eq!(retagged.id, track.id);
    assert_eq!(retagged.artist.name, "Artist E");
    let artists = setup.library.list_artists().await.unwrap();
    let names: Vec<_> = artists.iter().map(|artist| artist.name.as_str()).collect();
    assert_eq!(names, ["Artist A", "Artist B", "Artist E"]);
    let albums = setup
        .library
        .list_albums(AlbumFilter::default())
        .await
        .unwrap();
    let titles: Vec<_> = albums.iter().map(|album| album.title.as_str()).collect();
    assert_eq!(titles, ["Album A", "Album B", "Album E"]);
    let results = setup.library.search("Artist C", 10).await.unwrap();
    assert!(
        results.artists.is_empty() && results.albums.is_empty(),
        "{results:?}"
    );
}

#[tokio::test]
async fn a_deleted_file_is_missing_until_it_comes_back() {
    let setup = Setup::new().await;
    setup.scan().await;
    let track = setup.track("Tone Opus").await;
    let path = setup.path("Album B/tone.opus");

    fs::remove_file(&path).unwrap();
    assert_eq!(setup.scan().await, report(0, 0, 0, 1, 4));
    let since = setup.missing_since(&track).await;
    assert!(since.is_some());
    let err = setup
        .library
        .resolve_audio_source(track.id)
        .await
        .unwrap_err();
    assert!(matches!(err, LibraryError::Storage(_)), "{err:?}");
    let missing = err.source().unwrap().downcast_ref::<MissingFile>().unwrap();
    assert_eq!(missing.path, path);
    // The track and its fields stay while the file is missing.
    assert_eq!(setup.track("Tone Opus").await, track);

    // A second scan still reports it, and keeps when it went missing.
    assert_eq!(setup.scan().await, report(0, 0, 0, 1, 4));
    assert_eq!(setup.missing_since(&track).await, since);

    fs::copy(Path::new(FIXTURES).join("tone.opus"), &path).unwrap();
    set_mtime(&path, 0);
    assert_eq!(setup.scan().await, report(0, 1, 0, 0, 4));
    assert_eq!(setup.missing_since(&track).await, None);
    let source = setup.library.resolve_audio_source(track.id).await.unwrap();
    assert_eq!(source, AudioSource::LocalPath(path));
}

/// Asserts that `track` lasts 1 s, within 1 ms.
fn assert_one_second(track: &Track) {
    let off = track.duration.abs_diff(Duration::from_secs(1));
    assert!(
        off <= Duration::from_millis(1),
        "{}: {:?}",
        track.title,
        track.duration
    );
}

#[tokio::test]
async fn an_mp3_duration_leaves_out_the_lame_delay_and_padding() {
    let setup = Setup::with(&[
        ("tone-22k.mp3", "tone-22k.mp3"),
        ("tone-no-xing.mp3", "tone-no-xing.mp3"),
    ])
    .await;
    assert_eq!(setup.scan().await, report(2, 0, 0, 0, 0));

    // At 22.05 kHz the delay and padding are 71 ms of the 41 frames.
    assert_one_second(&setup.track("Tone MP3 22 kHz").await);

    // Without a Xing/LAME frame there is nothing to subtract: the duration
    // is Lofty's, which counts whole frames.
    let track = setup.track("Tone MP3 without Xing").await;
    let lofty = lofty::read_from_path(setup.path("tone-no-xing.mp3"))
        .unwrap()
        .properties()
        .duration();
    assert_eq!(track.duration, lofty);
    assert!(track.duration > Duration::from_millis(1020), "{lofty:?}");
}

#[tokio::test]
async fn an_m4a_without_an_edit_list_takes_its_length_from_itunsmpb() {
    let setup = Setup::with(&[("tone-aac.m4a", "tone-aac.m4a")]).await;
    let path = setup.path("tone-aac.m4a");
    // Hide the edit list by renaming its `edts` box to `free`, which keeps
    // every size and offset.
    let mut bytes = fs::read(&path).unwrap();
    let edts = bytes.windows(4).position(|w| w == b"edts").unwrap();
    bytes[edts..edts + 4].copy_from_slice(b"free");
    fs::write(&path, bytes).unwrap();
    set_mtime(&path, 0);

    // Without an edit list or an `iTunSMPB` tag the duration is Lofty's,
    // priming and padding included.
    assert_eq!(setup.scan().await, report(1, 0, 0, 0, 0));
    let track = setup.track("Tone AAC").await;
    assert!(track.duration > Duration::from_millis(1010), "{track:?}");

    // 1024 samples of priming, 960 of padding and 48 000 of audio.
    let mut file = fs::File::open(&path).unwrap();
    let mut m4a = Mp4File::read_from(&mut file, ParseOptions::new()).unwrap();
    m4a.ilst_mut().unwrap().insert(Atom::new(
        AtomIdent::Freeform {
            mean: "com.apple.iTunes".into(),
            name: "iTunSMPB".into(),
        },
        AtomData::UTF8(" 00000000 00000400 000003C0 000000000000BB80".to_owned()),
    ));
    m4a.save_to_path(&path, WriteOptions::default()).unwrap();
    set_mtime(&path, 1);
    assert_eq!(setup.scan().await, report(0, 1, 0, 0, 0));
    assert_one_second(&setup.track("Tone AAC").await);
}

#[tokio::test]
async fn a_missing_file_found_elsewhere_is_a_move() {
    let setup = Setup::new().await;
    setup.scan().await;
    let track = setup.track("Tone AAC").await;

    let parked = setup._dir.path().join("parked.m4a");
    fs::rename(setup.path("Album B/tone-aac.m4a"), &parked).unwrap();
    assert_eq!(setup.scan().await, report(0, 0, 0, 1, 4));
    fs::rename(&parked, setup.path("back.m4a")).unwrap();
    assert_eq!(setup.scan().await, report(0, 0, 1, 0, 4));

    assert_eq!(setup.missing_since(&track).await, None);
    let source = setup.library.resolve_audio_source(track.id).await.unwrap();
    assert_eq!(source, AudioSource::LocalPath(setup.path("back.m4a")));
}

#[tokio::test]
async fn an_empty_root_with_tracks_is_left_alone() {
    // The folder stays, as the mount point of an unmounted drive does.
    a_root_is_left_alone(|root, parked| {
        fs::create_dir(parked).unwrap();
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            fs::rename(entry.path(), parked.join(entry.file_name())).unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn an_unreadable_root_is_left_alone() {
    a_root_is_left_alone(|root, parked| fs::rename(root, parked).unwrap()).await;
}

/// Scans the fixtures, makes the music folder unavailable with `unmount`,
/// which may move what it holds to the path it is given, and copies a file
/// into a second root. The rescan reports the music folder once and leaves
/// its tracks as they were, so that the copy is a new track rather than
/// taking the original's.
async fn a_root_is_left_alone(unmount: impl FnOnce(&Path, &Path)) {
    let setup = Setup::new().await;
    setup.scan().await;
    let before = setup.tracks().await;
    let original = &before["Tone FLAC"];
    let root = fs::canonicalize(setup.music.path()).unwrap();

    unmount(&root, &setup._dir.path().join("parked"));
    let backup = TempDir::new().unwrap();
    let copy = backup.path().join("tone.flac");
    fs::copy(Path::new(FIXTURES).join("tone.flac"), &copy).unwrap();
    set_mtime(&copy, 0);
    setup.library.add_root(backup.path()).await.unwrap();

    let mut expected = report(1, 0, 0, 0, 0);
    expected.errors = vec![(
        root.clone(),
        "the root could not be read or holds no audio files; its 5 files were left as they were"
            .to_owned(),
    )];
    assert_eq!(setup.scan().await, expected);
    for track in before.values() {
        assert_eq!(setup.missing_since(track).await, None, "{}", track.title);
    }
    let source = setup
        .library
        .resolve_audio_source(original.id)
        .await
        .unwrap();
    assert_eq!(
        source,
        AudioSource::LocalPath(root.join("Album A/02 tone.flac"))
    );
    let copies = setup.library.search("Tone FLAC", 10).await.unwrap().tracks;
    assert_eq!(copies.len(), 2, "{copies:?}");
}

#[tokio::test]
async fn an_unreadable_file_is_reported_and_the_scan_goes_on() {
    let setup = Setup::new().await;
    // Bytes from a fixed linear congruential generator, so every run sees
    // the same file.
    let mut state: u32 = 12_345;
    let noise: Vec<u8> = (0..4096)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u8
        })
        .collect();
    fs::write(setup.path("noise.mp3"), noise).unwrap();

    let report = setup.scan().await;
    assert_eq!(report.added, 5);
    let paths: Vec<_> = report.errors.iter().map(|(path, _)| path.clone()).collect();
    assert_eq!(paths, [setup.path("noise.mp3")], "{:?}", report.errors);
}

#[tokio::test]
async fn the_compilation_tag_makes_a_compilation() {
    let setup = Setup::new().await;
    // Two tracks by different artists on "Album A", without an album
    // artist tag.
    retag(&setup.path("Album A/01 tone.mp3"), 0, |tag| {
        tag.insert_text(ItemKey::FlagCompilation, "1".to_owned());
    });
    retag(&setup.path("Album A/02 tone.flac"), 0, |tag| {
        tag.set_artist("Artist B".to_owned());
        tag.insert_text(ItemKey::FlagCompilation, "1".to_owned());
    });
    setup.scan().await;

    let filter = AlbumFilter {
        album_artist: None,
        kind: Some(AlbumKind::Compilation),
    };
    let compilations = setup.library.list_albums(filter).await.unwrap();
    assert_eq!(compilations.len(), 1, "{compilations:?}");
    assert_eq!(compilations[0].title, "Album A");
    let tracks = setup
        .library
        .list_tracks(
            TrackFilter {
                artist: None,
                album: Some(compilations[0].id),
            },
            Page::ALL,
        )
        .await
        .unwrap();
    let artists: Vec<_> = tracks
        .iter()
        .map(|track| track.artist.name.as_str())
        .collect();
    assert_eq!(artists, ["Artist A", "Artist B"]);
}

#[tokio::test]
async fn retagging_an_album_updates_its_year_and_kind() {
    let setup = Setup::new().await;
    // With an album artist tag, the retag below keeps the album's key, so
    // the scan finds the same album rather than making a new one.
    let path = setup.path("tone-alac.m4a");
    retag(&path, 0, |tag| {
        tag.insert_text(ItemKey::AlbumArtist, "Artist C".to_owned());
    });
    setup.scan().await;
    let album = setup.track("Tone ALAC").await.album.unwrap().id;

    retag(&path, 1, |tag| {
        tag.insert_text(ItemKey::RecordingDate, "2004".to_owned());
        tag.insert_text(ItemKey::FlagCompilation, "1".to_owned());
    });
    assert_eq!(setup.scan().await, report(0, 1, 0, 0, 4));

    let albums = setup
        .library
        .list_albums(AlbumFilter::default())
        .await
        .unwrap();
    let retagged = albums.iter().find(|a| a.title == "Album C").unwrap();
    assert_eq!(retagged.id, album);
    assert_eq!(retagged.album_artist.name, "Artist C");
    assert_eq!(retagged.year, Some(2004));
    assert_eq!(retagged.kind, AlbumKind::Compilation);
}

#[tokio::test]
async fn embedded_art_is_recorded() {
    let setup = Setup::new().await;
    retag(&setup.path("Album A/01 tone.mp3"), 0, |tag| {
        let picture = Picture::unchecked(b"\x89PNG\r\n\x1a\nsynthetic".to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build();
        tag.push_picture(picture);
    });
    setup.scan().await;

    let art: Vec<(String, bool)> =
        sqlx::query_as("SELECT rel_path, has_embedded_art FROM media_file ORDER BY rel_path")
            .fetch_all(&setup.library.pool)
            .await
            .unwrap();
    let with_art: Vec<_> = art
        .iter()
        .filter(|(_, art)| *art)
        .map(|(path, _)| path.as_str())
        .collect();
    assert_eq!(with_art, ["Album A/01 tone.mp3"]);
}

#[tokio::test]
async fn roots_do_not_overlap_and_removing_one_removes_its_tracks() {
    let setup = Setup::new().await;
    setup.scan().await;
    let root = fs::canonicalize(setup.music.path()).unwrap();
    assert_eq!(
        setup.library.list_roots().await.unwrap(),
        std::slice::from_ref(&root)
    );

    for overlapping in [
        root.clone(),
        root.join("Album A"),
        root.parent().unwrap().to_owned(),
    ] {
        let err = setup.library.add_root(&overlapping).await.unwrap_err();
        assert!(
            matches!(err, Error::RootOverlaps { .. }),
            "{overlapping:?}: {err:?}"
        );
    }
    let file = setup.library.add_root(setup.path("tone.mp3")).await;
    assert!(matches!(file, Err(Error::Io(_))), "{file:?}");

    assert!(setup.library.remove_root(setup.music.path()).await.unwrap());
    assert!(!setup.library.remove_root(setup.music.path()).await.unwrap());
    assert!(setup.library.list_roots().await.unwrap().is_empty());
    assert!(setup.tracks().await.is_empty());
    assert!(setup.library.list_artists().await.unwrap().is_empty());
    assert!(
        setup
            .library
            .list_albums(AlbumFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        setup.library.search("tone", 10).await.unwrap(),
        Default::default()
    );
}
