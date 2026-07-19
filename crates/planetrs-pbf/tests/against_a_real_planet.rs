//! Integrated tests against a real OSM PBF file.
//!
//! Every unit test in this crate runs against fixtures built by hand, which proves the decoding
//! matches the spec as written. It does not prove the spec matches what `planet-dump-ng` actually
//! writes, and the four bugs this reader was built around all produced plausible output rather than
//! errors. These tests close that gap.
//!
//! They read a 94 GB file, so they are `#[ignore]` and run only on request:
//!
//! ```sh
//! make test-system
//! PLANET=/path/to/planet.osm.pbf cargo test --release -- --ignored
//! ```

use std::path::PathBuf;

use planetrs_pbf::{Planet, Section, block, block_contents, id_span};

/// The planet file to read, from `PLANET`.
///
/// Panics when it is unset or absent rather than passing quietly: a system test that reports
/// success without touching a planet file is worse than no test at all.
fn planet_path() -> PathBuf {
    let path = std::env::var_os("PLANET")
        .map(PathBuf::from)
        .expect("PLANET is unset; point it at a planet file, as `make test-system` does");

    assert!(
        path.exists(),
        "no planet file at {}; set PLANET to the absolute path of one, as `make test-system` does",
        path.display()
    );
    path
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_planet_is_opened_then_it_declares_the_sort_order_the_bisection_depends_on() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");

    assert!(
        planet.header.is_sorted_by_type_then_id(),
        "without Sort.Type_then_ID neither the section bisection nor the id index is sound"
    );
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_planet_is_opened_then_this_reader_can_honour_everything_it_requires() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");

    assert_eq!(planet.header.unsupported_requirements(), Vec::<&str>::new());
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_planet_is_walked_then_every_byte_is_accounted_for_by_a_blob() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");

    let last = planet.blobs.last().expect("at least one data blob");

    assert_eq!(last.offset + last.data_size as u64, planet.file_size);
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_planet_is_bisected_then_the_three_sections_run_in_order_and_cover_every_blob() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");

    let sections = planet.sections().expect("the sections bisect");

    assert_eq!(sections.nodes.start, 0);
    assert_eq!(sections.nodes.end, sections.ways.start);
    assert_eq!(sections.ways.end, sections.relations.start);
    assert_eq!(sections.relations.end, planet.blobs.len());
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_planets_boundary_blobs_are_read_then_each_holds_the_section_it_was_placed_in() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");
    let sections = planet.sections().expect("the sections bisect");

    assert_eq!(planet.section_of(sections.nodes.end - 1).unwrap(), Section::Nodes);
    assert_eq!(planet.section_of(sections.ways.start).unwrap(), Section::Ways);
    assert_eq!(planet.section_of(sections.ways.end - 1).unwrap(), Section::Ways);
    assert_eq!(
        planet.section_of(sections.relations.start).unwrap(),
        Section::Relations
    );
}

/// The property the id index rests on, checked against the file rather than against the spec: a
/// blob's first id must exceed the previous blob's last, or pruning by id would skip real entities.
#[test]
#[ignore = "reads a planet file"]
fn when_real_blobs_are_sampled_in_order_then_their_id_spans_ascend_without_overlapping() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");
    let sections = planet.sections().expect("the sections bisect");

    let mut payload = Vec::new();
    let mut inflated = Vec::new();

    for range in [sections.nodes, sections.ways, sections.relations] {
        let step = (range.len() / 12).max(1);
        let mut previous: Option<(usize, i64)> = None;

        for at in range.clone().step_by(step) {
            planet
                .inflate_blob(at, &mut payload, &mut inflated)
                .expect("the blob inflates");
            let decoded = block(&inflated).expect("the block decodes");
            let span = id_span(&decoded)
                .expect("the span reads")
                .expect("the blob holds entities");

            assert!(span.first <= span.last, "blob {at} spans backwards");
            if let Some((before, last)) = previous {
                assert!(
                    span.first > last,
                    "blob {at} starts at {} but blob {before} ended at {last}",
                    span.first
                );
            }
            previous = Some((at, span.last));
        }
    }
}

#[test]
#[ignore = "reads a planet file"]
fn when_a_real_blob_is_read_then_its_span_agrees_with_the_section_its_contents_report() {
    let path = planet_path();
    let planet = Planet::open(&path).expect("the planet file opens");
    let sections = planet.sections().expect("the sections bisect");

    let mut payload = Vec::new();
    let mut inflated = Vec::new();
    planet
        .inflate_blob(sections.ways.start + 1, &mut payload, &mut inflated)
        .unwrap();
    let decoded = block(&inflated).unwrap();

    assert_eq!(
        id_span(&decoded).unwrap().map(|span| span.section),
        Some(block_contents(&decoded).unwrap().section())
    );
}
