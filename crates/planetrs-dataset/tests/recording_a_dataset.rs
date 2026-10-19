//! A build recording the parts of a dataset into a directory, and the manifest keeping count.

use std::path::PathBuf;

use planetrs_dataset::directory::{MANIFEST, record};
use planetrs_dataset::{Entry, Manifest, Part, Provenance};

type Outcome = Result<(), Box<dyn std::error::Error>>;

fn planet() -> Provenance {
    Provenance {
        source_name: "planet-260907.osm.pbf".into(),
        file_size: 94_746_421_723,
        blob_count: 52_810,
        replication_timestamp: 1_788_739_204,
        replication_sequence: 4_321,
        writing_program: "planet-dump-ng 1.2.7".into(),
    }
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "planetrs-dataset-recording-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

#[test]
fn given_an_empty_directory_when_a_gazetteer_is_recorded_then_the_file_and_a_manifest_listing_it_appear()
-> Outcome {
    let directory = scratch("first");

    record(
        &directory,
        &planet(),
        Part::Gazetteer,
        "gazetteer.parquet",
        b"PAR1 a gazetteer PAR1",
        3,
    )?;

    assert_eq!(
        std::fs::read(directory.join("gazetteer.parquet"))?,
        b"PAR1 a gazetteer PAR1"
    );
    assert_eq!(
        Manifest::from_json(&std::fs::read_to_string(directory.join(MANIFEST))?)?,
        Manifest {
            planet: planet(),
            files: vec![Entry {
                part: Part::Gazetteer,
                file: "gazetteer.parquet".into(),
                url: None,
                bytes: 21,
                sha256: "6b4914a574a6da19a6a71157fc79486645632c732add132be96f06417c5179ae".into(),
                rows: 3,
            }],
        }
    );
    Ok(())
}

#[test]
fn given_parts_recorded_in_any_order_and_one_again_when_the_manifest_is_read_then_each_part_appears_once_in_a_fixed_order()
-> Outcome {
    let directory = scratch("again");
    let planet = planet();
    let monaco = Part::GeocoderIndex { country: "MC".into() };
    let france = Part::GeocoderIndex { country: "FR".into() };

    record(
        &directory,
        &planet,
        monaco.clone(),
        "geocoder-MC.parquet",
        b"monaco",
        1,
    )?;
    record(
        &directory,
        &planet,
        Part::Gazetteer,
        "gazetteer.parquet",
        b"first gazetteer",
        2,
    )?;
    record(
        &directory,
        &planet,
        france.clone(),
        "geocoder-FR.parquet",
        b"france",
        3,
    )?;
    let manifest = record(
        &directory,
        &planet,
        Part::Gazetteer,
        "gazetteer.parquet",
        b"second gazetteer",
        4,
    )?;

    assert_eq!(
        manifest
            .files
            .iter()
            .map(|entry| (entry.part.clone(), entry.rows))
            .collect::<Vec<_>>(),
        vec![(Part::Gazetteer, 4), (france, 3), (monaco, 1)]
    );
    Ok(())
}
