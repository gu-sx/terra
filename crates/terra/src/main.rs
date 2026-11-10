mod backend;
mod cli;
mod tui;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    match cli::Cli::parse().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terra: {error}");
            ExitCode::FAILURE
        }
    }
}
