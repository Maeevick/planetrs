//! A build recording the parts of a dataset into a directory, and the manifest keeping count.

use std::path::PathBuf;

use planetrs_dataset::directory::{
    CarriedOver, MANIFEST, RecordError, Written, carry_over, record, record_all_written, record_written,
    written_beside,
};
use planetrs_dataset::testing::FilesInMemory;
use planetrs_dataset::{Dataset, DirectoryFiles, Entry, Manifest, Part, Provenance, geocoder_index};

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
                bbox: None,
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

#[test]
fn given_a_dataset_of_one_planet_when_a_part_of_another_is_recorded_then_it_is_refused_and_never_written()
-> Outcome {
    let directory = scratch("other-planet");
    record(
        &directory,
        &planet(),
        Part::Gazetteer,
        "gazetteer.parquet",
        b"september",
        2,
    )?;
    let october = Provenance {
        source_name: "planet-261012.osm.pbf".into(),
        replication_timestamp: 1_791_763_204,
        ..planet()
    };

    let refused = record(
        &directory,
        &october,
        Part::GeocoderIndex { country: "FR".into() },
        "geocoder-FR.parquet",
        b"october",
        3,
    );

    assert!(
        matches!(&refused, Err(RecordError::OtherPlanet { recorded, offered })
            if recorded.source_name == "planet-260907.osm.pbf" && offered.source_name == "planet-261012.osm.pbf"),
        "{refused:?}"
    );
    assert!(!directory.join("geocoder-FR.parquet").exists());
    Ok(())
}

#[test]
fn given_a_dataset_read_once_through_a_cache_when_its_parts_are_gone_then_the_same_reads_answer_from_the_cache()
-> Outcome {
    let directory = scratch("cached");
    let cache = scratch("cache");
    let planet = planet();
    let entries = vec![
        geocoder_index::Entry::Names { count: 1 },
        geocoder_index::Entry::Middle {
            commune: 946_433,
            street: "AVENUE GARE".into(),
            position: [3.268_208_2, 48.197_500_6],
        },
    ];
    let index = geocoder_index::to_parquet("FR", &entries, 0.0)?;
    record(
        &directory,
        &planet,
        Part::GeocoderIndex { country: "FR".into() },
        "geocoder-FR.parquet",
        &index,
        2,
    )?;
    let first = Dataset::open(DirectoryFiles::at(&directory))?.cached_in(&cache);
    let france = first.country_index("FR")?.ok_or("France is indexed")?;
    let read_first = first.commune(&france, 946_433)?;
    let manifest = std::fs::read(directory.join(MANIFEST))?;

    let only_the_manifest = FilesInMemory::default().with(MANIFEST, &manifest);
    let again = Dataset::open(only_the_manifest)?.cached_in(&cache);
    let france_again = again.country_index("FR")?.ok_or("France is indexed")?;
    let read_again = again.commune(&france_again, 946_433)?;

    assert_eq!(read_again, read_first);
    assert_eq!(read_first, entries[1..].to_vec());
    Ok(())
}

fn parts_listed_in(directory: &std::path::Path) -> Result<Vec<Part>, Box<dyn std::error::Error>> {
    let manifest = Manifest::from_json(&std::fs::read_to_string(directory.join(MANIFEST))?)?;
    Ok(manifest.files.into_iter().map(|entry| entry.part).collect())
}

#[test]
fn given_both_tools_parts_and_a_rebuild_of_the_geocoders_when_carried_over_then_only_the_map_index_joins_the_rebuild()
-> Outcome {
    let (previous, building) = (scratch("carry-previous"), scratch("carry-building"));
    let planet = planet();
    let map_of_france = Part::MapIndex { country: "FR".into() };
    record(
        &previous,
        &planet,
        Part::Gazetteer,
        "gazetteer.parquet",
        b"old gazetteer",
        1,
    )?;
    record(
        &previous,
        &planet,
        Part::GeocoderIndex { country: "FR".into() },
        "geocoder-FR.parquet",
        b"old france",
        1,
    )?;
    record(
        &previous,
        &planet,
        map_of_france.clone(),
        "map-FR.parquet",
        b"map of france",
        1,
    )?;
    record(
        &building,
        &planet,
        Part::Gazetteer,
        "gazetteer.parquet",
        b"new gazetteer",
        1,
    )?;
    record(
        &building,
        &planet,
        Part::GeocoderIndex { country: "MC".into() },
        "geocoder-MC.parquet",
        b"new monaco",
        1,
    )?;

    let carried = carry_over(&previous, &building, |part| matches!(part, Part::MapIndex { .. }))?;

    assert_eq!(
        parts_listed_in(&building)?,
        vec![
            Part::Gazetteer,
            Part::GeocoderIndex { country: "MC".into() },
            map_of_france.clone()
        ]
    );
    assert_eq!(std::fs::read(building.join("map-FR.parquet"))?, b"map of france");
    assert_eq!(
        std::fs::read(building.join("gazetteer.parquet"))?,
        b"new gazetteer"
    );
    assert!(!building.join("geocoder-FR.parquet").exists());
    assert_eq!(
        carried,
        CarriedOver {
            kept: vec![map_of_france],
            left: vec![]
        }
    );
    Ok(())
}

