//! A dataset opened for reading, wherever its files are: the manifest first, then the parts it lists.

use crate::directory::MANIFEST;
use crate::files::{ForReadingFiles, Unreachable};
use crate::manifest::{Entry, Manifest, ManifestError, Part};
use crate::tables::{self, Column, Footer, RowGroupBytes, TableError, Tail};
use crate::{gazetteer, geocoder_index};

/// A dataset whose manifest has been read.
pub struct Dataset<F: ForReadingFiles> {
    files: F,
    manifest: Manifest,
}

/// Why a dataset, or one of its parts, could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatasetError {
    /// A file could not be reached.
    Unreachable(Unreachable),
    /// The manifest could not be read.
    Manifest(ManifestError),
    /// A part could not be read as what the manifest says it is.
    Table {
        /// The file.
        file: String,
        /// What went wrong.
        source: TableError,
    },
    /// The manifest lists no gazetteer, and nothing can be settled without one.
    NoGazetteer,
}

impl std::fmt::Display for DatasetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(reason) => write!(formatter, "{reason}"),
            Self::Manifest(reason) => write!(formatter, "{reason}"),
            Self::Table { file, source } => write!(formatter, "{file}: {source}"),
            Self::NoGazetteer => write!(formatter, "the dataset lists no gazetteer"),
        }
    }
}

impl std::error::Error for DatasetError {}

impl<F: ForReadingFiles> Dataset<F> {
    /// The dataset whose files `files` reaches, once its manifest is read.
    pub fn open(files: F) -> Result<Self, DatasetError> {
        let text = whole(&files, MANIFEST)?;
        let manifest =
            Manifest::from_json(&String::from_utf8_lossy(&text)).map_err(DatasetError::Manifest)?;
        Ok(Self { files, manifest })
    }

    /// The same dataset keeping every range it reads of a part in `directory`, under the part's
    /// checksum, and answering from there the next time: the manifest is always read afresh, so a
    /// part republished under another checksum is never served stale.
    pub fn cached_in(self, directory: impl Into<std::path::PathBuf>) -> Self {
        let _ = directory.into();
        self
    }

    /// What the dataset holds and where it came from.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Every area of the gazetteer, read whole.
    pub fn gazetteer(&self) -> Result<Vec<gazetteer::Area>, DatasetError> {
        let entry = self.entry_of(&Part::Gazetteer).ok_or(DatasetError::NoGazetteer)?;
        self.read_whole(entry, gazetteer::from_parquet)
    }

    fn entry_of(&self, part: &Part) -> Option<&Entry> {
        self.manifest.files.iter().find(|entry| entry.part == *part)
    }

    /// A part read whole, at its published address when it has one, and parsed as what it is.
    fn read_whole<T>(
        &self,
        entry: &Entry,
        parse: fn(&[u8]) -> Result<T, TableError>,
    ) -> Result<T, DatasetError> {
        let address = entry.url.as_deref().unwrap_or(&entry.file);
        parse(&whole(&self.files, address)?).map_err(|source| DatasetError::Table {
            file: entry.file.clone(),
            source,
        })
    }
}

impl<F: ForReadingFiles> Dataset<F> {
    /// The entries of `country`'s geocoder index read whole, or nothing when the dataset does not
    /// index that country.
    pub fn geocoder_index(&self, country: &str) -> Result<Option<Vec<geocoder_index::Entry>>, DatasetError> {
        let wanted = Part::GeocoderIndex {
            country: country.to_string(),
        };
        self.entry_of(&wanted)
            .map(|entry| self.read_whole(entry, geocoder_index::from_parquet))
            .transpose()
    }
}

/// One country's geocoder index, its footer read: entries are fetched a row group at a time.
pub struct CountryIndex<'a, F: ForReadingFiles> {
    dataset: &'a Dataset<F>,
    entry: &'a Entry,
    footer: Footer,
}

/// How much of a file's end is read first in the hope of finding its whole footer there: a country's
/// footer of a few hundred kilobytes takes one more read, and a small one takes none.
const FIRST_TAIL: u64 = 64 * 1024;

