//! Decoding one PrimitiveBlock: the layer under the framing, where the actual map lives.
//!
//! Two encodings carry nearly everything and both are decoded lazily here, because a planet file
//! holds around nine billion nodes and allocating for each one is not an option:
//!
//! - **delta coding.** Ids and coordinates are stored as differences from the previous entry, so a
//!   run of neighbouring nodes costs one or two bytes each instead of eight.
//! - **a per-block string table.** Tags are pairs of indices into it, so `highway` is stored once
//!   per block rather than once per way.
//!
//! Coordinates are integers in units of `granularity` nanodegrees, offset by the block's own origin.

use flate2::{Decompress, FlushDecompress};

use crate::error::PbfError;
use crate::proto::{Packed, Reader, Undeltaed, Value, zigzag};

/// The spec's ceiling on an inflated blob. A larger `raw_size` means a corrupt header, and
/// reserving on it would hand a number taken from the file straight to the allocator.
pub const MAX_INFLATED: usize = 32 * 1024 * 1024;

/// Unwraps a Blob: either stored bytes, or a zlib stream plus the size it inflates to.
pub fn inflate(payload: &[u8], into: &mut Vec<u8>) -> Result<(), PbfError> {
    let mut reader = Reader::new(payload);
    let mut raw = None;
    let mut compressed = None;
    let mut raw_size = 0usize;

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(bytes)) => raw = Some(bytes),
            (2, Value::Varint(size)) => raw_size = size as usize,
            (3, Value::Bytes(bytes)) => compressed = Some(bytes),
            _ => {}
        }
    }

    if raw_size > MAX_INFLATED {
        return Err(PbfError::InflatedTooLarge {
            size: raw_size,
            limit: MAX_INFLATED,
        });
    }

    into.clear();
    match (raw, compressed) {
        (Some(bytes), _) => {
            into.extend_from_slice(bytes);
            Ok(())
        }
        (None, Some(bytes)) => {
            into.reserve(raw_size);
            Decompress::new(true)
                .decompress_vec(bytes, into, FlushDecompress::Finish)
                .map(|_| ())
                .map_err(|error| PbfError::Inflate(error.to_string()))
        }
        (None, None) => Err(PbfError::EmptyBlob),
    }
}

pub struct Block<'a> {
    pub strings: Vec<&'a [u8]>,
    pub granularity: i64,
    pub lat_offset: i64,
    pub lon_offset: i64,
    pub groups: Vec<&'a [u8]>,
}

impl<'a> Block<'a> {
    pub fn string(&self, index: usize) -> &'a [u8] {
        self.strings.get(index).copied().unwrap_or(b"")
    }

    /// Nanodegrees to degrees, applying the block's own origin and granularity.
    pub fn latitude(&self, value: i64) -> f64 {
        1e-9 * self.nanodegrees_latitude(value) as f64
    }

    pub fn longitude(&self, value: i64) -> f64 {
        1e-9 * self.nanodegrees_longitude(value) as f64
    }

    /// The block's own units turned into nanodegrees, which are exact and independent of the block
    /// they came from. Coordinates are kept this way everywhere past the reader.
    pub fn nanodegrees_latitude(&self, value: i64) -> i64 {
        self.lat_offset + self.granularity * value
    }

    pub fn nanodegrees_longitude(&self, value: i64) -> i64 {
        self.lon_offset + self.granularity * value
    }

    /// The index of a key in this block's string table, if it holds it at all.
    ///
    /// Resolving a key once per block turns every later tag test into an integer compare rather
    /// than a string one, which matters when the test runs against nine billion nodes.
    pub fn string_id(&self, key: &[u8]) -> Option<u32> {
        self.strings
            .iter()
            .position(|text| *text == key)
            .map(|at| at as u32)
    }
}

pub fn block(bytes: &[u8]) -> Result<Block<'_>, PbfError> {
    let mut reader = Reader::new(bytes);
    let mut strings = Vec::new();
    let mut groups = Vec::new();
    let mut granularity = 100i64;
    let mut lat_offset = 0i64;
    let mut lon_offset = 0i64;

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(table)) => {
                let mut table = Reader::new(table);
                while let Some(entry) = table.next_field()? {
                    if let (1, Value::Bytes(text)) = (entry.number, entry.value) {
                        strings.push(text);
                    }
                }
            }
            (2, Value::Bytes(group)) => groups.push(group),
            (17, Value::Varint(value)) => granularity = value as i64,
            (19, Value::Varint(value)) => lat_offset = zigzag(value),
            (20, Value::Varint(value)) => lon_offset = zigzag(value),
            _ => {}
        }
    }

    Ok(Block {
        strings,
        granularity,
        lat_offset,
        lon_offset,
        groups,
    })
}

