//! Telling a reader how far a long run has got, on stderr and only when someone is watching.
//!
//! Progress never goes to stdout, where answers and maps do. When stderr is not a terminal there is
//! nobody to reassure and the carriage returns would only fill a log, so nothing is drawn at all.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A line on stderr redrawn in place, or nothing when stderr is not a terminal.
///
/// It is shared between threads as it stands, since a pass over a planet reads its blobs on all of
/// them at once.
pub struct Progress {
    watching: bool,
    drawn: AtomicUsize,
}

impl Progress {
    /// Draws on stderr when it is a terminal and the run is not quiet.
    pub fn to_stderr(quiet: bool) -> Self {
        Self {
            watching: !quiet && std::io::stderr().is_terminal(),
            drawn: AtomicUsize::new(0),
        }
    }

    /// Says where the run is, in place of whatever the line said before.
    pub fn note(&self, line: &str) {
        if self.watching {
            self.draw(line);
        }
    }

    /// Wipes the line, so whatever is written next starts clean.
    pub fn clear(&self) {
        let width = self.drawn.swap(0, Ordering::Relaxed);
        if self.watching && width > 0 {
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, "\r{}\r", " ".repeat(width));
            let _ = stderr.flush();
        }
    }

    fn draw(&self, line: &str) {
        let mut stderr = std::io::stderr().lock();
        let width = line.chars().count();
        let padding = self.drawn.swap(width, Ordering::Relaxed).saturating_sub(width);
        let _ = write!(stderr, "\r{line}{}", " ".repeat(padding));
        let _ = stderr.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_quiet_run_when_made_then_nothing_is_watched() {
        assert!(!Progress::to_stderr(true).watching);
    }
}
