//! Turning a place or a postal address into a coordinate, from OpenStreetMap data alone.
//!
//! The work is split in two. [`build_index`] reads an OpenStreetMap planet file and writes an
//! index: every municipality of the planet and, for the countries asked for, every street, door,
//! hamlet and shop. [`Geocoder::open`] loads that index, and [`Geocoder::locate`] places a [`Site`]
//! with no network and no planet file.
//!
//! ```no_run
//! use planetrs_geocoder::{Geocoder, Site};
//!
//! let geocoder = Geocoder::open("index")?;
//! let site = Site {
//!     label: String::new(),
//!     address: "8 Route des Tilleuls".into(),
//!     postcode: "89100".into(),
//!     town: "Sens".into(),
//!     country: "France".into(),
//! };
//! if let Some(answer) = geocoder.locate(&site) {
//!     println!("{:.6};{:.6};{}", answer.latitude(), answer.longitude(), answer.precision);
//! }
//! # Ok::<(), planetrs_geocoder::OpenError>(())
//! ```
//!
//! # Every answer says what kind of answer it is
//!
//! A door and the middle of a town are both "found", and only one of them is an address, so every
//! [`Answer`] carries its [`Precision`], best first:
//!
//! | precision            | where the coordinate stands                                              |
//! | -------------------- | ------------------------------------------------------------------------ |
//! | `HouseNumber`        | the asked door of the asked street                                       |
//! | `NearestHouseNumber` | the recorded door of the asked street closest to the asked number        |
//! | `Shop`               | a shop of the site's label on the asked street, or alone in the commune  |
//! | `Street`             | the middle of the asked street, or of the named hamlet                   |
//! | `Commune`            | the commune's centre                                                     |
//!
//! The commune is settled first, from its name and postcode, and the street is only ever looked for
//! inside it. [`refusal_of`] says when an answer should be seen by a person before it is used: a
//! commune's centre is refused unless the commune is small enough for its centre to be near the site
//! wherever it stands.
//!
//! # Scope
//!
//! The gazetteer covers the planet. Streets, doors and shops are indexed for the countries named at
//! build time, France and Monaco unless told otherwise ([`DEFAULT_COUNTRIES`]). The reading rules,
//! street types, designators such as `ZA` or `C.Cial`, road numbers and sound-alike spellings, are
//! French; addresses of other countries are read with them all the same, which suits them less.
//!
//! # Cost
//!
//! A build reads the planet several times and holds a country's streets in memory, about 20 GB for
//! France. The index of France and Monaco is about 900 MB on disk. Opening it takes a few seconds
//! and about 4 GB of memory, and each lookup after that takes well under a millisecond.
//!
//! # Data and licence
//!
//! The code is MIT. The data is © OpenStreetMap contributors, under the Open Data Commons Open
//! Database License 1.0 (<https://www.openstreetmap.org/copyright>). An index is a Derivative
//! Database of OpenStreetMap, and every file it writes names the licence and links to it. Under the
//! OSMF geocoding guideline, a geocoding result on its own is an insubstantial extract and brings no
//! share-alike, but whoever offers geocoding built on OpenStreetMap credits it.
//!
//! # Stability
//!
//! Until 1.0, the API and the index format may change between minor versions. An index records the
//! planet it was built from: rebuild it after upgrading.

#![warn(missing_docs)]

mod assemble;
mod boundary;
mod file_streets;
mod filing;
mod gazetteer;
mod harvest;
mod loose;
mod rarity;
mod rings;
mod shops;
mod similarity;
mod street;
mod street_index;
mod territory;
mod text;

pub use gazetteer::Basis;
pub use street::Quality;
