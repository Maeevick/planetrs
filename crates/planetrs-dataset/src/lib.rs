//! The dataset both planetrs tools read instead of the planet: a manifest, a planet-wide gazetteer,
//! a geocoder index per country, and a map index cut into tiles.
//!
//! A dataset is built whole from one planet file, so every file in it answers for the same minute of
//! OpenStreetMap history, and the manifest says which. It is read from its directory file by file and
//! range by range, so that a lookup or a map reads only the row groups it needs.
//!
//! Data read or written through this crate is © OpenStreetMap contributors, under the Open Data
//! Commons Open Database License 1.0.

#![warn(missing_docs)]

pub mod dataset;
pub mod directory;
pub mod files;
pub mod gazetteer;
pub mod geocoder_index;
mod geoparquet;
pub mod manifest;
pub mod map_index;
mod tables;
pub mod territory;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use dataset::{CommunesRead, CountryIndex, Dataset, DatasetError, MapTile};
pub use files::{DirectoryFiles, ForReadingFiles, Unreachable};
pub use manifest::{Entry, Manifest, ManifestError, Part, Provenance};
pub use tables::TableError;