impl<F: ForReadingFiles> Dataset<F> {
    /// `country`'s geocoder index with only its footer read, or nothing when the dataset does not
    /// index that country.
    pub fn country_index(&self, country: &str) -> Result<Option<CountryIndex<'_, F>>, DatasetError> {
        let wanted = Part::GeocoderIndex {
            country: country.to_string(),
        };
        self.entry_of(&wanted)
            .map(|entry| {
                self.footer_of(entry).map(|footer| CountryIndex {
                    dataset: self,
                    entry,
                    footer,
                })
            })
            .transpose()
    }

    /// A part's footer, read from its end in one read or, when it is longer than that, two.
    fn footer_of(&self, entry: &Entry) -> Result<Footer, DatasetError> {
        let address = entry.url.as_deref().unwrap_or(&entry.file);
        let length = self.files.length(address).map_err(DatasetError::Unreachable)?;
        let mut wanted = FIRST_TAIL.min(length);
        loop {
            let tail = self
                .files
                .read(address, length - wanted..length)
                .map_err(DatasetError::Unreachable)?;
            match tables::footer_from_tail(&tail, length).map_err(|source| table_error(entry, source))? {
                Tail::Footer(footer) => return Ok(footer),
                Tail::Short { needed } if needed > wanted && needed <= length => wanted = needed,
                Tail::Short { .. } => {
                    return Err(table_error(
                        entry,
                        TableError::Unreadable {
                            detail: "the footer runs past the start of the file".into(),
                        },
                    ));
                }
            }
        }
    }

    /// The columns of one row group of a part, fetched as one range.
    fn row_group_of(
        &self,
        entry: &Entry,
        footer: &Footer,
        group: &RowGroupBytes,
    ) -> Result<Vec<Column>, DatasetError> {
        let address = entry.url.as_deref().unwrap_or(&entry.file);
        let bytes = self
            .files
            .read(address, group.bytes.clone())
            .map_err(DatasetError::Unreachable)?;
        footer
            .read_row_group(group, bytes)
            .map_err(|source| table_error(entry, source))
    }
}

fn table_error(entry: &Entry, source: TableError) -> DatasetError {
    DatasetError::Table {
        file: entry.file.clone(),
        source,
    }
}

impl<F: ForReadingFiles> CountryIndex<'_, F> {
    /// How many street names the country counts and how many hold each word, read from their own
    /// row groups.
    pub fn word_counts(&self) -> Result<Vec<geocoder_index::Entry>, DatasetError> {
        self.entries_where(|group| group.highest == Some(0))
    }

    /// The entries of one commune, read from the row group holding it.
    pub fn commune(&self, relation: i64) -> Result<Vec<geocoder_index::Entry>, DatasetError> {
        let entries = self.entries_where(|group| {
            group.lowest.is_some_and(|lowest| lowest <= relation)
                && group.highest.is_some_and(|highest| relation <= highest)
        })?;
        Ok(entries
            .into_iter()
            .filter(|entry| geocoder_index::commune_of(entry) == Some(relation))
            .collect())
    }

    fn entries_where(
        &self,
        chosen: impl Fn(&RowGroupBytes) -> bool,
    ) -> Result<Vec<geocoder_index::Entry>, DatasetError> {
        let mut entries = Vec::new();
        for group in self
            .footer
            .row_groups("commune")
            .iter()
            .filter(|group| chosen(group))
        {
            let columns = self.dataset.row_group_of(self.entry, &self.footer, group)?;
            entries.extend(
                geocoder_index::entries_of(&columns).map_err(|source| table_error(self.entry, source))?,
            );
        }
        Ok(entries)
    }
}

