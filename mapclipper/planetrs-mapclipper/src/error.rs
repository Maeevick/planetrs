//! What can go wrong cutting a map, and what to do about each.

use std::io;
use std::path::{Path, PathBuf};

use planetrs_geo::GeoError;
use planetrs_pbf::PbfError;

/// Why a map could not be cut. Each message says its own part and leaves the cause to `source`, so
/// a chain of causes reads without repeating itself.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClipError {
    /// A file could not be read.
    #[error("reading {}", path.display())]
    Reading {
        /// The file.
        path: PathBuf,
        /// Why it could not be read.
        source: io::Error,
    },
    /// A file could not be written.
    #[error("writing {}", path.display())]
    Writing {
        /// The file.
        path: PathBuf,
        /// Why it could not be written.
        source: io::Error,
    },
    /// The planet file could not be read as one.
    #[error(transparent)]
    Planet(#[from] PbfError),
    /// The area asked for is not one this tool can collect.
    #[error(transparent)]
    Area(#[from] GeoError),
    /// The file is not a harvest at all.
    #[error("this file is not a harvest")]
    NotAHarvest,
    /// The harvest was written by a build that wrote another version of the format.
    #[error("this harvest is version {found} and this build reads version {expected}")]
    HarvestVersion {
        /// The version the file states.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// The harvest stops part way through.
    #[error("this harvest ends part way through")]
    HarvestTruncated,
    /// The harvest does not cover the ground the frame collects.
    #[error("the harvest covers {collected} and the area asked for collects {wanted}")]
    HarvestTooSmall {
        /// The ground the harvest covers, as a reader reads it.
        collected: String,
        /// The ground the frame collects.
        wanted: String,
    },
    /// A theme file holds a line it cannot be read from.
    #[error("{}:{line}: {detail}", path.display())]
    Theme {
        /// The theme file.
        path: PathBuf,
        /// The line, from 1.
        line: usize,
        /// What is wrong with it.
        detail: String,
    },
    /// No built-in theme has that name, and no file is at that path.
    #[error("no theme called {named}")]
    UnknownTheme {
        /// The name asked for.
        named: String,
        /// The themes built in.
        known: Vec<String>,
    },
    /// The icon file holds no drawing.
    #[error("{}: no <svg> element with a drawing in it", path.display())]
    NotAnIcon {
        /// The icon file.
        path: PathBuf,
    },
    /// The canvas has no pixel one way or the other.
    #[error("the canvas must be at least one pixel each way")]
    CanvasNotPositive,
    /// The picture could not be rasterised.
    #[error("the svg did not rasterise: {detail}")]
    Raster {
        /// What the rasteriser said.
        detail: String,
    },
}

impl ClipError {
    /// A file that could not be read, and why.
    pub fn reading(path: &Path, source: io::Error) -> Self {
        Self::Reading {
            path: path.to_path_buf(),
            source,
        }
    }

    /// A file that could not be written, and why.
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

    /// The failure and its causes, joined as a command line prints them.
    fn joined(error: &dyn std::error::Error) -> String {
        let mut text = error.to_string();
        let mut cause = error.source();
        while let Some(source) = cause {
            text.push_str(&format!(": {source}"));
            cause = source.source();
        }
        text
    }

    #[test]
    fn given_failures_with_causes_when_joined_with_them_then_each_cause_is_said_once() {
        let unread = ClipError::reading(Path::new("/tmp/absent.mapclip"), io::Error::other("no such file"));
        let unsorted = ClipError::from(PbfError::UnsortedSections);

        assert_eq!(joined(&unread), "reading /tmp/absent.mapclip: no such file");
        assert_eq!(
            joined(&unsorted),
            "the file does not declare Sort.Type_then_ID, so its sections cannot be bisected"
        );
    }
}
