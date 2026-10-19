//! A dataset opened for lookups, which is all a caller needs once the dataset is built.
//!
//! Opening reads the gazetteer and nothing else. A country's word counts are read the first time a
//! site in it is looked up, and a commune's streets and shops the first time a lookup needs them,
//! with the communes stored beside it in the same row group, so that a file of sites in one region
//! reads the region once.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{
    CountryIndex, Dataset, DatasetError, DirectoryFiles, ForReadingFiles, Part, Unreachable, geocoder_index,
};
use thiserror::Error;

use crate::gazetteer::{Gazetteer, MUNICIPALITY_LEVELS};
use crate::locate::{Answer, Site, locate, settle};
use crate::parts::{indexes_of, settling_gazetteer};
use crate::rarity::WordRarity;
use crate::shops::{ShopIndex, metres_between};
use crate::street_index::StreetIndex;
use crate::trace::trace;

/// A dataset opened for lookups: its gazetteer, and of each country what lookups have read so far.
pub struct Geocoder<F: ForReadingFiles> {
    dataset: Dataset<F>,
    gazetteer: Gazetteer,
    countries: HashMap<String, Option<Read>>,
    nowhere: (StreetIndex, ShopIndex),
}

/// A country's index as far as lookups have read it: its word counts, and the streets and shops of
/// the communes read so far.
struct Read {
    index: CountryIndex,
    streets: StreetIndex,
    shops: ShopIndex,
    communes: Vec<RangeInclusive<i64>>,
}

/// Why a dataset could not be opened.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OpenError {
    /// The directory holds no dataset: it has no manifest.
    #[error("no index in {}", directory.display())]
    NoIndex {
        /// The directory that was searched.
        directory: PathBuf,
    },
    /// A dataset is there but its manifest or its gazetteer could not be read.
    #[error("the dataset could not be read")]
    Unreadable(#[source] DatasetError),
}

/// Why a lookup could not be answered: a part of the dataset it needed could not be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocateError {
    /// What failed, and in which file.
    #[error("reading the dataset failed")]
    Unreadable(#[source] DatasetError),
}

impl Geocoder<DirectoryFiles> {
    /// Opens the dataset in `directory`.
    ///
    /// Only the gazetteer is required. A dataset indexing no country opens all the same, and its
    /// lookups stop at the commune.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, OpenError> {
        let directory = directory.as_ref();
        let dataset = Dataset::open(DirectoryFiles::at(directory)).map_err(|source| match source {
            DatasetError::Unreachable(Unreachable::Missing { file }) if file == MANIFEST => {
                OpenError::NoIndex {
                    directory: directory.to_path_buf(),
                }
            }
            other => OpenError::Unreadable(other),
        })?;
        Self::over(dataset)
    }
}

impl<F: ForReadingFiles> Geocoder<F> {
    /// A geocoder over an opened dataset, wherever its files are: its gazetteer is read now, and
    /// every other part when a lookup first needs it.
    pub fn over(dataset: Dataset<F>) -> Result<Self, OpenError> {
        let gazetteer = settling_gazetteer(dataset.gazetteer().map_err(OpenError::Unreadable)?);
        Ok(Self {
            dataset,
            gazetteer,
            countries: HashMap::new(),
            nowhere: (StreetIndex::default(), ShopIndex::default()),
        })
    }

    /// Where the site is and how precisely, or nothing when not even its commune could be settled.
    ///
    /// An answer at a commune's centre is still an answer: [`crate::refusal_of`] says whether a
    /// person should look at it before it is used.
    pub fn locate(&mut self, site: &Site) -> Result<Option<Answer>, LocateError> {
        let country = self.read_what(site)?;
        let (streets, shops) = self.read_of(country.as_deref());
        Ok(locate(&self.gazetteer, streets, shops, site))
    }

    /// Every decision [`Geocoder::locate`] takes for the site, one line each, ending with its answer.
    ///
    /// Its streets of the same name within 20 km come from every country the dataset indexes, as a
    /// border is no reason for a street a few hundred metres away to go unmentioned.
    pub fn trace(&mut self, site: &Site) -> Result<Vec<String>, LocateError> {
        let country = self.read_what(site)?;
        let searched = settle(&self.gazetteer, site)
            .map(|settled| settled.searched())
            .unwrap_or_default();
        let neighbours = self.read_neighbours(site)?;
        let (streets, shops) = self.read_of(country.as_deref());
        let mut seen = streets.only(searched.iter().copied());
        for (code, commune) in &neighbours {
            let (theirs, _) = self.read_of(Some(code));
            seen.absorb(theirs.only([*commune]));
        }
        Ok(trace(&self.gazetteer, &seen, shops, site))
    }

