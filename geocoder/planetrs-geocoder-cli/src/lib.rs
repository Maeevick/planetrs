//! The `planetrs-geocoder` command line, kept in a library so each piece is tested without a process.
//!
//! The binary parses the command line, hands it to [`run`], and ends as every planetrs command does,
//! through [`planetrs_cli::finish`]: answers on stdout, everything else on stderr.

pub mod cli;
pub mod commands;
pub mod download;
pub mod sites;

pub use commands::run;
