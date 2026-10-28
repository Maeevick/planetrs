//! The binary as a person or a script runs it, against a tiny planet written for the occasion.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use planetrs_cli::testing::RangeServer;
use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{Manifest, Part};
use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};

const ATTRIBUTION: &str =
    "Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";
const FRAME: [&str; 8] = [
    "--lat",
    "43.735",
    "--lon",
    "7.42",
    "--width-km",
    "2",
    "--height-km",
    "1.14",
];

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("planetrs-mapclipper-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// A quay and a named square in Monaco, enough ground for a map.
fn monaco() -> PlanetFile {
    PlanetFile::sorted()
        .replicated_at(1_788_739_204, 4_321)
        .with_node(Node::at(1, 43.7347, 7.4186))
        .with_node(Node::at(2, 43.7352, 7.4214))
        .with_node(
            Node::at(3, 43.7350, 7.4200)
                .tagged("name", "Place d'Armes")
                .tagged("place", "square"),
        )
        .with_way(Way::through(10, [1, 2]).tagged("man_made", "quay"))
}

/// The same Monaco with the principality's border around it, so that a map index can be built of it.
fn monaco_with_its_border() -> PlanetFile {
    let corners = [(43.70, 7.39), (43.70, 7.45), (43.76, 7.45), (43.76, 7.39)];
    corners
        .iter()
        .zip(1001..)
        .fold(monaco(), |planet, ((latitude, longitude), id)| {
            planet.with_node(Node::at(id, *latitude, *longitude))
        })
        .with_way(Way::through(2001, [1001, 1002, 1003, 1004, 1001]))
        .with_relation(
            Relation::of(3001)
                .tagged("type", "boundary")
                .tagged("boundary", "administrative")
                .tagged("admin_level", "2")
                .tagged("ISO3166-1", "MC")
                .with_member(MemberKind::Way, 2001, "outer"),
        )
}

fn manifest_of(dataset: &Path) -> Manifest {
    Manifest::from_json(&std::fs::read_to_string(dataset.join(MANIFEST)).unwrap()).unwrap()
}

fn parts_of(dataset: &Path) -> Vec<Part> {
    manifest_of(dataset)
        .files
        .into_iter()
        .map(|entry| entry.part)
        .collect()
}

/// The binary indexing Monaco, border included, into the dataset `dataset`.
fn indexing(directory: &Path, dataset: &str) -> Output {
    let planet = directory.join("monaco.osm.pbf");
    monaco_with_its_border().write(&planet).unwrap();
    run(
        directory,
        &[
            "--dataset",
            dataset,
            "index",
            "--planet",
            &planet.display().to_string(),
            "--countries",
            "mc",
        ],
    )
}

fn mapclipper(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_planetrs-mapclipper"));
    command
        .current_dir(directory)
        .env_remove("PLANETRS_DATA_DIR")
        .env("HOME", directory)
        .env_remove("XDG_DATA_HOME");
    command
}

fn run(directory: &Path, arguments: &[&str]) -> Output {
    mapclipper(directory).args(arguments).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The files in `directory` named `stem-…extension`, as a dated fallback name is.
fn dated(directory: &Path, stem: &str, extension: &str) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(&format!("{stem}-")) && name.ends_with(extension))
        .collect()
}

/// The binary harvesting the frame from the fixture planet into `monaco.mapclip`.
fn harvesting(directory: &Path, options: &[&str]) -> Output {
    let planet = directory.join("monaco.osm.pbf");
    monaco().write(&planet).unwrap();
    let planet = planet.display().to_string();
    let arguments: Vec<&str> = options
        .iter()
        .copied()
        .chain(["harvest", "--planet", planet.as_str()])
        .chain(FRAME)
        .chain(["--out", "monaco.mapclip"])
        .collect();
    run(directory, &arguments)
}

/// A harvest of the frame, cut by the binary itself from the fixture planet.
fn harvest_in(directory: &Path) -> String {
    let harvested = harvesting(directory, &[]);
    assert!(harvested.status.success(), "{}", text(&harvested.stderr));
    "monaco.mapclip".to_string()
}

