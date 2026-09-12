//! Every decision the lookup takes, for a site whose answer needs explaining.

use std::io::Write;

use anyhow::Result;

use super::{Context, read_input};
use crate::cli::TraceArgs;
use crate::sites::{Row, read_one_site, read_site_file};

/// Prints the trace of each site the command line names, a file's sites each under its line.
pub fn run(context: &Context, arguments: TraceArgs) -> Result<()> {
    let rows: Vec<Row> = match (arguments.site, arguments.file) {
        (Some(site), _) => vec![read_one_site(&site)?],
        (None, Some(file)) => read_site_file(&read_input(&file)?)?.rows,
        (None, None) => Vec::new(),
    };
    let geocoder = context.open_geocoder()?;
    let mut stdout = std::io::stdout().lock();
    for row in &rows {
        if row.line > 0 {
            writeln!(stdout, "── line {} ──", row.line)?;
        }
        for line in geocoder.trace(&row.site()) {
            writeln!(stdout, "{line}")?;
        }
    }
    stdout.flush()?;
    context.attribute();
    Ok(())
}
