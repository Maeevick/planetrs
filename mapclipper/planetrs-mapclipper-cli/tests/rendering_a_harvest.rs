//! End to end over the real commands, with a harvest built in memory rather than read from a planet.
//!
//! These write files, so they are integrated rather than unit tests, and they are the only place
//! that proves the CLI's own wiring: that `render` reads a harvest, refuses one that is too small,
//! and writes the same bytes twice.

use std::path::PathBuf;

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::{Harvest, Member, NamedNode, Node, Provenance, Relation, Way, format};
use planetrs_mapclipper_cli::cli::{Command, Drawing, Framing, LabelKind, MarkKind, Outputs};
use planetrs_mapclipper_cli::run::run;

const AT: (f64, f64) = (5.269, -4.012);

fn scratch(named: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("mapclipper-test-{named}"));
    let _ = std::fs::remove_file(&path);
    path
}

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

/// A square of `metres` centred on the frame, as node ids `from..from+4`.
fn square(from: i64, metres: f64) -> Vec<Node> {
    let half = metres / 2.0 / 111_320.0;
    [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .enumerate()
        .map(|(index, (dx, dy))| Node {
            id: from + index as i64,
            latitude: ((AT.0 + dy) * 1e9) as i64,
            longitude: ((AT.1 + dx) * 1e9) as i64,
        })
        .collect()
}

fn a_harvest() -> Harvest {
    let area = planetrs_geo::Area::around(AT.0, AT.1, 40.0, 40.0, 0.1).unwrap();

    Harvest {
        provenance: Provenance {
            source_name: "planet-260907.osm.pbf".into(),
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 0,
            writing_program: "planet-dump-ng 1.2.4".into(),
        },
        collected: area.collected(),
        nodes: [square(1, 3000.0), square(10, 2000.0)].concat(),
        // Off the middle of the frame on purpose: a marker sits there, and a name drawn over it
        // would hide the thing the marker was put there to mark.
        named_nodes: vec![NamedNode {
            id: 500,
            latitude: ((AT.0 + 0.02) * 1e9) as i64,
            longitude: ((AT.1 + 0.02) * 1e9) as i64,
            tags: tags(&[("place", "village"), ("name", "Adjamé")]),
        }],
        ways: vec![
            Way {
                id: 7,
                tags: tags(&[("waterway", "dock"), ("name", "Bassin")]),
                refs: vec![1, 2, 3, 4, 1],
            },
            Way {
                id: 8,
                tags: tags(&[("landuse", "forest")]),
                refs: vec![10, 11, 12, 13, 10],
            },
            Way {
                id: 9,
                tags: tags(&[]),
                refs: vec![1, 2, 3],
            },
            Way {
                id: 10,
                tags: tags(&[]),
                refs: vec![3, 4, 1],
            },
        ],
        relations: vec![Relation {
            id: 99,
            tags: tags(&[("type", "multipolygon"), ("natural", "water"), ("name", "Lagune")]),
            members: vec![
                Member {
                    way: 9,
                    role: "outer".into(),
                },
                Member {
                    way: 10,
                    role: "outer".into(),
                },
            ],
        }],
    }
    .sorted()
}

fn framing(width_km: f64) -> Framing {
    Framing {
        lat: AT.0,
        lon: AT.1,
        width_km,
        height_km: width_km * 0.57,
        margin: 0.1,
    }
}

fn drawing(theme: &str) -> Drawing {
    Drawing {
        width_px: 1000.0,
        height_px: 570.0,
        theme: theme.to_string(),
        marker: MarkKind::LatLon,
        poi_lat: None,
        poi_lon: None,
        label: None,
        names: vec!["en=Port of Abidjan".into()],
        icon: None,
        icon_size: None,
        dot_size: None,
        label_size: None,
        urban_density: None,
        labels: vec![LabelKind::None],
    }
}

/// A label test has no business competing with the marker for the middle of the frame, so these
/// draw no marker at all. That the two do compete is asserted on its own, in the svg module.
fn drawing_with_labels(theme: &str, labels: Vec<LabelKind>) -> Drawing {
    Drawing {
        labels,
        marker: MarkKind::None,
        ..drawing(theme)
    }
}

fn outputs(geojson: Option<PathBuf>, svg: Option<PathBuf>) -> Outputs {
    Outputs {
        geojson,
        svg,
        png: None,
        scale: 1.0,
        quiet: true,
    }
}

fn harvest_at(named: &str) -> PathBuf {
    let path = scratch(named);
    format::write(&path, &a_harvest()).expect("the harvest writes");
    path
}

#[test]
fn when_a_harvest_is_rendered_then_the_geojson_says_what_it_is_of() {
    let harvest = harvest_at("says.mapclip");
    let geojson = scratch("says.geojson");

    run(Command::Render {
        harvest: harvest.clone(),
        framing: framing(29.0),
        drawing: drawing("atlas"),
        outputs: outputs(Some(geojson.clone()), None),
    })
    .expect("the render succeeds");

    let written = std::fs::read_to_string(&geojson).unwrap();

    assert!(written.contains("\"type\":\"FeatureCollection\""));
    assert!(written.contains("Port of Abidjan"));
    assert!(written.contains("planet-260907.osm.pbf"));
}

/// ODbL makes the GeoJSON a derivative database and the SVG a produced work. Both carry the notice,
/// and a build that drops it from either is one nobody may publish the output of: the GeoJSON names
/// the licence and links to it, as data has to, and the picture spells out the copyright page.
#[test]
fn when_anything_is_written_then_it_carries_the_openstreetmap_attribution() {
    let harvest = harvest_at("licence.mapclip");
    let geojson = scratch("licence.geojson");
    let svg = scratch("licence.svg");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing("linework"),
        outputs: outputs(Some(geojson.clone()), Some(svg.clone())),
    })
    .expect("the render succeeds");

    for path in [&geojson, &svg] {
        let written = std::fs::read_to_string(path).unwrap();
        assert!(
            written.contains("OpenStreetMap contributors"),
            "{} carries no attribution",
            path.display()
        );
    }
    let data = std::fs::read_to_string(&geojson).unwrap();
    assert!(
        data.contains("Open Database License")
            && data.contains("https://opendatacommons.org/licenses/odbl/1-0/")
    );
    assert!(
        std::fs::read_to_string(&svg)
            .unwrap()
            .contains("https://www.openstreetmap.org/copyright")
    );
}

