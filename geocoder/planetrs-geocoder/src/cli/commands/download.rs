//! Downloading the planet into the data directory, and saying what to do with it.

use std::path::Path;

use anyhow::Result;
use planetrs_cli::data_dir::planet_date;
use planetrs_cli::download::fetch_planet;

use super::Context;
use crate::cli::arguments::DownloadArgs;

/// The command that builds the index from a downloaded file.
pub fn next_step(planet: &Path) -> String {
    if planet_date(planet).is_some() {
        "→ next: planetrs-geocoder index".to_string()
    } else {
        format!("→ next: planetrs-geocoder index --planet {}", planet.display())
    }
}

/// Downloads the planet the command line names into the data directory.
pub fn run(context: &Context, arguments: DownloadArgs) -> Result<()> {
    let planet = fetch_planet(&arguments.from, &context.data.planets(), context.quiet, &|line| {
        context.say(line)
    })?;
    context.say(&next_step(&planet));
    context.attribute();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_dated_planet_when_downloaded_then_index_finds_it_alone() {
        assert_eq!(
            next_step(Path::new("/data/planets/planet-260907.osm.pbf")),
            "→ next: planetrs-geocoder index"
        );
    }

    #[test]
    fn given_an_extract_when_downloaded_then_index_is_told_where_it_is() {
        assert_eq!(
            next_step(Path::new("/data/planets/monaco-latest.osm.pbf")),
            "→ next: planetrs-geocoder index --planet /data/planets/monaco-latest.osm.pbf"
        );
    }
}
