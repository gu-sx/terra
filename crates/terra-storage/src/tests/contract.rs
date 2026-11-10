//! The `terra_core` contract suite, run against a `LocalLibrary` holding the
//! sample library in a temporary directory.
//!
//! `contract_tests!` cannot keep the directory alive for the length of a
//! case, so this file lists the cases itself; keep it in step with the macro.

use tempfile::TempDir;
use terra_core::testing::contract;

use super::sample_library;

macro_rules! cases {
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

cases! {
    create_playlist_lists_it,
    create_playlist_rejects_an_empty_name,
    create_playlist_rejects_a_duplicate_name_ignoring_case,
    list_playlists_orders_by_name_ignoring_case,
    rename_playlist_changes_the_name,
    rename_playlist_rejects_a_duplicate_name,
    rename_playlist_of_a_missing_playlist_is_not_found,
    delete_playlist_removes_it_and_keeps_the_tracks,
    add_to_playlist_appends_at_the_end,
    add_to_playlist_inserts_at_a_position,
    add_to_playlist_rejects_a_position_past_the_end,
    add_to_playlist_allows_duplicate_entries,
    add_to_playlist_with_a_missing_track_changes_nothing,
    add_to_playlist_of_a_missing_playlist_is_not_found,
    remove_from_playlist_closes_the_gaps,
    remove_from_playlist_with_a_missing_entry_changes_nothing,
    move_playlist_entry_down_shifts_the_entries_between_up,
    move_playlist_entry_up_shifts_the_entries_between_down,
    move_playlist_entry_to_its_own_position_changes_nothing,
    move_playlist_entry_out_of_range_changes_nothing,
    search_matches_a_prefix_of_a_title_word,
    search_ignores_case,
    search_ignores_accents,
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
    record_play_then_recently_played_returns_the_latest_first,
    record_play_rejects_a_missing_track_or_an_empty_device,
}
