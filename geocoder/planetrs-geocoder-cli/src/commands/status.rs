//! Where things are, what is there, and what to do next.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::Result;

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

/// What an index file's first line says about how the index was built.
pub fn provenance(header: &str) -> String {
    let after = |marker: &str| header.split_once(marker).map(|(_, rest)| rest);
    let replicated = after("replication timestamp ")
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse::<u64>().ok())
        .map(|seconds| Day::of(UNIX_EPOCH + Duration::from_secs(seconds)).to_string());
    let countries = after(", countries ").map(str::trim);
    let planet = match replicated {
        Some(day) => format!("from the planet of {day}"),
        None => "from an unknown planet".to_string(),
    };
    match countries {
        Some(countries) => format!("{planet}, streets of {countries}"),
        None => format!("{planet}, communes only"),
    }
}

fn first_line(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    BufReader::new(file).lines().next()?.ok()
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
    let header = first_line(&index.join("streets.tsv")).or_else(|| first_line(&index.join("communes.tsv")));
    match header {
        Some(header) => writeln!(
            stdout,
            "index           {}\n                built {}",
            index.display(),
            provenance(&header)
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

    #[test]
    fn given_a_streets_header_when_read_then_the_planet_day_and_the_countries_are_named() {
        let header = "# planetrs streets v1, planet 94746421723 bytes, 52810 blobs, replication timestamp \
                      1788739204, countries FR,MC";

        assert_eq!(
            provenance(header),
            "from the planet of 2026-09-07, streets of FR,MC"
        );
    }

    #[test]
    fn given_a_gazetteer_header_when_read_then_it_says_there_are_communes_only() {
        let header = "# planetrs gazetteer v1, planet 1 bytes, 1 blobs, replication timestamp 0";

        assert_eq!(provenance(header), "from the planet of 1970-01-01, communes only");
    }
}
