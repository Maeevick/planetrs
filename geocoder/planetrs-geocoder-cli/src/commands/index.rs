//! Building the index from a planet file, beside the old one, and swapping it in once complete with
//! the map index the old one held of the same planet.

use std::path::PathBuf;

use anyhow::Result;
use planetrs_cli::UsageError;
use planetrs_cli::dataset::{country_codes, rebuild};
use planetrs_dataset::directory::CarriedOver;
use planetrs_dataset::{Location, Part};
use planetrs_geocoder::build_index;

use super::Context;
use crate::cli::IndexArgs;

/// Planet files from this size on are whole planets or continents, which need the memory said.
const LARGE_PLANET_BYTES: u64 = 10_000_000_000;

/// Whether a part of a dataset is one the mapclipper builds, which indexing never rebuilds.
fn is_the_mapclippers(part: &Part) -> bool {
    matches!(
        part,
        Part::MapSummary | Part::MapTile { .. } | Part::MapIndex { .. }
    )
}

/// What a rebuild says about the map index it kept or left out.
fn carried_lines(carried: &CarriedOver) -> Vec<String> {
    let mut lines = Vec::new();
    if !carried.kept.is_empty() {
        lines.push("kept the map index".to_string());
    }
    if !carried.left.is_empty() {
        lines.push(
            "left out the map index, built from another planet\n  → rebuild it: planetrs-mapclipper index"
                .to_string(),
        );
    }
    lines
}

fn planet_to_read(context: &Context, asked: Option<PathBuf>) -> Result<PathBuf> {
    let planet = match asked {
        Some(planet) => planet,
        None => context.data.newest_planet().ok_or_else(|| {
            anyhow::anyhow!(
                "no planet file in {}\n  \
                 → download one:  planetrs-geocoder download\n  \
                 → or name one:   planetrs-geocoder index --planet PATH",
                context.data.planets().display()
            )
        })?,
    };
    anyhow::ensure!(
        planet.is_file(),
        "no planet file at {}\n  → download one: planetrs-geocoder download",
        planet.display()
    );
    Ok(planet)
}

/// Builds the index the command line asks for.
pub fn run(context: &Context, arguments: IndexArgs) -> Result<()> {
    let planet = planet_to_read(context, arguments.planet)?;
    let codes = country_codes(&arguments.countries);
    let Location::Directory(destination) = context.dataset.clone() else {
        return Err(UsageError(format!(
            "a dataset is built into a directory, and {} is a web address\n  \
             → build into the data directory's: drop --dataset, or name a directory with --dataset DIR",
            context.dataset_name()
        ))
        .into());
    };
    context.say(&format!(
        "indexing {} for {} into {}",
        planet.display(),
        codes.join(","),
        destination.display()
    ));
    if std::fs::metadata(&planet).is_ok_and(|metadata| metadata.len() >= LARGE_PLANET_BYTES) {
        context.say("this reads the planet several times and needs about 20 GB of memory");
    }

    let country_refs: Vec<&str> = codes.iter().map(String::as_str).collect();
    let (summary, carried) = rebuild(&destination, is_the_mapclippers, |building| {
        Ok(build_index(&planet, building, &country_refs, &|line| {
            context.say(line)
        })?)
    })?;
    for line in carried_lines(&carried) {
        context.say(&line);
    }

    context.say(&format!(
        "indexed {} communes in {} countries; {} doors, {} roads, {} hamlets and {} shops in {}",
        summary.communes,
        summary.countries,
        summary.doors,
        summary.roads,
        summary.hamlets,
        summary.shops,
        codes.join(",")
    ));
    context.say("→ geocode: planetrs-geocoder \"label;address;postal code;city;country\"");
    context.attribute();
    Ok(())
}