    /// Reads every commune of an indexed country close enough to the site's for one of its streets
    /// to lie within 20 km of it, and names them with their country.
    fn read_neighbours(&mut self, site: &Site) -> Result<Vec<(String, i64)>, LocateError> {
        let Some(settled) = settle(&self.gazetteer, site) else {
            return Ok(Vec::new());
        };
        let origin = settled.located.position;
        let indexed: Vec<String> = self
            .dataset
            .manifest()
            .files
            .iter()
            .filter_map(|entry| match &entry.part {
                Part::GeocoderIndex { country } => Some(country.clone()),
                _ => None,
            })
            .collect();
        let mut reaches = HashMap::new();
        for code in &indexed {
            self.read_country(code)?;
            if let Some(Some(read)) = self.countries.get(code) {
                reaches.insert(code.clone(), read.index.reach_metres());
            }
        }
        let near: Vec<(String, i64)> = self
            .gazetteer
            .communes()
            .iter()
            .filter(|commune| MUNICIPALITY_LEVELS.contains(&commune.level))
            .filter_map(|commune| {
                let code = commune.country.as_ref()?;
                let reach = reaches.get(code)?;
                let centre = commune.centre?;
                (metres_between(centre, origin) < WITHIN_TWENTY_KILOMETRES + reach + ROUNDING_ALLOWANCE)
                    .then(|| (code.clone(), commune.relation))
            })
            .collect();
        for (code, commune) in &near {
            self.read_commune(code, *commune)?;
        }
        Ok(near)
    }

    /// Reads a country's footer and word counts, once.
    fn read_country(&mut self, code: &str) -> Result<(), LocateError> {
        if !self.countries.contains_key(code) {
            let first = first_read(&self.dataset, code).map_err(LocateError::Unreadable)?;
            self.countries.insert(code.to_string(), first);
        }
        Ok(())
    }

    /// Reads a commune of a country, with its row group's neighbours, once.
    fn read_commune(&mut self, code: &str, commune: i64) -> Result<(), LocateError> {
        let Some(Some(read)) = self.countries.get_mut(code) else {
            return Ok(());
        };
        if !read.communes.iter().any(|span| span.contains(&commune)) {
            let found = self
                .dataset
                .communes_with(&read.index, commune)
                .map_err(LocateError::Unreadable)?;
            let (streets, shops) = indexes_of(found.entries);
            read.streets.absorb(streets);
            read.shops.absorb(shops);
            read.communes.extend(found.spans);
            read.communes.push(commune..=commune);
        }
        Ok(())
    }

    /// Reads whatever of the site's country a lookup of it needs and is not read yet, and names the
    /// country, or nothing when the site names none the dataset indexes.
    fn read_what(&mut self, site: &Site) -> Result<Option<String>, LocateError> {
        let Some(code) = self
            .gazetteer
            .country_code_for(&site.without_parentheses().country)
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let communes = settle(&self.gazetteer, site)
            .map(|settled| settled.searched())
            .unwrap_or_default();
        self.read_country(&code)?;
        for commune in communes {
            self.read_commune(&code, commune)?;
        }
        Ok(Some(code))
    }

    /// The streets and shops read of a country, or none.
    fn read_of(&self, country: Option<&str>) -> (&StreetIndex, &ShopIndex) {
        country
            .and_then(|code| self.countries.get(code))
            .and_then(Option::as_ref)
            .map_or((&self.nowhere.0, &self.nowhere.1), |read| {
                (&read.streets, &read.shops)
            })
    }
}

/// How far from the settled commune a trace looks for streets of the asked name.
const WITHIN_TWENTY_KILOMETRES: f64 = 20_000.0;

/// A metre's slack on the reach, so that rounding in two great-circle distances never leaves out a
/// commune whose street lies right at the edge.
const ROUNDING_ALLOWANCE: f64 = 1.0;