/// Which of the three sections a blob belongs to. `Empty` covers a blob holding none of them.
///
/// Ordered, because bisecting the file for a section boundary needs the predicate to be monotonic,
/// which it is only because the file declares `Sort.Type_then_ID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Empty,
    Nodes,
    Ways,
    Relations,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Contents {
    pub dense: u64,
    pub nodes: u64,
    pub ways: u64,
    pub relations: u64,
}

impl Contents {
    pub fn section(&self) -> Section {
        match self {
            _ if self.relations > 0 => Section::Relations,
            _ if self.ways > 0 => Section::Ways,
            _ if self.dense + self.nodes > 0 => Section::Nodes,
            _ => Section::Empty,
        }
    }
}

/// What a group holds, without decoding it: the field numbers alone say which of the four it is.
pub fn contents(group: &[u8]) -> Result<Contents, PbfError> {
    let mut reader = Reader::new(group);
    let mut found = Contents::default();

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(_)) => found.nodes += 1,
            (2, Value::Bytes(dense)) => found.dense += count_dense(dense)?,
            (3, Value::Bytes(_)) => found.ways += 1,
            (4, Value::Bytes(_)) => found.relations += 1,
            _ => {}
        }
    }

    Ok(found)
}

pub fn block_contents(block: &Block<'_>) -> Result<Contents, PbfError> {
    block
        .groups
        .iter()
        .try_fold(Contents::default(), |mut total, group| {
            let found = contents(group)?;
            total.dense += found.dense;
            total.nodes += found.nodes;
            total.ways += found.ways;
            total.relations += found.relations;
            Ok(total)
        })
}

fn count_dense(dense: &[u8]) -> Result<u64, PbfError> {
    let mut reader = Reader::new(dense);
    while let Some(field) = reader.next_field()? {
        if let (1, Value::Bytes(ids)) = (field.number, field.value) {
            return Ok(Packed::new(ids).count() as u64);
        }
    }
    Ok(0)
}

/// The lowest and highest entity id a block holds, with the section they belong to.
///
/// Sound as a pruning key only because the file declares `Sort.Type_then_ID`, which makes the spans
/// of successive blobs ascending and disjoint. Verified against the September 2026 planet: every
/// sampled blob's first id exceeds the previous blob's last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdSpan {
    pub section: Section,
    pub first: i64,
    pub last: i64,
}

impl IdSpan {
    pub fn holds_any(&self, wanted: &[i64]) -> bool {
        wanted.iter().any(|id| *id >= self.first && *id <= self.last)
    }
}

pub fn id_span(block: &Block<'_>) -> Result<Option<IdSpan>, PbfError> {
    let mut span: Option<IdSpan> = None;

    for group in &block.groups {
        let mut reader = Reader::new(group);
        while let Some(field) = reader.next_field()? {
            let found = match (field.number, field.value) {
                (2, Value::Bytes(dense)) => {
                    dense_id_bounds(dense)?.map(|(first, last)| (Section::Nodes, first, last))
                }
                (3, Value::Bytes(way)) => entity_id(way)?.map(|id| (Section::Ways, id, id)),
                (4, Value::Bytes(relation)) => entity_id(relation)?.map(|id| (Section::Relations, id, id)),
                _ => None,
            };

            if let Some((section, first, last)) = found {
                span = Some(match span {
                    None => IdSpan { section, first, last },
                    Some(seen) => IdSpan {
                        section: seen.section.max(section),
                        first: seen.first.min(first),
                        last: seen.last.max(last),
                    },
                });
            }
        }
    }

    Ok(span)
}

fn dense_id_bounds(dense: &[u8]) -> Result<Option<(i64, i64)>, PbfError> {
    let mut reader = Reader::new(dense);
    while let Some(field) = reader.next_field()? {
        if let (1, Value::Bytes(ids)) = (field.number, field.value) {
            let mut walk = Undeltaed::new(ids);
            let Some(first) = walk.next().transpose()? else {
                return Ok(None);
            };
            let mut last = first;
            for id in walk {
                last = id?;
            }
            return Ok(Some((first, last)));
        }
    }
    Ok(None)
}

