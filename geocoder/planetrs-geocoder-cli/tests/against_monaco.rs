//! The whole chain against the real world: download Geofabrik's extract of Monaco, index it, and
//! geocode a public address. It needs the network, so it runs only on request:
//!
//! ```sh
//! make test-system
//! cargo test -p planetrs-geocoder-cli --release -- --ignored
//! ```

use std::path::Path;
use std::process::{Command, Output};

const MONACO: &str = "https://download.geofabrik.de/europe/monaco-latest.osm.pbf";
const MUSEUM: (f64, f64) = (43.730_747, 7.425_299);

fn run(data: &Path, arguments: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_planetrs-geocoder"))
        .arg("--data-dir")
        .arg(data)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn metres_between((latitude, longitude): (f64, f64), (other_latitude, other_longitude): (f64, f64)) -> f64 {
    let (from, to) = (latitude.to_radians(), other_latitude.to_radians());
    let half = ((to - from) / 2.0).sin().powi(2)
        + from.cos() * to.cos() * ((other_longitude - longitude).to_radians() / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * half.sqrt().asin()
}

#[test]
#[ignore = "downloads from download.geofabrik.de"]
fn given_monaco_downloaded_and_indexed_when_a_public_address_is_geocoded_then_it_lands_beside_it() {
    let data = std::env::temp_dir().join(format!("planetrs-monaco-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let planet = data.join("planets").join("monaco-latest.osm.pbf");

    run(&data, &["download", "--from", MONACO]);
    run(
        &data,
        &[
            "index",
            "--planet",
            &planet.display().to_string(),
            "--countries",
            "MC",
        ],
    );
    let answer = run(
        &data,
        &["Musée océanographique;Avenue Saint-Martin;98000;Monaco;Monaco"],
    );

    let printed = String::from_utf8_lossy(&answer.stdout).into_owned();
    let fields: Vec<&str> = printed
        .lines()
        .nth(1)
        .expect("an answer row")
        .split(';')
        .collect();
    let at = (
        fields[5].parse::<f64>().unwrap(),
        fields[6].parse::<f64>().unwrap(),
    );
    assert!(
        ["HouseNumber", "NearestHouseNumber", "Shop", "Street"].contains(&fields[7]),
        "{printed}"
    );
    assert!(metres_between(at, MUSEUM) < 500.0, "{printed}");
}
