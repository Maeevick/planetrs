//! A dataset opened for reading, wherever its files are: the manifest first, then the parts it lists.

use crate::directory::MANIFEST;
use crate::files::{ForReadingFiles, Unreachable};
use crate::gazetteer;
use crate::manifest::{Manifest, ManifestError};
use crate::tables::TableError;

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
        let _ = MANIFEST;
        Err(DatasetError::Unreachable(Unreachable::Missing {
            file: "manifest.json".into(),
        }))
        .map(|manifest| Self { files, manifest })
    }

    /// What the dataset holds and where it came from.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Every area of the gazetteer, read whole.
    pub fn gazetteer(&self) -> Result<Vec<gazetteer::Area>, DatasetError> {
        let _ = &self.files;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Entry, Part, Provenance};
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
}
