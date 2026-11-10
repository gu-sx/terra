//! The `terra` binary, run with its library in a temporary directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

/// The audio fixtures of `terra-storage`, which the scan reads.
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../terra-storage/tests/fixtures"
);

/// The fixtures and where each is copied in the music folder.
const FILES: &[(&str, &str)] = &[
    ("tone.mp3", "Album A/01 tone.mp3"),
    ("tone.flac", "Album A/02 tone.flac"),
    ("tone.opus", "Album B/tone.opus"),
    ("tone-aac.m4a", "Album B/tone-aac.m4a"),
    ("tone-alac.m4a", "tone-alac.m4a"),
];

/// A temporary directory for the library, which `TERRA_LIBRARY` names, and
/// one for music.
struct Setup {
    dir: TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
        }
    }

    fn library(&self) -> PathBuf {
        self.dir.path().join("library")
    }

    fn music(&self) -> PathBuf {
        self.dir.path().join("music")
    }

    /// Copies every fixture into the music folder.
    fn copy_fixtures(&self) {
        for (fixture, rel_path) in FILES {
            let to = self.music().join(rel_path);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(Path::new(FIXTURES).join(fixture), &to).unwrap();
        }
    }

    /// Runs terra with `TERRA_LIBRARY` set to the library directory.
    fn terra(&self, args: &[&str]) -> Output {
        run(terra().env("TERRA_LIBRARY", self.library()).args(args))
    }

    /// Runs terra and asserts that it succeeded; returns its standard
    /// output.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.terra(args);
        assert!(
            output.status.success(),
            "args: {args:?}\n{}",
            stderr(&output)
        );
        stdout(&output)
    }
}

/// The terra binary, with no library directory or data directory from the
/// environment of the tests.
fn terra() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terra"));
    command
        .env_remove("TERRA_LIBRARY")
        .env_remove("XDG_DATA_HOME")
        .stdin(Stdio::null());
    command
}

