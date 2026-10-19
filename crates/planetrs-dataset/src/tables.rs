//! Tables of plain values in and out of Parquet, the one place the `parquet` and `arrow` crates are
//! named, so that swapping them touches nothing else.

/// Why a table could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableError {
    /// The table could not be written as Parquet.
    Unwritable {
        /// What the Parquet writer said.
        detail: String,
    },
    /// The bytes could not be read as the table expected.
    Unreadable {
        /// What went wrong.
        detail: String,
    },
}

impl std::fmt::Display for TableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unwritable { detail } => write!(formatter, "the table could not be written: {detail}"),
            Self::Unreadable { detail } => write!(formatter, "the table could not be read: {detail}"),
        }
    }
}

impl std::error::Error for TableError {}
