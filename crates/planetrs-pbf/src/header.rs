//! The file's first blob, which declares what the rest of it may be assumed to be.
//!
//! Two of its fields decide whether this crate can read the file at all. `DenseNodes` says the
//! nodes are in parallel delta-coded arrays rather than one message each. `Sort.Type_then_ID` says
//! every node precedes every way, and that ids ascend within a section, which is what makes both
//! the section bisection and the id pruning sound. Without that declaration neither is, and this is
//! where that is checked rather than assumed.

use crate::error::PbfError;
use crate::proto::{Reader, Value, zigzag};

pub const DENSE_NODES: &str = "DenseNodes";
pub const SORT_TYPE_THEN_ID: &str = "Sort.Type_then_ID";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileHeader {
    pub required_features: Vec<String>,
    pub optional_features: Vec<String>,
    pub writing_program: Option<String>,
    pub source: Option<String>,
    /// Seconds since the epoch, naming the minute of OSM history this file is a snapshot of.
    pub replication_timestamp: Option<i64>,
    pub replication_sequence: Option<i64>,
    pub replication_base_url: Option<String>,
    /// West, south, east, north, in nanodegrees.
    pub bounding_box: Option<(i64, i64, i64, i64)>,
}

impl FileHeader {
    /// Whether entities are sorted by type and then by id, which both the section bisection and the
    /// id pruning depend on for correctness.
    pub fn is_sorted_by_type_then_id(&self) -> bool {
        self.optional_features
            .iter()
            .any(|feature| feature == SORT_TYPE_THEN_ID)
    }

    pub fn requires_dense_nodes(&self) -> bool {
        self.required_features
            .iter()
            .any(|feature| feature == DENSE_NODES)
    }

    /// The features this crate does not know how to honour.
    ///
    /// A required feature is a promise the reader must keep, so meeting an unknown one means the
    /// file may decode into something plausible and wrong rather than into an error.
    pub fn unsupported_requirements(&self) -> Vec<&str> {
        self.required_features
            .iter()
            .map(String::as_str)
            .filter(|feature| !matches!(*feature, DENSE_NODES | "OsmSchema-V0.6"))
            .collect()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn file_header(inflated: &[u8]) -> Result<FileHeader, PbfError> {
    let mut reader = Reader::new(inflated);
    let mut header = FileHeader::default();

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(bytes)) => header.bounding_box = bounding_box(bytes)?,
            (4, Value::Bytes(bytes)) => header.required_features.push(text(bytes)),
            (5, Value::Bytes(bytes)) => header.optional_features.push(text(bytes)),
            (16, Value::Bytes(bytes)) => header.writing_program = Some(text(bytes)),
            (17, Value::Bytes(bytes)) => header.source = Some(text(bytes)),
            (32, Value::Varint(value)) => header.replication_timestamp = Some(value as i64),
            (33, Value::Varint(value)) => header.replication_sequence = Some(value as i64),
            (34, Value::Bytes(bytes)) => header.replication_base_url = Some(text(bytes)),
            _ => {}
        }
    }

    Ok(header)
}

