//! What every output says about the data it holds, in the form the OpenStreetMap Foundation asks
//! for each medium (<https://www.openstreetmap.org/copyright>).

/// Where the credit links, wherever a link works.
pub const COPYRIGHT_PAGE: &str = "https://www.openstreetmap.org/copyright";

/// The notice of OpenStreetMap data handed on as data, a harvest or a GeoJSON: it names the licence
/// and links to it directly.
pub const DATA_NOTICE: &str = "© OpenStreetMap contributors, under the Open Database License (ODbL) 1.0: \
     https://opendatacommons.org/licenses/odbl/1-0/ (https://www.openstreetmap.org/copyright)";

/// The credit a map shows on the picture itself, one line each. A picture holds no link, so the
/// copyright page is written out in full, as on a printed page.
pub const MAP_CREDIT: [&str; 2] = ["© OpenStreetMap contributors", COPYRIGHT_PAGE];
