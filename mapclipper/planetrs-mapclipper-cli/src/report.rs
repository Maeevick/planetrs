//! Saying what happened, on stderr, so that stdout stays free for the map itself.

use std::fmt::Write as _;
use std::io::Write as _;

/// A count with its digits grouped in threes, `94 746 421 723`.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// A duration in seconds to a tenth, `58.3s`.
pub fn seconds(elapsed: std::time::Duration) -> String {
    format!("{:.1}s", elapsed.as_secs_f64())
}

/// A size in the decimal unit that keeps it under five digits, `135 MB`.
pub fn bytes(count: u64) -> String {
    match count {
        0..=9_999 => format!("{count} B"),
        10_000..=9_999_999 => format!("{} kB", thousands(count / 1_000)),
        10_000_000..=9_999_999_999 => format!("{} MB", thousands(count / 1_000_000)),
        _ => format!("{} GB", thousands(count / 1_000_000_000)),
    }
}

/// Lines of `label  value`, held until they can be written in one go.
#[derive(Default)]
pub struct Report {
    lines: String,
    quiet: bool,
}

impl Report {
    /// An empty report, held back from stderr when the run is quiet.
    pub fn new(quiet: bool) -> Self {
        Self {
            lines: String::new(),
            quiet,
        }
    }

    /// Adds a line of a label and its value, the values aligned.
    pub fn line(&mut self, label: &str, value: impl AsRef<str>) -> &mut Self {
        let _ = writeln!(self.lines, "{label:<16}{}", value.as_ref());
        self
    }

    /// Adds a line of free text.
    pub fn note(&mut self, text: impl AsRef<str>) -> &mut Self {
        let _ = writeln!(self.lines, "{}", text.as_ref());
        self
    }

    /// Adds an empty line.
    pub fn blank(&mut self) -> &mut Self {
        self.lines.push('\n');
        self
    }

    /// Writes the lines on stderr, unless the run is quiet.
    pub fn to_stderr(&self) {
        if !self.quiet && !self.lines.is_empty() {
            let _ = std::io::stderr().write_all(self.lines.as_bytes());
        }
    }

    /// The lines as written so far.
    pub fn text(&self) -> &str {
        &self.lines
    }
}

/// A JSON string with the four characters that need it escaped.
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            other if (other as u32) < 0x20 => out.push(' '),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_number_is_long_then_it_is_grouped_in_threes() {
        assert_eq!(thousands(94_746_421_723), "94 746 421 723");
        assert_eq!(thousands(52_810), "52 810");
        assert_eq!(thousands(7), "7");
    }

    #[test]
    fn when_a_size_is_reported_then_it_is_given_in_units_a_reader_can_hold() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(135_000_000), "135 MB");
        assert_eq!(bytes(94_746_421_723), "94 GB");
    }

    /// The report is diagnostics about a side effect, so piping a map into another command has to
    /// carry the map and nothing else.
    #[test]
    fn when_a_run_is_quiet_then_the_report_is_held_rather_than_written() {
        let mut report = Report::new(true);
        report.line("frame", "29.0 x 16.5 km");

        assert!(!report.text().is_empty(), "the lines are still built");
        report.to_stderr();
    }

    #[test]
    fn when_a_name_holds_a_quote_then_the_json_stays_parseable() {
        assert_eq!(quoted("Port of \"Abidjan\""), "\"Port of \\\"Abidjan\\\"\"");
    }
}
