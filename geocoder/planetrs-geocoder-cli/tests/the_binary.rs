//! The binary as a person or a script runs it, against a tiny index written for the occasion.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use planetrs_geocoder::index::{
    AddressPoint, Commune, Country, HouseNumber, Street, StreetIndex, write_index, write_streets,
};

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

fn index_in(directory: &Path) -> PathBuf {
    let index = directory.join("index");
    let sens = Commune {
        relation: 1,
        level: 8,
        names: vec!["SENS".into()],
        postcodes: vec!["89100".into()],
        centre: Some([3.283, 48.197]),
        country: Some("FR".into()),
    };
    let france = Country {
        code: "FR".into(),
        names: vec!["FRANCE".into()],
    };
    write_index(&index, &[sens], &[france], "a test").unwrap();
    let door = AddressPoint {
        number: HouseNumber {
            number: 8,
            suffix: String::new(),
        },
        position: [3.2652, 48.2],
    };
    let tilleuls = Street {
        name: "ROUTE TILLEULS".into(),
        addresses: vec![door],
        path: vec![],
    };
    write_streets(&index, &StreetIndex::of(vec![(1, tilleuls)]), "a test").unwrap();
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
