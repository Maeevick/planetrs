//! The imperative shell: read a file, call the pure half, write what came back.
//!
//! Everything that decides anything lives in [`planetrs_mapclipper::scene`] and is a function of
//! its arguments.
//! What is left here is opening files, handing work to the passes, and saying what happened.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use planetrs_cli::Progress;
use planetrs_geo::Area;
use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::passes::{HarvestReport, harvest_area};
use planetrs_mapclipper::harvest::{Harvest, format};
use planetrs_mapclipper::output::{Mark, Marker, geojson, svg};
use planetrs_mapclipper::scene::{self, Scene};
use planetrs_mapclipper::theme::{BUILT_IN, Theme};
use planetrs_pbf::{IdIndex, Planet};

use crate::cli::{Command, Drawing, Framing, MarkKind, Outputs, Shape};
use crate::report::{Report, bytes, quoted, seconds, thousands};

/// Where a sidecar index goes when the caller did not say: beside the planet file.
fn index_beside(planet: &Path) -> PathBuf {
    let mut named = planet.as_os_str().to_os_string();
    named.push(".mapclip-index");
    PathBuf::from(named)
}

fn source_name(planet: &Path) -> String {
    planet
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Writes to a path, or to stdout when the path is `-`.
fn put(path: &Path, contents: &str) -> Result<u64, ClipError> {
    if path.as_os_str() == "-" {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(contents.as_bytes())
            .map_err(|error| ClipError::writing(path, error))?;
        let _ = stdout.flush();
        return Ok(contents.len() as u64);
    }

    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| ClipError::writing(path, error))?;
    }
    std::fs::write(path, contents).map_err(|error| ClipError::writing(path, error))?;
    Ok(contents.len() as u64)
}

pub fn run(command: Command) -> Result<(), ClipError> {
    match command {
        Command::Harvest {
            planet,
            framing,
            out,
            index,
        } => {
            let area = framing.area()?;
            let (harvest, pass_report) = read_planet(&planet, &area, index.as_deref())?;
            let size = format::write(&out, &harvest)?;

            let mut report = Report::new(false);
            describe_harvest(&mut report, &harvest, &pass_report);
            report.line("written", format!("{} ({})", out.display(), bytes(size)));
            report.blank();
            report.note(format!(
                "→ draw it:  planetrs-mapclipper render {} --lat {} --lon {} --width-km {} --height-km {} --svg map.svg",
                out.display(), framing.lat, framing.lon, framing.width_km, framing.height_km
            ));
            report.to_stderr();
            Ok(())
        }

        Command::Render {
            harvest,
            framing,
            drawing,
            outputs,
        } => {
            let held = format::read(&harvest)?;
            draw(&held, &framing, &drawing, &outputs, None)
        }

        Command::Clip {
            planet,
            framing,
            drawing,
            outputs,
            keep_harvest,
            index,
        } => {
            let area = framing.area()?;
            let (held, pass_report) = read_planet(&planet, &area, index.as_deref())?;

            if let Some(path) = &keep_harvest {
                let size = format::write(path, &held)?;
                let mut kept = Report::new(outputs.quiet);
                kept.line("harvest", format!("{} ({})", path.display(), bytes(size)));
                kept.to_stderr();
            }

            draw(&held, &framing, &drawing, &outputs, Some(pass_report))
        }

        Command::Inspect { file, output } => inspect(&file, output),

        Command::Themes { output } => {
            let listed = match output {
                Shape::Json => themes_as_json()?,
                Shape::Text => themes_as_text()?,
            };
            print!("{listed}");
            Ok(())
        }
    }
}

fn read_planet(
    planet: &Path,
    area: &Area,
    index_at: Option<&Path>,
) -> Result<(Harvest, HarvestReport), ClipError> {
    let opened = Planet::open(planet)?;
    let index_at = index_at
        .map(Path::to_path_buf)
        .unwrap_or_else(|| index_beside(planet));
    let mut index = IdIndex::read_for(&index_at, opened.fingerprint())
        .unwrap_or_else(|| IdIndex::empty(opened.fingerprint(), opened.blobs.len()));

    let progress = Progress::to_stderr(false);
    let harvested = harvest_area(&opened, &source_name(planet), area, &mut index, &|line| {
        progress.note(line)
    });
    progress.clear();
    let (harvest, report) = harvested?;

    if let Err(error) = index.write(&index_at) {
        eprintln!(
            "the id index could not be kept at {}: {error}",
            index_at.display()
        );
    }

    Ok((harvest, report))
}

