//! The dataset both planetrs tools read instead of the planet: a manifest, a planet-wide gazetteer,
//! and one geocoder index and one map index per country.
//!
//! A dataset is built whole from one planet file, so every file in it answers for the same minute of
//! OpenStreetMap history, and the manifest says which. It is read from a directory or, file by file
//! and range by range, over HTTP.
//!
//! Data read or written through this crate is © OpenStreetMap contributors, under the Open Data
//! Commons Open Database License 1.0.

#![warn(missing_docs)]

pub mod directory;
pub mod files;
pub mod gazetteer;
mod geoparquet;
#[cfg(feature = "remote")]
pub mod http;
pub mod manifest;
mod tables;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use files::{DirectoryFiles, ForReadingFiles, Unreachable};
#[cfg(feature = "remote")]
pub use http::HttpFiles;
pub use manifest::{Entry, Manifest, ManifestError, Part, Provenance};
pub use tables::TableError;
