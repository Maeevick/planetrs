//! Drawing a harvest, at any framing it covers, in any theme.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::passes::HarvestReport;
use planetrs_mapclipper::harvest::{Harvest, format};
use planetrs_mapclipper::output::{Mark, Marker, geojson, svg};
use planetrs_mapclipper::scene::{self, Scene};
use planetrs_mapclipper::theme::Theme;

use super::extent_of;
use super::harvest::describe_harvest;
use crate::cli::{Drawing, Framing, MarkKind, Outputs, RenderArgs};
use crate::report::{Report, bytes, seconds, thousands};

/// Draws the harvest the command line names into the outputs it asks for.
pub fn run(arguments: RenderArgs) -> Result<(), ClipError> {
    let held = format::read(&arguments.harvest)?;
    draw(
        &held,
        &arguments.framing,
        &arguments.drawing,
        &arguments.outputs,
        None,
    )
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

pub(super) fn draw(
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
