//! What each command does, given a parsed command line.

pub mod clip;
pub mod download;
pub mod harvest;
pub mod index;
pub mod inspect;
pub mod render;
pub mod themes;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use planetrs_cli::dataset::{location_of, name_of};
use planetrs_cli::{ATTRIBUTION, DataDir, Day, write_new};
use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{DatasetError, Location, Unreachable};
use planetrs_mapclipper::error::ClipError;
use planetrs_pbf::PbfError;

use crate::cli::{Cli, Command, Global};

/// What every command knows before it starts: where things are, and how much to say.
pub struct Context {
    /// Where planets, the dataset and the cache are kept.
    pub data: DataDir,
    /// The dataset to read or build: `--dataset`, else `PLANETRS_DATASET`, else the data directory's.
    pub dataset: Location,
    /// Whether to say anything on stderr beyond errors and the attribution.
    pub quiet: bool,
}

impl Context {
    /// Reads the settings every command shares, the environment standing behind `--data-dir` and
    /// `--dataset`.
    pub fn of(global: &Global) -> anyhow::Result<Self> {
        let data = DataDir::resolve(global.data_dir.as_deref(), |name| std::env::var_os(name))?;
        let dataset = location_of(global.dataset.as_deref(), &data, |name| std::env::var(name).ok());
        Ok(Self {
            data,
            dataset,
            quiet: global.quiet,
        })
    }

    /// The dataset's location as a person reads it.
    pub fn dataset_name(&self) -> String {
        name_of(&self.dataset)
    }

    /// A line on stderr, unless the run is quiet.
    pub fn say(&self, line: &str) {
        if !self.quiet {
            eprintln!("{line}");
        }
    }
}

/// Runs the command the command line asks for, ending with the OpenStreetMap attribution when it
/// read OpenStreetMap data.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    let context = Context::of(&cli.global)?;
    let reads_openstreetmap_data = !matches!(cli.command, Command::Themes(_));
    match cli.command {
        Command::Harvest(arguments) => harvest::run(&context, arguments).map_err(explained),
        Command::Render(arguments) => render::run(&context, arguments).map_err(explained),
        Command::Clip(arguments) => clip::run(&context, arguments).map_err(explained),
        Command::Inspect(arguments) => inspect::run(arguments).map_err(explained),
        Command::Themes(arguments) => themes::run(arguments).map_err(explained),
        Command::Index(arguments) => index::run(&context, arguments),
        Command::Download(arguments) => download::run(&context, arguments),
    }?;
    if reads_openstreetmap_data {
        eprintln!("{ATTRIBUTION}");
    }
    Ok(())
}

/// Writes an output to stdout when its path is `-`, else to a new file under the first free name
/// for its path, never over another, and gives back the name it took and its size.
fn put(path: &Path, contents: &[u8]) -> Result<(PathBuf, u64), ClipError> {
    let size = contents.len() as u64;
    let writing = |error| ClipError::writing(path, error);
    if path.as_os_str() == "-" {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(contents)
            .and_then(|()| stdout.flush())
            .map_err(writing)?;
        return Ok((path.to_path_buf(), size));
    }
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(writing)?;
    }
    let (written, ()) =
        write_new::<_, io::Error>(path, Day::of(SystemTime::now()), |out| out.write_all(contents))
            .map_err(writing)?;
    Ok((written, size))
}

/// A failure, said with what to try next when there is something worth trying.
pub(super) fn explained(failure: ClipError) -> anyhow::Error {
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
        ClipError::Dataset(DatasetError::Unreachable(Unreachable::Missing { file })) if file == MANIFEST => {
            Some("build one: planetrs-mapclipper index, or read the planet: --planet PATH".into())
        }
        ClipError::NoMapIndex => Some("build one: planetrs-mapclipper index".into()),
        ClipError::OutsideTheDataset { countries } => Some(format!(
            "read the planet: --planet PATH, or index the countries it reaches as well: \
             planetrs-mapclipper index --countries {},…",
            countries.join(",")
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
    fn given_a_harvest_from_an_older_build_when_refused_then_the_error_says_how_to_replace_it() {
        let error = ClipError::HarvestVersion {
            found: 0,
            expected: 1,
        };

        assert!(next_step(&error).unwrap().contains("planetrs-mapclipper harvest"));
    }

    #[test]
    fn given_a_theme_that_is_not_known_when_refused_then_the_error_lists_the_ones_that_are() {
        let error = ClipError::UnknownTheme {
            named: "chartreuse".into(),
            known: vec!["plain".into(), "clean".into()],
        };

        assert!(next_step(&error).unwrap().contains("plain, clean"));
    }

    #[test]
    fn given_a_planet_file_that_is_not_sorted_when_refused_then_the_error_says_which_files_are() {
        let error = ClipError::Planet(PbfError::UnsortedSections);

        assert!(next_step(&error).unwrap().contains("Sort.Type_then_ID"));
    }

    #[test]
    fn given_a_box_reaching_beyond_the_countries_indexed_when_refused_then_the_error_says_to_read_the_planet_or_index_those_it_reaches()
     {
        let error = ClipError::OutsideTheDataset {
            countries: vec!["FR".into(), "MC".into()],
        };

        let step = next_step(&error).unwrap();

        assert!(step.contains("--planet PATH"), "{step}");
        assert!(
            step.contains("planetrs-mapclipper index --countries FR,MC,"),
            "{step}"
        );
    }
}
