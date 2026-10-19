//! Where things are, what is there, and what to do next.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Result;

use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{Manifest, Part};

use super::Context;
use crate::data_dir::planet_date;
use crate::files::Day;

/// A size as a reader compares it, in decimal units: `94.7 GB`.
pub fn size_text(bytes: u64) -> String {
    let units = ["bytes", "kB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < units.len() {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

/// What a dataset's manifest says about how it was built: from which planet, and for which
/// countries its streets were indexed.
pub fn built_from(manifest: &Manifest) -> String {
    let seconds = u64::try_from(manifest.planet.replication_timestamp).unwrap_or_default();
    let day = Day::of(UNIX_EPOCH + Duration::from_secs(seconds));
    let countries: Vec<&str> = manifest
        .files
        .iter()
        .filter_map(|entry| match &entry.part {
            Part::GeocoderIndex { country } => Some(country.as_str()),
            _ => None,
        })
        .collect();
    if countries.is_empty() {
        format!("from the planet of {day}, communes only")
    } else {
        format!("from the planet of {day}, streets of {}", countries.join(","))
    }
}

/// Every OpenStreetMap file among `paths`: dated planets first, newest first, then the others by
/// name.
pub fn in_reading_order(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let is_osm_file = |path: &PathBuf| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".osm.pbf"))
    };
    let mut found: Vec<PathBuf> = paths.into_iter().filter(is_osm_file).collect();
    found.sort_by_key(|path| (std::cmp::Reverse(planet_date(path)), path.clone()));
    found
}

fn planets_in(directory: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    in_reading_order(entries.filter_map(Result::ok).map(|entry| entry.path()))
        .into_iter()
        .map(|path| {
            let size = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            (path, size)
        })
        .collect()
}

/// Prints the data directory, its planets and the index, with the next step when one is missing.
pub fn run(context: &Context) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "data directory  {} ({})",
        context.data.path.display(),
        context.data.source
    )?;

    let planets = planets_in(&context.data.planets());
    let read_by_default = context.data.newest_planet();
    if planets.is_empty() {
        writeln!(
            stdout,
            "planets         none yet\n                → planetrs-geocoder download"
        )?;
    }
    for (at, (planet, size)) in planets.iter().enumerate() {
        let label = if at == 0 { "planets" } else { "" };
        let default = if read_by_default.as_ref() == Some(planet) {
            ", what index reads by default"
        } else {
            ""
        };
        writeln!(
            stdout,
            "{label:<16}{}, {}{default}",
            planet.display(),
            size_text(*size)
        )?;
    }

    let index = context.index_directory();
    let manifest = std::fs::read_to_string(index.join(MANIFEST))
        .ok()
        .and_then(|text| Manifest::from_json(&text).ok());
    match manifest {
        Some(manifest) => writeln!(
            stdout,
            "index           {}\n                built {}",
            index.display(),
            built_from(&manifest)
        )?,
        None if planets.is_empty() => writeln!(
            stdout,
            "index           none in {}\n                → planetrs-geocoder download, then planetrs-geocoder index",
            index.display()
        )?,
        None => writeln!(
            stdout,
            "index           none in {}\n                → planetrs-geocoder index",
            index.display()
        )?,
    }
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use planetrs_dataset::{Entry, Provenance};

    use super::*;

    #[test]
    fn given_sizes_across_the_units_when_written_then_each_reads_in_its_own() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(1_536), "1.5 kB");
        assert_eq!(size_text(94_746_421_723), "94.7 GB");
    }

    #[test]
    fn given_planets_and_extracts_when_ordered_then_dated_planets_come_first_newest_first() {
        let found = in_reading_order(
            [
                "monaco-latest.osm.pbf",
                "planet-260907.osm.pbf",
                "notes.txt",
                "planet-261005.osm.pbf",
                "planet-261012.osm.pbf.part",
            ]
            .map(PathBuf::from),
        );

        assert_eq!(
            found,
            [
                "planet-261005.osm.pbf",
                "planet-260907.osm.pbf",
                "monaco-latest.osm.pbf"
            ]
            .map(PathBuf::from)
        );
    }

    fn manifest_of(replication_timestamp: i64, countries: &[&str]) -> Manifest {
        let entry = |part: Part, file: &str| Entry {
            part,
            file: file.into(),
            url: None,
            bytes: 1,
            sha256: String::new(),
            rows: 1,
        };
        Manifest {
            planet: Provenance {
                source_name: "planet-260907.osm.pbf".into(),
                file_size: 94_746_421_723,
                blob_count: 52_810,
                replication_timestamp,
                replication_sequence: 4_321,
                writing_program: "planet-dump-ng 1.2.7".into(),
            },
            files: std::iter::once(entry(Part::Gazetteer, "gazetteer.parquet"))
                .chain(countries.iter().map(|country| {
                    entry(
                        Part::GeocoderIndex {
                            country: country.to_string(),
                        },
                        &format!("geocoder-{country}.parquet"),
                    )
                }))
                .collect(),
        }
    }

    #[test]
    fn given_a_dataset_indexing_streets_when_described_then_the_planet_day_and_the_countries_are_named() {
        let manifest = manifest_of(1_788_739_204, &["FR", "MC"]);

        assert_eq!(
            built_from(&manifest),
            "from the planet of 2026-09-07, streets of FR,MC"
        );
    }

    #[test]
    fn given_a_dataset_of_the_gazetteer_alone_when_described_then_it_says_there_are_communes_only() {
        let manifest = manifest_of(0, &[]);

        assert_eq!(
            built_from(&manifest),
            "from the planet of 1970-01-01, communes only"
        );
    }
}
