//! Test support, behind the feature `testing`.
//!
//! - [`MemoryLibrary`] is an in-memory [`LibraryBackend`](crate::LibraryBackend).
//! - [`sample`] is the synthetic library tests run against.
//! - [`contract`] is the suite every backend must pass, and
//!   [`contract_tests!`](crate::contract_tests) turns it into one `#[test]`
//!   per case for a given backend.
//! - [`block_on`] drives a future to completion on the current thread, for
//!   backends that need no async runtime.

pub mod contract;
mod memory;
pub mod sample;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

pub use memory::MemoryLibrary;

/// Runs a future to completion on the current thread, parking the thread
/// while the future waits.
///
/// It provides no reactor and no timers, so it only suits futures that some
/// other thread or nothing at all wakes, such as [`MemoryLibrary`]'s.
pub fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

/// Generates one `#[test]` for each case of the [contract suite](contract).
///
/// - `run` is a function that takes a future and drives it to completion,
///   such as [`block_on`] or a wrapper around an async runtime.
/// - `backend` is an expression for a backend that holds the
///   [sample library](sample) and no playlists or plays. Each test evaluates
///   it once, inside an `async` block, so it may `.await`: it is the backend
///   factory.
///
/// ```no_run
/// mod contract {
///     terra_core::contract_tests! {
///         run: terra_core::testing::block_on,
///         backend: terra_core::testing::MemoryLibrary::sample(),
///     }
/// }
/// ```
#[macro_export]
macro_rules! contract_tests {
    (run: $run:path, backend: $backend:expr $(,)?) => {
        $crate::contract_tests! {
            @cases $run, $backend;
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
    };
    (@cases $run:path, $backend:expr; $($case:ident),* $(,)?) => {
        $(
            #[test]
            fn $case() {
                $run(async { $crate::testing::contract::$case($backend).await });
            }
        )*
    };
}
