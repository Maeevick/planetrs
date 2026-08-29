//! The index on disk, for writing one by hand or reading one without a [`crate::Geocoder`].
//!
//! An index is a directory of tab-separated files, each opening with comment lines that say which
//! planet it was built from and carry the OpenStreetMap attribution:
//!
//! | file            | one line per                                         | required |
//! | --------------- | ---------------------------------------------------- | -------- |
//! | `communes.tsv`  | municipality: relation, level, postcodes, names, centre, country | yes |
//! | `countries.tsv` | country: ISO code and every name it answers to        | yes      |
//! | `streets.tsv`   | door of a street, or the middle of a road or hamlet   | no       |
//! | `shops.tsv`     | shop: its names, its own street and where it stands   | no       |
//! | `parents.tsv`   | former town, and the commune that absorbed it        | no       |
//! | `areas.tsv`     | outlined commune, and its area in square kilometres   | no       |
//!
//! Every file is sorted, so two builds of one planet are byte-identical, and every name is filed as
//! its comparison key, upper case, accents and filler words gone. The index is a Derivative Database
//! of OpenStreetMap under the ODbL 1.0: whoever distributes one distributes it under that licence,
//! with the attribution its headers carry.

pub use crate::boundary::Country;
pub use crate::gazetteer::Commune;
pub use crate::index_file::{
    IndexError, read_areas, read_index, read_parents, read_shops, read_streets, write_areas, write_index,
    write_parents, write_shops, write_streets,
};
pub use crate::shops::{Shop, ShopIndex};
pub use crate::street_index::{AddressPoint, Street, StreetIndex};
pub use crate::text::HouseNumber;
