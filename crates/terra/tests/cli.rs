use std::process::{Command, Output, Stdio};

fn terra(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_terra"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run the terra binary")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_prints_name_and_version() {
    let output = terra(&["--version"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "terra 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn help_lists_the_tui_command() {
    let output = terra(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: terra <COMMAND>"), "{stdout}");
    assert!(stdout.contains("tui"), "{stdout}");
}

#[test]
fn no_arguments_print_usage() {
    let output = terra(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("Usage: terra <COMMAND>"));
}

#[test]
fn unknown_arguments_print_usage() {
    for args in [&["play"][..], &["--verbose"], &["tui", "--demo", "extra"]] {
        let output = terra(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert!(stderr(&output).contains("Usage: terra"), "args: {args:?}");
    }
}

#[test]
fn tui_requires_demo_until_the_local_library_exists() {
    let output = terra(&["tui"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = stderr(&output);
    assert!(stderr.contains("--demo"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_prints_usage() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let output = Command::new(env!("CARGO_BIN_EXE_terra"))
        .arg(OsStr::from_bytes(b"\xff"))
        .output()
        .expect("run the terra binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("Usage: terra"));
}
