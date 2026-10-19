//! A dataset's files held in memory, and the contract every way of reading files must honour.
//!
//! The fake stands in for a directory or a web server in the tests of whatever reads a dataset, and
//! the contract is run against it and against every real adapter, so the fake cannot drift from
//! what the adapters do.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::files::{ForReadingFiles, Unreachable};

/// A dataset's files held in memory, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilesInMemory {
    files: BTreeMap<String, Vec<u8>>,
}

impl FilesInMemory {
    /// The same files, with one more.
    pub fn with(mut self, name: &str, bytes: &[u8]) -> Self {
        self.files.insert(name.to_string(), bytes.to_vec());
        self
    }
}

impl ForReadingFiles for FilesInMemory {
    fn length(&self, file: &str) -> Result<u64, Unreachable> {
        Err(Unreachable::Missing {
            file: file.to_string(),
        })
    }

    fn read(&self, file: &str, _range: Range<u64>) -> Result<Vec<u8>, Unreachable> {
        Err(Unreachable::Missing {
            file: file.to_string(),
        })
    }
}

/// What every way of reading files must do, checked against one built to hold `holding`.
///
/// Panics, as a test does, at the first behaviour the files do not honour.
pub fn honours_for_reading_files<F: ForReadingFiles>(holding: impl Fn(&[(&str, &[u8])]) -> F) {
    let manifest: &[u8] = br#"{"format":"planetrs dataset 1"}"#;
    let files = holding(&[("manifest.json", manifest), ("empty.parquet", b"")]);

    assert_eq!(
        files.length("manifest.json"),
        Ok(manifest.len() as u64),
        "a file's length is its count of bytes"
    );
    assert_eq!(
        files.read("manifest.json", 2..10),
        Ok(manifest[2..10].to_vec()),
        "a range reads exactly its bytes"
    );
    assert_eq!(
        files.read("manifest.json", 20..manifest.len() as u64),
        Ok(manifest[20..].to_vec()),
        "a range ending at the end reads the tail"
    );
    assert_eq!(
        files.length("empty.parquet"),
        Ok(0),
        "an empty file has length zero"
    );
    assert_eq!(
        files.read("manifest.json", 20..40),
        Err(Unreachable::PastTheEnd {
            file: "manifest.json".into(),
            range: 20..40,
            length: manifest.len() as u64,
        }),
        "a range past the end is refused rather than shortened"
    );
    assert_eq!(
        files.length("gazetteer.parquet"),
        Err(Unreachable::Missing {
            file: "gazetteer.parquet".into()
        }),
        "a file the dataset lacks is missing"
    );
    assert_eq!(
        files.read("gazetteer.parquet", 0..4),
        Err(Unreachable::Missing {
            file: "gazetteer.parquet".into()
        }),
        "a range of a file the dataset lacks is missing too"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_files_held_in_memory_when_read_then_they_honour_the_contract() {
        honours_for_reading_files(|holding| {
            holding
                .iter()
                .fold(FilesInMemory::default(), |files, (name, bytes)| {
                    files.with(name, bytes)
                })
        });
    }
}