#[test]
fn given_a_dataset_of_an_older_planet_when_carried_over_then_nothing_joins_the_rebuild_and_the_other_tools_parts_are_left_behind()
-> Outcome {
    let (previous, building) = (scratch("carry-older-previous"), scratch("carry-older-building"));
    let older = Provenance {
        source_name: "planet-260831.osm.pbf".into(),
        replication_timestamp: 1_788_134_404,
        ..planet()
    };
    let map_of_france = Part::MapIndex { country: "FR".into() };
    record(
        &previous,
        &older,
        Part::Gazetteer,
        "gazetteer.parquet",
        b"old gazetteer",
        1,
    )?;
    record(
        &previous,
        &older,
        map_of_france.clone(),
        "map-FR.parquet",
        b"older map of france",
        1,
    )?;
    record(
        &building,
        &planet(),
        Part::Gazetteer,
        "gazetteer.parquet",
        b"new gazetteer",
        1,
    )?;

    let carried = carry_over(&previous, &building, |part| matches!(part, Part::MapIndex { .. }))?;

    assert_eq!(parts_listed_in(&building)?, vec![Part::Gazetteer]);
    assert!(!building.join("map-FR.parquet").exists());
    assert_eq!(
        carried,
        CarriedOver {
            kept: vec![],
            left: vec![map_of_france]
        }
    );
    Ok(())
}

#[test]
fn given_a_part_written_beside_the_dataset_when_recorded_then_it_takes_its_name_and_the_manifest_lists_it_with_its_checksum()
-> Outcome {
    let directory = scratch("written");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        written_beside(&directory, "map-MC.parquet"),
        b"PAR1 a gazetteer PAR1",
    )?;

    record_written(
        &directory,
        &planet(),
        Part::MapIndex { country: "MC".into() },
        "map-MC.parquet",
        3,
    )?;

    assert_eq!(
        std::fs::read(directory.join("map-MC.parquet"))?,
        b"PAR1 a gazetteer PAR1"
    );
    assert!(!written_beside(&directory, "map-MC.parquet").exists());
    let manifest = Manifest::from_json(&std::fs::read_to_string(directory.join(MANIFEST))?)?;
    assert_eq!(
        manifest.files,
        vec![Entry {
            part: Part::MapIndex { country: "MC".into() },
            file: "map-MC.parquet".into(),
            url: None,
            bytes: 21,
            sha256: "6b4914a574a6da19a6a71157fc79486645632c732add132be96f06417c5179ae".into(),
            rows: 3,
            bbox: None,
        }]
    );
    Ok(())
}

fn is_map(part: &Part) -> bool {
    matches!(part, Part::MapSummary | Part::MapTile { .. })
}

#[test]
fn given_a_map_index_recorded_in_two_tiles_when_a_rebuild_records_it_in_one_then_the_manifest_lists_the_new_parts_alone_and_the_old_tiles_are_gone()
-> Outcome {
    let directory = scratch("map-index");
    record(
        &directory,
        &planet(),
        Part::Gazetteer,
        "gazetteer.parquet",
        b"PAR1 a gazetteer PAR1",
        3,
    )?;
    let tile = |level: u8, x: u32, y: u32| Part::MapTile { level, x, y };
    let beside = |name: &str, bytes: &[u8]| std::fs::write(written_beside(&directory, name), bytes);
    let written = |part: Part, name: &str, rows: u64, bbox: Option<[f64; 4]>| Written {
        part,
        name: name.into(),
        rows,
        bbox,
    };
    beside("map-summary.json", b"{\"countries\":[\"FR\",\"MC\"]}")?;
    beside("map-1-0-1.parquet", b"PAR1 abidjan PAR1")?;
    beside("map-1-1-1.parquet", b"PAR1 monaco PAR1")?;
    record_all_written(
        &directory,
        &planet(),
        is_map,
        vec![
            written(Part::MapSummary, "map-summary.json", 2, None),
            written(tile(1, 0, 1), "map-1-0-1.parquet", 1, Some([-4.012, 5.269, -4.010, 5.270])),
            written(tile(1, 1, 1), "map-1-1-1.parquet", 6, Some([7.4186, 43.7347, 7.51, 43.81])),
        ],
    )?;
    beside("map-summary.json", b"{\"countries\":[\"MC\"]}")?;
    beside("map-0-0-0.parquet", b"PAR1 monaco alone PAR1")?;

    record_all_written(
        &directory,
        &planet(),
        is_map,
        vec![
            written(Part::MapSummary, "map-summary.json", 1, None),
            written(tile(0, 0, 0), "map-0-0-0.parquet", 6, Some([7.4186, 43.7347, 7.51, 43.81])),
        ],
    )?;

    assert_eq!(
        parts_listed_in(&directory)?,
        vec![Part::Gazetteer, Part::MapSummary, tile(0, 0, 0)]
    );
    assert!(!directory.join("map-1-0-1.parquet").exists());
    assert!(!directory.join("map-1-1-1.parquet").exists());
    assert_eq!(
        std::fs::read(directory.join("map-0-0-0.parquet"))?,
        b"PAR1 monaco alone PAR1"
    );
    Ok(())
}
