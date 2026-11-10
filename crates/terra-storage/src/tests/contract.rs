//! The `terra_core` contract suite, run against a `LocalLibrary` holding the
//! sample library in a temporary directory.

use std::future::Future;

use tempfile::TempDir;

use super::sample_library;

/// Drives `future` to completion on a current-thread Tokio runtime, as
/// `#[tokio::test]` does.
fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

terra_core::contract_tests! {
    run: run,
    fixture: TempDir::new().unwrap(),
    backend: |dir| sample_library(dir.path()).await,
}
