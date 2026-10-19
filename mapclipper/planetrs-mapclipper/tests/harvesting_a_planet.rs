//! What a harvest takes out of a planet, pinned on a fixture before the dataset has to reproduce it.
//!
//! Every rule here is one the map index will have to give back byte for byte: which nodes count as
//! inside, which ways and multipolygons come with them, and which positions are left out so that the
//! scene can still tell where a way leaves the box.

use std::path::PathBuf;
use std::sync::Mutex;

use planetrs_geo::{Area, BoundingBox};
use planetrs_mapclipper::harvest::passes::harvest_area;
use planetrs_mapclipper::harvest::{Harvest, Member, NamedNode, Provenance};
use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};
use planetrs_pbf::{IdIndex, Planet};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// A box over Monaco, collected with no margin so that its edges are the fixture's own numbers.
fn monaco() -> Result<Area, planetrs_geo::GeoError> {
    Area::new(
        BoundingBox {
            west: 7.40,
            south: 43.72,
            east: 7.44,
            north: 43.75,
        },
        0.0,
    )
}

/// Nodes inside, outside and on the edge of the box, ways touching it in every way a way can, and a
/// multipolygon with one member far outside.
fn around_monaco() -> PlanetFile {
    PlanetFile::sorted()
        .replicated_at(1_788_739_204, 4_321)
        .with_node(
            Node::at(1, 43.7384, 7.4246)
                .tagged("name", "Monaco")
                .tagged("place", "city"),
        )
        .with_node(Node::at(2, 43.73, 7.42))
        .with_node(Node::at(3, 43.76, 7.45))
        .with_node(Node::at(4, 43.75, 7.44))
        .with_node(Node::at(5, 43.735, 7.43).tagged("name:en", "Monaco"))
        .with_node(Node::at(6, 43.735, 7.38))
        .with_node(Node::at(7, 43.735, 7.46))
        .with_node(Node::at(8, 43.80, 7.50))
        .with_node(Node::at(9, 43.81, 7.51))
        .with_node(Node::at(15, 43.76, 7.40))
        .with_way(Way::through(10, [2, 3]).tagged("highway", "primary"))
        .with_way(Way::through(11, [2, 99]))
        .with_way(Way::through(12, [8, 9, 8]))
        .with_way(Way::through(13, [6, 7]).tagged("highway", "secondary"))
        .with_way(Way::through(14, [5, 15]))
        .with_relation(
            Relation::of(100)
                .tagged("type", "multipolygon")
                .tagged("natural", "water")
                .with_member(MemberKind::Way, 10, "outer")
                .with_member(MemberKind::Way, 12, "inner"),
        )
        .with_relation(
            Relation::of(101)
                .tagged("type", "route")
                .with_member(MemberKind::Way, 10, ""),
        )
}

fn written(planet: &PlanetFile, name: &str) -> std::io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "planetrs-mapclipper-harvesting-{name}-{}.osm.pbf",
        std::process::id()
    ));
    planet.write(&path)?;
    Ok(path)
}

