//! What each command does, given a parsed command line.

pub mod clip;
pub mod harvest;
pub mod inspect;
pub mod render;
pub mod themes;

use planetrs_cli::ATTRIBUTION;
use planetrs_mapclipper::error::ClipError;
use planetrs_pbf::PbfError;

use crate::cli::{Cli, Command, Global};

/// What every command knows before it starts: how much to say.
pub struct Context {
    /// Whether to say anything on stderr beyond errors and the attribution.
    pub quiet: bool,
}

impl Context {
    /// Reads the settings every command shares.
    pub fn of(global: &Global) -> Self {
        Self { quiet: global.quiet }
    }
}

/// Runs the command the command line asks for, ending with the OpenStreetMap attribution when it
/// read OpenStreetMap data.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    let context = Context::of(&cli.global);
    let reads_openstreetmap_data = !matches!(cli.command, Command::Themes(_));
    match cli.command {
        Command::Harvest(arguments) => harvest::run(&context, arguments),
        Command::Render(arguments) => render::run(&context, arguments),
        Command::Clip(arguments) => clip::run(&context, arguments),
        Command::Inspect(arguments) => inspect::run(arguments),
        Command::Themes(arguments) => themes::run(arguments),
    }
    .map_err(explained)?;
    if reads_openstreetmap_data {
        eprintln!("{ATTRIBUTION}");
    }
    Ok(())
}

/// A failure, said with what to try next when there is something worth trying.
fn explained(failure: ClipError) -> anyhow::Error {
    match next_step(&failure) {
        Some(step) => anyhow::anyhow!("{failure}\n  → {step}"),
        None => failure.into(),
    }
}

/// What a reader should try next after a failure, when there is something worth trying.
fn next_step(failure: &ClipError) -> Option<String> {
    match failure {
        ClipError::NotAHarvest | ClipError::HarvestVersion { .. } => Some(
            "cut a fresh one with: planetrs-mapclipper harvest <planet.osm.pbf> --lat … --lon … \
             --width-km … --height-km … --out <file.mapclip>"
                .into(),
        ),
        ClipError::HarvestTooSmall { .. } => {
            Some("harvest a wider area, or lower --margin, then render again".into())
        }
        ClipError::UnknownTheme { known, .. } => Some(format!(
            "built-in themes are {}, or pass a path to a .toml",
            known.join(", ")
        )),
        ClipError::Planet(PbfError::UnsortedSections) => Some(
            "this reader needs a planet file written with Sort.Type_then_ID, as \
             planet.openstreetmap.org publishes"
                .into(),
        ),
        _ => None,
    }
}

pub(super) fn extent_of(box_: &planetrs_geo::BoundingBox) -> String {
    format!("{:.1} x {:.1} km", box_.width_km(), box_.height_km())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An error without a next step is an obstacle rather than information, and the two failures a
    /// reader meets most are both recoverable in one command.
    #[test]
    fn when_a_harvest_is_from_an_older_build_then_the_error_says_how_to_replace_it() {
        let error = ClipError::HarvestVersion {
            found: 0,
            expected: 1,
        };

        assert!(next_step(&error).unwrap().contains("planetrs-mapclipper harvest"));
    }

    #[test]
    fn when_a_theme_is_not_known_then_the_error_lists_the_ones_that_are() {
        let error = ClipError::UnknownTheme {
            named: "chartreuse".into(),
            known: vec!["plain".into(), "clean".into()],
        };

        assert!(next_step(&error).unwrap().contains("plain, clean"));
    }

    #[test]
    fn when_a_planet_file_is_not_sorted_then_the_error_says_which_files_are() {
        let error = ClipError::Planet(PbfError::UnsortedSections);

        assert!(next_step(&error).unwrap().contains("Sort.Type_then_ID"));
    }
}
