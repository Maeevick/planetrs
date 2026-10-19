//! Turning "twelve kilometres across, centred here" into a box in degrees, and back into the
//! integer units a PBF block stores coordinates in.
//!
//! Ground size is asked for in kilometres and never in degrees. A degree of longitude is
//! `1/cos(latitude)` shorter than a degree of latitude, so the same box in raw degrees is 1.62
//! times too wide at Rotterdam and 2.88 times too wide at Tromsø.
//!
//! An [`Outline`] closes a boundary's ways into rings and says whether a point lies inside, and a
//! [`territory::TerritoryMask`] answers the same question coarsely for nine billion nodes at once.

mod area;
mod bounds;
pub mod outline;
pub mod territory;
mod window;
pub mod wkb;

pub use area::{Area, GeoError};
pub use bounds::{BoundingBox, metres_per_degree};
pub use outline::Outline;
pub use window::Window;
