//! Where a dataset is: a directory on this machine, or a web address it is published under.

use std::path::PathBuf;

use crate::dataset::DatasetError;
use crate::files::ForReadingFiles;

/// Where a dataset is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A directory on this machine.
    Directory(PathBuf),
    /// A web address the dataset is published under, read over HTTP range requests.
    Address(String),
}

impl Location {
    /// The location a command line or a setting names: a web address when it starts with
    /// `http://` or `https://`, a directory otherwise.
    pub fn of(named: &str) -> Self {
        let _ = named;
        Self::Directory(PathBuf::new())
    }

    /// The files of the dataset there, read the way the location calls for.
    pub fn files(&self) -> Result<Box<dyn ForReadingFiles>, DatasetError> {
        Err(DatasetError::NoGazetteer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_names_of_dataset_locations_when_read_then_web_addresses_and_directories_are_told_apart() {
        let read = [
            "https://static.data.gouv.fr/resources/planetrs/",
            "http://127.0.0.1:8080",
            "/Users/someone/.local/share/planetrs/dataset",
            "dataset",
        ]
        .map(Location::of);

        assert_eq!(
            read,
            [
                Location::Address("https://static.data.gouv.fr/resources/planetrs/".into()),
                Location::Address("http://127.0.0.1:8080".into()),
                Location::Directory("/Users/someone/.local/share/planetrs/dataset".into()),
                Location::Directory("dataset".into()),
            ]
        );
    }
}
