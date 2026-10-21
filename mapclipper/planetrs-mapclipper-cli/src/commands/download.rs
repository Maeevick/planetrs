//! Downloading the planet into the data directory, and saying how to index it.

use std::path::Path;

use anyhow::Result;
use planetrs_cli::data_dir::planet_date;
use planetrs_cli::download::fetch_planet;

use super::Context;
use crate::cli::DownloadArgs;

/// The command that builds the map index from a downloaded file.
pub fn next_step(planet: &Path) -> String {
    if planet_date(planet).is_some() {
        "→ next: planetrs-mapclipper index".to_string()
    } else {
        format!("→ next: planetrs-mapclipper index --planet {}", planet.display())
    }
}

/// Downloads the planet the command line names into the data directory.
pub fn run(context: &Context, arguments: DownloadArgs) -> Result<()> {
    let planet = fetch_planet(&arguments.from, &context.data.planets(), context.quiet, &|line| {
        context.say(line)
    })?;
    context.say(&next_step(&planet));
    Ok(())
}
