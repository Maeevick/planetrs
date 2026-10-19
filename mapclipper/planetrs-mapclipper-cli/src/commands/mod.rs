//! What each command does, given a parsed command line.

pub mod clip;
pub mod harvest;
pub mod inspect;
pub mod render;
pub mod themes;

use planetrs_mapclipper::error::ClipError;

use crate::cli::Command;

/// Runs the command the command line asks for.
pub fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Harvest(arguments) => harvest::run(arguments),
        Command::Render(arguments) => render::run(arguments),
        Command::Clip(arguments) => clip::run(arguments),
        Command::Inspect(arguments) => inspect::run(arguments),
        Command::Themes(arguments) => themes::run(arguments),
    }
    .map_err(explained)
}

/// A failure, said with what to try next when there is something worth trying.
fn explained(failure: ClipError) -> anyhow::Error {
    match failure.next_step() {
        Some(step) => anyhow::anyhow!("{failure}\n  → {step}"),
        None => failure.into(),
    }
}

pub(super) fn extent_of(box_: &planetrs_geo::BoundingBox) -> String {
    format!("{:.1} x {:.1} km", box_.width_km(), box_.height_km())
}
