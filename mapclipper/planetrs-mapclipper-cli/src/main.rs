//! `planetrs-mapclipper`: a small illustrated map cut out of the OpenStreetMap planet file, offline.

use std::process::ExitCode;

use clap::Parser;
use planetrs_mapclipper_cli::cli::Cli;
use planetrs_mapclipper_cli::run;

fn main() -> ExitCode {
    planetrs_cli::finish(run(Cli::parse()))
}
