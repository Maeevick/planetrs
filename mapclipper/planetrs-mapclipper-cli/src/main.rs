//! The entry point, which parses, runs, and turns a failure into a message and an exit code.

use std::process::ExitCode;

use clap::Parser;
use planetrs_mapclipper_cli::cli::Cli;
use planetrs_mapclipper_cli::run;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");

            let mut cause = std::error::Error::source(&error);
            while let Some(source) = cause {
                eprintln!("  caused by: {source}");
                cause = std::error::Error::source(source);
            }

            if let Some(step) = error.next_step() {
                eprintln!("\n{step}");
            }

            ExitCode::FAILURE
        }
    }
}
