//! The binary as a person or a script runs it, against a tiny index written for the occasion.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};

const SITE: &str = "Acme;8 Route des Tilleuls;89100;Sens;France";
const HEADER: &str = "label;address;postal code;city;country;lat;lon;precision";
const ANSWER: &str = "Acme;8 Route des Tilleuls;89100;Sens;France;48.200000;3.265200;HouseNumber";
const ATTRIBUTION: &str =
    "Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("planetrs-geocoder-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

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

/// France around Sens, Sens with its outline and centre, and a door on the Route des Tilleuls.
fn yonne() -> PlanetFile {
    let (country_corners, country_outline) = ring(2001, 1001, 47.9, 3.0, 48.5, 3.6);
    let (commune_corners, commune_outline) = ring(2011, 1011, 48.15, 3.22, 48.24, 3.32);
    country_corners
        .into_iter()
        .chain(commune_corners)
        .fold(PlanetFile::sorted(), PlanetFile::with_node)
        .with_node(Node::at(1020, 48.197, 3.283).tagged("place", "town"))
        .with_node(Node::at(1031, 48.1995, 3.2645))
        .with_node(Node::at(1032, 48.2005, 3.2660))
        .with_node(
            Node::at(1041, 48.2, 3.2652)
                .tagged("addr:housenumber", "8")
                .tagged("addr:street", "Route des Tilleuls"),
        )
        .with_way(country_outline)
        .with_way(commune_outline)
        .with_way(
            Way::through(2031, [1031, 1032])
                .tagged("highway", "residential")
                .tagged("name", "Route des Tilleuls"),
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
                .with_member(MemberKind::Node, 1020, "admin_centre")
                .with_member(MemberKind::Way, 2011, "outer"),
        )
}

/// A dataset built by the binary itself from the fixture planet, as a person would build one.
fn index_in(directory: &Path) -> PathBuf {
    let planet = directory.join("yonne.osm.pbf");
    yonne().write(&planet).unwrap();
    let index = directory.join("index");
    let built = run(
        directory,
        &[
            "--index",
            &index.display().to_string(),
            "index",
            "--planet",
            &planet.display().to_string(),
            "--countries",
            "FR",
        ],
    );
    assert!(built.status.success(), "{}", text(&built.stderr));
    index
}

fn geocoder(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_planetrs-geocoder"));
    command
        .current_dir(directory)
        .env_remove("PLANETRS_DATA_DIR")
        .env("HOME", directory)
        .env_remove("XDG_DATA_HOME");
    command
}

fn run(directory: &Path, arguments: &[&str]) -> Output {
    geocoder(directory).args(arguments).output().unwrap()
}

fn run_with_stdin(directory: &Path, arguments: &[&str], input: &str) -> Output {
    let mut child = geocoder(directory)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn index_argument(directory: &Path) -> String {
    index_in(directory).display().to_string()
}

#[test]
fn given_one_site_when_geocoded_then_stdout_is_the_header_and_its_answer_and_stderr_ends_with_the_attribution()
 {
    let directory = scratch("one-site");
    let index = index_argument(&directory);

    let output = run(&directory, &["--index", &index, SITE]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("{HEADER}\n{ANSWER}\n"));
    assert_eq!(text(&output.stderr).lines().last(), Some(ATTRIBUTION));
}

#[test]
fn given_a_file_when_geocoded_then_its_rows_are_the_one_site_answers() {
    let directory = scratch("file");
    let index = index_argument(&directory);
    let other = "Bazar;3 Rue Inconnue;89100;Sens;France";
    std::fs::write(
        directory.join("sites.csv"),
        format!("label;address;postal code;city;country\n{SITE}\n{other}\n"),
    )
    .unwrap();

    let output = run(
        &directory,
        &["--index", &index, "-f", "sites.csv", "-o", "answers.csv"],
    );
    let alone = run(&directory, &["--index", &index, other]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let written = std::fs::read_to_string(directory.join("answers.csv")).unwrap();
    let rows: Vec<&str> = written.lines().collect();
    assert_eq!(rows[..2], [HEADER, ANSWER]);
    assert_eq!(Some(rows[2]), text(&alone.stdout).lines().nth(1));
}

#[test]
fn given_the_output_file_exists_when_geocoded_again_then_it_is_kept_and_a_dated_file_is_written() {
    let directory = scratch("never-overwrite");
    let index = index_argument(&directory);
    std::fs::write(
        directory.join("sites.csv"),
        format!("label;address;postal code;city;country\n{SITE}\n"),
    )
    .unwrap();
    std::fs::write(directory.join("output.csv"), "kept").unwrap();

    let output = run(&directory, &["--index", &index, "-f", "sites.csv"]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        std::fs::read_to_string(directory.join("output.csv")).unwrap(),
        "kept"
    );
    let dated: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("output-") && name.ends_with(".csv"))
        .collect();
    assert_eq!(dated.len(), 1, "{dated:?}");
    assert!(
        text(&output.stderr).contains(&dated[0]),
        "the name used is printed"
    );
}

#[test]
fn given_sites_on_stdin_and_output_to_stdout_when_geocoded_then_answers_flow_through() {
    let directory = scratch("pipe");
    let index = index_argument(&directory);

    let output = run_with_stdin(
        &directory,
        &["--index", &index, "-f", "-", "-o", "-", "--quiet"],
        &format!("\u{feff}Label;Address;Postal Code;City;Country\r\n{SITE}\r\n"),
    );

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("\u{feff}{HEADER}\n{ANSWER}\n"));
    assert_eq!(text(&output.stderr), format!("{ATTRIBUTION}\n"));
}

