//! Harvesting and drawing in one go, for the first look at a place.

use crate::error::ClipError;
use crate::harvest::format;

use super::harvest::harvest_of;
use super::render::draw;
use super::{Context, put};
use crate::cli::arguments::ClipArgs;
use crate::cli::report::{Report, bytes};

/// Harvests the area the command line frames, keeps the harvest if asked, and draws it.
pub fn run(context: &Context, arguments: ClipArgs) -> Result<(), ClipError> {
    let ClipArgs {
        planet,
        framing,
        drawing,
        outputs,
        keep_harvest,
    } = arguments;
    let area = framing.area()?;
    let (held, passes) = harvest_of(context, planet.as_deref(), &area)?;

    if let Some(path) = &keep_harvest {
        let (written, size) = put(path, &format::to_bytes(&held))?;
        let mut kept = Report::new(context.quiet);
        kept.line("harvest", format!("{} ({})", written.display(), bytes(size)));
        kept.to_stderr();
    }

    draw(context, &held, &framing, &drawing, &outputs, passes)
}