fn draw(
    held: &Harvest,
    framing: &Framing,
    drawing: &Drawing,
    outputs: &Outputs,
    passes: Option<HarvestReport>,
) -> Result<(), ClipError> {
    let area = framing.area()?;
    if !held.answers_for(&area.collected()) {
        return Err(ClipError::HarvestTooSmall {
            collected: extent_of(&held.collected),
            wanted: extent_of(&area.collected()),
        });
    }

    let canvas = drawing.canvas()?;
    let theme = drawing.theme()?;
    let place = drawing.place(framing);

    let started = Instant::now();
    let scene = scene::build(held, &area, canvas, place, &drawing.rules());
    let drawn_in = started.elapsed();

    let marker = marker_for(drawing, framing)?;
    let mut report = Report::new(outputs.quiet);

    if let Some(passes) = passes {
        describe_harvest(&mut report, held, &passes);
    }
    describe_scene(&mut report, &scene, &theme, drawn_in);

    if !outputs.any() {
        report.blank();
        report.note("nothing was written: give --geojson, --svg or --png");
        report.to_stderr();
        return Ok(());
    }

    if let Some(path) = &outputs.geojson {
        let size = put(path, &geojson::write(&scene))?;
        report.line("geojson", format!("{} ({})", path.display(), bytes(size)));
    }

    let labels = drawing.label_classes();
    let drawn = (outputs.svg.is_some() || outputs.png.is_some())
        .then(|| svg::write(&scene, &theme, &marker, &labels));

    if let (Some(path), Some(drawn)) = (&outputs.svg, drawn.as_ref()) {
        let size = put(path, drawn)?;
        report.line("svg", format!("{} ({})", path.display(), bytes(size)));
    }

    if let Some(path) = &outputs.png {
        let drawn = drawn.as_ref().expect("the svg is drawn whenever a png is wanted");
        let size = rasterise(path, drawn, outputs.scale)?;
        report.line("png", format!("{} ({})", path.display(), bytes(size)));
    }

    report.to_stderr();
    Ok(())
}

#[cfg(feature = "png")]
fn rasterise(path: &Path, drawn: &str, scale: f64) -> Result<u64, ClipError> {
    planetrs_mapclipper::output::png::write(path, drawn, scale as f32)
}

#[cfg(not(feature = "png"))]
fn rasterise(_path: &Path, _drawn: &str, _scale: f64) -> Result<u64, ClipError> {
    Err(ClipError::Raster {
        detail: "this build was made without the png feature; write --svg and rasterise it".into(),
    })
}

fn marker_for(drawing: &Drawing, framing: &Framing) -> Result<Marker, ClipError> {
    let place = drawing.place(framing);
    let mark = match drawing.marker {
        MarkKind::None => Mark::None,
        MarkKind::Dot => Mark::Dot,
        MarkKind::LatLon => Mark::Coordinates,
        MarkKind::Name => Mark::Named(
            drawing
                .label
                .clone()
                .or_else(|| place.any_name().map(str::to_string))
                .unwrap_or_else(|| svg::coordinates_label(place.at.0, place.at.1)),
        ),
    };

    Ok(Marker {
        mark,
        at: place.at,
        icon: drawing.icon.as_deref().map(svg::icon_at).transpose()?,
        icon_size: drawing.icon_size,
        dot_size: drawing.dot_size,
        label_size: drawing.label_size,
    })
}

fn extent_of(box_: &planetrs_geo::BoundingBox) -> String {
    format!("{:.1} x {:.1} km", box_.width_km(), box_.height_km())
}

fn describe_harvest(report: &mut Report, harvest: &Harvest, passes: &HarvestReport) {
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
                "{} blobs skipped in the passes that fetch by id{}",
                thousands(passes.blobs_pruned),
                if passes.index_was_reused {
                    ", reused from disk"
                } else {
                    ""
                }
            ),
        );
    }
    report.line("harvest", format!("{} in the passes", seconds(passes.elapsed())));
}

