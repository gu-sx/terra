//! Creating, migrating and opening a library.

use std::path::Path;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use tempfile::TempDir;
use terra_core::{LibraryBackend, TrackFilter};

use crate::library::MIGRATOR;
use crate::{DATABASE_FILE, Error, LocalLibrary};

/// Makes a database at `path` with a table of its own and no migrations.
async fn plain_database(path: &Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    sqlx::query("CREATE TABLE note (body TEXT NOT NULL)")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("INSERT INTO note (body) VALUES ('synthetic')")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
}

#[tokio::test]
async fn create_then_open() {
    let dir = TempDir::new().unwrap();
    let created = LocalLibrary::create(dir.path().join("library"))
        .await
        .unwrap();
    created.close().await;

    let opened = LocalLibrary::open(dir.path().join("library"))
        .await
        .unwrap();
    let tracks = opened
        .list_tracks(TrackFilter::default(), terra_core::Page::ALL)
        .await
        .unwrap();
    assert!(tracks.is_empty());
}

#[tokio::test]
async fn open_turns_on_foreign_keys_and_wal() {
    let dir = TempDir::new().unwrap();
    LocalLibrary::create(dir.path())
        .await
        .unwrap()
        .close()
        .await;
    let library = LocalLibrary::open(dir.path()).await.unwrap();

    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&library.pool)
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&library.pool)
        .await
        .unwrap();
    assert_eq!(journal_mode, "wal");
}

#[tokio::test]
async fn the_bundled_sqlite_has_fts5() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    let fts5: bool = sqlx::query_scalar("SELECT sqlite_compileoption_used('ENABLE_FTS5')")
        .fetch_one(&library.pool)
        .await
        .unwrap();
    assert!(fts5);
}

#[tokio::test]
async fn open_without_migrations_is_outdated_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(DATABASE_FILE);
    plain_database(&path).await;
    let before = std::fs::read(&path).unwrap();
    let files_before = std::fs::read_dir(dir.path()).unwrap().count();

    let result = LocalLibrary::open(dir.path()).await;
    assert!(
        matches!(result, Err(Error::SchemaOutdated)),
        "expected SchemaOutdated, got {result:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "open changed the file"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), files_before);
}

#[tokio::test]
async fn open_an_empty_file_is_outdated() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(DATABASE_FILE);
    std::fs::write(&path, b"").unwrap();

    let result = LocalLibrary::open(dir.path()).await;
    assert!(matches!(result, Err(Error::SchemaOutdated)), "{result:?}");
    assert!(std::fs::read(&path).unwrap().is_empty());
}

#[tokio::test]
async fn open_with_a_pending_migration_is_outdated() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    let latest = MIGRATOR.iter().map(|m| m.version).max().unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?1")
        .bind(latest)
        .execute(&library.pool)
        .await
        .unwrap();
    library.close().await;

    let result = LocalLibrary::open(dir.path()).await;
    assert!(matches!(result, Err(Error::SchemaOutdated)), "{result:?}");
}

#[tokio::test]
async fn migrate_then_open() {
    let dir = TempDir::new().unwrap();
    plain_database(&dir.path().join(DATABASE_FILE)).await;

    LocalLibrary::migrate(dir.path()).await.unwrap();
    let library = LocalLibrary::open(dir.path()).await.unwrap();
    // The table the database had before stays.
    let notes: i64 = sqlx::query_scalar("SELECT count(*) FROM note")
        .fetch_one(&library.pool)
        .await
        .unwrap();
    assert_eq!(notes, 1);
    library.close().await;

    // A second migration has nothing to do.
    LocalLibrary::migrate(dir.path()).await.unwrap();
}

#[tokio::test]
async fn open_a_library_from_a_newer_version_is_too_new() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (999999, 'from the future', TRUE, x'00', 0)",
    )
    .execute(&library.pool)
    .await
    .unwrap();
    library.close().await;

    let result = LocalLibrary::open(dir.path()).await;
    assert!(matches!(result, Err(Error::SchemaTooNew)), "{result:?}");
    let result = LocalLibrary::migrate(dir.path()).await;
    assert!(matches!(result, Err(Error::SchemaTooNew)), "{result:?}");
}

#[tokio::test]
async fn open_a_library_with_a_changed_migration_is_a_mismatch() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = x'00' WHERE version = 1")
        .execute(&library.pool)
        .await
        .unwrap();
    library.close().await;

    let result = LocalLibrary::open(dir.path()).await;
    assert!(
        matches!(result, Err(Error::SchemaMismatch { version: 1 })),
        "{result:?}"
    );
}

#[tokio::test]
async fn create_refuses_an_existing_library() {
    let dir = TempDir::new().unwrap();
    LocalLibrary::create(dir.path())
        .await
        .unwrap()
        .close()
        .await;

    let result = LocalLibrary::create(dir.path()).await;
    assert!(
        matches!(&result, Err(Error::AlreadyExists(path)) if *path == dir.path().join(DATABASE_FILE)),
        "{result:?}"
    );
}

#[tokio::test]
async fn open_and_migrate_without_a_database_are_not_found() {
    let dir = TempDir::new().unwrap();
    let expected = dir.path().join(DATABASE_FILE);

    let result = LocalLibrary::open(dir.path()).await;
    assert!(
        matches!(&result, Err(Error::NotFound(path)) if *path == expected),
        "{result:?}"
    );
    let result = LocalLibrary::migrate(dir.path()).await;
    assert!(
        matches!(&result, Err(Error::NotFound(path)) if *path == expected),
        "{result:?}"
    );
    assert!(!expected.exists());
}

/// The names of the files in a directory, sorted.
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn open_an_outdated_wal_library_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let library = LocalLibrary::create(dir.path()).await.unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations")
        .execute(&library.pool)
        .await
        .unwrap();
    // Closing checkpoints the WAL and removes the -wal and -shm files.
    library.close().await;
    let path = dir.path().join(DATABASE_FILE);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(file_names(dir.path()), [DATABASE_FILE]);

    let result = LocalLibrary::open(dir.path()).await;
    assert!(matches!(result, Err(Error::SchemaOutdated)), "{result:?}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "open changed the file"
    );
    assert_eq!(
        file_names(dir.path()),
        [DATABASE_FILE],
        "open created files next to the database"
    );
}

#[tokio::test]
async fn open_reads_migrations_still_in_the_wal() {
    let dir = TempDir::new().unwrap();
    // While the creator stays open, the migrations are in the -wal file and
    // not yet in the database file, which alone would read as outdated.
    let creator = LocalLibrary::create(dir.path()).await.unwrap();
    assert!(dir.path().join(format!("{DATABASE_FILE}-wal")).exists());

    let opened = LocalLibrary::open(dir.path()).await;
    assert!(opened.is_ok(), "{opened:?}");
    creator.close().await;
}
