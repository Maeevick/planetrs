//! The binary as a person or a script runs it, against a tiny planet written for the occasion.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use planetrs_pbf::testing::{Node, PlanetFile, Way};

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

/// The binary harvesting the frame from the fixture planet into `monaco.mapclip`.
fn harvesting(directory: &Path, options: &[&str]) -> Output {
    let planet = directory.join("monaco.osm.pbf");
    monaco().write(&planet).unwrap();
    let planet = planet.display().to_string();
    let arguments: Vec<&str> = options
        .iter()
        .copied()
        .chain(["harvest", planet.as_str()])
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
