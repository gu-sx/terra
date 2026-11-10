//! The read cases of the `terra_core` contract suite, run against a
//! `LocalLibrary` holding the sample library. The playlist and history cases
//! wait for issue #9, which implements those methods.

use tempfile::TempDir;
use terra_core::testing::contract;

use super::sample_library;

macro_rules! read_cases {
    ($($case:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $case() {
                let dir = TempDir::new().unwrap();
                contract::$case(sample_library(dir.path()).await).await;
            }
        )*
    };
}

read_cases! {
    search_matches_a_prefix_of_a_title_word,
    search_ignores_case,
    search_matches_the_artist,
    search_matches_the_album,
    search_requires_every_term,
    search_returns_at_most_limit_per_category,
    search_without_terms_finds_nothing,
    list_artists_orders_by_sort_name,
    list_albums_orders_by_title_and_filters,
    list_tracks_orders_filters_and_pages,
    get_track_returns_the_track_or_not_found,
    recently_added_returns_the_newest_first,
    resolve_audio_source_of_a_missing_track_is_not_found,
}