#[test]
fn given_a_town_the_index_does_not_know_when_geocoded_then_the_row_says_not_found_and_the_run_succeeds() {
    let directory = scratch("not-found");
    let index = index_argument(&directory);

    let output = run(
        &directory,
        &["--index", &index, "Acme;1 Rue Haute;99999;Nulle Part;France"],
    );

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).ends_with(";;;NotFound\n"),
        "{}",
        text(&output.stdout)
    );
}

#[test]
fn given_no_index_when_geocoded_then_it_fails_with_one_and_says_how_to_build_one() {
    let directory = scratch("no-index");

    let output = run(&directory, &["--index", "missing", SITE]);

    assert_eq!(output.status.code(), Some(1));
    let errors = text(&output.stderr);
    assert!(errors.contains("no index in missing"), "{errors}");
    assert!(errors.contains("planetrs-geocoder index"), "{errors}");
}

#[test]
fn given_a_site_without_five_fields_when_geocoded_then_it_fails_with_two_before_opening_any_index() {
    let directory = scratch("malformed-site");

    let output = run(&directory, &["--index", "missing", "8 Route des Tilleuls;Sens"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("five fields"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn given_a_file_with_a_bad_row_when_geocoded_then_it_fails_with_two_naming_the_line_and_writes_nothing() {
    let directory = scratch("malformed-file");
    let index = index_argument(&directory);
    std::fs::write(
        directory.join("sites.csv"),
        format!("label;address;postal code;city;country\n{SITE}\nAcme;Sens\n"),
    )
    .unwrap();

    let output = run(&directory, &["--index", &index, "-f", "sites.csv"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("line 3: 2 fields"),
        "{}",
        text(&output.stderr)
    );
    assert!(!directory.join("output.csv").exists());
}

#[test]
fn given_a_remote_index_when_asked_then_it_fails_with_two_as_not_supported_yet() {
    let directory = scratch("remote");

    let output = run(&directory, &["--index", "https://example.org/index", SITE]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("not supported yet"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn given_a_site_when_traced_then_the_last_line_is_its_answer() {
    let directory = scratch("trace");
    let index = index_argument(&directory);

    let output = run(&directory, &["--index", &index, "trace", SITE]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let traced = text(&output.stdout);
    assert!(
        traced
            .lines()
            .last()
            .is_some_and(|line| line.contains("HouseNumber")),
        "{traced}"
    );
}

#[test]
fn given_an_empty_data_directory_when_status_is_asked_then_it_says_what_to_do_first() {
    let directory = scratch("status");

    let output = run(&directory, &["status", "--data-dir", "data"]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let shown = text(&output.stdout);
    assert!(
        shown.contains("data directory  data (from --data-dir)"),
        "{shown}"
    );
    assert!(shown.contains("→ planetrs-geocoder download"), "{shown}");
}

#[test]
fn given_index_with_no_planet_anywhere_when_run_then_it_fails_and_says_where_to_get_one() {
    let directory = scratch("index-no-planet");

    let output = run(&directory, &["--data-dir", "data", "index"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("planetrs-geocoder download"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn given_the_long_help_when_asked_then_it_ends_with_the_attribution() {
    let directory = scratch("help");

    let output = run(&directory, &["--help"]);

    assert!(output.status.success());
    assert_eq!(text(&output.stdout).trim_end().lines().last(), Some(ATTRIBUTION));
}

#[test]
fn given_a_file_and_a_command_together_when_run_then_it_fails_with_two_naming_the_command() {
    let directory = scratch("mixed");

    let output = run(&directory, &["-f", "sites.csv", "status"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("status takes no site"),
        "{}",
        text(&output.stderr)
    );
}
