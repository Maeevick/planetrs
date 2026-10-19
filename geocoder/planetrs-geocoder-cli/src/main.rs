//! `planetrs-geocoder`: a coordinate for a place or a postal address, from OpenStreetMap data alone.

use std::process::ExitCode;

use clap::Parser;
use planetrs_geocoder_cli::cli::Cli;
use planetrs_geocoder_cli::run;

fn main() -> ExitCode {
    planetrs_cli::finish(run(Cli::parse()))
}
