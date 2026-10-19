//! Where a dataset's files are read from: a directory, a web server, or memory in a test.
//!
//! A dataset never needs a whole file at once. It asks for a file's length, then for the ranges it
//! wants, so that the same reader serves a local directory and a remote one over HTTP range
//! requests without knowing which.

use std::ops::Range;

/// What a dataset needs from wherever its files are: a file's length and any range of its bytes.
pub trait ForReadingFiles {
    /// The length of `file`, in bytes.
    fn length(&self, file: &str) -> Result<u64, Unreachable>;

    /// The bytes of `file` over `range`, exactly that many or an error.
    fn read(&self, file: &str, range: Range<u64>) -> Result<Vec<u8>, Unreachable>;
}

/// Why a file, or a range of it, could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unreachable {
    /// There is no such file.
    Missing {
        /// The file asked for.
        file: String,
    },
    /// The range runs past the file's end.
    PastTheEnd {
        /// The file asked for.
        file: String,
        /// The range asked for.
        range: Range<u64>,
        /// The file's length.
        length: u64,
    },
    /// The file could not be read, for a reason outside the dataset.
    Failed {
        /// The file asked for.
        file: String,
        /// What went wrong.
        detail: String,
    },
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { file } => write!(formatter, "the dataset has no file {file}"),
            Self::PastTheEnd { file, range, length } => write!(
                formatter,
                "bytes {}..{} of {file} were asked for, and it holds {length}",
                range.start, range.end
            ),
            Self::Failed { file, detail } => write!(formatter, "reading {file} failed: {detail}"),
        }
    }
}

impl std::error::Error for Unreachable {}
