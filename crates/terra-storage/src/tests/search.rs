//! Full-text search over the sample library.

use tempfile::TempDir;
use terra_core::testing::sample;
use terra_core::{AlbumKind, LibraryBackend, SearchResults};

use super::{new_track, sample_library};
use crate::LocalLibrary;
use crate::write::{NewAlbum, RootId, ensure_album, ensure_artist, insert_track};

/// The names of what a search found: artists, albums and tracks.
async fn found(library: &LocalLibrary, query: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let SearchResults {
        artists,
        albums,
        tracks,
    } = library
        .search(query, 10)
        .await
        .unwrap_or_else(|err| panic!("searching {query:?} failed: {err:?}"));
    (
        artists.into_iter().map(|artist| artist.name).collect(),
        albums.into_iter().map(|album| album.title).collect(),
        tracks.into_iter().map(|track| track.title).collect(),
    )
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test]
async fn prefixes_in_mixed_case_match_title_artist_and_album() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    // A title prefix.
    assert_eq!(found(&library, "MIDn").await.2, names(&["Midnight Drive"]));
    // An artist prefix: "Artist E" and its tracks.
    assert_eq!(
        found(&library, "aRtIsT e").await,
        (
            names(&["Artist E"]),
            names(&[]),
            names(&["Last Light", "Demo Tape"])
        )
    );
    // An album prefix finds the album and its tracks.
    assert_eq!(
        found(&library, "NOCT").await,
        (
            names(&[]),
            names(&["Nocturnes"]),
            names(&["Midnight Drive"])
        )
    );
    // Terms may match different fields: a title word and an album word.
    assert_eq!(found(&library, "sun DAY").await.2, names(&["Sunrise"]));
    // An album artist prefix finds the album, not the tracks.
    assert_eq!(
        found(&library, "vari").await,
        (
            names(&["Various Artists"]),
            names(&["Late Shift"]),
            names(&[])
        )
    );
}

#[tokio::test]
async fn fts_syntax_in_the_query_is_plain_text() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    // Each query must succeed. Punctuation only separates terms.
    let midnight = names(&["Midnight Drive"]);
    for query in [
        r#""midnight"#,
        r#"midnight""#,
        r#""mid" "dri""#,
        "midnight*",
        "*midnight",
        "-midnight",
        "midnight -drive",
        "mid-dri",
        "(midnight",
        "midnight^",
        "{midnight}:",
    ] {
        assert_eq!(found(&library, query).await.2, midnight, "{query:?}");
    }
    // NEAR, AND, OR, NOT and column names are words like any other, which
    // no sample record has, so every term must match and none does.
    let nothing = (names(&[]), names(&[]), names(&[]));
    for query in [
        "NEAR",
        "near",
        "\"NEAR\"",
        "NEAR(midnight drive)",
        "midnight NEAR drive",
        "midnight AND drive",
        "midnight OR sunrise",
        "midnight NOT drive",
        "title:midnight",
    ] {
        assert_eq!(found(&library, query).await, nothing, "{query:?}");
    }
    // Queries without words find nothing.
    for query in ["\"", "*", "-", "\"\"", "\"*\"", "- * \" ^ :", "()"] {
        assert_eq!(found(&library, query).await, nothing, "{query:?}");
    }
}

#[tokio::test]
async fn accents_on_latin_letters_are_ignored() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    // Two synthetic tracks: one title precomposed (é is U+00E9), the other
    // decomposed (e followed by U+0301).
    let (template, _) = &sample::library().tracks[0];
    let mut conn = library.pool.acquire().await.unwrap();
    for title in ["Café Lights", "Cafe\u{301} Noir"] {
        let mut track = template.clone();
        track.title = title.to_owned();
        insert_track(&mut conn, &new_track(&track, RootId(1), title))
            .await
            .unwrap();
    }
    drop(conn);

    let expected = (
        names(&["Café Ensemble"]),
        names(&[]),
        // Ordered by lower-cased title: e (U+0065) sorts before é (U+00E9).
        names(&["Cafe\u{301} Noir", "Café Lights"]),
    );
    for query in ["cafe", "CAFÉ", "cafe\u{301}", "Café", "CAFE\u{301}"] {
        assert_eq!(found(&library, query).await, expected, "{query:?}");
    }
    // A combining mark inside a word does not split it, wherever it is: "ca"
    // and "fe" would be two terms, and no word starts with "fe".
    for query in [
        "cafe\u{301} lig",
        "ca\u{301}fe lig",
        "cáfe lig",
        "c\u{327}afe lig",
    ] {
        assert_eq!(
            found(&library, query).await.2,
            names(&["Café Lights"]),
            "{query:?}"
        );
    }
    assert_eq!(found(&library, "ártist c").await.0, names(&["Artist C"]));
}

#[tokio::test]
async fn any_combining_mark_is_ignored() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;

    // The tokenizer reads the overline (U+0305) as a separator, so it only
    // matches because the query and the index are folded before it sees them.
    for query in ["cafe", "café", "ca\u{301}fe", "ca\u{305}fe"] {
        assert_eq!(
            found(&library, query).await.0,
            names(&["Café Ensemble"]),
            "{query:?}"
        );
    }
}

#[tokio::test]
async fn names_are_folded_where_they_are_indexed() {
    let dir = TempDir::new().unwrap();
    let library = sample_library(dir.path()).await;
    // An artist, album and track whose names hold an overline (U+0305),
    // which the tokenizer would read as a separator, and precomposed Greek
    // letters (έ, ύ, Ώ), whose accents it would keep.
    let artist_name = "Ca\u{305}fe Δέλτα";
    let album_title = "Νύχτα Ove\u{305}r";
    let track_title = "Ma\u{305}rk Ώρα";
    let (template, _) = &sample::library().tracks[0];
    let mut conn = library.pool.acquire().await.unwrap();
    let artist = ensure_artist(&mut conn, artist_name, artist_name)
        .await
        .unwrap();
    let album = NewAlbum {
        title: album_title,
        album_artist: Some(artist),
        year: None,
        kind: AlbumKind::Album,
    };
    let album = ensure_album(&mut conn, &album).await.unwrap();
    let mut track = new_track(template, RootId(1), "folded.flac");
    track.title = track_title;
    track.artist = artist;
    track.album = Some(album);
    insert_track(&mut conn, &track).await.unwrap();
    drop(conn);

    // The artist matches itself, its album (as album artist) and its track.
    // The names come back as they were written.
    let everything = (
        names(&[artist_name]),
        names(&[album_title]),
        names(&[track_title]),
    );
    for query in ["cafe δελτα", "ca\u{305}fe ΔΈΛΤΑ", "CAFE δέλτα"] {
        assert_eq!(found(&library, query).await, everything, "{query:?}");
    }
    // The album matches itself and its track.
    for query in ["over νυχτα", "ove\u{305}r Νύχτα"] {
        assert_eq!(
            found(&library, query).await,
            (names(&[]), names(&[album_title]), names(&[track_title])),
            "{query:?}"
        );
    }
    // The track matches by its title alone.
    for query in ["mark ωρα", "ma\u{305}rk Ώρα", "MARK"] {
        assert_eq!(
            found(&library, query).await,
            (names(&[]), names(&[]), names(&[track_title])),
            "{query:?}"
        );
    }
}