/// The reproducible pair. A harvest plus a framing plus a theme has to give the same bytes every
/// time, or a checksum over a published map says nothing about what produced it.
#[test]
fn when_the_same_harvest_is_rendered_twice_then_both_outputs_are_byte_identical() {
    let harvest = harvest_at("twice.mapclip");

    let render_to = |suffix: &str| {
        let geojson = scratch(&format!("twice-{suffix}.geojson"));
        let svg = scratch(&format!("twice-{suffix}.svg"));
        run(Command::Render {
            harvest: harvest.clone(),
            framing: framing(29.0),
            drawing: drawing("atlas"),
            outputs: outputs(Some(geojson.clone()), Some(svg.clone())),
        })
        .expect("the render succeeds");
        (std::fs::read(&geojson).unwrap(), std::fs::read(&svg).unwrap())
    };

    assert_eq!(render_to("a"), render_to("b"));
}

/// The failure that cost seven full rescans in the spike: a frame that still fits, asked for with a
/// margin the harvest never collected.
#[test]
fn when_a_frame_reaches_past_the_harvest_then_it_is_refused_with_both_extents_named() {
    let harvest = harvest_at("small.mapclip");

    let refused = run(Command::Render {
        harvest,
        framing: framing(200.0),
        drawing: drawing("atlas"),
        outputs: outputs(None, Some(scratch("small.svg"))),
    })
    .unwrap_err();

    assert!(matches!(refused, ClipError::HarvestTooSmall { .. }));
    assert!(refused.next_step().is_some(), "a refusal has to say what to try");
}

#[test]
fn when_a_theme_is_not_known_then_the_render_is_refused_rather_than_drawn_plain() {
    let harvest = harvest_at("theme.mapclip");

    let refused = run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing("chartreuse"),
        outputs: outputs(None, Some(scratch("theme.svg"))),
    })
    .unwrap_err();

    assert!(matches!(refused, ClipError::UnknownTheme { .. }));
}

#[test]
fn when_a_file_is_not_a_harvest_then_the_render_says_so_rather_than_misreading_it() {
    let not_a_harvest = scratch("foreign.mapclip");
    std::fs::write(&not_a_harvest, b"a map, but described in words").unwrap();

    let refused = run(Command::Render {
        harvest: not_a_harvest,
        framing: framing(29.0),
        drawing: drawing("atlas"),
        outputs: outputs(None, Some(scratch("foreign.svg"))),
    })
    .unwrap_err();

    assert!(matches!(refused, ClipError::NotAHarvest));
}

