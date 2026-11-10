use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
};
use sqlx::{ConnectOptions, Connection};

use crate::error::Error;

/// The name of the database file in a library's directory.
pub const DATABASE_FILE: &str = "library.db";

/// The collation that orders text ignoring case, as the
/// [`LibraryBackend`](terra_core::LibraryBackend) contract asks: it compares
/// the Unicode lower-case forms. Every connection registers it; the schema
/// never names it, so other SQLite tools can still read the database.
pub(crate) const NOCASE: &str = "unicode_nocase";

/// The table where SQLx records the migrations it applied. It is SQLx's
/// default name, which `migrate!` uses unless `sqlx.toml` changes it.
const MIGRATIONS_TABLE: &str = "_sqlx_migrations";

pub(crate) static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// A music library on this machine: an SQLite database in a directory.
///
/// Nothing happens when a `LocalLibrary` is built except opening the
/// database; the schema changes only through [`create`](Self::create) and
/// [`migrate`](Self::migrate). The methods need a Tokio runtime.
#[derive(Debug, Clone)]
pub struct LocalLibrary {
    pub(crate) pool: SqlitePool,
}

impl LocalLibrary {
    /// Creates a library in `dir`, which is created if it does not exist:
    /// the database file [`DATABASE_FILE`] with every migration applied.
    ///
    /// # Errors
    ///
    /// [`Error::AlreadyExists`] if `dir` already holds a database file. If a
    /// migration fails, the file stays, and [`migrate`](Self::migrate)
    /// finishes the job.
    pub async fn create(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join(DATABASE_FILE);
        // Creating the file first, and only if it does not exist, keeps two
        // calls from both creating the same library. SQLite takes an empty
        // file as an empty database.
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::AlreadyExists(path));
            }
            Err(err) => return Err(err.into()),
        }
        let pool = connect(&path).await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    /// Applies the migrations the library in `dir` lacks. A library that has
    /// them all is left as it is.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if `dir` holds no database file,
    /// [`Error::SchemaTooNew`] if a newer version of terra made it, and
    /// [`Error::SchemaMismatch`] if an applied migration differs from this
    /// version's.
    pub async fn migrate(dir: impl AsRef<Path>) -> Result<(), Error> {
        let path = existing_database(dir.as_ref())?;
        let pool = connect(&path).await?;
        let result = MIGRATOR.run(&pool).await;
        pool.close().await;
        Ok(result?)
    }

    /// Opens the library in `dir`.
    ///
    /// It first reads, without writing, which migrations the database has.
    /// When any is missing it returns [`Error::SchemaOutdated`] and leaves
    /// the file as it was.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if `dir` holds no database file,
    /// [`Error::SchemaOutdated`] if it needs [`migrate`](Self::migrate),
    /// [`Error::SchemaTooNew`] if a newer version of terra made it, and
    /// [`Error::SchemaMismatch`] if an applied migration differs from this
    /// version's.
    pub async fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let path = existing_database(dir.as_ref())?;
        let mut conn = SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .connect()
            .await?;
        let checked = check_schema(&mut conn).await;
        conn.close().await?;
        checked?;
        Ok(Self {
            pool: connect(&path).await?,
        })
    }

    /// Closes the library's connections, waiting for the ones in use.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

fn existing_database(dir: &Path) -> Result<PathBuf, Error> {
    let path = dir.join(DATABASE_FILE);
    if path.try_exists()? {
        Ok(path)
    } else {
        Err(Error::NotFound(path))
    }
}

/// Opens a pool on an existing database, with foreign keys on and the
/// journal in WAL mode.
async fn connect(path: &Path) -> Result<SqlitePool, Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .collation(NOCASE, |a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    Ok(SqlitePoolOptions::new().connect_with(options).await?)
}

/// Compares the migrations the database records with this version's.
async fn check_schema(conn: &mut SqliteConnection) -> Result<(), Error> {
    let recorded: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(MIGRATIONS_TABLE)
    .fetch_one(&mut *conn)
    .await?;
    if !recorded {
        return Err(Error::SchemaOutdated);
    }
    if let Some(version) = conn.dirty_version(MIGRATIONS_TABLE).await? {
        return Err(Error::SchemaMismatch { version });
    }
    let applied = conn.list_applied_migrations(MIGRATIONS_TABLE).await?;
    for migration in &applied {
        let known = MIGRATOR
            .iter()
            .find(|known| known.version == migration.version)
            .ok_or(Error::SchemaTooNew)?;
        if known.checksum != migration.checksum {
            return Err(Error::SchemaMismatch {
                version: migration.version,
            });
        }
    }
    let pending = MIGRATOR.iter().any(|known| {
        !known.migration_type.is_down_migration()
            && !applied
                .iter()
                .any(|migration| migration.version == known.version)
    });
    if pending {
        return Err(Error::SchemaOutdated);
    }
    Ok(())
}
