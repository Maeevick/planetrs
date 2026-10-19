//! A planet filed under its countries as their map indexes, pinned on a fixture of the Riviera.
//!
//! France and Monaco lie side by side, with named and unnamed nodes in each, a road across their
//! border, a road leaving France for land no index covers, and a lake whose inner ring lies far away:
//! every rule of what a country's map index keeps, in one small planet.

use std::path::{Path, PathBuf};

use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::map_index::{self, Element, Line, Position};
use planetrs_dataset::{Manifest, Part};
use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::index::build_map_index;
use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A ring of four corners closed on its first node, as one way.
fn ring(way: i64, first_node: i64, south: f64, west: f64, north: f64, east: f64) -> (Vec<Node>, Way) {
    let corners = [(south, west), (south, east), (north, east), (north, west)];
    let nodes = corners
        .iter()
        .zip(first_node..)
        .map(|((latitude, longitude), id)| Node::at(id, *latitude, *longitude))
        .collect();
    (
        nodes,
        Way::through(way, (first_node..first_node + 4).chain([first_node])),
    )
}

fn country(relation: i64, code: &str, outline: i64) -> Relation {
    Relation::of(relation)
        .tagged("type", "boundary")
        .tagged("boundary", "administrative")
        .tagged("admin_level", "2")
        .tagged("ISO3166-1", code)
        .with_member(MemberKind::Way, outline, "outer")
}

fn riviera() -> PlanetFile {
    let (france_corners, france) = ring(2001, 1001, 43.60, 7.00, 43.80, 7.40);
    let (monaco_corners, monaco) = ring(2002, 1011, 43.70, 7.40, 43.76, 7.45);
    let planet = PlanetFile::sorted()
        .replicated_at(1_788_739_204, 4_321)
        .with_node(
            Node::at(1, 43.70, 7.26)
                .tagged("name", "Nice")
                .tagged("place", "city"),
        )
        .with_node(Node::at(2, 43.65, 7.10))
        .with_node(
            Node::at(3, 43.73, 7.42)
                .tagged("name", "Monaco")
                .tagged("place", "city"),
        )
        .with_node(Node::at(4, 43.71, 7.35))
        .with_node(Node::at(5, 43.73, 7.43))
        .with_node(Node::at(6, 43.69, 7.30))
        .with_node(Node::at(7, 44.20, 7.30))
        .with_node(Node::at(8, 43.62, 7.05))
        .with_node(Node::at(9, 43.62, 7.08))
        .with_node(Node::at(13, 43.64, 7.08))
        .with_node(Node::at(15, 45.00, 8.00))
        .with_node(Node::at(16, 45.00, 8.01))
        .with_node(Node::at(17, 45.01, 8.01));
    france_corners
        .into_iter()
        .chain(monaco_corners)
        .fold(planet, PlanetFile::with_node)
        .with_way(
            Way::through(10, [4, 5])
                .tagged("highway", "primary")
                .tagged("name", "Basse Corniche"),
        )
        .with_way(Way::through(11, [6, 7]).tagged("highway", "secondary"))
        .with_way(Way::through(12, [8, 9, 13, 8]))
        .with_way(Way::through(14, [15, 16, 17, 15]))
        .with_way(france)
        .with_way(monaco)
        .with_relation(
            Relation::of(100)
                .tagged("type", "multipolygon")
                .tagged("natural", "water")
                .with_member(MemberKind::Way, 12, "outer")
                .with_member(MemberKind::Way, 14, "inner"),
        )
        .with_relation(country(3001, "FR", 2001))
        .with_relation(country(3002, "MC", 2002))
}

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
    let directory = scratch(name);
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("riviera.osm.pbf");
    riviera().write(&planet)?;
    let dataset = directory.join("dataset");
    build_map_index(&planet, &dataset, &["FR", "MC"], &|_| {})?;
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
            at(43_710_000_000, 7_350_000_000),
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

    let built = build_map_index(&planet, &dataset, &["FR", "BE"], &|_| {});

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