/// The binary drawing a harvest of the frame into `map.svg`.
fn rendering(directory: &Path, harvest: &str, options: &[&str]) -> Output {
    let arguments: Vec<&str> = options
        .iter()
        .copied()
        .chain(["render", harvest])
        .chain(FRAME)
        .chain(["--svg", "map.svg"])
        .collect();
    run(directory, &arguments)
}

#[test]
fn given_a_harvest_too_small_for_the_frame_when_rendered_then_it_fails_with_one_naming_both_extents_and_what_to_try()
 {
    let directory = scratch("too-small");
    let harvest = harvest_in(&directory);

    let output = run(
        &directory,
        &[
            "render",
            &harvest,
            "--lat",
            "43.735",
            "--lon",
            "7.42",
            "--width-km",
            "40",
            "--height-km",
            "22.8",
            "--svg",
            "map.svg",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let errors = text(&output.stderr);
    assert!(
        errors.starts_with(
            "error: the harvest covers 3.0 x 1.7 km and the area asked for collects 60.0 x 34.2 km"
        ),
        "{errors}"
    );
    assert!(
        errors.contains("\n  → harvest a wider area, or lower --margin, then render again"),
        "{errors}"
    );
}

#[test]
fn given_a_planet_when_harvested_and_rendered_then_both_runs_end_with_the_attribution_on_stderr() {
    let directory = scratch("attribution");

    let harvested = harvesting(&directory, &[]);
    let rendered = rendering(&directory, "monaco.mapclip", &[]);

    assert!(rendered.status.success(), "{}", text(&rendered.stderr));
    assert_eq!(text(&harvested.stderr).lines().last(), Some(ATTRIBUTION));
    assert_eq!(text(&rendered.stderr).lines().last(), Some(ATTRIBUTION));
}

#[test]
fn given_quiet_before_or_after_the_command_when_harvested_and_rendered_then_stderr_holds_only_the_attribution()
 {
    let directory = scratch("quiet");

    let harvested = harvesting(&directory, &["-q"]);
    let rendered = run(
        &directory,
        &[
            "render",
            "monaco.mapclip",
            "--lat",
            "43.735",
            "--lon",
            "7.42",
            "--width-km",
            "2",
            "--height-km",
            "1.14",
            "--svg",
            "map.svg",
            "--quiet",
        ],
    );

    assert_eq!(text(&harvested.stderr), format!("{ATTRIBUTION}\n"));
    assert_eq!(text(&rendered.stderr), format!("{ATTRIBUTION}\n"));
}

#[test]
fn given_the_map_and_the_harvest_exist_when_cut_again_then_both_are_kept_and_dated_files_are_written() {
    let directory = scratch("never-overwrite");
    let harvest = harvest_in(&directory);
    let first_harvest = std::fs::read(directory.join(&harvest)).unwrap();
    std::fs::write(directory.join("map.svg"), "kept").unwrap();

    let rendered = rendering(&directory, &harvest, &[]);
    let harvested = harvesting(&directory, &[]);

    assert!(rendered.status.success(), "{}", text(&rendered.stderr));
    assert!(harvested.status.success(), "{}", text(&harvested.stderr));
    assert_eq!(
        std::fs::read_to_string(directory.join("map.svg")).unwrap(),
        "kept"
    );
    assert_eq!(std::fs::read(directory.join(&harvest)).unwrap(), first_harvest);
    let maps = dated(&directory, "map", ".svg");
    let harvests = dated(&directory, "monaco", ".mapclip");
    assert_eq!((maps.len(), harvests.len()), (1, 1), "{maps:?} {harvests:?}");
    assert!(
        text(&rendered.stderr).contains(&maps[0]),
        "the name used is printed"
    );
    assert!(
        text(&harvested.stderr).contains(&harvests[0]),
        "the name used is printed"
    );
}

#[test]
fn given_a_harvest_when_inspected_as_json_then_stdout_names_the_planet_it_was_cut_from() {
    let directory = scratch("inspect");
    let harvest = harvest_in(&directory);

    let output = run(&directory, &["inspect", &harvest, "-O", "json"]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).contains("\"source\":\"monaco.osm.pbf\""),
        "{}",
        text(&output.stdout)
    );
}

#[test]
fn given_a_planet_when_indexed_then_the_dataset_holds_its_countrys_map_index_in_one_tile_and_stderr_ends_with_the_attribution()
 {
    let directory = scratch("index");

    let indexed = indexing(&directory, "dataset");

    assert!(indexed.status.success(), "{}", text(&indexed.stderr));
    assert_eq!(
        parts_of(&directory.join("dataset")),
        vec![Part::MapSummary, Part::MapTile { level: 0, x: 0, y: 0 }]
    );
    assert_eq!(text(&indexed.stderr).lines().last(), Some(ATTRIBUTION));
}

#[test]
fn given_a_dataset_of_monaco_when_a_box_inside_it_is_harvested_without_a_planet_then_it_is_the_planets_harvest_to_the_byte()
 {
    let directory = scratch("from-the-dataset");
    let indexed = indexing(&directory, "dataset");
    assert!(indexed.status.success(), "{}", text(&indexed.stderr));
    let planet = directory.join("monaco.osm.pbf").display().to_string();
    let harvest = |options: &[&str], out: &str| {
        let arguments: Vec<&str> = ["--dataset", "dataset", "harvest"]
            .into_iter()
            .chain(options.iter().copied())
            .chain(FRAME)
            .chain(["--out", out])
            .collect();
        run(&directory, &arguments)
    };

    let from_dataset = harvest(&[], "from-dataset.mapclip");
    let from_planet = harvest(&["--planet", &planet], "from-planet.mapclip");

    assert!(from_dataset.status.success(), "{}", text(&from_dataset.stderr));
    assert!(from_planet.status.success(), "{}", text(&from_planet.stderr));
    assert_eq!(
        std::fs::read(directory.join("from-dataset.mapclip")).unwrap(),
        std::fs::read(directory.join("from-planet.mapclip")).unwrap()
    );
}

/// A file standing for a planet, and its checksum as planet.openstreetmap.org publishes one.
const PUBLISHED_PLANET: &[u8] = b"a planet of Monaco, published with its checksum";
const PUBLISHED_CHECKSUM: &str = "4124c6bf7c53aaa75c88ba15c36948bf  monaco-latest.osm.pbf\n";

#[test]
fn given_a_planet_published_with_its_checksum_when_downloaded_then_it_takes_its_name_in_the_data_directory_and_the_next_step_is_to_index_it()
 {
    let directory = scratch("download");
    let server = RangeServer::serving(&[
        ("monaco-latest.osm.pbf", PUBLISHED_PLANET),
        ("monaco-latest.osm.pbf.md5", PUBLISHED_CHECKSUM.as_bytes()),
    ])
    .unwrap();
    let source = format!("{}monaco-latest.osm.pbf", server.url());

    let output = run(&directory, &["--data-dir", "data", "download", "--from", &source]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        std::fs::read(directory.join("data/planets/monaco-latest.osm.pbf")).unwrap(),
        PUBLISHED_PLANET
    );
    assert!(
        text(&output.stderr).contains("→ next: planetrs-mapclipper index --planet"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn given_a_planet_without_any_country_when_indexed_as_a_whole_then_its_map_index_answers_with_the_planets_harvest()
 {
    let directory = scratch("world");
    let planet = directory.join("monaco.osm.pbf");
    monaco().write(&planet).unwrap();
    let planet = planet.display().to_string();
    let harvest = |source: &[&str], out: &str| {
        let arguments: Vec<&str> = source
            .iter()
            .copied()
            .chain(FRAME)
            .chain(["--out", out])
            .collect();
        run(&directory, &arguments)
    };

    let indexed = run(
        &directory,
        &["--dataset", "dataset", "index", "--planet", &planet, "--world"],
    );
    let from_dataset = harvest(&["--dataset", "dataset", "harvest"], "from-dataset.mapclip");
    let from_planet = harvest(&["harvest", "--planet", &planet], "from-planet.mapclip");

    assert!(indexed.status.success(), "{}", text(&indexed.stderr));
    assert!(
        text(&indexed.stderr).contains("indexed the planet:"),
        "{}",
        text(&indexed.stderr)
    );
    assert!(from_dataset.status.success(), "{}", text(&from_dataset.stderr));
    assert!(from_planet.status.success(), "{}", text(&from_planet.stderr));
    assert_eq!(
        std::fs::read(directory.join("from-dataset.mapclip")).unwrap(),
        std::fs::read(directory.join("from-planet.mapclip")).unwrap()
    );
}
