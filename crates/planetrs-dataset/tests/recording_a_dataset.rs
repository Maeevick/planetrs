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
