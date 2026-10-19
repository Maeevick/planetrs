//! What can go wrong reading a PBF file, and where.
//!
//! The variants separate the two outcomes a caller acts on differently: an I/O failure, which may
//! be transient, and a malformed file, which never is. Both carry the blob offset when one is
//! known, because the only useful next step on a 94 GB file is to inspect the blob that failed.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum PbfError {
    Io(io::Error),
    Truncated,
    VarintTooLong,
    UnknownWireType(u8),
    HeaderTooLarge { length: u32, limit: u32 },
    PayloadTooLarge { size: usize, limit: usize },
    InflatedTooLarge { size: usize, limit: usize },
    MissingDataSize,
    EmptyBlob,
    Inflate(String),
    UnsortedSections,
    AtBlob { offset: u64, source: Box<PbfError> },
}

impl PbfError {
    /// The same failure, tagged with the blob it happened in.
    ///
    /// Applied at the one place that knows the offset, so every layer below stays free of it.
    pub fn at_blob(self, offset: u64) -> Self {
        match self {
            already @ Self::AtBlob { .. } => already,
            source => Self::AtBlob {
                offset,
                source: Box::new(source),
            },
        }
    }
}

impl fmt::Display for PbfError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(out, "{error}"),
            Self::Truncated => write!(out, "the message ends mid-field"),
            Self::VarintTooLong => write!(out, "a varint runs past 64 bits"),
            Self::UnknownWireType(kind) => write!(out, "wire type {kind} is not one protobuf defines"),
            Self::HeaderTooLarge { length, limit } => {
                write!(
                    out,
                    "a blob header claims {length} bytes, past the {limit} the spec allows"
                )
            }
            Self::PayloadTooLarge { size, limit } => {
                write!(
                    out,
                    "a blob claims {size} bytes, past the {limit} the spec allows"
                )
            }
            Self::InflatedTooLarge { size, limit } => {
                write!(
                    out,
                    "a blob claims it inflates to {size} bytes, past the {limit} the spec allows"
                )
            }
            Self::MissingDataSize => write!(out, "a blob header states no payload size"),
            Self::EmptyBlob => write!(out, "a blob holds neither raw nor compressed bytes"),
            Self::Inflate(detail) => write!(out, "the zlib stream did not inflate: {detail}"),
            Self::UnsortedSections => {
                write!(
                    out,
                    "the file does not declare Sort.Type_then_ID, so its sections cannot be bisected"
                )
            }
            Self::AtBlob { offset, .. } => write!(out, "at blob offset {offset}"),
        }
    }
}

impl std::error::Error for PbfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => error.source(),
            Self::AtBlob { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for PbfError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_failure_is_tagged_with_its_blob_then_the_offset_leads_the_message() {
        let tagged = PbfError::Truncated.at_blob(94_746_421);

        assert_eq!(
            joined(&tagged),
            "at blob offset 94746421: the message ends mid-field"
        );
    }

    /// Tagging twice would bury the offset that was actually observed under the outer one.
    #[test]
    fn when_a_tagged_failure_is_tagged_again_then_the_first_offset_survives() {
        let tagged = PbfError::Truncated.at_blob(1000).at_blob(2000);

        assert_eq!(joined(&tagged), "at blob offset 1000: the message ends mid-field");
    }

    #[test]
    fn when_a_failure_is_tagged_then_the_underlying_cause_stays_reachable() {
        let tagged = PbfError::Truncated.at_blob(64);

        assert!(std::error::Error::source(&tagged).is_some());
    }

    /// The failure and its causes, joined as a command line prints them.
    fn joined(error: &dyn std::error::Error) -> String {
        let mut text = error.to_string();
        let mut cause = error.source();
        while let Some(source) = cause {
            text.push_str(&format!(": {source}"));
            cause = source.source();
        }
        text
    }

    #[test]
    fn given_a_failure_at_a_blob_when_joined_with_its_causes_then_each_is_said_once() {
        let failed = PbfError::Io(io::Error::other("the disk went away")).at_blob(4_096);

        assert_eq!(joined(&failed), "at blob offset 4096: the disk went away");
    }
}
