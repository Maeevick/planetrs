//! Reading the planet for one area, and keeping what was read as a harvest.

use std::path::Path;

use planetrs_cli::Progress;
use planetrs_geo::Area;
use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::passes::{HarvestReport, harvest_area};
use planetrs_mapclipper::harvest::{Harvest, format};
use planetrs_pbf::Planet;

use super::{Context, extent_of, put};
use crate::cli::HarvestArgs;
use crate::report::{Report, bytes, seconds, thousands};

/// Harvests the area the command line frames, and says how to draw it.
pub fn run(context: &Context, arguments: HarvestArgs) -> Result<(), ClipError> {
    let HarvestArgs { planet, framing, out } = arguments;
    let area = framing.area()?;
    let (harvest, pass_report) = read_planet(context, &planet, &area)?;
    let (written, size) = put(&out, &format::to_bytes(&harvest))?;

    let mut report = Report::new(context.quiet);
    describe_harvest(&mut report, &harvest, &pass_report);
    report.line("written", format!("{} ({})", written.display(), bytes(size)));
    report.blank();
    report.note(format!(
        "→ draw it:  planetrs-mapclipper render {} --lat {} --lon {} --width-km {} --height-km {} --svg map.svg",
        written.display(), framing.lat, framing.lon, framing.width_km, framing.height_km
    ));
    report.to_stderr();
    Ok(())
}

fn source_name(planet: &Path) -> String {
    planet
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(super) fn read_planet(
    context: &Context,
    planet: &Path,
    area: &Area,
) -> Result<(Harvest, HarvestReport), ClipError> {
    let opened = Planet::open(planet)?;
    let progress = Progress::to_stderr(context.quiet);
    let harvested = harvest_area(&opened, &source_name(planet), area, &|line| progress.note(line));
    progress.clear();
    harvested
}

pub(super) fn describe_harvest(report: &mut Report, harvest: &Harvest, passes: &HarvestReport) {
    let provenance = &harvest.provenance;
    report.line(
        "source",
        format!(
            "{} (replication {})",
            provenance.source_name, provenance.replication_timestamp
        ),
    );
    report.line("collected", extent_of(&harvest.collected));

    for (label, pass) in [
        ("nodes", passes.nodes),
        ("ways", passes.ways),
        ("relations", passes.relations),
        ("member ways", passes.member_ways),
        ("member nodes", passes.member_nodes),
    ] {
        report.line(
            label,
            format!(
                "{} over {} blobs, {} inflated, {} kept",
                seconds(pass.elapsed),
                thousands(pass.blobs),
                bytes(pass.inflated_bytes),
                thousands(pass.kept)
            ),
        );
    }

    if passes.blobs_pruned > 0 {
        report.line(
            "id index",
            format!(
                "{} blobs skipped in the passes that fetch by id",
                thousands(passes.blobs_pruned)
            ),
        );
    }
    report.line("harvest", format!("{} in the passes", seconds(passes.elapsed())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_planet_is_named_then_its_file_name_is_what_the_harvest_records() {
        assert_eq!(
            source_name(Path::new("/data/planet-260907.osm.pbf")),
            "planet-260907.osm.pbf"
        );
    }
}