fn run(command: &mut Command) -> Output {
    command.output().expect("run the terra binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_prints_name_and_version() {
    let output = run(terra().arg("--version"));
    assert!(output.status.success());
    assert_eq!(stdout(&output), "terra 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn help_lists_every_command() {
    let output = run(terra().arg("--help"));
    assert!(output.status.success());
    let stdout = stdout(&output);
    assert!(
        stdout.contains("Usage: terra [OPTIONS] [COMMAND]"),
        "{stdout}"
    );
    for command in ["init", "root", "scan", "tui"] {
        assert!(
            stdout
                .lines()
                .any(|line| line.trim_start().starts_with(command)),
            "{command}: {stdout}"
        );
    }
    assert!(stdout.contains("--library <DIR>"), "{stdout}");
    assert!(stdout.contains("TERRA_LIBRARY"), "{stdout}");

    let output = run(terra().args(["root", "--help"]));
    assert!(output.status.success());
    let stdout = self::stdout(&output);
    for command in ["add", "list", "remove"] {
        assert!(
            stdout
                .lines()
                .any(|line| line.trim_start().starts_with(command)),
            "{command}: {stdout}"
        );
    }
}

#[test]
fn unknown_arguments_print_usage() {
    for args in [
        &["play"][..],
        &["--verbose"],
        &["tui", "--demo", "extra"],
        &["root"],
        &["root", "add"],
        &["scan", "extra"],
    ] {
        let output = run(terra().args(args));
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert!(stderr(&output).contains("Usage: terra"), "args: {args:?}");
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_prints_usage() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let output = run(terra().arg(OsStr::from_bytes(b"\xff")));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("Usage: terra"));
}

#[test]
fn init_creates_the_library_and_then_changes_nothing() {
    let setup = Setup::new();
    let database = setup.library().join("library.db");

    let stdout = setup.ok(&["init"]);
    assert!(stdout.starts_with("Created a library at "), "{stdout}");
    assert!(database.is_file());
    let created = fs::read(&database).unwrap();

    let stdout = setup.ok(&["init"]);
    assert!(stdout.contains("is up to date"), "{stdout}");
    assert_eq!(fs::read(&database).unwrap(), created);
}

#[test]
fn library_flag_overrides_the_environment() {
    let setup = Setup::new();
    let flagged = setup.dir.path().join("flagged");
    let output = setup.terra(&["--library", flagged.to_str().unwrap(), "init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(flagged.join("library.db").is_file());
    assert!(!setup.library().exists());
}

#[test]
fn library_defaults_to_the_xdg_data_directory() {
    let setup = Setup::new();
    let data = setup.dir.path().join("data");
    let output = run(terra().env("XDG_DATA_HOME", &data).arg("init"));
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(data.join("terra/library.db").is_file());

    // An empty TERRA_LIBRARY counts as unset.
    let output = run(terra()
        .env("XDG_DATA_HOME", &data)
        .env("TERRA_LIBRARY", "")
        .arg("init"));
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("is up to date"));
}

#[test]
fn root_add_scan_and_rescan() {
    let setup = Setup::new();
    setup.copy_fixtures();
    setup.ok(&["init"]);
    let music = setup.music();
    let music = music.to_str().unwrap();

    let added = setup.ok(&["root", "add", music]);
    let resolved = fs::canonicalize(music).unwrap();
    assert!(
        added.starts_with(&format!("Added {}.", resolved.display())),
        "{added}"
    );
    assert_eq!(
        setup.ok(&["root", "list"]),
        format!("{}\n", resolved.display())
    );

    let output = setup.terra(&["scan"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Scanned 5 files: 5 added, 0 updated, 0 moved, 0 unchanged, 0 failed; 0 missing.\n"
    );
    // Standard error is not a terminal, so progress is a line per file.
    let progress = stderr(&output);
    assert_eq!(progress.lines().count(), 5, "{progress}");
    assert!(progress.lines().last().unwrap().starts_with("[5/5] "));

    assert_eq!(
        setup.ok(&["scan"]),
        "Scanned 5 files: 0 added, 0 updated, 0 moved, 5 unchanged, 0 failed; 0 missing.\n"
    );

    let removed = setup.ok(&["root", "remove", music]);
    assert!(removed.starts_with("Removed "), "{removed}");
    assert_eq!(setup.ok(&["root", "list"]), "");
}

#[test]
fn scan_lists_unreadable_files_and_succeeds() {
    let setup = Setup::new();
    setup.copy_fixtures();
    fs::write(setup.music().join("broken.mp3"), b"not audio").unwrap();
    setup.ok(&["init"]);
    setup.ok(&["root", "add", setup.music().to_str().unwrap()]);

    let output = setup.terra(&["scan"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Scanned 6 files: 5 added, 0 updated, 0 moved, 0 unchanged, 1 failed; 0 missing.\n"
    );
    let stderr = stderr(&output);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("terra: could not read ") && line.contains("broken.mp3")),
        "{stderr}"
    );
}

#[test]
fn scan_without_roots_says_how_to_add_one() {
    let setup = Setup::new();
    setup.ok(&["init"]);
    let stdout = setup.ok(&["scan"]);
    assert!(stdout.contains("terra root add"), "{stdout}");
}

#[test]
fn root_remove_of_a_folder_that_is_not_a_root_fails() {
    let setup = Setup::new();
    setup.ok(&["init"]);
    let output = setup.terra(&["root", "remove", setup.music().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("is not a library root"));
}

#[test]
fn root_add_of_a_missing_folder_fails() {
    let setup = Setup::new();
    setup.ok(&["init"]);
    let output = setup.terra(&["root", "add", setup.music().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr(&output);
    assert!(stderr.starts_with("terra: cannot add "), "{stderr}");
}

#[test]
fn commands_on_an_uninitialised_library_say_to_run_init() {
    let setup = Setup::new();
    for args in [
        &[][..],
        &["tui"],
        &["scan"],
        &["root", "list"],
        &["root", "add", "."],
    ] {
        let output = setup.terra(args);
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        let stderr = stderr(&output);
        assert!(stderr.contains("terra init"), "args: {args:?}: {stderr}");
        assert!(!setup.library().exists(), "args: {args:?}");
    }
}

#[test]
fn init_migrates_a_library_that_needs_it() {
    let setup = Setup::new();
    // A library whose creation stopped before the migrations ran.
    fs::create_dir_all(setup.library()).unwrap();
    fs::write(setup.library().join("library.db"), b"").unwrap();

    let output = setup.terra(&["scan"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr(&output);
    assert!(stderr.contains("needs a migration"), "{stderr}");
    assert!(stderr.contains("terra init"), "{stderr}");

    let stdout = setup.ok(&["init"]);
    assert!(stdout.starts_with("Migrated the library at "), "{stdout}");
    let stdout = setup.ok(&["root", "list"]);
    assert_eq!(stdout, "");
}
