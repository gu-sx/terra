use std::process::{Command, Output};

fn terra(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_terra"))
        .args(args)
        .output()
        .expect("run the terra binary")
}

#[test]
fn version_prints_name_and_version() {
    let output = terra(&["--version"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "terra 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn no_arguments_print_usage() {
    let output = terra(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "usage: terra --version\n"
    );
}

#[test]
fn unknown_arguments_print_usage() {
    for args in [&["--help"][..], &["play"], &["--version", "extra"]] {
        let output = terra(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "usage: terra --version\n",
            "args: {args:?}"
        );
    }
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
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "usage: terra --version\n"
    );
}
