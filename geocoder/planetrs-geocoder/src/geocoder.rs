//! An index opened for lookups, which is all a caller needs once the index is built.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{Dataset, DatasetError, DirectoryFiles, Part, Unreachable};
use thiserror::Error;

use crate::gazetteer::Gazetteer;
use crate::locate::{Answer, Site, locate};
use crate::parts::{indexes_of, settling_gazetteer};
use crate::shops::ShopIndex;
use crate::street_index::StreetIndex;
use crate::trace::trace;

/// The communes, streets, doors and shops of a dataset, held in memory for lookups.
///
/// Opening reads the gazetteer and every country's geocoder index once; each lookup after that is
/// pure and touches no disk, and searches the streets and shops of the site's own country.
pub struct Geocoder {
    gazetteer: Gazetteer,
    countries: HashMap<String, (StreetIndex, ShopIndex)>,
    nowhere: (StreetIndex, ShopIndex),
}

/// Why an index could not be opened.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OpenError {
    /// The directory holds no dataset: it has no manifest.
    #[error("no index in {}", directory.display())]
    NoIndex {
        /// The directory that was searched.
        directory: PathBuf,
    },
    /// A dataset is there but one of its files could not be read.
    #[error("the index in {} could not be read", directory.display())]
    Unreadable {
        /// The directory holding the dataset.
        directory: PathBuf,
        /// The file that failed, and how.
        #[source]
        source: DatasetError,
    },
}

impl Geocoder {
    /// Opens the dataset in `directory`.
    ///
    /// Only the gazetteer is required. A dataset indexing no country opens all the same, and its
    /// lookups stop at the commune.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, OpenError> {
        let directory = directory.as_ref();
        let unreadable = |source| OpenError::Unreadable {
            directory: directory.to_path_buf(),
            source,
        };
        let dataset = Dataset::open(DirectoryFiles::at(directory)).map_err(|source| match source {
            DatasetError::Unreachable(Unreachable::Missing { file }) if file == MANIFEST => {
                OpenError::NoIndex {
                    directory: directory.to_path_buf(),
                }
            }
            other => unreadable(other),
        })?;
        let gazetteer = settling_gazetteer(dataset.gazetteer().map_err(unreadable)?);
        let mut indexed = HashMap::new();
        for entry in &dataset.manifest().files {
            if let Part::GeocoderIndex { country } = &entry.part {
                let entries = dataset
                    .geocoder_index(country)
                    .map_err(unreadable)?
                    .unwrap_or_default();
                indexed.insert(country.clone(), indexes_of(entries));
            }
        }
        Ok(Self {
            gazetteer,
            countries: indexed,
            nowhere: (StreetIndex::default(), ShopIndex::default()),
        })
    }

    /// The streets and shops of the country a site names, or none when the dataset does not index it.
    fn indexes_for(&self, site: &Site) -> &(StreetIndex, ShopIndex) {
        self.gazetteer
            .country_code_for(&site.without_parentheses().country)
            .and_then(|code| self.countries.get(code))
            .unwrap_or(&self.nowhere)
    }

    /// Where the site is and how precisely, or nothing when not even its commune could be settled.
    ///
    /// An answer at a commune's centre is still an answer: [`crate::refusal_of`] says whether a
    /// person should look at it before it is used.
    pub fn locate(&self, site: &Site) -> Option<Answer> {
        let (streets, shops) = self.indexes_for(site);
        locate(&self.gazetteer, streets, shops, site)
    }

    /// Every decision [`Geocoder::locate`] takes for the site, one line each, ending with its answer.
    pub fn trace(&self, site: &Site) -> Vec<String> {
        let (streets, shops) = self.indexes_for(site);
        trace(&self.gazetteer, streets, shops, site)
    }
}

#[cfg(test)]
mod tests {
    use planetrs_dataset::directory::record;
    use planetrs_dataset::{Provenance, gazetteer, geocoder_index};

    use super::*;
    use crate::boundary::Country;
    use crate::gazetteer::Commune;
    use crate::locate::Precision;
    use crate::parts::{gazetteer_of, geocoder_index_of};
    use crate::street_index::{AddressPoint, Street};
    use crate::text::HouseNumber;

    fn planet() -> Provenance {
        Provenance {
            source_name: "a test planet".into(),
            file_size: 1,
            blob_count: 1,
            replication_timestamp: 0,
            replication_sequence: 0,
            writing_program: "a test".into(),
        }
    }

    /// A dataset holding the given communes and countries, and, when there are any, France's streets.
    fn written_dataset(name: &str, communes: &[Commune], streets: Option<StreetIndex>) -> PathBuf {
        let directory = scratch(name);
        let relations = HashMap::from([("FR".to_string(), 2_202_162)]);
        let areas = gazetteer_of(
            communes,
            &[france()],
            &relations,
            &HashMap::new(),
            &HashMap::new(),
        );
        let bytes = gazetteer::to_parquet(&areas, &planet()).expect("the gazetteer is encoded");
        record(
            &directory,
            &planet(),
            Part::Gazetteer,
            "gazetteer.parquet",
            &bytes,
            1,
        )
        .expect("the gazetteer is recorded");
        if let Some(streets) = streets {
            let entries = geocoder_index_of("FR", communes, &streets, &ShopIndex::default());
            let bytes = geocoder_index::to_parquet("FR", &entries, &planet()).expect("the index is encoded");
            let part = Part::GeocoderIndex { country: "FR".into() };
            record(&directory, &planet(), part, "geocoder-FR.parquet", &bytes, 1)
                .expect("the index is recorded");
        }
        directory
    }

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
        written_dataset(name, &[sens()], Some(streets))
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
        let directory = written_dataset("communes-only", &[sens()], None);

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
        std::fs::write(directory.join("geocoder-FR.parquet"), "not a parquet file")
            .expect("the file is spoilt");

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
