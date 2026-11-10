//! The command line: its arguments, where the library is, and what each
//! command does.
//!
//! `docs/decisions/0003-cli-and-library-location.md` records the choices.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use etcetera::BaseStrategy;
use terra_storage::{Error as StorageError, LocalLibrary, ScanProgress, ScanReport};
use tokio::runtime::Runtime;

use crate::backend::Backend;
use crate::tui::{self, App};

/// The environment variable that names the library directory.
pub const LIBRARY_ENV: &str = "TERRA_LIBRARY";

/// A keyboard-first music library and player.
///
/// Without a command, terra opens the library in the terminal UI.
#[derive(Debug, Parser)]
#[command(name = "terra", version)]
pub struct Cli {
    /// The library directory [env: TERRA_LIBRARY] [default:
    /// $XDG_DATA_HOME/terra]
    #[arg(long, global = true, value_name = "DIR")]
    library: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create the library, or apply the migrations it lacks.
    Init,
    /// Manage the folders the library scans.
    #[command(subcommand)]
    Root(RootCommand),
    /// Read the library's folders into the library.
    Scan,
    /// Browse the library in the terminal (the default).
    Tui(TuiArgs),
}

#[derive(Debug, Subcommand)]
enum RootCommand {
    /// Add a folder of audio files.
    Add {
        /// The folder.
        dir: PathBuf,
    },
    /// List the folders.
    List,
    /// Remove a folder and the tracks under it.
    Remove {
        /// The folder.
        dir: PathBuf,
    },
}

#[derive(Debug, Args)]
struct TuiArgs {
    /// Use the synthetic sample library instead of the local library.
    #[arg(long)]
    demo: bool,
}

/// Why a command failed.
#[derive(Debug)]
pub enum Error {
    /// Neither `--library` nor the environment names the library, and there
    /// is no home directory to put it in.
    NoLibraryDir,
    /// The library in this directory failed.
    Library(PathBuf, StorageError),
    /// `root add` could not add this folder.
    AddRoot(PathBuf, StorageError),
    /// `root remove` was given a folder that is not a root.
    NotARoot(PathBuf),
    /// The terminal or the runtime failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLibraryDir => write!(
                f,
                "cannot find a home directory for the library; pass --library DIR or set {LIBRARY_ENV}"
            ),
            Self::Library(dir, StorageError::NotFound(_)) => write!(
                f,
                "there is no library at {}; run `terra init` to create it",
                dir.display()
            ),
            Self::Library(dir, StorageError::SchemaOutdated) => write!(
                f,
                "the library at {} needs a migration; run `terra init` to apply it",
                dir.display()
            ),
            Self::Library(_, error) => write_chain(f, error),
            Self::AddRoot(dir, error) => {
                write!(f, "cannot add {}: ", dir.display())?;
                write_chain(f, error)
            }
            Self::NotARoot(dir) => write!(f, "{} is not a library root", dir.display()),
            Self::Io(error) => write_chain(f, error),
        }
    }
}

impl std::error::Error for Error {}

/// Writes an error and its sources, separated by colons.
fn write_chain(f: &mut fmt::Formatter<'_>, error: &dyn std::error::Error) -> fmt::Result {
    write!(f, "{error}")?;
    let mut source = error.source();
    while let Some(error) = source {
        write!(f, ": {error}")?;
        source = error.source();
    }
    Ok(())
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Cli {
    /// Runs the command.
    pub fn run(self) -> Result<(), Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let command = self
            .command
            .unwrap_or(Command::Tui(TuiArgs { demo: false }));
        if let Command::Tui(TuiArgs { demo: true }) = command {
            return Ok(tui::run_demo(&runtime)?);
        }
        let dir = library_dir(self.library, env::var_os(LIBRARY_ENV), || {
            etcetera::choose_base_strategy()
                .ok()
                .map(|strategy| strategy.data_dir())
        })?;
        let storage = |error| Error::Library(dir.clone(), error);
        match command {
            Command::Init => runtime.block_on(init(&dir)).map_err(storage),
            Command::Root(command) => runtime.block_on(root(&dir, command)),
            Command::Scan => runtime.block_on(scan(&dir)).map_err(storage),
            Command::Tui(_) => run_tui(&runtime, &dir),
        }
    }
}

