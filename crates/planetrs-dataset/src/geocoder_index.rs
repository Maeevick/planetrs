//! A country's geocoder index: what lies along its streets, commune by commune, and how rare each
//! word of a street name is there.
//!
//! The word counts come first, in row groups of their own, so that a reader fetches them once per
//! country. Every commune's entries follow, in numeric order of the commune, never split across row
//! groups, so that a reader fetches one row group to look a street up in one commune.

use planetrs_geo::wkb;

use crate::geoparquet;
use crate::manifest::{self, Provenance};
use crate::tables::{
    self, Column, TableError, Values, integers, optional_bytes, optional_integers, optional_texts,
    text_lists, texts,
};

/// One entry of a geocoder index.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// How many street names the word counts are over: every name of every commune, once each.
    Names {
        /// The count.
        count: u64,
    },
    /// How many street names hold a word.
    Word {
        /// The word, as a comparison key.
        word: String,
        /// How many names hold it.
        count: u64,
    },
    /// A door: a house number on a street of a commune.
    Door {
        /// The commune's relation.
        commune: i64,
        /// The street's name, as a comparison key.
        street: String,
        /// The house number.
        number: u32,
        /// What follows the number, such as `BIS`, or nothing.
        suffix: String,
        /// Where the door stands, longitude then latitude.
        position: [f64; 2],
    },
    /// The middle of a street, a road or a hamlet with no door of its own.
    Middle {
        /// The commune's relation.
        commune: i64,
        /// The street's name, as a comparison key.
        street: String,
        /// Where its middle lies, longitude then latitude.
        position: [f64; 2],
    },
    /// A shop, by every name it goes by.
    Shop {
        /// The commune's relation.
        commune: i64,
        /// Its names, as comparison keys, its name then its brand.
        names: Vec<String>,
        /// The street it stands on, by its own address, or nothing.
        street: String,
        /// Where it stands, longitude then latitude.
        position: [f64; 2],
    },
}

/// The geocoder index of `country` as a GeoParquet file, its entries in the order given: the word
/// counts first, then each commune's entries together, communes in numeric order.
pub fn to_parquet(country: &str, entries: &[Entry], planet: &Provenance) -> Result<Vec<u8>, TableError> {
    tables::write(
        &[columns_of(entries)],
        &[
            geoparquet::geo(&["Point"]),
            manifest::notice(&format!("geocoder-{country}"), planet),
        ],
    )
}

/// The entries of a geocoder index file, in its order.
pub fn from_parquet(bytes: &[u8]) -> Result<Vec<Entry>, TableError> {
    let columns = tables::read(bytes)?;
    let communes = integers(&columns, "commune")?;
    let kinds = texts(&columns, "kind")?;
    let names = optional_texts(&columns, "name")?;
    let shop_names = text_lists(&columns, "names")?;
    let counts = optional_integers(&columns, "count")?;
    let numbers = optional_integers(&columns, "number")?;
    let suffixes = optional_texts(&columns, "suffix")?;
    let streets = optional_texts(&columns, "street")?;
    let geometries = optional_bytes(&columns, "geometry")?;
    (0..communes.len())
        .map(|row| {
            let commune = communes[row];
            let name = || names[row].clone().ok_or_else(|| unexpected("name"));
            let count = || {
                counts[row]
                    .and_then(|count| u64::try_from(count).ok())
                    .ok_or_else(|| unexpected("count"))
            };
            let position = || {
                geometries[row]
                    .as_deref()
                    .and_then(|bytes| wkb::read_point(bytes).ok())
                    .map(|(longitude, latitude)| [longitude, latitude])
                    .ok_or_else(|| unexpected("geometry"))
            };
            match kinds[row].as_str() {
                NAMES => Ok(Entry::Names { count: count()? }),
                WORD => Ok(Entry::Word {
                    word: name()?,
                    count: count()?,
                }),
                DOOR => Ok(Entry::Door {
                    commune,
                    street: name()?,
                    number: numbers[row]
                        .and_then(|number| u32::try_from(number).ok())
                        .ok_or_else(|| unexpected("number"))?,
                    suffix: suffixes[row].clone().unwrap_or_default(),
                    position: position()?,
                }),
                MIDDLE => Ok(Entry::Middle {
                    commune,
                    street: name()?,
                    position: position()?,
                }),
                SHOP => Ok(Entry::Shop {
                    commune,
                    names: shop_names[row].clone(),
                    street: streets[row].clone().unwrap_or_default(),
                    position: position()?,
                }),
                _ => Err(unexpected("kind")),
            }
        })
        .collect()
}

const NAMES: &str = "names";
const WORD: &str = "word";
const DOOR: &str = "door";
const MIDDLE: &str = "middle";
const SHOP: &str = "shop";

