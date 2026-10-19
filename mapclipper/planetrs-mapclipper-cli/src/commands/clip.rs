//! Harvesting and drawing in one go, for the first look at a place.

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::harvest::format;

use super::harvest::read_planet;
use super::render::draw;
use crate::cli::ClipArgs;
use crate::report::{Report, bytes};

/// Harvests the area the command line frames, keeps the harvest if asked, and draws it.
pub fn run(arguments: ClipArgs) -> Result<(), ClipError> {
    let ClipArgs {
        planet,
        framing,
        drawing,
        outputs,
        keep_harvest,
        index,
    } = arguments;
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
