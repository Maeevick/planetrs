//! Cutting a small illustrated map out of the OpenStreetMap planet file, offline.
//!
//! The work splits in two, and the split is the whole design. **Harvesting** reads the planet and
//! is expensive: there is no spatial index in a PBF file, so every question about a place is a full
//! scan. **Rendering** draws what was harvested and is not: it is milliseconds, and it can be asked
//! again at another framing, another size, another theme.
//!
//! Everything between a harvest and an output is a pure function, so the interesting half is
//! testable without a planet file anywhere near it.
//!
//! Data read through this crate is © OpenStreetMap contributors, ODbL 1.0.

pub mod credit;
pub mod error;
pub mod progress;
pub mod report;

pub use error::ClipError;
