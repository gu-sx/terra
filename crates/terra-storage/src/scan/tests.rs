//! The scanner given a walk that could not read everything, which a test
//! cannot make the file system do without changing permissions.

use std::fs;

use super::{known, roots, walk};
use crate::tests::scan::{Setup, report};

#[tokio::test]
async fn the_known_files_under_an_unreadable_directory_are_left_alone() {
    let setup = Setup::new().await;
    setup.scan().await;
    let mp3 = setup.track("Tone MP3").await;
    let flac = setup.track("Tone FLAC").await;
    let opus = setup.track("Tone Opus").await;

    // A copy of a file under the unreadable directory, which would take its
    // track if that file were a candidate for a move, and a file gone
    // elsewhere.
    fs::copy(setup.path("Album A/01 tone.mp3"), setup.path("copy.mp3")).unwrap();
    fs::remove_file(setup.path("Album B/tone.opus")).unwrap();

    let mut conn = setup.library.pool.acquire().await.unwrap();
    let roots = roots(&mut conn).await.unwrap();
    let known = known(&mut conn).await.unwrap();
    drop(conn);
    let mut walk = walk::walk(&roots);
    // The walk could not list "Album A", so saw nothing under it.
    let album_a = roots[0].path.join("Album A");
    walk.seen.retain(|seen| !seen.path.starts_with(&album_a));
    walk.errors
        .push((album_a.clone(), "permission denied".to_owned()));
    walk.unread.push(album_a.clone());

    let scanned = setup
        .library
        .apply(&roots, &known, walk, |_| {})
        .await
        .unwrap();
    let mut expected = report(1, 0, 0, 1, 2);
    expected.errors = vec![(album_a, "permission denied".to_owned())];
    assert_eq!(scanned, expected);
    assert_eq!(setup.missing_since(&mp3).await, None);
    assert_eq!(setup.missing_since(&flac).await, None);
    assert!(setup.missing_since(&opus).await.is_some());
}