fn describe_scene(report: &mut Report, scene: &Scene, theme: &Theme, drawn_in: std::time::Duration) {
    let stretch = scene.canvas.stretch(&scene.frame);
    report.line(
        "frame",
        format!(
            "{} at {:.5}, {:.5} on {:.0} x {:.0} px",
            extent_of(&scene.frame),
            scene.frame.middle_latitude(),
            scene.frame.middle_longitude(),
            scene.canvas.width,
            scene.canvas.height
        ),
    );

    if stretch > 1.02 {
        report.line(
            "warning",
            format!(
                "the frame and the canvas disagree in shape by {stretch:.2}x, so the map is stretched; \
                 try --height-km {:.2}",
                scene.frame.width_km() * scene.canvas.height / scene.canvas.width
            ),
        );
    }

    report.line("theme", format!("{} — {}", theme.name, theme.description));
    report.line(
        "features",
        format!(
            "{} kept, {} dropped as off-frame or under a pixel, drawn in {}",
            thousands(scene.report.kept),
            thousands(scene.report.dropped),
            seconds(drawn_in)
        ),
    );

    if scene.report.vertices_before > 0 {
        report.line(
            "vertices",
            format!(
                "{} simplified to {} ({:.0}% removed)",
                thousands(scene.report.vertices_before),
                thousands(scene.report.vertices_after),
                (1.0 - scene.report.vertices_after as f64 / scene.report.vertices_before.max(1) as f64)
                    * 100.0
            ),
        );
    }

    if scene.report.rings_closed > 0 || !scene.report.relations_left_open.is_empty() {
        report.line(
            "multipolygons",
            format!(
                "{} rings closed, {} relations left open",
                thousands(scene.report.rings_closed),
                scene.report.relations_left_open.len()
            ),
        );
    }

    if let Some([p50, p75, p90, p99]) = scene.report.built_up {
        report.line(
            "built-up share",
            format!("p50 {p50:.3}  p75 {p75:.3}  p90 {p90:.3}  p99 {p99:.3}"),
        );
    }

    for (category, count) in &scene.report.by_category {
        report.line(&format!("  {}", category.name()), thousands(*count));
    }
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

fn themes_as_text() -> Result<String, ClipError> {
    let mut report = Report::new(false);
    for (name, text) in BUILT_IN {
        let theme = planetrs_mapclipper::theme::parse(text, Path::new(name))?;
        report.line(name, &theme.description);
    }
    report.blank();
    report.note("any of these, or a path to a theme file of your own, goes to --theme");
    Ok(report.text().to_string())
}

fn themes_as_json() -> Result<String, ClipError> {
    let described: Vec<String> = BUILT_IN
        .iter()
        .map(|(name, text)| {
            let theme = planetrs_mapclipper::theme::parse(text, Path::new(name))?;
            let drawn: Vec<String> = theme
                .order
                .iter()
                .map(|category| quoted(category.name()))
                .collect();
            Ok(format!(
                "{{\"name\":{},\"description\":{},\"background\":{},\"draws\":[{}]}}",
                quoted(name),
                quoted(&theme.description),
                quoted(&theme.background),
                drawn.join(",")
            ))
        })
        .collect::<Result<_, ClipError>>()?;

    Ok(format!("[{}]\n", described.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_no_index_path_is_given_then_it_is_kept_beside_the_planet_file() {
        let beside = index_beside(Path::new("/data/planet-260907.osm.pbf"));

        assert_eq!(beside, Path::new("/data/planet-260907.osm.pbf.mapclip-index"));
    }

    #[test]
    fn when_a_planet_is_named_then_its_file_name_is_what_the_harvest_records() {
        assert_eq!(
            source_name(Path::new("/data/planet-260907.osm.pbf")),
            "planet-260907.osm.pbf"
        );
    }

    #[test]
    fn when_the_themes_are_listed_as_json_then_every_built_in_is_there() {
        let listed = themes_as_json().unwrap();

        assert!(
            BUILT_IN
                .iter()
                .all(|(name, _)| listed.contains(&format!("\"name\":\"{name}\"")))
        );
    }

    #[test]
    fn when_the_themes_are_listed_as_text_then_each_carries_its_description() {
        let listed = themes_as_text().unwrap();

        assert!(listed.contains("linework"));
        assert!(listed.contains("road network"));
    }
}
