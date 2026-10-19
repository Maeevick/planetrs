//! Building the index from a planet file, beside the old one, and swapping it in once complete.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use planetrs_geocoder::build_index;

use planetrs_dataset::Location;

use super::{Context, UsageError};
use crate::cli::IndexArgs;
use crate::files::replace_directory;

/// Planet files from this size on are whole planets or continents, which need the memory said.
const LARGE_PLANET_BYTES: u64 = 10_000_000_000;

/// The country codes as the index wants them: upper case, trimmed, each once, in the order given.
pub fn country_codes(asked: &[String]) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for code in asked
        .iter()
        .map(|code| code.trim().to_uppercase())
        .filter(|code| !code.is_empty())
    {
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

/// The directory a build happens in before it replaces `destination`.
pub fn building_beside(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".building");
    destination.with_file_name(name)
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
    let building = building_beside(&destination);
    if building.exists() {
        std::fs::remove_dir_all(&building)
            .with_context(|| format!("removing an unfinished build at {}", building.display()))?;
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
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
    let built = build_index(&planet, &building, &country_refs, &|line| context.say(line));
    let summary = match built {
        Ok(summary) => summary,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&building);
            return Err(error).context("the index was not built; the previous one, if any, is untouched");
        }
    };
    replace_directory(&building, &destination)
        .with_context(|| format!("moving the new index into {}", destination.display()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_codes_in_any_case_and_repeated_when_read_then_each_is_kept_once_in_upper_case() {
        let asked = [" fr", "MC", "Fr", ""].map(String::from);

        assert_eq!(country_codes(&asked), vec!["FR", "MC"]);
    }

    #[test]
    fn given_an_index_directory_when_built_then_the_build_happens_beside_it() {
        assert_eq!(
            building_beside(Path::new("/data/geocoder/index")),
            PathBuf::from("/data/geocoder/index.building")
        );
    }
}