/// The library directory: `given` by `--library`, or else the value of
/// [`LIBRARY_ENV`] unless it is empty, or else `terra` in the data
/// directory, which `data_dir` looks up only when it is needed.
fn library_dir(
    given: Option<PathBuf>,
    env: Option<OsString>,
    data_dir: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, Error> {
    if let Some(dir) = given {
        return Ok(dir);
    }
    if let Some(dir) = env.filter(|dir| !dir.is_empty()) {
        return Ok(dir.into());
    }
    data_dir()
        .map(|dir| dir.join("terra"))
        .ok_or(Error::NoLibraryDir)
}

/// Creates the library in `dir`, or brings it up to date. Running it again
/// changes nothing.
async fn init(dir: &Path) -> Result<(), StorageError> {
    match LocalLibrary::open(dir).await {
        Ok(library) => {
            library.close().await;
            println!("The library at {} is up to date.", dir.display());
            return Ok(());
        }
        Err(StorageError::NotFound(_)) => {}
        Err(StorageError::SchemaOutdated) => {
            LocalLibrary::migrate(dir).await?;
            println!("Migrated the library at {}.", dir.display());
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    match LocalLibrary::create(dir).await {
        Ok(library) => {
            library.close().await;
            println!("Created a library at {}.", dir.display());
        }
        // Another `terra init` created it in the meantime.
        Err(StorageError::AlreadyExists(_)) => {
            LocalLibrary::migrate(dir).await?;
            println!("The library at {} is up to date.", dir.display());
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

async fn root(dir: &Path, command: RootCommand) -> Result<(), Error> {
    let library = LocalLibrary::open(dir)
        .await
        .map_err(|error| Error::Library(dir.to_owned(), error))?;
    let result = match command {
        RootCommand::Add { dir } => match library.add_root(&dir).await {
            Ok(path) => {
                println!("Added {}.", path.display());
                println!("Run `terra scan` to read it into the library.");
                Ok(())
            }
            Err(error) => Err(Error::AddRoot(dir, error)),
        },
        RootCommand::List => match library.list_roots().await {
            Ok(roots) => {
                for root in roots {
                    println!("{}", root.display());
                }
                Ok(())
            }
            Err(error) => Err(Error::Library(dir.to_owned(), error)),
        },
        RootCommand::Remove { dir: root } => match library.remove_root(&root).await {
            Ok(true) => {
                println!("Removed {}.", root.display());
                Ok(())
            }
            Ok(false) => Err(Error::NotARoot(root)),
            Err(error) => Err(Error::Library(dir.to_owned(), error)),
        },
    };
    library.close().await;
    result
}

/// Scans the library's roots. A file that cannot be read is listed, and
/// the scan goes on; only a failure of the library itself is an error.
async fn scan(dir: &Path) -> Result<(), StorageError> {
    let library = LocalLibrary::open(dir).await?;
    let roots = match library.list_roots().await {
        Ok(roots) => roots,
        Err(error) => {
            library.close().await;
            return Err(error);
        }
    };
    if roots.is_empty() {
        library.close().await;
        println!("The library has no folders. Add one with `terra root add <DIR>`.");
        return Ok(());
    }
    let mut progress = Progress::new();
    let report = library.scan(|step| progress.show(&step)).await;
    progress.finish();
    library.close().await;
    let report = report?;
    for (path, reason) in &report.errors {
        eprintln!("terra: could not read {}: {reason}", path.display());
    }
    println!("{}", summary(&report));
    Ok(())
}

/// One line that sums up a scan.
fn summary(report: &ScanReport) -> String {
    format!(
        "Scanned {}: {} added, {} updated, {} moved, {} unchanged, {} failed; {} missing.",
        files(
            report.added + report.updated + report.moved + report.unchanged + report.errors.len()
        ),
        report.added,
        report.updated,
        report.moved,
        report.unchanged,
        report.errors.len(),
        report.missing,
    )
}

fn files(count: usize) -> String {
    match count {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    }
}

/// Shows a scan's progress on standard error: a count rewritten in place
/// on a terminal, otherwise a line per file with its path.
struct Progress {
    terminal: bool,
    shown: bool,
}

impl Progress {
    fn new() -> Self {
        Self {
            terminal: io::stderr().is_terminal(),
            shown: false,
        }
    }

    fn show(&mut self, step: &ScanProgress) {
        let mut stderr = io::stderr().lock();
        // Progress is a courtesy: a closed standard error does not stop
        // the scan.
        let _ = if self.terminal {
            // Return to the start of the line and clear it. A path could
            // wrap the line, so only the count is shown.
            write!(stderr, "\r\x1b[2KScanning {}/{}", step.done, step.total)
                .and_then(|()| stderr.flush())
        } else {
            let path = step.path.display();
            writeln!(stderr, "[{}/{}] {path}", step.done, step.total)
        };
        self.shown = true;
    }

    fn finish(&self) {
        if self.terminal && self.shown {
            let _ = writeln!(io::stderr());
        }
    }
}

/// Opens the local library in the TUI.
fn run_tui(runtime: &Runtime, dir: &Path) -> Result<(), Error> {
    let library = runtime
        .block_on(LocalLibrary::open(dir))
        .map_err(|error| Error::Library(dir.to_owned(), error))?;
    let backend = Backend::Local(library);
    let result = tui::run(runtime, &backend, App::new(dir.display().to_string()));
    runtime.block_on(backend.close());
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir() -> Option<PathBuf> {
        Some(PathBuf::from("/data"))
    }

    #[test]
    fn library_dir_prefers_the_flag_then_the_environment_then_the_data_directory() {
        let flag = Some(PathBuf::from("/flag"));
        let env = Some(OsString::from("/env"));
        let dir = |flag, env| library_dir(flag, env, data_dir).unwrap();
        assert_eq!(dir(flag.clone(), env.clone()), Path::new("/flag"));
        assert_eq!(dir(flag, None), Path::new("/flag"));
        assert_eq!(dir(None, env), Path::new("/env"));
        assert_eq!(dir(None, Some(OsString::new())), Path::new("/data/terra"));
        assert_eq!(dir(None, None), Path::new("/data/terra"));
    }

    #[test]
    fn library_dir_looks_up_the_data_directory_only_when_needed() {
        let unused = || panic!("the data directory was looked up");
        let dir = library_dir(Some(PathBuf::from("/flag")), None, unused).unwrap();
        assert_eq!(dir, Path::new("/flag"));
        let dir = library_dir(None, Some(OsString::from("/env")), unused).unwrap();
        assert_eq!(dir, Path::new("/env"));
    }

    #[test]
    fn library_dir_without_a_data_directory_is_an_error() {
        let error = library_dir(None, None, || None).unwrap_err();
        assert!(matches!(error, Error::NoLibraryDir));
        assert!(error.to_string().contains("--library"));
    }

    #[test]
    fn summary_counts_every_file() {
        let report = ScanReport {
            added: 1,
            updated: 2,
            moved: 3,
            missing: 4,
            unchanged: 5,
            errors: vec![(PathBuf::from("a"), "bad".to_owned())],
        };
        assert_eq!(
            summary(&report),
            "Scanned 12 files: 1 added, 2 updated, 3 moved, 5 unchanged, 1 failed; 4 missing."
        );
        let report = ScanReport {
            added: 1,
            ..ScanReport::default()
        };
        assert!(summary(&report).starts_with("Scanned 1 file: "));
    }
}
