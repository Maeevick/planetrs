//! What the planetrs command lines share, so that both tools keep their data in the same place,
//! never write over an output, draw progress the same way and end the same way.
//!
//! Data read by either tool is © OpenStreetMap contributors, under the Open Data Commons Open
//! Database License 1.0, and every run that reads it ends with [`ATTRIBUTION`] on stderr.

#![warn(missing_docs)]

pub mod data_dir;
pub mod dataset;
pub mod download;
pub mod files;
pub mod progress;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::io;
use std::process::ExitCode;

pub use data_dir::{DataDir, NoDataDir};
pub use files::{Day, write_new};
pub use progress::Progress;

/// A size in bytes as a person reads it, in powers of a thousand to one decimal: `94.7 GB`.
pub fn size_text(bytes: u64) -> String {
    let units = ["bytes", "kB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < units.len() {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

/// The line every run that reads OpenStreetMap data ends with, on stderr.
pub const ATTRIBUTION: &str =
    "Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

/// A command asked for in a way it cannot be done, which exits 2 like a parsing error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(pub String);

/// Ends a run with the exit code its outcome deserves, saying why on stderr when it failed: 0 on
/// success or when only the reader of stdout went away, 2 when it was asked for in a way it cannot
/// be done, 1 for any other failure.
pub fn finish(outcome: anyhow::Result<()>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(exit_code_of(&error))
        }
    }
}

/// The exit code for a failed run: 2 when a [`UsageError`] caused it, 1 otherwise.
pub fn exit_code_of(error: &anyhow::Error) -> u8 {
    let asked_wrongly = error.chain().any(|cause| cause.is::<UsageError>());
    if asked_wrongly { 2 } else { 1 }
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
    fn given_a_usage_mistake_under_some_context_when_failed_then_the_exit_code_is_two() {
        let error = anyhow::Error::new(UsageError("no site".into())).context("reading sites.csv");

        assert_eq!(exit_code_of(&error), 2);
    }

    #[test]
    fn given_any_other_failure_when_failed_then_the_exit_code_is_one() {
        assert_eq!(exit_code_of(&anyhow::anyhow!("no dataset")), 1);
    }

    #[test]
    fn given_a_closed_pipe_under_some_context_when_failed_then_it_is_recognised() {
        let error = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe)).context("writing answers");

        assert!(is_broken_pipe(&error));
        assert!(!is_broken_pipe(&anyhow::anyhow!("no dataset")));
    }

    #[test]
    fn given_sizes_from_bytes_to_a_planet_when_written_then_they_read_in_powers_of_a_thousand() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(1_536), "1.5 kB");
        assert_eq!(size_text(94_746_421_723), "94.7 GB");
    }

    #[test]
    fn given_a_closed_pipe_when_finished_then_the_run_ends_as_a_success() {
        let outcome = Err(anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe)));

        assert_eq!(finish(outcome), ExitCode::SUCCESS);
    }
}
