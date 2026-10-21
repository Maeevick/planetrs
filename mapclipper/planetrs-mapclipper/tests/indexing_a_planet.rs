//! A planet filed under the countries asked for as one map index cut into tiles, pinned on a fixture
//! of the Riviera.
//!
//! France and Monaco lie side by side, with named and unnamed nodes in each, a road across their
//! border, a road leaving France for land no index covers, and a lake whose inner ring lies far away:
//! every rule of what a map index keeps, in one small planet.

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

fn manifest_of(dataset: &Path) -> Result<Manifest, Box<dyn std::error::Error>> {
    Ok(Manifest::from_json(&std::fs::read_to_string(
        dataset.join(MANIFEST),
    )?)?)
}

/// Every element of every tile of the dataset's map index.
fn elements_of(dataset: &Path) -> Result<Vec<Element>, Box<dyn std::error::Error>> {
    let mut elements = Vec::new();
    for entry in manifest_of(dataset)?.files {
        if matches!(entry.part, Part::MapTile { .. }) {
            elements.extend(map_index::from_parquet(&std::fs::read(dataset.join(&entry.file))?)?.1);
        }
    }
    Ok(elements)
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
fn given_the_riviera_when_indexed_then_its_map_index_keeps_the_named_and_lone_nodes_of_both_countries_and_never_a_ways_vertex()
-> Outcome {
    let dataset = indexed("nodes")?;

    let kept = nodes(&elements_of(&dataset)?);

    assert!(
        kept.contains(&(1, 2)) && kept.contains(&(2, 0)) && kept.contains(&(3, 2)),
        "{kept:?}"
    );
    assert!(
        [4, 5, 6, 8, 9, 13]
            .iter()
            .all(|vertex| !kept.iter().any(|(id, _)| id == vertex)),
        "{kept:?}"
    );
    Ok(())
}

#[test]
fn given_a_road_across_a_border_when_indexed_then_its_map_index_keeps_it_once_with_every_node_placed()
-> Outcome {
    let dataset = indexed("border")?;

    let elements = elements_of(&dataset)?;

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
    assert_eq!(way(&elements, 10), Some(corniche));
    assert_eq!(
        elements
            .iter()
            .filter(|element| matches!(element, Element::Way(line) if line.id == 10))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn given_a_road_leaving_for_land_no_index_covers_when_indexed_then_its_map_index_keeps_it_whole_with_every_node_placed_the_one_outside_included()
-> Outcome {
    let dataset = indexed("leaving")?;

    let road = way(&elements_of(&dataset)?, 11);

    assert_eq!(
        road.map(|road| road.positions),
        Some(vec![
            at(43_690_000_000, 7_300_000_000),
            at(44_200_000_000, 7_300_000_000)
        ])
    );
    Ok(())
}

#[test]
fn given_a_lake_whose_inner_ring_lies_far_away_when_indexed_then_its_map_index_keeps_both_members_whole_and_placed()
-> Outcome {
    let dataset = indexed("lake")?;

    let lake = elements_of(&dataset)?
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

    let (members, ways) = lake.ok_or("the lake is not in the map index")?;
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
fn given_the_riviera_when_indexed_then_the_manifest_lists_its_map_index_built_from_it() -> Outcome {
    let dataset = indexed("manifest")?;

    let manifest = manifest_of(&dataset)?;

    assert_eq!(
        manifest
            .files
            .iter()
            .map(|entry| entry.part.clone())
            .collect::<Vec<_>>(),
        vec![Part::MapSummary, Part::MapTile { level: 0, x: 0, y: 0 }]
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
fn given_the_riviera_indexed_twice_in_tiles_when_the_files_are_compared_then_they_are_identical() -> Outcome {
    let once = indexed_in_tiles_of("once", 200)?;
    let again = indexed_in_tiles_of("again", 200)?;

    let files = manifest_of(&once)?.files;
    assert_eq!(files, manifest_of(&again)?.files);
    for entry in files {
        assert_eq!(
            std::fs::read(once.join(&entry.file))?,
            std::fs::read(again.join(&entry.file))?,
            "{}",
            entry.file
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
