//! The `planetrs-geocoder` command line, kept in a library so each piece is tested without a process.
//!
//! The binary parses the command line, hands it to [`run`], and turns the outcome into an exit code
//! with [`exit_code_of`]: answers on stdout, everything else on stderr.

pub mod cli;
pub mod commands;
pub mod data_dir;
pub mod download;
pub mod files;
pub mod progress;
pub mod sites;

use std::io;

/// The line every run that reads OpenStreetMap data ends with, on stderr.
pub const ATTRIBUTION: &str =
    "Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

pub use commands::run;

/// The exit code for a failed run: 2 when the command or its sites were malformed, 1 otherwise.
pub fn exit_code_of(error: &anyhow::Error) -> u8 {
    let malformed = error.chain().any(|cause| {
        cause.is::<commands::UsageError>() || cause.is::<sites::SitesError>() || cause.is::<clap::Error>()
    });
    if malformed { 2 } else { 1 }
}

/// Whether a run failed only because the reader of its stdout went away, as `| head` does, which
/// ends it quietly.
pub fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|cause| cause.kind() == io::ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_malformed_site_file_when_failed_then_the_exit_code_is_two() {
        let error = anyhow::Error::new(sites::SitesError::Empty).context("reading sites.csv");

        assert_eq!(exit_code_of(&error), 2);
    }

    #[test]
    fn given_a_usage_mistake_when_failed_then_the_exit_code_is_two() {
        assert_eq!(exit_code_of(&commands::UsageError("no site".into()).into()), 2);
    }

    #[test]
    fn given_any_other_failure_when_failed_then_the_exit_code_is_one() {
        assert_eq!(exit_code_of(&anyhow::anyhow!("no index")), 1);
    }

    #[test]
    fn given_a_closed_pipe_under_some_context_when_failed_then_it_is_recognised() {
        let error = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe)).context("writing answers");

        assert!(is_broken_pipe(&error));
        assert!(!is_broken_pipe(&anyhow::anyhow!("no index")));
    }
}
