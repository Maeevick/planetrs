//! What an index holds once read, for whoever wants to look inside one without a
//! [`crate::Geocoder`].
//!
//! An index is the geocoder's share of a dataset (see `planetrs-dataset`): the planet-wide gazetteer
//! of communes and countries, and one geocoder index per country with its doors, the middles of its
//! roads and hamlets, its shops, and how rare each word of its street names is there. Every name is
//! filed as its comparison key, upper case, accents and filler words gone, and every position is
//! kept to seven decimals of a degree. An index is a Derivative Database of OpenStreetMap under the
//! ODbL 1.0: whoever distributes one distributes it under that licence, with the attribution its
//! files carry.

pub use crate::boundary::Country;
pub use crate::gazetteer::Commune;
pub use crate::shops::{Shop, ShopIndex};
pub use crate::street_index::{AddressPoint, Street, StreetIndex};
pub use crate::text::HouseNumber;
