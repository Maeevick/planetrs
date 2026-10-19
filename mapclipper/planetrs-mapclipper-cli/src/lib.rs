//! The `planetrs-mapclipper` command line, kept in a library so each piece is tested without a
//! process.
//!
//! The binary parses the command line and hands it to [`run::run`]: maps and harvests go to the
//! files asked for, or to stdout for `-`, and everything said about them goes to stderr.

pub mod cli;
pub mod progress;
pub mod report;
pub mod run;
