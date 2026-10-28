//! `planetrs-mapclipper`: a small illustrated map cut out of the OpenStreetMap planet file, offline.

use std::process::ExitCode;

use clap::Parser;
use planetrs_mapclipper::cli::arguments::Cli;
use planetrs_mapclipper::cli::run;

fn main() -> ExitCode {
    planetrs_cli::finish(run(Cli::parse()))
}
