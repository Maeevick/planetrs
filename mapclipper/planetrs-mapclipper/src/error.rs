//! What can go wrong cutting a map, and what to do about each.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use planetrs_geo::GeoError;
use planetrs_pbf::PbfError;

#[derive(Debug)]
pub enum ClipError {
    Reading {
        path: PathBuf,
        source: io::Error,
    },
    Writing {
        path: PathBuf,
        source: io::Error,
    },
    Planet(PbfError),
    Area(GeoError),
    NotAHarvest,
    HarvestVersion {
        found: u32,
        expected: u32,
    },
    HarvestTruncated,
    HarvestTooSmall {
        collected: String,
        wanted: String,
    },
    Theme {
        path: PathBuf,
        line: usize,
        detail: String,
    },
    UnknownTheme {
        named: String,
        known: Vec<String>,
    },
    NotAnIcon {
        path: PathBuf,
    },
    CanvasNotPositive,
    Raster {
        detail: String,
    },
}

impl ClipError {
    pub fn reading(path: &Path, source: io::Error) -> Self {
        Self::Reading {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn writing(path: &Path, source: io::Error) -> Self {
        Self::Writing {
            path: path.to_path_buf(),
            source,
        }
    }

    /// What a reader should try next, when there is something worth trying.
    pub fn next_step(&self) -> Option<String> {
        match self {
            Self::NotAHarvest | Self::HarvestVersion { .. } => {
                Some("cut a fresh one with: planetrs-mapclipper harvest <planet.osm.pbf> --lat … --lon … --width-km … --height-km … --out <file.mapclip>".into())
            }
            Self::HarvestTooSmall { .. } => {
                Some("harvest a wider area, or lower --margin, then render again".into())
            }
            Self::UnknownTheme { known, .. } => Some(format!(
                "built-in themes are {}, or pass a path to a .toml",
                known.join(", ")
            )),
            Self::Planet(PbfError::UnsortedSections) => Some(
                "this reader needs a planet file written with Sort.Type_then_ID, as \
                 planet.openstreetmap.org publishes"
                    .into(),
            ),
            _ => None,
        }
    }
}

impl fmt::Display for ClipError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reading { path, source } => write!(out, "reading {}: {source}", path.display()),
            Self::Writing { path, source } => write!(out, "writing {}: {source}", path.display()),
            Self::Planet(error) => write!(out, "{error}"),
            Self::Area(error) => write!(out, "{error}"),
            Self::NotAHarvest => write!(out, "this file is not a harvest"),
            Self::HarvestVersion { found, expected } => {
                write!(
                    out,
                    "this harvest is version {found} and this build reads version {expected}"
                )
            }
            Self::HarvestTruncated => write!(out, "this harvest ends part way through"),
            Self::HarvestTooSmall { collected, wanted } => write!(
                out,
                "the harvest covers {collected} and the area asked for collects {wanted}"
            ),
            Self::Theme { path, line, detail } => {
                write!(out, "{}:{line}: {detail}", path.display())
            }
            Self::UnknownTheme { named, .. } => write!(out, "no theme called {named}"),
            Self::NotAnIcon { path } => {
                write!(out, "{}: no <svg> element with a drawing in it", path.display())
            }
            Self::CanvasNotPositive => write!(out, "the canvas must be at least one pixel each way"),
            Self::Raster { detail } => write!(out, "the svg did not rasterise: {detail}"),
        }
    }
}

impl std::error::Error for ClipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Reading { source, .. } | Self::Writing { source, .. } => Some(source),
            Self::Planet(error) => Some(error),
            Self::Area(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PbfError> for ClipError {
    fn from(error: PbfError) -> Self {
        Self::Planet(error)
    }
}

impl From<GeoError> for ClipError {
    fn from(error: GeoError) -> Self {
        Self::Area(error)
    }
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

        assert!(error.next_step().unwrap().contains("planetrs-mapclipper harvest"));
    }

    #[test]
    fn when_a_harvest_is_too_small_then_the_error_names_both_boxes() {
        let error = ClipError::HarvestTooSmall {
            collected: "29.0 x 16.5 km".into(),
            wanted: "40.0 x 22.8 km".into(),
        };

        assert!(error.to_string().contains("29.0 x 16.5 km"));
        assert!(error.to_string().contains("40.0 x 22.8 km"));
    }

    #[test]
    fn when_a_theme_is_not_known_then_the_error_lists_the_ones_that_are() {
        let error = ClipError::UnknownTheme {
            named: "chartreuse".into(),
            known: vec!["plain".into(), "clean".into()],
        };

        assert!(error.next_step().unwrap().contains("plain, clean"));
    }

    #[test]
    fn when_a_planet_file_is_not_sorted_then_the_error_says_which_files_are() {
        let error = ClipError::Planet(PbfError::UnsortedSections);

        assert!(error.next_step().unwrap().contains("Sort.Type_then_ID"));
    }

    #[test]
    fn when_a_file_cannot_be_read_then_the_error_names_it() {
        let error = ClipError::reading(Path::new("/tmp/absent.mapclip"), io::Error::other("no such file"));

        assert!(error.to_string().contains("/tmp/absent.mapclip"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
