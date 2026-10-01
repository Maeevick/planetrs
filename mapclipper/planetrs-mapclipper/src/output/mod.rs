//! Turning a scene into something to look at or to hand on.
//!
//! Three shapes, and they are not the same thing under ODbL. The **GeoJSON is a derivative
//! database**, so share-alike applies to it and shipping it to a browser is arguably conveying it.
//! The **SVG and PNG are produced works**, where attribution is the obligation and share-alike is
//! not. Both carry the notice; only one carries the rest.

pub mod geojson;
#[cfg(feature = "png")]
pub mod png;
pub mod svg;

pub use svg::{Icon, Mark, Marker};
