//! Telling a reader how far a long run has got, on stderr and only when someone is watching.
//!
//! Progress never goes to stdout, where answers do. When stderr is not a terminal there is nobody
//! to reassure and the carriage returns would only fill a log, so nothing is drawn at all.

use std::cell::Cell;
use std::io::{IsTerminal, Write};

/// How many sites pass between two redraws: a few times a second at worst, which costs nothing
/// beside the lookups.
const EVERY: usize = 250;

/// A line on stderr redrawn in place, or nothing when stderr is not a terminal.
pub struct Progress {
    watching: bool,
    drawn: Cell<usize>,
}

impl Progress {
    /// Draws on stderr when it is a terminal and the run is not quiet.
    pub fn to_stderr(quiet: bool) -> Self {
        Self {
            watching: !quiet && std::io::stderr().is_terminal(),
            drawn: Cell::new(0),
        }
    }

    /// Says how many of the sites are done, every few hundred and at the last one.
    pub fn sites(&self, done: usize, total: usize) {
        if self.watching && is_worth_drawing(done, total) {
            self.draw(&sites_line(done, total));
        }
    }

    /// Says what is happening, for a step with no count.
    pub fn note(&self, line: &str) {
        if self.watching {
            self.draw(line);
        }
    }

    /// Wipes the line, so whatever is written next starts clean.
    pub fn clear(&self) {
        let width = self.drawn.replace(0);
        if self.watching && width > 0 {
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, "\r{}\r", " ".repeat(width));
            let _ = stderr.flush();
        }
    }

    fn draw(&self, line: &str) {
        let mut stderr = std::io::stderr().lock();
        let padding = self.drawn.get().saturating_sub(line.chars().count());
        let _ = write!(stderr, "\r{line}{}", " ".repeat(padding));
        let _ = stderr.flush();
        self.drawn.set(line.chars().count());
    }
}

fn is_worth_drawing(done: usize, total: usize) -> bool {
    done.is_multiple_of(EVERY) || done == total
}

fn sites_line(done: usize, total: usize) -> String {
    format!("geocoding {done} / {total} sites")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_count_between_two_redraws_when_asked_then_it_is_not_drawn() {
        assert!(!is_worth_drawing(251, 3_805));
    }

    #[test]
    fn given_the_last_site_when_asked_then_it_is_drawn_whatever_the_count() {
        assert!(is_worth_drawing(3_805, 3_805));
    }

    #[test]
    fn given_a_count_when_written_then_it_reads_done_of_total() {
        assert_eq!(sites_line(500, 3_805), "geocoding 500 / 3805 sites");
    }

    #[test]
    fn given_a_quiet_run_when_made_then_nothing_is_watched() {
        assert!(!Progress::to_stderr(true).watching);
    }
}
