//! An index opened for lookups, which is all a caller needs once the index is built.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::gazetteer::Gazetteer;
use crate::index_file::{
    COMMUNES, IndexError, read_areas, read_index, read_parents, read_shops, read_streets,
};
use crate::locate::{Answer, Site, locate};
use crate::shops::ShopIndex;
use crate::street_index::StreetIndex;
use crate::trace::trace;

/// The communes, streets, doors and shops of an index, held in memory for lookups.
///
/// Opening reads every file of the index once; each lookup after that is pure and touches no disk.
pub struct Geocoder {
    gazetteer: Gazetteer,
    streets: StreetIndex,
    shops: ShopIndex,
}

/// Why an index could not be opened.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OpenError {
    /// The directory holds no index: it has no communes file.
    #[error("no index in {}", directory.display())]
    NoIndex {
        /// The directory that was searched.
        directory: PathBuf,
    },
    /// An index is there but one of its files could not be read.
    #[error("the index in {} could not be read", directory.display())]
    Unreadable {
        /// The directory holding the index.
        directory: PathBuf,
        /// The file that failed, and how.
        #[source]
        source: IndexError,
    },
}

impl Geocoder {
    /// Opens the index built into `directory`.
    ///
    /// Only the communes and countries are required. An index built without streets, shops, former
    /// towns or areas opens all the same, and its lookups stop at the commune.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, OpenError> {
        let directory = directory.as_ref();
        if !directory.join(COMMUNES).is_file() {
            return Err(OpenError::NoIndex {
                directory: directory.to_path_buf(),
            });
        }
        let unreadable = |source| OpenError::Unreadable {
            directory: directory.to_path_buf(),
            source,
        };
        let (communes, countries) = read_index(directory).map_err(unreadable)?;
        let parents = read_parents(directory).map_err(unreadable)?;
        let areas = read_areas(directory).map_err(unreadable)?;
        let gazetteer = Gazetteer::of(communes, countries)
            .with_parents(parents)
            .with_areas(areas);
        let streets = read_streets(directory).map_err(unreadable)?;
        let shops = read_shops(directory).map_err(unreadable)?;
        Ok(Self {
            gazetteer,
            streets,
            shops,
        })
    }

    /// Where the site is and how precisely, or nothing when not even its commune could be settled.
    ///
    /// An answer at a commune's centre is still an answer: [`crate::refusal_of`] says whether a
    /// person should look at it before it is used.
    pub fn locate(&self, site: &Site) -> Option<Answer> {
        locate(&self.gazetteer, &self.streets, &self.shops, site)
    }

    /// Every decision [`Geocoder::locate`] takes for the site, one line each, ending with its answer.
    pub fn trace(&self, site: &Site) -> Vec<String> {
        trace(&self.gazetteer, &self.streets, &self.shops, site)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Country;
    use crate::gazetteer::Commune;
    use crate::index_file::{write_index, write_streets};
    use crate::locate::Precision;
    use crate::street_index::{AddressPoint, Street};
    use crate::text::HouseNumber;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("planetrs-geocoder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn sens() -> Commune {
        Commune {
            relation: 1,
            level: 8,
            names: vec!["SENS".into()],
            postcodes: vec!["89100".into()],
            centre: Some([3.283, 48.197]),
            country: Some("FR".into()),
        }
    }

    fn france() -> Country {
        Country {
            code: "FR".into(),
            names: vec!["FRANCE".into()],
        }
    }

    fn site(address: &str) -> Site {
        Site {
            label: String::new(),
            address: address.into(),
            postcode: "89100".into(),
            town: "SENS".into(),
            country: "FRANCE".into(),
        }
    }

    fn written_index(name: &str) -> PathBuf {
        let directory = scratch(name);
        write_index(&directory, &[sens()], &[france()], "a test").expect("the gazetteer is written");
        let streets = StreetIndex::of(vec![(
            1,
            Street {
                name: "ROUTE TILLEULS".into(),
                addresses: vec![AddressPoint {
                    number: HouseNumber {
                        number: 8,
                        suffix: String::new(),
                    },
                    position: [3.2652, 48.2],
                }],
                path: vec![],
            },
        )]);
        write_streets(&directory, &streets, "a test").expect("the streets are written");
        directory
    }

    #[test]
    fn given_an_index_written_to_a_directory_when_opened_then_its_doors_answer() {
        let directory = written_index("doors");

        let found = Geocoder::open(&directory)
            .expect("the index opens")
            .locate(&site("8 Route des Tilleuls"))
            .expect("an answer");

        assert_eq!(
            (found.position, found.precision),
            ([3.2652, 48.2], Precision::HouseNumber)
        );
    }

    #[test]
    fn given_an_index_without_streets_when_opened_then_the_commune_still_answers() {
        let directory = scratch("communes-only");
        write_index(&directory, &[sens()], &[france()], "a test").expect("the gazetteer is written");

        let found = Geocoder::open(&directory)
            .expect("the index opens")
            .locate(&site("8 Route des Tilleuls"))
            .expect("an answer");

        assert_eq!(found.precision, Precision::Commune);
    }

    #[test]
    fn given_a_directory_with_no_index_when_opened_then_it_says_there_is_none() {
        let directory = scratch("empty");
        std::fs::create_dir_all(&directory).expect("the directory exists");

        assert!(matches!(
            Geocoder::open(&directory),
            Err(OpenError::NoIndex { directory: searched }) if searched == directory
        ));
    }

    #[test]
    fn given_an_index_file_that_does_not_parse_when_opened_then_it_is_unreadable() {
        let directory = written_index("malformed");
        std::fs::write(directory.join("streets.tsv"), "not\tan\tindex\tline\n").expect("the file is spoilt");

        assert!(matches!(
            Geocoder::open(&directory),
            Err(OpenError::Unreadable { .. })
        ));
    }

    #[test]
    fn given_an_opened_index_when_a_site_is_traced_then_the_last_line_is_its_answer() {
        let directory = written_index("trace");

        let written = Geocoder::open(&directory)
            .expect("the index opens")
            .trace(&site("8 Route des Tilleuls"));

        assert!(
            written.last().is_some_and(|line| line.contains("HouseNumber")),
            "{written:?}"
        );
    }
}
