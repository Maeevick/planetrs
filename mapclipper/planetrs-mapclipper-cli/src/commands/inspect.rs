//! Reporting what a planet file or a harvest holds.

use std::path::Path;

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::format;
use planetrs_pbf::Planet;

use super::extent_of;
use crate::cli::{InspectArgs, Shape};
use crate::report::{Report, bytes, quoted, thousands};

/// Reports on the file the command line names, as text or JSON.
pub fn run(arguments: InspectArgs) -> Result<(), ClipError> {
    inspect(&arguments.file, arguments.output)
}

fn inspect(file: &Path, output: Shape) -> Result<(), ClipError> {
    let mut head = [0u8; 8];
    let looks_like_a_harvest = {
        use std::io::Read;
        let mut opened = std::fs::File::open(file).map_err(|error| ClipError::reading(file, error))?;
        opened.read_exact(&mut head).is_ok() && &head == b"MAPCLIP\0"
    };

    let described = if looks_like_a_harvest {
        describe_harvest_file(file, output)?
    } else {
        describe_planet_file(file, output)?
    };

    print!("{described}");
    Ok(())
}

fn describe_harvest_file(file: &Path, output: Shape) -> Result<String, ClipError> {
    let held = format::read(file)?;
    let provenance = &held.provenance;

    if output == Shape::Json {
        return Ok(format!(
            "{{\"kind\":\"harvest\",\"source\":{},\"replication_timestamp\":{},\
             \"replication_sequence\":{},\"collected\":[{:.6},{:.6},{:.6},{:.6}],\
             \"nodes\":{},\"ways\":{},\"relations\":{}}}\n",
            quoted(&provenance.source_name),
            provenance.replication_timestamp,
            provenance.replication_sequence,
            held.collected.west,
            held.collected.south,
            held.collected.east,
            held.collected.north,
            held.nodes.len(),
            held.ways.len(),
            held.relations.len()
        ));
    }

    let mut report = Report::new(false);
    report.line("kind", "harvest");
    report.line("source", &provenance.source_name);
    report.line(
        "replication",
        format!(
            "timestamp {} sequence {}",
            provenance.replication_timestamp, provenance.replication_sequence
        ),
    );
    report.line("written by", &provenance.writing_program);
    report.line("collected", extent_of(&held.collected));
    report.line("nodes", thousands(held.nodes.len() as u64));
    report.line("ways", thousands(held.ways.len() as u64));
    report.line("relations", thousands(held.relations.len() as u64));
    Ok(report.text().to_string())
}

fn describe_planet_file(file: &Path, output: Shape) -> Result<String, ClipError> {
    let planet = Planet::open(file)?;
    let header = &planet.header;
    let sections = planet.sections()?;

    if output == Shape::Json {
        return Ok(format!(
            "{{\"kind\":\"planet\",\"file_size\":{},\"blobs\":{},\
             \"replication_timestamp\":{},\"sorted_by_type_then_id\":{},\
             \"sections\":{{\"nodes\":[{},{}],\"ways\":[{},{}],\"relations\":[{},{}]}}}}\n",
            planet.file_size,
            planet.blobs.len(),
            header.replication_timestamp.unwrap_or_default(),
            header.is_sorted_by_type_then_id(),
            sections.nodes.start,
            sections.nodes.end,
            sections.ways.start,
            sections.ways.end,
            sections.relations.start,
            sections.relations.end,
        ));
    }

    let total = planet.bytes_in(0..planet.blobs.len()) as f64;
    let share = |range: std::ops::Range<usize>| {
        let count = planet.bytes_in(range.clone());
        format!(
            "blobs {}..{}  {}  {:.1}%",
            range.start,
            range.end,
            bytes(count),
            count as f64 / total * 100.0
        )
    };

    let mut report = Report::new(false);
    report.line("kind", "planet");
    report.line(
        "file",
        format!("{} ({})", file.display(), bytes(planet.file_size)),
    );
    report.line("blobs", thousands(planet.blobs.len() as u64));
    report.line(
        "written by",
        header.writing_program.as_deref().unwrap_or("unstated"),
    );
    report.line(
        "replication",
        format!(
            "timestamp {} sequence {}",
            header.replication_timestamp.unwrap_or_default(),
            header.replication_sequence.unwrap_or_default()
        ),
    );
    report.line(
        "sorted",
        if header.is_sorted_by_type_then_id() {
            "by type then id"
        } else {
            "not declared"
        },
    );
    report.line("nodes", share(sections.nodes));
    report.line("ways", share(sections.ways));
    report.line("relations", share(sections.relations));
    Ok(report.text().to_string())
}
