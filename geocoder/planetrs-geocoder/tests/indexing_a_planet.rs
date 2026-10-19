//! A planet indexed then geocoded, pinned on a fixture before the index changes format.
//!
//! The fixture is a country, one commune inside it with its outline and centre, a named street, a
//! door on it and a shop: enough for every precision the geocoder answers with, from the door to the
//! commune's centre, to come out of a real build rather than out of an index written by hand.

use std::path::PathBuf;

use planetrs_dataset::{DirectoryFiles, Manifest, Part};
use planetrs_geocoder::{Geocoder, Precision, Site, build_index};
use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};

type Outcome = Result<(), Box<dyn std::error::Error>>;

const SENS_CENTRE: (f64, f64) = (48.197_855_9, 3.282_606);
const DOOR: (f64, f64) = (48.198_06, 3.268_240_2);
const SHOP: (f64, f64) = (48.176_905_6, 3.290_627);

/// A ring of four corners closed on its first node, as one way.
fn ring(way: i64, first_node: i64, south: f64, west: f64, north: f64, east: f64) -> (Vec<Node>, Way) {
    let corners = [(south, west), (south, east), (north, east), (north, west)];
    let nodes = corners
        .iter()
        .zip(first_node..)
        .map(|((latitude, longitude), id)| Node::at(id, *latitude, *longitude))
        .collect();
    let refs = (first_node..first_node + 4).chain([first_node]);
    (nodes, Way::through(way, refs))
}

fn yonne() -> PlanetFile {
    let (country_corners, country_outline) = ring(2001, 1001, 47.9, 3.0, 48.5, 3.6);
    let (commune_corners, commune_outline) = ring(2011, 1011, 48.15, 3.22, 48.24, 3.32);
    let planet = country_corners
        .into_iter()
        .chain(commune_corners)
        .fold(PlanetFile::sorted(), PlanetFile::with_node);
    planet
        .with_node(Node::at(1020, SENS_CENTRE.0, SENS_CENTRE.1).tagged("place", "town"))
        .with_node(Node::at(1031, 48.1975, 3.2675))
        .with_node(Node::at(1032, 48.1985, 3.2690))
        .with_node(
            Node::at(1041, DOOR.0, DOOR.1)
                .tagged("addr:housenumber", "1")
                .tagged("addr:street", "Avenue de la Gare"),
        )
        .with_node(
            Node::at(1051, SHOP.0, SHOP.1)
                .tagged("shop", "supermarket")
                .tagged("name", "ACME")
                .tagged("addr:street", "Chemin des Cannetières"),
        )
        .with_way(country_outline)
        .with_way(commune_outline)
        .with_way(
            Way::through(2031, [1031, 1032])
                .tagged("highway", "residential")
                .tagged("name", "Avenue de la Gare"),
        )
        .with_relation(
            Relation::of(3001)
                .tagged("boundary", "administrative")
                .tagged("admin_level", "2")
                .tagged("ISO3166-1", "FR")
                .tagged("name", "France")
                .with_member(MemberKind::Way, 2001, "outer"),
        )
        .with_relation(
            Relation::of(3011)
                .tagged("boundary", "administrative")
                .tagged("admin_level", "8")
                .tagged("name", "Sens")
                .tagged("postal_code", "89100")
                .tagged("ref:INSEE", "89387")
                .with_member(MemberKind::Node, 1020, "admin_centre")
                .with_member(MemberKind::Way, 2011, "outer"),
        )
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "planetrs-geocoder-indexing-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn indexed(name: &str) -> Result<Geocoder<DirectoryFiles>, Box<dyn std::error::Error>> {
    let directory = scratch(name);
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("yonne.osm.pbf");
    yonne().write(&planet)?;
    let index = directory.join("index");
    build_index(&planet, &index, &["FR"], &|_| {})?;
    Ok(Geocoder::open(&index)?)
}

fn site(label: &str, address: &str) -> Site {
    Site {
        label: label.into(),
        address: address.into(),
        postcode: "89100".into(),
        town: "Sens".into(),
        country: "France".into(),
    }
}

fn reads_about(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-7
}

#[test]
fn given_a_door_on_a_named_street_when_its_address_is_geocoded_then_the_door_answers() -> Outcome {
    let mut geocoder = indexed("door")?;

    let answer = geocoder
        .locate(&site("", "1 Avenue de la Gare"))?
        .ok_or("no answer")?;

    assert_eq!(answer.precision, Precision::HouseNumber);
    assert!(reads_about(answer.latitude(), DOOR.0), "{}", answer.latitude());
    assert!(reads_about(answer.longitude(), DOOR.1), "{}", answer.longitude());
    Ok(())
}

#[test]
fn given_a_street_the_commune_lacks_when_geocoded_then_the_communes_centre_answers_with_its_area() -> Outcome
{
    let mut geocoder = indexed("centre")?;

    let answer = geocoder
        .locate(&site("", "12 Rue Inconnue"))?
        .ok_or("no answer")?;

    assert_eq!(answer.precision, Precision::Commune);
    assert!(
        reads_about(answer.latitude(), SENS_CENTRE.0),
        "{}",
        answer.latitude()
    );
    assert!(
        reads_about(answer.longitude(), SENS_CENTRE.1),
        "{}",
        answer.longitude()
    );
    assert!(
        answer
            .commune_area
            .is_some_and(|area| (70.0..80.0).contains(&area)),
        "{:?}",
        answer.commune_area
    );
    Ok(())
}

#[test]
fn given_the_only_shop_of_a_label_when_its_site_has_no_address_then_the_shop_answers() -> Outcome {
    let mut geocoder = indexed("shop")?;

    let answer = geocoder.locate(&site("ACME", ""))?.ok_or("no answer")?;

    assert_eq!(answer.precision, Precision::Shop);
    assert!(reads_about(answer.latitude(), SHOP.0), "{}", answer.latitude());
    assert!(reads_about(answer.longitude(), SHOP.1), "{}", answer.longitude());
    Ok(())
}

#[test]
fn given_a_planet_indexed_for_france_when_its_directory_is_read_then_a_manifest_lists_the_gazetteer_and_frances_index()
-> Outcome {
    let directory = scratch("manifest");
    std::fs::create_dir_all(&directory)?;
    let planet = directory.join("yonne.osm.pbf");
    yonne().write(&planet)?;
    let index = directory.join("index");

    build_index(&planet, &index, &["FR"], &|_| {})?;

    let manifest = Manifest::from_json(&std::fs::read_to_string(index.join("manifest.json"))?)?;
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|entry| entry.part.clone())
            .collect::<Vec<_>>(),
        vec![Part::Gazetteer, Part::GeocoderIndex { country: "FR".into() }]
    );
    assert_eq!(manifest.planet.source_name, "yonne.osm.pbf");
    Ok(())
}