fn harvested_saying(
    planet: &PlanetFile,
    name: &str,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Harvest, Box<dyn std::error::Error>> {
    let path = written(planet, name)?;
    let opened = Planet::open(&path)?;
    let mut index = IdIndex::empty(opened.fingerprint(), opened.blobs.len());
    let (harvest, _) = harvest_area(&opened, "monaco.osm.pbf", &monaco()?, &mut index, progress)?;
    Ok(harvest)
}

fn harvested(planet: &PlanetFile, name: &str) -> Result<Harvest, Box<dyn std::error::Error>> {
    harvested_saying(planet, name, &|_| {})
}

fn node_ids(harvest: &Harvest) -> Vec<i64> {
    harvest.nodes.iter().map(|node| node.id).collect()
}

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn given_a_box_when_harvested_then_every_node_inside_it_is_kept_its_edges_included() -> Outcome {
    let harvest = harvested(&around_monaco(), "inside")?;

    let ids = node_ids(&harvest);

    assert!([1, 2, 4, 5].iter().all(|id| ids.contains(id)), "{ids:?}");
    assert!([6, 7, 15].iter().all(|id| !ids.contains(id)), "{ids:?}");
    Ok(())
}

#[test]
fn given_a_named_node_inside_when_harvested_then_it_keeps_all_its_tags_and_a_translated_name_alone_does_not_count()
-> Outcome {
    let harvest = harvested(&around_monaco(), "named")?;

    assert_eq!(
        harvest.named_nodes,
        vec![NamedNode {
            id: 1,
            latitude: 43_738_400_000,
            longitude: 7_424_600_000,
            tags: tags(&[("name", "Monaco"), ("place", "city")]),
        }]
    );
    Ok(())
}

#[test]
fn given_a_way_with_one_node_inside_when_harvested_then_it_keeps_every_ref_but_only_the_positions_inside()
-> Outcome {
    let harvest = harvested(&around_monaco(), "half-inside")?;

    let way = harvest.ways.iter().find(|way| way.id == 14);

    assert_eq!(way.map(|way| way.refs.clone()), Some(vec![5, 15]));
    assert!(!node_ids(&harvest).contains(&15));
    Ok(())
}

#[test]
fn given_a_way_crossing_the_box_without_a_vertex_inside_when_harvested_then_it_is_left_out() -> Outcome {
    let harvest = harvested(&around_monaco(), "crossing")?;

    assert!(harvest.ways.iter().all(|way| way.id != 13));
    Ok(())
}

#[test]
fn given_a_multipolygon_touching_the_box_when_harvested_then_its_far_members_and_all_their_nodes_come_with_it()
-> Outcome {
    let harvest = harvested(&around_monaco(), "multipolygon")?;

    let multipolygon = harvest.relations.iter().find(|relation| relation.id == 100);

    assert_eq!(
        multipolygon.map(|relation| relation.members.clone()),
        Some(vec![
            Member {
                way: 10,
                role: "outer".to_string()
            },
            Member {
                way: 12,
                role: "inner".to_string()
            },
        ])
    );
    assert!(harvest.ways.iter().any(|way| way.id == 12));
    assert!([3, 8, 9].iter().all(|id| node_ids(&harvest).contains(id)));
    Ok(())
}

#[test]
fn given_a_relation_that_is_not_a_multipolygon_when_harvested_then_it_is_left_out() -> Outcome {
    let harvest = harvested(&around_monaco(), "route")?;

    assert_eq!(
        harvest
            .relations
            .iter()
            .map(|relation| relation.id)
            .collect::<Vec<_>>(),
        vec![100]
    );
    Ok(())
}

#[test]
fn given_a_way_through_a_node_the_planet_lacks_when_harvested_then_the_ref_stays_without_a_position()
-> Outcome {
    let harvest = harvested(&around_monaco(), "dangling")?;

    let way = harvest.ways.iter().find(|way| way.id == 11);

    assert_eq!(way.map(|way| way.refs.clone()), Some(vec![2, 99]));
    assert!(!node_ids(&harvest).contains(&99));
    Ok(())
}

#[test]
fn given_a_whole_box_when_harvested_then_exactly_these_nodes_ways_and_relations_come_back() -> Outcome {
    let harvest = harvested(&around_monaco(), "whole")?;

    assert_eq!(node_ids(&harvest), vec![1, 2, 3, 4, 5, 8, 9]);
    assert_eq!(
        harvest.ways.iter().map(|way| way.id).collect::<Vec<_>>(),
        vec![10, 11, 12, 14]
    );
    Ok(())
}

#[test]
fn given_a_replicated_planet_when_harvested_then_the_harvest_names_the_file_and_the_minute_it_came_from()
-> Outcome {
    let planet = around_monaco();
    let size = planet.to_bytes().len() as u64;

    let harvest = harvested(&planet, "provenance")?;

    assert_eq!(
        harvest.provenance,
        Provenance {
            source_name: "monaco.osm.pbf".to_string(),
            file_size: size,
            blob_count: 3,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 4_321,
            writing_program: "planetrs fixture".to_string(),
        }
    );
    Ok(())
}

#[test]
fn given_a_planet_when_harvested_then_each_pass_says_how_many_of_its_blobs_it_has_read() -> Outcome {
    let said = Mutex::new(Vec::new());

    harvested_saying(&around_monaco(), "progress", &|line| {
        said.lock().expect("one writer at a time").push(line.to_string());
    })?;

    let said = said.into_inner()?;
    for pass in ["nodes", "ways", "relations", "member ways", "member nodes"] {
        assert!(said.contains(&format!("{pass}: 1 / 1 blobs")), "{said:?}");
    }
    Ok(())
}

#[test]
fn given_a_planet_cut_into_small_blobs_when_harvested_then_the_same_ground_comes_back() -> Outcome {
    let whole = harvested(&around_monaco(), "one-blob")?;

    let cut = harvested(&around_monaco().in_blobs_of(2), "small-blobs")?;

    assert_eq!(
        (cut.nodes, cut.named_nodes, cut.ways, cut.relations),
        (whole.nodes, whole.named_nodes, whole.ways, whole.relations)
    );
    Ok(())
}