fn entity_id(entity: &[u8]) -> Result<Option<i64>, PbfError> {
    let mut reader = Reader::new(entity);
    while let Some(field) = reader.next_field()? {
        if let (1, Value::Varint(id)) = (field.number, field.value) {
            return Ok(Some(id as i64));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_block_omits_granularity_then_the_protobuf_default_applies() {
        let decoded = block(b"").unwrap();

        assert_eq!(
            (decoded.granularity, decoded.lat_offset, decoded.lon_offset),
            (100, 0, 0)
        );
    }

    #[test]
    fn when_a_coordinate_is_read_then_granularity_turns_it_into_degrees() {
        let decoded = block(b"").unwrap();

        assert!((decoded.latitude(52_700_000) - 5.27).abs() < 1e-9);
    }

    /// Blocks are allowed their own origin and step. The planet file uses 100/0/0 throughout, but a
    /// regional extract cut by another tool may not, and reading those wrongly would silently place
    /// every node somewhere else.
    #[test]
    fn when_a_block_declares_its_own_origin_then_coordinates_move_with_it() {
        let shifted = Block {
            strings: vec![],
            granularity: 1000,
            lat_offset: 5_000_000_000,
            lon_offset: 0,
            groups: vec![],
        };

        assert_eq!(shifted.nanodegrees_latitude(200_000), 5_200_000_000);
    }

    #[test]
    fn when_a_blob_holds_uncompressed_bytes_then_they_are_taken_as_they_are() {
        let mut inflated = Vec::new();
        inflate(b"\x0a\x05hello", &mut inflated).unwrap();

        assert_eq!(inflated, b"hello");
    }

    #[test]
    fn when_a_blob_holds_neither_raw_nor_compressed_bytes_then_it_is_refused() {
        let mut inflated = Vec::new();

        assert!(matches!(inflate(b"", &mut inflated), Err(PbfError::EmptyBlob)));
    }

    /// `raw_size` comes out of the file, and reserving on it before checking would let a corrupt
    /// header ask the allocator for an arbitrary amount.
    #[test]
    fn when_a_blob_claims_to_inflate_past_the_spec_ceiling_then_it_is_refused_before_allocating() {
        let mut inflated = Vec::new();
        let oversized = [b"\x10".as_slice(), b"\x80\x80\x80\x80\x01".as_slice()].concat();

        assert!(matches!(
            inflate(&oversized, &mut inflated),
            Err(PbfError::InflatedTooLarge { .. })
        ));
    }

    #[test]
    fn when_a_block_holds_ways_then_its_section_is_ways() {
        let contents = Contents {
            dense: 0,
            nodes: 0,
            ways: 12,
            relations: 0,
        };

        assert_eq!(contents.section(), Section::Ways);
    }

    /// A blob straddling a boundary reports the later of the two sections, which is what makes the
    /// bisection predicate monotonic.
    #[test]
    fn when_a_block_mixes_ways_and_relations_then_it_reports_the_later_section() {
        let contents = Contents {
            dense: 0,
            nodes: 0,
            ways: 12,
            relations: 1,
        };

        assert_eq!(contents.section(), Section::Relations);
    }

    #[test]
    fn when_a_block_holds_nothing_then_it_belongs_to_no_section() {
        assert_eq!(Contents::default().section(), Section::Empty);
    }

    #[test]
    fn when_a_group_holds_dense_nodes_then_its_span_runs_from_the_first_id_to_the_last() {
        let dense = b"\x0a\x03\x02\x04\x06";
        let mut group = vec![0x12, dense.len() as u8];
        group.extend_from_slice(dense);
        let decoded = Block {
            strings: vec![],
            granularity: 100,
            lat_offset: 0,
            lon_offset: 0,
            groups: vec![&group],
        };

        assert_eq!(
            id_span(&decoded).unwrap(),
            Some(IdSpan {
                section: Section::Nodes,
                first: 1,
                last: 6
            })
        );
    }

    #[test]
    fn when_a_group_holds_ways_then_its_span_covers_every_way_id() {
        let group = b"\x1a\x02\x08\x07\x1a\x02\x08\x0b";
        let decoded = Block {
            strings: vec![],
            granularity: 100,
            lat_offset: 0,
            lon_offset: 0,
            groups: vec![group],
        };

        assert_eq!(
            id_span(&decoded).unwrap(),
            Some(IdSpan {
                section: Section::Ways,
                first: 7,
                last: 11
            })
        );
    }

    #[test]
    fn when_a_block_holds_nothing_then_it_has_no_span() {
        let decoded = Block {
            strings: vec![],
            granularity: 100,
            lat_offset: 0,
            lon_offset: 0,
            groups: vec![],
        };

        assert_eq!(id_span(&decoded).unwrap(), None);
    }

    /// Pruning by id is only sound when a span that misses every wanted id means the blob can be
    /// skipped without reading it.
    #[test]
    fn when_no_wanted_id_falls_in_a_span_then_the_blob_can_be_skipped() {
        let span = IdSpan {
            section: Section::Ways,
            first: 100,
            last: 200,
        };

        assert!(span.holds_any(&[50, 150, 900]));
        assert!(!span.holds_any(&[50, 900]));
    }
}
