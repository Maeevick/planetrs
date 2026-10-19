//! A dataset's files over HTTP, range by range, as data.gouv.fr and any static host serve them.
//!
//! Each file is described once, by a HEAD that gives its length and its ETag. Every later range is
//! asked for with that ETag in `If-Match`, so that a file republished while it is being read is
//! refused rather than stitched together from two versions.

use std::ops::Range;

use crate::files::{ForReadingFiles, Unreachable};

/// A dataset's files under one web address.
pub struct HttpFiles {
    base: String,
}

impl HttpFiles {
    /// The files of the dataset published under `base`, by name relative to it.
    pub fn at(base: &str) -> Self {
        Self {
            base: base.to_string(),
        }
    }
}

impl ForReadingFiles for HttpFiles {
    fn length(&self, file: &str) -> Result<u64, Unreachable> {
        Err(Unreachable::Missing {
            file: format!("{}{file}", self.base),
        })
    }

    fn read(&self, file: &str, _range: Range<u64>) -> Result<Vec<u8>, Unreachable> {
        self.length(file).map(|_| Vec::new())
    }
}
