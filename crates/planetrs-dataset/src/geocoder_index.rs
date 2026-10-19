//! A country's geocoder index: what lies along its streets, commune by commune, and how rare each
//! word of a street name is there.
//!
//! The word counts come first, in row groups of their own, so that a reader fetches them once per
//! country. Every commune's entries follow, in numeric order of the commune, never split across row
//! groups, so that a reader fetches one row group to look a street up in one commune.

use crate::manifest::Provenance;
use crate::tables::TableError;

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
    let _ = (country, entries, planet);
    Ok(Vec::new())
}

/// The entries of a geocoder index file, in its order.
pub fn from_parquet(bytes: &[u8]) -> Result<Vec<Entry>, TableError> {
    let _ = bytes;
    Ok(Vec::new())
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