/// A whole file, read as one range.
fn whole(files: &impl ForReadingFiles, file: &str) -> Result<Vec<u8>, DatasetError> {
    let length = files.length(file).map_err(DatasetError::Unreachable)?;
    files.read(file, 0..length).map_err(DatasetError::Unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Provenance;
    use crate::testing::FilesInMemory;

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

    fn sens() -> Vec<gazetteer::Area> {
        vec![gazetteer::Area {
            relation: 946_433,
            level: 8,
            country: Some("FR".into()),
            names: vec!["SENS".into()],
            postcodes: vec!["89100".into()],
            square_kilometres: Some(105.08),
            absorbed_by: None,
            centre: Some([3.282_606, 48.197_855_9]),
        }]
    }

    fn holding_sens() -> Result<FilesInMemory, TableError> {
        let gazetteer = gazetteer::to_parquet(&sens(), &planet())?;
        let manifest = Manifest {
            planet: planet(),
            files: vec![Entry {
                part: Part::Gazetteer,
                file: "gazetteer.parquet".into(),
                url: None,
                bytes: gazetteer.len() as u64,
                sha256: String::new(),
                rows: 1,
            }],
        };
        Ok(FilesInMemory::default()
            .with(MANIFEST, manifest.to_json().as_bytes())
            .with("gazetteer.parquet", &gazetteer))
    }

    #[test]
    fn given_a_dataset_when_opened_then_its_gazetteer_reads_back_whole()
    -> Result<(), Box<dyn std::error::Error>> {
        let dataset = Dataset::open(holding_sens()?)?;

        let areas = dataset.gazetteer()?;

        assert_eq!(areas, sens());
        Ok(())
    }

    fn holding_sens_and_its_streets() -> Result<FilesInMemory, TableError> {
        let index = geocoder_index::to_parquet("FR", &streets_of_sens(), &planet())?;
        let gazetteer = gazetteer::to_parquet(&sens(), &planet())?;
        let entry = |part: Part, file: &str, bytes: &[u8]| Entry {
            part,
            file: file.into(),
            url: None,
            bytes: bytes.len() as u64,
            sha256: String::new(),
            rows: 1,
        };
        let manifest = Manifest {
            planet: planet(),
            files: vec![
                entry(Part::Gazetteer, "gazetteer.parquet", &gazetteer),
                entry(
                    Part::GeocoderIndex { country: "FR".into() },
                    "geocoder-FR.parquet",
                    &index,
                ),
            ],
        };
        Ok(FilesInMemory::default()
            .with(MANIFEST, manifest.to_json().as_bytes())
            .with("gazetteer.parquet", &gazetteer)
            .with("geocoder-FR.parquet", &index))
    }

    fn streets_of_sens() -> Vec<geocoder_index::Entry> {
        vec![
            geocoder_index::Entry::Names { count: 1 },
            geocoder_index::Entry::Middle {
                commune: 946_433,
                street: "AVENUE GARE".into(),
                position: [3.268_208_2, 48.197_500_6],
            },
        ]
    }

    #[test]
    fn given_a_dataset_indexing_france_when_france_then_belgium_are_asked_then_only_france_comes_back()
    -> Result<(), Box<dyn std::error::Error>> {
        let dataset = Dataset::open(holding_sens_and_its_streets()?)?;

        let asked = (dataset.geocoder_index("FR")?, dataset.geocoder_index("BE")?);

        assert_eq!(asked, (Some(streets_of_sens()), None));
        Ok(())
    }

    fn street(commune: i64, name: &str) -> geocoder_index::Entry {
        geocoder_index::Entry::Middle {
            commune,
            street: name.into(),
            position: [3.29, 48.21],
        }
    }

    /// France's index in row groups of two entries: its counts, then communes 10, 20 and 30.
    fn france_in_small_row_groups() -> Vec<geocoder_index::Entry> {
        vec![
            geocoder_index::Entry::Names { count: 3 },
            geocoder_index::Entry::Word {
                word: "GARE".into(),
                count: 3,
            },
            street(10, "AVENUE GARE"),
            street(10, "RUE GARE"),
            street(20, "PLACE GARE"),
            street(30, "QUAI GARE"),
        ]
    }

    /// A dataset whose French index has every byte outside `kept` and its footer zeroed, so that a
    /// read reaching anywhere else fails.
    fn holding_france_with_only(kept: &[std::ops::Range<usize>]) -> Result<FilesInMemory, TableError> {
        let mut index = geocoder_index::cut_at("FR", &france_in_small_row_groups(), &planet(), 2)?;
        let length = index.len();
        let footer_length = u32::from_le_bytes([
            index[length - 8],
            index[length - 7],
            index[length - 6],
            index[length - 5],
        ]) as usize;
        let footer_start = length - 8 - footer_length;
        for (at, byte) in index.iter_mut().enumerate() {
            if at < footer_start && !kept.iter().any(|range| range.contains(&at)) {
                *byte = 0;
            }
        }
        let manifest = Manifest {
            planet: planet(),
            files: vec![Entry {
                part: Part::GeocoderIndex { country: "FR".into() },
                file: "geocoder-FR.parquet".into(),
                url: None,
                bytes: index.len() as u64,
                sha256: String::new(),
                rows: 6,
            }],
        };
        Ok(FilesInMemory::default()
            .with(MANIFEST, manifest.to_json().as_bytes())
            .with("geocoder-FR.parquet", &index))
    }

    /// Where each row group of the French index lies, by commune span.
    fn row_group_ranges() -> Result<Vec<(i64, std::ops::Range<usize>)>, TableError> {
        let index = geocoder_index::cut_at("FR", &france_in_small_row_groups(), &planet(), 2)?;
        Ok(crate::tables::row_group_bytes(&index, "commune")?
            .into_iter()
            .map(|(lowest, range)| (lowest, range.start as usize..range.end as usize))
            .collect())
    }

    #[test]
    fn given_a_countrys_index_when_its_word_counts_are_read_then_they_come_from_their_own_row_groups_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let counts: Vec<std::ops::Range<usize>> = row_group_ranges()?
            .into_iter()
            .filter(|(lowest, _)| *lowest == 0)
            .map(|(_, range)| range)
            .collect();
        let dataset = Dataset::open(holding_france_with_only(&counts)?)?;
        let france = dataset.country_index("FR")?.ok_or("France is indexed")?;

        let read = france.word_counts()?;

        assert_eq!(read, france_in_small_row_groups()[..2].to_vec());
        Ok(())
    }

    #[test]
    fn given_a_countrys_index_when_one_commune_is_read_then_its_entries_come_from_its_row_group_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let commune: Vec<std::ops::Range<usize>> = row_group_ranges()?
            .into_iter()
            .filter(|(lowest, _)| *lowest == 20)
            .map(|(_, range)| range)
            .collect();
        let dataset = Dataset::open(holding_france_with_only(&commune)?)?;
        let france = dataset.country_index("FR")?.ok_or("France is indexed")?;

        let read = france.commune(20)?;

        assert_eq!(read, vec![street(20, "PLACE GARE")]);
        Ok(())
    }
}
