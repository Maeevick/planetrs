//! Building the index from a planet file: every pass over the planet, in the order they depend on
//! each other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use planetrs_dataset::directory::{RecordError, record};
use planetrs_dataset::{Part, Provenance, TableError, gazetteer, geocoder_index};
use planetrs_pbf::{PbfError, Planet};
use thiserror::Error;

use crate::assemble::assemble;
use crate::harvest::{harvest_boundaries, nodes_by_id, ways_by_id};
use crate::index_file::{IndexError, write_areas, write_index, write_parents, write_shops, write_streets};
use crate::index_streets::index_streets;
use crate::parts::{gazetteer_of, geocoder_index_of};

/// The countries an index covers when none are named: France and Monaco, whose addresses the
/// reading and matching rules were written for.
pub const DEFAULT_COUNTRIES: [&str; 2] = ["FR", "MC"];

/// What a finished build holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexSummary {
    /// Municipalities in the gazetteer, the whole planet over.
    pub communes: usize,
    /// Countries the gazetteer can name.
    pub countries: usize,
    /// Address points filed under a street of the chosen countries.
    pub doors: usize,
    /// Named roads filed under a commune of the chosen countries.
    pub roads: usize,
    /// Hamlets and localities filed as streets of one point.
    pub hamlets: usize,
    /// Shops filed under a commune of the chosen countries.
    pub shops: usize,
    /// Blobs of the planet that did not decode and were skipped.
    pub unreadable_blobs: usize,
}

/// Why a build stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BuildError {
    /// The planet file could not be opened or one of its passes failed.
    #[error("reading the planet file {} failed", path.display())]
    Planet {
        /// The planet file.
        path: PathBuf,
        /// What failed, and in which blob when one is known.
        #[source]
        source: PbfError,
    },
    /// An index file could not be written.
    #[error("writing the index to {} failed", directory.display())]
    Writing {
        /// The directory the index was being written to.
        directory: PathBuf,
        /// What failed.
        #[source]
        source: IndexError,
    },
    /// A part of the dataset could not be encoded as GeoParquet.
    #[error("encoding a part of the dataset failed")]
    Encoding(#[source] TableError),
    /// A part of the dataset could not be recorded in its directory.
    #[error("recording a part of the dataset in {} failed", directory.display())]
    Recording {
        /// The dataset's directory.
        directory: PathBuf,
        /// What failed.
        #[source]
        source: RecordError,
    },
}

