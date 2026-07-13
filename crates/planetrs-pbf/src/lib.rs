//! Reading an OpenStreetMap PBF file: the framing, the blocks, and an index over a sorted planet.
//!
//! The crate reads a planet file and nothing else. No API, no tile service, no network.
//!
//! Two properties of the format shape everything here. There is **no spatial index**, so any
//! question about a place is a full scan of the file. And the planet declares `Sort.Type_then_ID`,
//! so every node precedes every way, ids ascend within a section, and both the section bisection in
//! [`Planet::sections`] and the id pruning in [`IdIndex`] are sound. The second is checked against
//! the file's own header rather than assumed.
//!
//! Data read through this crate is © OpenStreetMap contributors, ODbL 1.0.

pub mod error;
pub mod proto;

pub use error::PbfError;
