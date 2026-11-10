mod tui;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

/// A keyboard-first music library and player.
#[derive(Debug, Parser)]
#[command(name = "terra", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Browse the library in the terminal.
    Tui(TuiArgs),
}

#[derive(Debug, Args)]
struct TuiArgs {
    /// Use the synthetic sample library. Required until the local library
    /// exists.
    #[arg(long, required = true)]
    demo: bool,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        // Clap rejects `tui` without `--demo`.
        Command::Tui(TuiArgs { demo: _ }) => match tui::run_demo() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("terra: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