fn bounding_box(bytes: &[u8]) -> Result<Option<(i64, i64, i64, i64)>, PbfError> {
    let mut reader = Reader::new(bytes);
    let mut edges = [None; 4];

    while let Some(field) = reader.next_field()? {
        if let (number @ 1..=4, Value::Varint(value)) = (field.number, field.value) {
            edges[number as usize - 1] = Some(zigzag(value));
        }
    }

    Ok(match edges {
        [Some(left), Some(right), Some(top), Some(bottom)] => Some((left, bottom, right, top)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn text_field(number: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = varint(u64::from(number) << 3 | 2);
        out.extend(varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn number_field(number: u32, value: u64) -> Vec<u8> {
        let mut out = varint(u64::from(number) << 3);
        out.extend(varint(value));
        out
    }

    const REQUIRED: u32 = 4;
    const OPTIONAL: u32 = 5;
    const WRITING_PROGRAM: u32 = 16;
    const REPLICATION_TIMESTAMP: u32 = 32;

    fn planet_header() -> Vec<u8> {
        [
            text_field(REQUIRED, b"OsmSchema-V0.6"),
            text_field(REQUIRED, DENSE_NODES.as_bytes()),
            text_field(OPTIONAL, SORT_TYPE_THEN_ID.as_bytes()),
            text_field(WRITING_PROGRAM, b"planet-dump-ng 1.2.4"),
            number_field(REPLICATION_TIMESTAMP, 1_788_739_204),
        ]
        .concat()
    }

    #[test]
    fn when_the_file_declares_its_sort_order_then_bisection_is_sound() {
        assert!(file_header(&planet_header()).unwrap().is_sorted_by_type_then_id());
    }

    /// An extract cut by a tool that does not sort would decode field for field and bisect to the
    /// wrong blob, so the absence of the declaration has to be visible rather than assumed away.
    #[test]
    fn when_the_file_declares_no_sort_order_then_it_is_not_treated_as_sorted() {
        let unsorted = text_field(REQUIRED, DENSE_NODES.as_bytes());

        assert!(!file_header(&unsorted).unwrap().is_sorted_by_type_then_id());
    }

    #[test]
    fn when_the_file_requires_dense_nodes_then_the_reader_is_told_so() {
        assert!(file_header(&planet_header()).unwrap().requires_dense_nodes());
    }

    #[test]
    fn when_the_file_names_its_replication_moment_then_it_is_read_as_a_timestamp() {
        let header = file_header(&planet_header()).unwrap();

        assert_eq!(header.replication_timestamp, Some(1_788_739_204));
    }

    #[test]
    fn when_the_file_names_the_program_that_wrote_it_then_it_is_kept_for_provenance() {
        let header = file_header(&planet_header()).unwrap();

        assert_eq!(header.writing_program.as_deref(), Some("planet-dump-ng 1.2.4"));
    }

    /// A required feature is a promise the reader must keep. Meeting an unknown one silently would
    /// mean decoding the file into something plausible and wrong.
    #[test]
    fn when_the_file_requires_something_this_reader_cannot_do_then_it_is_named() {
        let exotic = [
            text_field(REQUIRED, DENSE_NODES.as_bytes()),
            text_field(REQUIRED, b"HistoricalInformation"),
        ]
        .concat();

        assert_eq!(
            file_header(&exotic).unwrap().unsupported_requirements(),
            vec!["HistoricalInformation"]
        );
    }

    #[test]
    fn when_the_file_requires_only_what_this_reader_does_then_nothing_is_named() {
        assert!(
            file_header(&planet_header())
                .unwrap()
                .unsupported_requirements()
                .is_empty()
        );
    }

    #[test]
    fn when_a_header_is_empty_then_it_declares_nothing_rather_than_failing() {
        assert_eq!(file_header(b"").unwrap(), FileHeader::default());
    }

    /// The bounding box stores longitudes first and latitudes second, and the order it declares
    /// them in is left, right, top, bottom rather than any of the usual conventions.
    #[test]
    fn when_the_file_states_its_extent_then_the_edges_come_back_as_west_south_east_north() {
        fn zigzagged(number: u32, value: i64) -> Vec<u8> {
            let mut out = varint(u64::from(number) << 3);
            out.extend(varint(((value << 1) ^ (value >> 63)) as u64));
            out
        }
        let extent = [
            zigzagged(1, -180_000_000_000),
            zigzagged(2, 180_000_000_000),
            zigzagged(3, 90_000_000_000),
            zigzagged(4, -90_000_000_000),
        ]
        .concat();

        let header = file_header(&text_field(1, &extent)).unwrap();

        assert_eq!(
            header.bounding_box,
            Some((-180_000_000_000, -90_000_000_000, 180_000_000_000, 90_000_000_000))
        );
    }
}