/// One entry as the row it is written as: every column, most of them empty for any one kind.
#[derive(Default)]
struct Row {
    commune: i64,
    kind: &'static str,
    name: Option<String>,
    names: Vec<String>,
    count: Option<i64>,
    number: Option<i64>,
    suffix: Option<String>,
    street: Option<String>,
    geometry: Option<Vec<u8>>,
}

fn row_of(entry: &Entry) -> Row {
    let point = |[longitude, latitude]: [f64; 2]| Some(wkb::point(longitude, latitude));
    match entry {
        Entry::Names { count } => Row {
            kind: NAMES,
            count: Some(*count as i64),
            ..Row::default()
        },
        Entry::Word { word, count } => Row {
            kind: WORD,
            name: Some(word.clone()),
            count: Some(*count as i64),
            ..Row::default()
        },
        Entry::Door {
            commune,
            street,
            number,
            suffix,
            position,
        } => Row {
            commune: *commune,
            kind: DOOR,
            name: Some(street.clone()),
            number: Some(i64::from(*number)),
            suffix: Some(suffix.clone()),
            geometry: point(*position),
            ..Row::default()
        },
        Entry::Middle {
            commune,
            street,
            position,
        } => Row {
            commune: *commune,
            kind: MIDDLE,
            name: Some(street.clone()),
            geometry: point(*position),
            ..Row::default()
        },
        Entry::Shop {
            commune,
            names,
            street,
            position,
        } => Row {
            commune: *commune,
            kind: SHOP,
            names: names.clone(),
            street: Some(street.clone()),
            geometry: point(*position),
            ..Row::default()
        },
    }
}

fn columns_of(entries: &[Entry]) -> Vec<Column> {
    let rows: Vec<Row> = entries.iter().map(row_of).collect();
    let column = |name: &str, values: Values| Column::of(name, values);
    vec![
        column(
            "commune",
            Values::Integers(rows.iter().map(|row| row.commune).collect()),
        ),
        column(
            "kind",
            Values::Texts(rows.iter().map(|row| row.kind.to_string()).collect()),
        ),
        column(
            "name",
            Values::OptionalTexts(rows.iter().map(|row| row.name.clone()).collect()),
        ),
        column(
            "names",
            Values::TextLists(rows.iter().map(|row| row.names.clone()).collect()),
        ),
        column(
            "count",
            Values::OptionalIntegers(rows.iter().map(|row| row.count).collect()),
        ),
        column(
            "number",
            Values::OptionalIntegers(rows.iter().map(|row| row.number).collect()),
        ),
        column(
            "suffix",
            Values::OptionalTexts(rows.iter().map(|row| row.suffix.clone()).collect()),
        ),
        column(
            "street",
            Values::OptionalTexts(rows.iter().map(|row| row.street.clone()).collect()),
        ),
        column(
            "geometry",
            Values::OptionalBytes(rows.into_iter().map(|row| row.geometry).collect()),
        ),
    ]
}

fn unexpected(name: &str) -> TableError {
    TableError::Unreadable {
        detail: format!("the geocoder index's column {name} is missing or does not fit its kind of entry"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planet() -> Provenance {
        Provenance {
            source_name: "planet-260907.osm.pbf".into(),
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 4_321,
            writing_program: "planet-dump-ng 1.2.7".into(),
        }
    }

    fn sens() -> Vec<Entry> {
        vec![
            Entry::Names { count: 3 },
            Entry::Word {
                word: "AVENUE".into(),
                count: 1,
            },
            Entry::Word {
                word: "GARE".into(),
                count: 2,
            },
            Entry::Door {
                commune: 946_433,
                street: "AVENUE GARE".into(),
                number: 1,
                suffix: "BIS".into(),
                position: [3.268_240_2, 48.198_06],
            },
            Entry::Door {
                commune: 946_433,
                street: "AVENUE GARE".into(),
                number: 3,
                suffix: String::new(),
                position: [3.268_3, 48.198_1],
            },
            Entry::Middle {
                commune: 946_433,
                street: "AVENUE GARE".into(),
                position: [3.268_208_2, 48.197_500_6],
            },
            Entry::Shop {
                commune: 946_433,
                names: vec!["ALDI".into(), "ALDI".into()],
                street: "CHEMIN CANNETIERES".into(),
                position: [3.290_627, 48.176_905_6],
            },
            Entry::Middle {
                commune: 1_402_847,
                street: "RUE GARE".into(),
                position: [3.29, 48.21],
            },
        ]
    }

    #[test]
    fn given_a_countrys_entries_when_written_as_parquet_and_read_back_then_they_are_the_same_entries_in_order()
    -> Result<(), TableError> {
        let entries = sens();

        let read_back = from_parquet(&to_parquet("FR", &entries, &planet())?)?;

        assert_eq!(read_back, entries);
        Ok(())
    }
}
