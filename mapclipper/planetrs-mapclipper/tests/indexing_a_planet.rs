//! A planet filed under its countries as their map indexes, pinned on a fixture of the Riviera.
//!
//! France and Monaco lie side by side, with named and unnamed nodes in each, a road across their
//! border, a road leaving France for land no index covers, and a lake whose inner ring lies far away:
//! every rule of what a country's map index keeps, in one small planet.

use std::path::{Path, PathBuf};

use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::map_index::{self, Element, Line, Position, TILE_WEIGHT};
use planetrs_dataset::{Manifest, Part};
use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::index::build_map_index;
use riviera::riviera;

mod riviera;

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "planetrs-mapclipper-indexing-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

/// The riviera indexed for France and Monaco into a fresh dataset, and the dataset's directory.
fn indexed(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    indexed_in_tiles_of(name, TILE_WEIGHT)
}

/// The riviera indexed for France and Monaco in tiles weighing at most `tile_weight`.
fn indexed_in_tiles_of(name: &str, tile_weight: u64) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let directory = scratch(name);
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("riviera.osm.pbf");
    riviera().write(&planet)?;
    let dataset = directory.join("dataset");
    build_map_index(&planet, &dataset, &["FR", "MC"], tile_weight, &|_| {})?;
    Ok(dataset)
}

fn elements_of(dataset: &Path, country: &str) -> Result<Vec<Element>, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(dataset.join(format!("map-{country}.parquet")))?;
    Ok(map_index::from_parquet(&bytes)?.1)
}

fn at(latitude: i64, longitude: i64) -> Option<Position> {
    Some(Position { latitude, longitude })
}

fn way(elements: &[Element], wanted: i64) -> Option<Line> {
    elements.iter().find_map(|element| match element {
        Element::Way(line) if line.id == wanted => Some(line.clone()),
        _ => None,
    })
}

