//! What a dataset holds and where it came from: the one file read before any other.
//!
//! The manifest names the planet every file was built from and lists each file with its size and
//! checksum, so a reader can tell a whole dataset from a mixed one and a copy from a corrupted one.

use std::path::Path;

use planetrs_pbf::Planet;
use serde::{Deserialize, Serialize};

/// The format a manifest is written in, so that a reader can tell it from anything else.
pub const FORMAT: &str = "planetrs dataset 1";

/// The licence every file of a dataset is under, by its SPDX identifier.
pub const LICENCE: &str = "ODbL-1.0";

/// Whom the data must be credited to, and where the terms are explained.
pub const ATTRIBUTION: &str = "© OpenStreetMap contributors, https://www.openstreetmap.org/copyright";

/// The planet a dataset was built from, as precisely as a planet file names itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The planet file's name, without its directory.
    pub source_name: String,
    /// Its size in bytes.
    pub file_size: u64,
    /// How many blobs it holds.
    pub blob_count: u64,
    /// The minute of OpenStreetMap history it is a snapshot of, in seconds since the epoch.
    pub replication_timestamp: i64,
    /// The replication sequence number of that minute.
    pub replication_sequence: i64,
    /// The program that wrote the planet file.
    pub writing_program: String,
}

impl Provenance {
    /// The provenance of the planet file at `path`, as every build of a part states it, so that the
    /// parts two tools build from one planet name the same planet.
    pub fn of(path: &Path, planet: &Planet) -> Self {
        Self {
            source_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            file_size: planet.file_size,
            blob_count: planet.blobs.len() as u64,
            replication_timestamp: planet.header.replication_timestamp.unwrap_or_default(),
            replication_sequence: planet.header.replication_sequence.unwrap_or_default(),
            writing_program: planet.header.writing_program.clone().unwrap_or_default(),
        }
    }
}

/// Which part of the dataset a file is, ordered as a manifest lists them: the gazetteer first, then
/// the geocoder indexes, then the map indexes, each by country.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "part", rename_all = "kebab-case")]
pub enum Part {
    /// The planet-wide gazetteer, read whole.
    Gazetteer,
    /// The geocoder index of one country.
    GeocoderIndex {
        /// The country's ISO 3166-1 code.
        country: String,
    },
    /// The map index of one country.
    MapIndex {
        /// The country's ISO 3166-1 code.
        country: String,
    },
}

/// One file of the dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// What the file is.
    #[serde(flatten)]
    pub part: Part,
    /// Its name, relative to the manifest.
    pub file: String,
    /// Where it is published, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256, in lower-case hexadecimal.
    pub sha256: String,
    /// How many rows it holds.
    pub rows: u64,
}

/// The table of contents of a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The planet every file was built from.
    pub planet: Provenance,
    /// Every file, in the order they were written.
    pub files: Vec<Entry>,
}

/// Why a manifest could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestError {
    /// The text is not a planetrs manifest.
    NotAManifest {
        /// What the JSON reader said.
        detail: String,
    },
    /// The manifest is written in a format this version does not read.
    UnknownFormat {
        /// The format it names.
        found: String,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAManifest { detail } => write!(formatter, "not a planetrs dataset manifest: {detail}"),
            Self::UnknownFormat { found } => {
                write!(
                    formatter,
                    "the manifest is in the format {found:?}, and this version reads {FORMAT:?}"
                )
            }
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// The manifest as the JSON file a dataset opens with, naming its format, licence and credit.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Written {
            format: FORMAT,
            licence: LICENCE,
            attribution: ATTRIBUTION,
            manifest: self,
        })
        .unwrap_or_default()
    }

    /// A manifest read from its JSON file.
    pub fn from_json(text: &str) -> Result<Self, ManifestError> {
        let read: Read = serde_json::from_str(text).map_err(|error| ManifestError::NotAManifest {
            detail: error.to_string(),
        })?;
        if read.format != FORMAT {
            return Err(ManifestError::UnknownFormat { found: read.format });
        }
        Ok(read.manifest)
    }
}

/// The `planetrs` key-value pair every file of a dataset carries: which file it is, the planet it
/// came from, the licence and whom to credit, so that a file copied out of its dataset still says.
pub(crate) fn notice(file: &str, planet: &Provenance) -> (&'static str, String) {
    let notice = serde_json::json!({
        "file": file,
        "format": FORMAT,
        "planet": planet,
        "licence": LICENCE,
        "attribution": ATTRIBUTION,
    });
    ("planetrs", notice.to_string())
}

/// The manifest as written: its fields after the format, the licence and the credit.
#[derive(Serialize)]
struct Written<'a> {
    format: &'a str,
    licence: &'a str,
    attribution: &'a str,
    #[serde(flatten)]
    manifest: &'a Manifest,
}

/// The manifest as read: the licence and the credit are the format's, so only the format is kept.
#[derive(Deserialize)]
struct Read {
    format: String,
    #[serde(flatten)]
    manifest: Manifest,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monaco() -> Manifest {
        Manifest {
            planet: Provenance {
                source_name: "planet-260907.osm.pbf".into(),
                file_size: 94_746_421_723,
                blob_count: 52_810,
                replication_timestamp: 1_788_739_204,
                replication_sequence: 4_321,
                writing_program: "planet-dump-ng 1.2.7".into(),
            },
            files: vec![
                Entry {
                    part: Part::Gazetteer,
                    file: "gazetteer.parquet".into(),
                    url: None,
                    bytes: 9_812_345,
                    sha256: "3f".repeat(32),
                    rows: 750_027,
                },
                Entry {
                    part: Part::GeocoderIndex { country: "MC".into() },
                    file: "geocoder-MC.parquet".into(),
                    url: Some("https://static.data.gouv.fr/resources/planetrs/geocoder-MC.parquet".into()),
                    bytes: 61_234,
                    sha256: "a0".repeat(32),
                    rows: 2_345,
                },
            ],
        }
    }

    #[test]
    fn given_a_manifest_when_written_as_json_and_read_back_then_it_is_the_same_manifest() {
        let manifest = monaco();

        let read_back = Manifest::from_json(&manifest.to_json());

        assert_eq!(read_back.ok(), Some(manifest));
    }

    #[test]
    fn given_a_manifest_when_written_then_it_names_its_format_the_odbl_and_whom_to_credit()
    -> Result<(), serde_json::Error> {
        let written: serde_json::Value = serde_json::from_str(&monaco().to_json())?;

        assert_eq!(written["format"], "planetrs dataset 1");
        assert_eq!(written["licence"], "ODbL-1.0");
        assert_eq!(
            written["attribution"],
            "© OpenStreetMap contributors, https://www.openstreetmap.org/copyright"
        );
        Ok(())
    }

    #[test]
    fn given_the_manifest_of_another_format_when_read_then_it_is_refused_with_the_format_it_names() {
        let foreign = monaco().to_json().replace(FORMAT, "planetrs dataset 2");

        let read = Manifest::from_json(&foreign);

        assert_eq!(
            read,
            Err(ManifestError::UnknownFormat {
                found: "planetrs dataset 2".into()
            })
        );
    }
}
