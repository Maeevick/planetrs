//! The `planetrs-mapclipper` command, behind the `cli` feature: the arguments clap reads, what each
//! command does, and how it reports.
//!
//! The binary parses the command line and hands it to [`run`]: maps and harvests go to the files
//! asked for, or to stdout for `-`, and everything said about them goes to stderr. The module is
//! public for the binary and its tests, and promises no stable interface.

pub mod arguments;
pub mod commands;
pub mod report;

pub use commands::run;