fn nodes(elements: &[Element]) -> Vec<(i64, usize)> {
    let mut found: Vec<(i64, usize)> = elements
        .iter()
        .filter_map(|element| match element {
            Element::Node { id, tags, .. } => Some((*id, tags.len())),
            _ => None,
        })
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn given_the_riviera_when_indexed_then_each_country_keeps_its_named_and_lone_nodes_and_never_a_ways_vertex()
-> Outcome {
    let dataset = indexed("nodes")?;

    let france = nodes(&elements_of(&dataset, "FR")?);
    let monaco = nodes(&elements_of(&dataset, "MC")?);

    assert!(france.contains(&(1, 2)) && france.contains(&(2, 0)), "{france:?}");
    assert!(monaco.contains(&(3, 2)), "{monaco:?}");
    assert!(
        [4, 5, 6, 8, 9, 13]
            .iter()
            .all(|vertex| !france.iter().any(|(id, _)| id == vertex)),
        "{france:?}"
    );
    assert!(!france.iter().any(|(id, _)| *id == 3) && !monaco.iter().any(|(id, _)| *id == 1));
    Ok(())
}

#[test]
fn given_a_road_across_a_border_when_indexed_then_both_countries_keep_it_with_every_node_placed() -> Outcome {
    let dataset = indexed("border")?;

    let in_france = way(&elements_of(&dataset, "FR")?, 10);
    let in_monaco = way(&elements_of(&dataset, "MC")?, 10);

    let corniche = Line {
        id: 10,
        tags: vec![
            ("highway".into(), "primary".into()),
            ("name".into(), "Basse Corniche".into()),
        ],
        refs: vec![4, 5],
        positions: vec![
            at(43_720_000_000, 7_350_000_000),
            at(43_730_000_000, 7_430_000_000),
        ],
    };
    assert_eq!(in_france, Some(corniche.clone()));
    assert_eq!(in_monaco, Some(corniche));
    Ok(())
}

#[test]
fn given_a_road_leaving_for_land_no_index_covers_when_indexed_then_its_country_keeps_it_with_the_node_outside_unplaced()
-> Outcome {
    let dataset = indexed("leaving")?;

    let road = way(&elements_of(&dataset, "FR")?, 11);

    assert_eq!(
        road.map(|road| road.positions),
        Some(vec![at(43_690_000_000, 7_300_000_000), None])
    );
    assert!(way(&elements_of(&dataset, "MC")?, 11).is_none());
    Ok(())
}

#[test]
fn given_a_lake_whose_inner_ring_lies_far_away_when_indexed_then_its_country_keeps_both_members_whole_and_placed()
-> Outcome {
    let dataset = indexed("lake")?;

    let lake = elements_of(&dataset, "FR")?
        .into_iter()
        .find_map(|element| match element {
            Element::Multipolygon {
                id: 100,
                members,
                ways,
                ..
            } => Some((members, ways)),
            _ => None,
        });

    let (members, ways) = lake.ok_or("the lake is not in France's map index")?;
    assert_eq!(
        members,
        vec![(12, "outer".to_string()), (14, "inner".to_string())]
    );
    assert_eq!(ways.iter().map(|line| line.id).collect::<Vec<_>>(), vec![12, 14]);
    assert_eq!(
        ways[1].positions,
        vec![
            at(45_000_000_000, 8_000_000_000),
            at(45_000_000_000, 8_010_000_000),
            at(45_010_000_000, 8_010_000_000),
            at(45_000_000_000, 8_000_000_000),
        ]
    );
    Ok(())
}

#[test]
fn given_the_riviera_when_indexed_then_the_manifest_lists_both_map_indexes_built_from_it() -> Outcome {
    let dataset = indexed("manifest")?;

    let manifest = Manifest::from_json(&std::fs::read_to_string(dataset.join(MANIFEST))?)?;

    assert_eq!(
        manifest
            .files
            .iter()
            .map(|entry| entry.part.clone())
            .collect::<Vec<_>>(),
        vec![
            Part::MapIndex { country: "FR".into() },
            Part::MapIndex { country: "MC".into() }
        ]
    );
    assert_eq!(manifest.planet.source_name, "riviera.osm.pbf");
    assert_eq!(manifest.planet.replication_timestamp, 1_788_739_204);
    Ok(())
}

#[test]
fn given_a_country_the_planet_has_no_outline_of_when_indexed_then_the_build_names_it_and_records_nothing()
-> Outcome {
    let directory = scratch("unknown");
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("riviera.osm.pbf");
    riviera().write(&planet)?;
    let dataset = directory.join("dataset");

    let built = build_map_index(&planet, &dataset, &["FR", "BE"], TILE_WEIGHT, &|_| {});

    assert!(
        matches!(&built, Err(ClipError::UnknownCountry { country }) if country == "BE"),
        "{built:?}"
    );
    assert!(!dataset.join(MANIFEST).exists());
    Ok(())
}

#[test]
fn given_the_riviera_indexed_twice_when_the_files_are_compared_then_they_are_identical() -> Outcome {
    let once = indexed("once")?;
    let again = indexed("again")?;

    for file in ["map-FR.parquet", "map-MC.parquet"] {
        assert_eq!(
            std::fs::read(once.join(file))?,
            std::fs::read(again.join(file))?,
            "{file}"
        );
    }
    Ok(())
}

#[test]
fn given_the_riviera_indexed_in_tiles_lighter_than_it_when_its_manifest_is_read_then_it_lists_its_summary_then_its_tiles_each_with_its_box()
-> Outcome {
    let dataset = indexed_in_tiles_of("tiles", 200)?;

    let manifest = Manifest::from_json(&std::fs::read_to_string(dataset.join(MANIFEST))?)?;

    let parts: Vec<Part> = manifest.files.iter().map(|entry| entry.part.clone()).collect();
    let tiles: Vec<&planetrs_dataset::Entry> = manifest
        .files
        .iter()
        .filter(|entry| matches!(entry.part, Part::MapTile { .. }))
        .collect();
    assert_eq!(parts.first(), Some(&Part::MapSummary), "{parts:?}");
    assert!(tiles.len() > 1, "{parts:?}");
    assert!(tiles.iter().all(|entry| entry.bbox.is_some()), "{parts:?}");
    assert_eq!(tiles.len() + 1, parts.len(), "{parts:?}");
    Ok(())
}