/// Builds the index of the planet file into `directory`: the gazetteer of every country, then the
/// streets, doors, shops, former towns and areas of the `countries` named by ISO code.
///
/// With no countries the index holds the gazetteer alone, and lookups stop at the commune.
/// `progress` receives one line per pass as it ends. The planet is read several times and the build
/// holds a country's streets in memory, which for France and Monaco peaks at about 20 GB.
pub fn build_index(
    planet: &Path,
    directory: &Path,
    countries: &[&str],
    progress: &dyn Fn(&str),
) -> Result<IndexSummary, BuildError> {
    let started = Instant::now();
    let elapsed = || started.elapsed().as_secs_f64();
    let reading = |source| BuildError::Planet {
        path: planet.to_path_buf(),
        source,
    };
    let writing = |source| BuildError::Writing {
        directory: directory.to_path_buf(),
        source,
    };

    let opened = Planet::open(planet).map_err(reading)?;
    let fingerprint = opened.fingerprint();
    progress(&format!(
        "framing        {} blobs in {:.1}s",
        opened.blobs.len(),
        elapsed()
    ));

    let harvest = harvest_boundaries(&opened).map_err(reading)?;
    progress(&format!(
        "relations      {} areas, {} countries in {:.1}s",
        harvest.areas.len(),
        harvest.countries.len(),
        elapsed()
    ));

    let wanted_ways = harvest
        .countries
        .iter()
        .flat_map(|country| country.outline_ways.iter().copied())
        .collect();
    let ways = ways_by_id(&opened, &wanted_ways).map_err(reading)?;
    progress(&format!(
        "ways           {} of {} country outline ways in {:.1}s",
        ways.found.len(),
        wanted_ways.len(),
        elapsed()
    ));

    let wanted_nodes = harvest
        .areas
        .iter()
        .filter_map(|area| area.centre_node)
        .chain(ways.found.values().flatten().copied())
        .collect();
    let nodes = nodes_by_id(&opened, &wanted_nodes).map_err(reading)?;
    progress(&format!(
        "nodes          {} of {} wanted in {:.1}s",
        nodes.found.len(),
        wanted_nodes.len(),
        elapsed()
    ));

    let assembled = assemble(&harvest, &ways.found, &nodes.found);
    let report = &assembled.report;
    progress(&format!(
        "assembled      {} areas, {} without a centre, {} outside every country, in {:.1}s",
        report.communes,
        report.communes_without_centre,
        report.communes_without_country,
        elapsed()
    ));
    progress(&format!(
        "countries      {} named, {} without an outline, {} broken chains, {} rings missing a node",
        report.countries,
        report.countries_without_outline.len(),
        report.broken_chains,
        report.rings_missing_nodes
    ));
    let mut summary = IndexSummary {
        communes: assembled.communes.len(),
        countries: assembled.countries.len(),
        unreadable_blobs: harvest.unreadable_blobs + ways.unreadable_blobs + nodes.unreadable_blobs,
        ..IndexSummary::default()
    };
    if summary.unreadable_blobs > 0 {
        progress(&format!(
            "unreadable     {} blobs did not decode",
            summary.unreadable_blobs
        ));
    }

    let provenance = format!(
        "planet {} bytes, {} blobs, replication timestamp {}",
        fingerprint.file_size, fingerprint.blob_count, fingerprint.replication_timestamp
    );
    let source = Provenance {
        source_name: planet
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_size: fingerprint.file_size,
        blob_count: fingerprint.blob_count,
        replication_timestamp: fingerprint.replication_timestamp,
        replication_sequence: opened.header.replication_sequence.unwrap_or_default(),
        writing_program: opened.header.writing_program.clone().unwrap_or_default(),
    };
    let mut country_relations: HashMap<String, i64> = HashMap::new();
    for harvested in &harvest.countries {
        let lowest = country_relations
            .entry(harvested.country.code.clone())
            .or_insert(harvested.relation);
        *lowest = (*lowest).min(harvested.relation);
    }
    let recording = |part: Part, name: &str, bytes: Vec<u8>, rows: usize| {
        record(directory, &source, part, name, &bytes, rows as u64).map_err(|source| BuildError::Recording {
            directory: directory.to_path_buf(),
            source,
        })
    };
    write_index(directory, &assembled.communes, &assembled.countries, &provenance).map_err(writing)?;

    if !countries.is_empty() {
        let streets_progress = |step: &str| progress(&format!("streets        {step} at {:.1}s", elapsed()));
        let (indexed, report) =
            index_streets(&opened, &harvest, &assembled, countries, &streets_progress).map_err(reading)?;
        let filing = &report.filing;
        progress(&format!(
            "filed          {} doors; {} with no street, {} outside every commune, {} unreadable numbers, {} buildings with no nodes",
            filing.addresses_filed,
            filing.addresses_without_a_street,
            filing.addresses_outside_every_commune,
            filing.addresses_with_an_unreadable_number,
            filing.buildings_without_nodes
        ));
        progress(&format!(
            "               {} roads, {} hamlets; {} outline broken chains, {} unreadable blobs",
            filing.highways_filed, filing.places_filed, report.outline_broken_chains, report.unreadable_blobs
        ));
        write_parents(directory, &indexed.parents, &provenance).map_err(writing)?;
        progress(&format!(
            "former towns   {} placed inside the commune that absorbed them",
            report.former_towns_placed
        ));
        write_shops(directory, &indexed.shops, &provenance).map_err(writing)?;
        write_areas(directory, &indexed.areas, &provenance).map_err(writing)?;
        progress(&format!(
            "shops          {} read, {} filed under a commune",
            report.shops_read, report.shops_filed
        ));
        write_streets(
            directory,
            &indexed.streets,
            &format!("{provenance}, countries {}", countries.join(",")),
        )
        .map_err(writing)?;
        progress(&format!("written        streets in {:.1}s", elapsed()));
        summary.doors = filing.addresses_filed;
        summary.roads = filing.highways_filed;
        summary.hamlets = filing.places_filed;
        summary.shops = report.shops_filed;
        summary.unreadable_blobs += report.unreadable_blobs;
        let areas = gazetteer_of(
            &assembled.communes,
            &assembled.countries,
            &country_relations,
            &indexed.parents,
            &indexed.areas,
        );
        let bytes = gazetteer::to_parquet(&areas, &source).map_err(BuildError::Encoding)?;
        recording(Part::Gazetteer, "gazetteer.parquet", bytes, areas.len())?;
        for country in countries {
            let entries = geocoder_index_of(country, &assembled.communes, &indexed.streets, &indexed.shops);
            let bytes =
                geocoder_index::to_parquet(country, &entries, &source).map_err(BuildError::Encoding)?;
            recording(
                Part::GeocoderIndex {
                    country: country.to_string(),
                },
                &format!("geocoder-{country}.parquet"),
                bytes,
                entries.len(),
            )?;
        }
    } else {
        let areas = gazetteer_of(
            &assembled.communes,
            &assembled.countries,
            &country_relations,
            &HashMap::new(),
            &HashMap::new(),
        );
        let bytes = gazetteer::to_parquet(&areas, &source).map_err(BuildError::Encoding)?;
        recording(Part::Gazetteer, "gazetteer.parquet", bytes, areas.len())?;
    }
    progress(&format!("written        the index in {:.1}s", elapsed()));
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_planet_file_that_is_not_there_when_built_then_the_planet_is_named_as_the_cause() {
        let missing = std::env::temp_dir().join("planetrs-geocoder-no-such-planet.osm.pbf");
        let directory = std::env::temp_dir().join("planetrs-geocoder-never-written");

        let failed = build_index(&missing, &directory, &DEFAULT_COUNTRIES, &|_| {});

        assert!(matches!(failed, Err(BuildError::Planet { path, .. }) if path == missing));
        assert!(
            !directory.exists(),
            "nothing is written when the planet cannot be read"
        );
    }
}