/// A country's index before any commune of it is read: its footer and its word counts, or nothing
/// when the dataset does not index it.
fn first_read<F: ForReadingFiles>(dataset: &Dataset<F>, country: &str) -> Result<Option<Read>, DatasetError> {
    let Some(index) = dataset.country_index(country)? else {
        return Ok(None);
    };
    let mut names = 0;
    let mut words = Vec::new();
    for entry in dataset.word_counts(&index)? {
        match entry {
            geocoder_index::Entry::Names { count } => names = count,
            geocoder_index::Entry::Word { word, count } => {
                words.push((word, u32::try_from(count).unwrap_or(u32::MAX)))
            }
            _ => {}
        }
    }
    Ok(Some(Read {
        index,
        streets: StreetIndex::default().with_rarity(WordRarity::counted(names, words)),
        shops: ShopIndex::default(),
        communes: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use planetrs_dataset::directory::record;
    use planetrs_dataset::{Part, Provenance, gazetteer, geocoder_index};

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
            let bytes =
                geocoder_index::to_parquet("FR", &entries, 0.0, &planet()).expect("the index is encoded");
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
            .expect("the index reads")
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
            .expect("the index reads")
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
    fn given_a_countrys_index_that_does_not_parse_when_a_site_in_it_is_looked_up_then_the_lookup_is_unreadable()
     {
        let directory = written_index("malformed");
        let spoilt = vec![
            0u8;
            std::fs::metadata(directory.join("geocoder-FR.parquet"))
                .map_or(0, |file| file.len() as usize)
        ];
        std::fs::write(directory.join("geocoder-FR.parquet"), spoilt).expect("the file is spoilt");
        let mut geocoder = Geocoder::open(&directory).expect("the gazetteer still opens");

        let looked_up = geocoder.locate(&site("8 Route des Tilleuls"));

        assert!(
            matches!(looked_up, Err(LocateError::Unreadable(_))),
            "{looked_up:?}"
        );
    }

    #[test]
    fn given_an_opened_index_when_a_site_is_traced_then_the_last_line_is_its_answer() {
        let directory = written_index("trace");

        let written = Geocoder::open(&directory)
            .expect("the index opens")
            .trace(&site("8 Route des Tilleuls"))
            .expect("the index reads");

        assert!(
            written.last().is_some_and(|line| line.contains("HouseNumber")),
            "{written:?}"
        );
    }

    #[test]
    fn given_a_street_named_as_asked_across_a_border_when_a_site_is_traced_then_the_trace_names_it_in_the_other_country()
     {
        let directory = scratch("across-the-border");
        let monaco = Country {
            code: "MC".into(),
            names: vec!["MONACO".into()],
        };
        let beyond = Commune {
            relation: 2,
            level: 8,
            names: vec!["LA CONDAMINE".into()],
            postcodes: vec!["98000".into()],
            centre: Some([3.30, 48.20]),
            country: Some("MC".into()),
        };
        let communes = [sens(), beyond];
        let relations = HashMap::from([("FR".to_string(), 2_202_162), ("MC".to_string(), 1_124_039)]);
        let areas = gazetteer_of(
            &communes,
            &[france(), monaco],
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
            2,
        )
        .expect("recorded");
        let street = |commune, name: &str| {
            (
                commune,
                Street {
                    name: name.into(),
                    addresses: vec![],
                    path: vec![[3.30, 48.20]],
                },
            )
        };
        for (country, filed) in [
            ("FR", street(1, "ROUTE TILLEULS")),
            ("MC", street(2, "RUE INCONNUE")),
        ] {
            let entries = geocoder_index_of(
                country,
                &communes,
                &StreetIndex::of(vec![filed]),
                &ShopIndex::default(),
            );
            let reach = crate::parts::reach_of(&entries, &communes);
            let bytes = geocoder_index::to_parquet(country, &entries, reach, &planet()).expect("encoded");
            let part = Part::GeocoderIndex {
                country: country.into(),
            };
            record(
                &directory,
                &planet(),
                part,
                &format!("geocoder-{country}.parquet"),
                &bytes,
                1,
            )
            .expect("recorded");
        }
        let mut geocoder = Geocoder::open(&directory).expect("the dataset opens");

        let written = geocoder
            .trace(&site("12 Rue Inconnue"))
            .expect("the dataset reads");

        assert!(
            written.iter().any(
                |line| line.contains("same street name in other communes within 20 km")
                    && line.contains("LA CONDAMINE")
            ),
            "{written:#?}"
        );
    }
}
