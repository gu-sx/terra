use std::error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use sqlx::migrate::MigrateError;
use terra_core::TrackId;

/// The error [`LocalLibrary`](crate::LocalLibrary)'s lifecycle methods
/// return: creating, migrating and opening a library.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// [`create`](crate::LocalLibrary::create) found a library database
    /// already at this path.
    AlreadyExists(PathBuf),
    /// There is no library database at this path.
    NotFound(PathBuf),
    /// The database lacks migrations this version of terra applies. Run
    /// [`migrate`](crate::LocalLibrary::migrate), then open it again.
    SchemaOutdated,
    /// The database has migrations this version of terra does not know: a
    /// newer version made it.
    SchemaTooNew,
    /// A migration the database records differs from this version's, or did
    /// not finish. The database needs repair, not a migration.
    SchemaMismatch {
        /// The migration's version.
        version: i64,
    },
    /// Reading or writing the library's directory failed.
    Io(io::Error),
    /// The database failed.
    Database(Box<dyn error::Error + Send + Sync + 'static>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(path) => {
                write!(f, "a library already exists at {}", path.display())
            }
            Self::NotFound(path) => write!(f, "there is no library at {}", path.display()),
            Self::SchemaOutdated => f.write_str("the library needs a migration"),
            Self::SchemaTooNew => f.write_str("the library was made by a newer version of terra"),
            Self::SchemaMismatch { version } => write!(
                f,
                "the library's migration {version} differs from this version's or did not finish"
            ),
            Self::Io(_) => f.write_str("reading the library's directory failed"),
            Self::Database(_) => f.write_str("the library database failed"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            Self::Database(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

impl From<sqlx::Error> for Error {
    fn from(source: sqlx::Error) -> Self {
        Self::Database(source.into())
    }
}

impl From<MigrateError> for Error {
    fn from(source: MigrateError) -> Self {
        match source {
            MigrateError::VersionMissing(_) => Self::SchemaTooNew,
            MigrateError::VersionMismatch(version) | MigrateError::Dirty(version) => {
                Self::SchemaMismatch { version }
            }
            source => Self::Database(source.into()),
        }
    }
}

/// The audio file of a track is missing: the last scan did not find it.
///
/// [`resolve_audio_source`](terra_core::LibraryBackend::resolve_audio_source)
/// returns it as the source of a
/// [`LibraryError::Storage`](terra_core::LibraryError::Storage), so a caller
/// can tell it from other failures with `downcast_ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    pub track: TrackId,
    /// Where the file was.
    pub path: PathBuf,
}

impl fmt::Display for MissingFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the file of track {} is missing: {}",
            self.track,
            self.path.display()
        )
    }
}

impl error::Error for MissingFile {}
