//! A harvest cut from a dataset's map indexes, held to the planet's own harvest byte for byte.
//!
//! Every box lies inside the countries indexed, where the dataset answers for the planet: a box
//! around Nice, a box across the border between France and Monaco, and a box around a lake whose
//! inner ring lies far away.

use std::path::PathBuf;

use planetrs_dataset::{Dataset, DirectoryFiles};
use planetrs_geo::{Area, BoundingBox};
use planetrs_mapclipper::harvest::dataset::harvest_from_dataset;
use planetrs_mapclipper::harvest::format;
use planetrs_mapclipper::harvest::passes::harvest_area;
use planetrs_mapclipper::index::build_map_index;
use planetrs_pbf::Planet;
use riviera::riviera;

mod riviera;

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// The riviera written as a planet and indexed for France and Monaco, both in one scratch
/// directory.
fn planet_and_dataset(name: &str) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "planetrs-mapclipper-harvesting-dataset-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("riviera.osm.pbf");
    riviera().write(&planet)?;
    let dataset = directory.join("dataset");
    build_map_index(&planet, &dataset, &["FR", "MC"], &|_| {})?;
    Ok((planet, dataset))
}

/// The bytes of the harvest of `box_`, cut from the planet and from the dataset.
fn both_harvests(name: &str, box_: BoundingBox) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    let (planet, dataset) = planet_and_dataset(name)?;
    let area = Area::new(box_, 0.0)?;
    let (from_planet, _) = harvest_area(&Planet::open(&planet)?, "riviera.osm.pbf", &area, &|_| {})?;
    let from_dataset = harvest_from_dataset(&Dataset::open(DirectoryFiles::at(&dataset))?, &area)?;
    Ok((format::to_bytes(&from_planet), format::to_bytes(&from_dataset)))
}

#[test]
fn given_a_box_around_nice_when_harvested_from_the_dataset_then_it_is_the_planets_harvest_to_the_byte()
-> Outcome {
    let (from_planet, from_dataset) = both_harvests(
        "nice",
        BoundingBox {
            west: 7.20,
            south: 43.68,
            east: 7.32,
            north: 43.72,
        },
    )?;

    assert_eq!(from_dataset, from_planet);
    Ok(())
}
