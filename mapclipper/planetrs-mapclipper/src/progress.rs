//! Telling a reader how far a long pass has got, on stderr and only when someone is watching.
//!
//! Progress belongs on stderr, never on stdout, so that piping a rendered map into another command
//! carries the map and not a counter. When stderr is not a terminal there is nobody to reassure and
//! the carriage returns would only fill a log file, so nothing is written at all.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// How often a counter is redrawn. A blob is a tenth of a second of work at worst, so this is
/// several times a second at most and costs nothing beside the inflate.
const EVERY: usize = 512;

pub struct Progress {
    watching: bool,
    last_width: AtomicU64,
}

impl Progress {
    pub fn to_stderr() -> Self {
        Self {
            watching: std::io::stderr().is_terminal(),
            last_width: AtomicU64::new(0),
        }
    }

    pub fn silent() -> Self {
        Self {
            watching: false,
            last_width: AtomicU64::new(0),
        }
    }

    pub fn start(&self, label: &str, total: usize) {
        if self.watching && total > 0 {
            self.draw(&format!("{label}: 0 / {total} blobs"));
        }
    }

    pub fn step(&self, label: &str, done: usize, total: usize) {
        if self.watching && (done.is_multiple_of(EVERY) || done == total) {
            self.draw(&format!("{label}: {done} / {total} blobs"));
        }
    }

    pub fn done(&self, _label: &str) {
        if self.watching {
            self.clear();
        }
    }

    fn draw(&self, line: &str) {
        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "\r{line}");
        let _ = stderr.flush();
        self.last_width.store(line.len() as u64, Ordering::Relaxed);
    }

    fn clear(&self) {
        let width = self.last_width.swap(0, Ordering::Relaxed) as usize;
        if width > 0 {
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, "\r{}\r", " ".repeat(width));
            let _ = stderr.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Piping a rendered map into another command must carry the map and not a counter, so when
    /// nobody is watching nothing is written anywhere.
    #[test]
    fn when_nobody_is_watching_then_nothing_is_drawn() {
        let quiet = Progress::silent();
        quiet.start("nodes", 100);
        quiet.step("nodes", 512, 100);

        assert_eq!(quiet.last_width.load(Ordering::Relaxed), 0);
    }
}
