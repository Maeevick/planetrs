//! The `planetrs-geocoder` command, behind the `cli` feature: the arguments clap reads, what each
//! command does, and the site files it reads and writes.
//!
//! The binary parses the command line, hands it to [`run`], and ends as every planetrs command does,
//! through [`planetrs_cli::finish`]: answers on stdout, everything else on stderr. The module is
//! public for the binary and its tests, and promises no stable interface.

pub mod arguments;
pub mod commands;
pub mod sites;

pub use commands::run;