/// A theme decides only what things look like, so the same harvest under two themes has to draw the
/// same ground and differ in ink alone.
#[test]
fn when_the_theme_changes_then_the_geojson_does_not() {
    let harvest = harvest_at("themed.mapclip");

    let geojson_under = |theme: &str| {
        let path = scratch(&format!("themed-{theme}.geojson"));
        run(Command::Render {
            harvest: harvest.clone(),
            framing: framing(29.0),
            drawing: drawing(theme),
            outputs: outputs(Some(path.clone()), None),
        })
        .expect("the render succeeds");
        std::fs::read(&path).unwrap()
    };

    assert_eq!(geojson_under("atlas"), geojson_under("linework"));
}

#[test]
fn when_a_relation_closes_then_the_lagoon_is_in_the_output() {
    let harvest = harvest_at("lagoon.mapclip");
    let geojson = scratch("lagoon.geojson");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing("atlas"),
        outputs: outputs(Some(geojson.clone()), None),
    })
    .expect("the render succeeds");

    let written = std::fs::read_to_string(&geojson).unwrap();

    assert!(written.contains("\"name\":\"Lagune\""));
    assert!(written.contains("\"osm\":99"));
}

#[test]
fn when_a_harvest_is_inspected_then_it_reports_the_planet_it_came_from() {
    let harvest = harvest_at("inspect.mapclip");

    run(Command::Inspect {
        file: harvest,
        output: planetrs_mapclipper_cli::cli::Shape::Json,
    })
    .expect("the inspection succeeds");
}

/// Names belong in the data whatever the picture was asked to carry, because a reader drawing its
/// own map places them its own way.
#[test]
fn when_no_labels_are_asked_for_then_the_names_are_still_in_the_geojson() {
    let harvest = harvest_at("names.mapclip");
    let geojson = scratch("names.geojson");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing("atlas"),
        outputs: outputs(Some(geojson.clone()), None),
    })
    .expect("the render succeeds");

    let written = std::fs::read_to_string(&geojson).unwrap();

    assert!(written.contains("Adjamé"), "the settlement is in the data");
    assert!(written.contains("Bassin"));
}

#[test]
fn when_no_labels_are_asked_for_then_the_picture_carries_none() {
    let harvest = harvest_at("bare.mapclip");
    let svg = scratch("bare.svg");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing("atlas"),
        outputs: outputs(None, Some(svg.clone())),
    })
    .expect("the render succeeds");

    assert!(!std::fs::read_to_string(&svg).unwrap().contains("Adjamé"));
}

#[test]
fn when_settlements_are_asked_for_then_the_village_is_named_on_the_picture() {
    let harvest = harvest_at("village.mapclip");
    let svg = scratch("village.svg");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing_with_labels("atlas", vec![LabelKind::Settlements]),
        outputs: outputs(None, Some(svg.clone())),
    })
    .expect("the render succeeds");

    assert!(std::fs::read_to_string(&svg).unwrap().contains(">Adjamé</text>"));
}

#[test]
fn when_only_settlements_are_asked_for_then_the_water_stays_unnamed() {
    let harvest = harvest_at("only-village.mapclip");
    let svg = scratch("only-village.svg");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing_with_labels("atlas", vec![LabelKind::Settlements]),
        outputs: outputs(None, Some(svg.clone())),
    })
    .expect("the render succeeds");

    let written = std::fs::read_to_string(&svg).unwrap();

    assert!(written.contains(">Adjamé</text>"));
    assert!(!written.contains(">Bassin</text>"));
}

#[test]
fn when_everything_is_asked_for_then_the_water_is_named_too() {
    let harvest = harvest_at("all-names.mapclip");
    let svg = scratch("all-names.svg");

    run(Command::Render {
        harvest,
        framing: framing(29.0),
        drawing: drawing_with_labels("atlas", vec![LabelKind::All]),
        outputs: outputs(None, Some(svg.clone())),
    })
    .expect("the render succeeds");

    let written = std::fs::read_to_string(&svg).unwrap();

    assert!(written.contains(">Adjamé</text>"));
    assert!(written.contains(">Bassin</text>"));
}

/// Labels are placed greedily in rank order, so the same scene has to place them the same way twice.
#[test]
fn when_a_labelled_map_is_drawn_twice_then_both_pictures_are_byte_identical() {
    let harvest = harvest_at("stable-names.mapclip");

    let draw = |suffix: &str| {
        let svg = scratch(&format!("stable-names-{suffix}.svg"));
        run(Command::Render {
            harvest: harvest.clone(),
            framing: framing(29.0),
            drawing: drawing_with_labels("atlas", vec![LabelKind::All]),
            outputs: outputs(None, Some(svg.clone())),
        })
        .expect("the render succeeds");
        std::fs::read(&svg).unwrap()
    };

    assert_eq!(draw("a"), draw("b"));
}
