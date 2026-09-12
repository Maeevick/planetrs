//! `planetrs-geocoder`: a coordinate for a place or a postal address, from OpenStreetMap data alone.

use std::process::ExitCode;

use clap::Parser;
use planetrs_geocoder_cli::cli::Cli;
use planetrs_geocoder_cli::{exit_code_of, is_broken_pipe, run};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(exit_code_of(&error))
        }
    }
}
