//! Building the map index of the countries asked for from a planet file, beside the dataset, and
//! putting it in place with the geocoder's parts the dataset held of the same planet.

use std::path::PathBuf;

use anyhow::Result;
use planetrs_cli::dataset::{country_codes, rebuild};
use planetrs_cli::{Progress, UsageError};
use planetrs_dataset::directory::CarriedOver;
use planetrs_dataset::map_index::{Scope, TILE_WEIGHT};
use planetrs_dataset::{Location, Part};
use planetrs_mapclipper::index::build_map_index;

use super::{Context, explained};
use crate::cli::IndexArgs;

/// Builds the map index of the countries the command line asks for.
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
    let scope = Scope::Countries(codes.clone());
    let progress = Progress::to_stderr(context.quiet);
    let (indexed, carried) = rebuild(&destination, is_the_geocoders, |building| {
        let built = build_map_index(&planet, building, &scope, TILE_WEIGHT, &|line| {
            progress.note(line)
        });
        progress.clear();
        built.map_err(explained)
    })?;
    for line in carried_lines(&carried) {
        context.say(&line);
    }
    let whom = match &indexed.scope {
        Scope::Planet => "the planet".to_string(),
        Scope::Countries(countries) => countries.join(", "),
    };
    context.say(&format!(
        "indexed {whom}: {} nodes, {} ways and {} multipolygons, in {} tiles",
        indexed.nodes, indexed.ways, indexed.multipolygons, indexed.tiles
    ));
    Ok(())
}

fn planet_to_read(context: &Context, asked: Option<PathBuf>) -> Result<PathBuf> {
    let planet = match asked {
        Some(planet) => planet,
        None => context.data.newest_planet().ok_or_else(|| {
            anyhow::anyhow!(
                "no planet file in {}\n  \
                 → download one:  planetrs-mapclipper download\n  \
                 → or name one:   planetrs-mapclipper index --planet PATH",
                context.data.planets().display()
            )
        })?,
    };
    anyhow::ensure!(planet.is_file(), "no planet file at {}", planet.display());
    Ok(planet)
}

/// Whether a part of a dataset is one the geocoder builds, which indexing maps never rebuilds.
fn is_the_geocoders(part: &Part) -> bool {
    matches!(part, Part::Gazetteer | Part::GeocoderIndex { .. })
}

/// What a rebuild says about the geocoder's parts it kept or left out.
fn carried_lines(carried: &CarriedOver) -> Vec<String> {
    let mut lines = Vec::new();
    if !carried.kept.is_empty() {
        lines.push("kept the geocoder's gazetteer and indexes".to_string());
    }
    if !carried.left.is_empty() {
        lines.push(
            "left out the geocoder's gazetteer and indexes, built from another planet\n  \
             → rebuild them: planetrs-geocoder index"
                .to_string(),
        );
    }
    lines
}
