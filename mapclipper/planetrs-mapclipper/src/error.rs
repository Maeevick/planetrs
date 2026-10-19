//! What can go wrong cutting a map.

use std::io;
use std::path::{Path, PathBuf};

use planetrs_dataset::directory::RecordError;
use planetrs_dataset::{DatasetError, TableError};
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
    /// A country asked for has no outline in the planet.
    #[error("the planet holds no outline of the country {country}")]
    UnknownCountry {
        /// The ISO 3166-1 code asked for.
        country: String,
    },
    /// The planet stores positions in a way a map index cannot hold.
    #[error("the planet's positions cannot be indexed: {detail}")]
    Grid {
        /// What is wrong with them.
        detail: String,
    },
    /// A map index could not be encoded.
    #[error("encoding a map index failed")]
    Encoding(#[source] TableError),
    /// The dataset could not be read.
    #[error(transparent)]
    Dataset(#[from] DatasetError),
    /// The dataset holds no map index to cut a harvest from.
    #[error("the dataset holds no map index")]
    NoMapIndex,
    /// The area reaches beyond the countries the dataset indexes, where it would lack what the planet
    /// holds.
    #[error("the area reaches beyond the countries the dataset indexes, {}", countries.join(", "))]
    OutsideTheDataset {
        /// The countries the dataset indexes.
        countries: Vec<String>,
    },
    /// A map index could not be recorded in the dataset.
    #[error("recording a map index in {} failed", directory.display())]
    Recording {
        /// The dataset's directory.
        directory: PathBuf,
        /// What failed.
        source: RecordError,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_harvest_smaller_than_the_area_wanted_when_refused_then_the_error_names_both_boxes() {
        let error = ClipError::HarvestTooSmall {
            collected: "29.0 x 16.5 km".into(),
            wanted: "40.0 x 22.8 km".into(),
        };

        assert!(error.to_string().contains("29.0 x 16.5 km"));
        assert!(error.to_string().contains("40.0 x 22.8 km"));
    }

    #[test]
    fn given_an_absent_file_when_read_then_the_error_names_it() {
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
