//! An in-memory map of the file, built once per run.
//!
//! A PBF file has no table of contents, but walking its framing costs a handful of seconds and
//! yields one entry per blob at 24 bytes each. Holding all of them costs about a megabyte and buys
//! two things: work can be handed to threads as slices of a list rather than through a shared
//! cursor, and the section boundaries can be found by bisection instead of by reading forwards.

use std::fs::File;
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::blob::{BlobKind, Framing, Location};
use crate::block::{IdSpan, Section, block, block_contents, inflate};
use crate::error::PbfError;
use crate::header::{FileHeader, file_header};

/// Where each section begins and ends, as ranges over the blob list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sections {
    pub nodes: Range<usize>,
    pub ways: Range<usize>,
    pub relations: Range<usize>,
}

pub struct Planet {
    file: File,
    pub blobs: Vec<Location>,
    pub file_size: u64,
    pub header: FileHeader,
}

impl Planet {
    pub fn open(path: &Path) -> Result<Self, PbfError> {
        let mut framing = Framing::open(path)?;
        let file_size = framing.file_size();
        let mut blobs = Vec::with_capacity(64 * 1024);
        let mut header_at = None;

        while let Some(location) = framing.next_location()? {
            match location.kind {
                BlobKind::Data => blobs.push(location),
                BlobKind::Header if header_at.is_none() => header_at = Some(location),
                _ => {}
            }
        }

        let file = File::open(path)?;
        let planet = Self {
            file,
            blobs,
            file_size,
            header: FileHeader::default(),
        };

        let header = match header_at {
            None => FileHeader::default(),
            Some(location) => {
                let mut payload = Vec::new();
                let mut inflated = Vec::new();
                planet.read(&location, &mut payload)?;
                inflate(&payload, &mut inflated).map_err(|error| error.at_blob(location.offset))?;
                file_header(&inflated).map_err(|error| error.at_blob(location.offset))?
            }
        };

        Ok(Self { header, ..planet })
    }

    /// Reads one payload by absolute offset.
    ///
    /// Takes `&self`, so threads share one open file: on unix `read_at` carries its own offset and
    /// never touches the descriptor's cursor.
    pub fn read(&self, location: &Location, into: &mut Vec<u8>) -> Result<(), PbfError> {
        into.resize(location.data_size, 0);
        self.file
            .read_exact_at(into, location.offset)
            .map_err(|error| PbfError::from(error).at_blob(location.offset))
    }

    /// Reads and inflates one blob, reusing both buffers.
    pub fn inflate_blob(
        &self,
        at: usize,
        payload: &mut Vec<u8>,
        inflated: &mut Vec<u8>,
    ) -> Result<(), PbfError> {
        let location = self.blobs.get(at).ok_or(PbfError::Truncated)?;
        self.read(location, payload)?;
        inflate(payload, inflated).map_err(|error| error.at_blob(location.offset))
    }

    pub fn section_of(&self, at: usize) -> Result<Section, PbfError> {
        let mut payload = Vec::new();
        let mut inflated = Vec::new();
        self.inflate_blob(at, &mut payload, &mut inflated)?;
        let decoded = block(&inflated)?;
        Ok(block_contents(&decoded)?.section())
    }

    /// The first blob holding `wanted` or anything after it, found by bisection.
    ///
    /// Sound only because the file declares `Sort.Type_then_ID`: sections are contiguous and in a
    /// known order, so the predicate is monotonic. Checked rather than assumed, because a file that
    /// does not declare it bisects to a plausible and wrong answer.
    pub fn first_blob_of(&self, wanted: Section) -> Result<usize, PbfError> {
        if !self.header.is_sorted_by_type_then_id() {
            return Err(PbfError::UnsortedSections);
        }

        let mut low = 0usize;
        let mut high = self.blobs.len();

        while low < high {
            let middle = low + (high - low) / 2;
            if self.section_of(middle)? >= wanted {
                high = middle;
            } else {
                low = middle + 1;
            }
        }

        Ok(low)
    }

    /// Where the three sections begin and end.
    ///
    /// Ranges are half-open and contiguous. A blob straddling a boundary reports the later of the
    /// two sections, so the earlier range stops before it and the later one includes it.
    pub fn sections(&self) -> Result<Sections, PbfError> {
        let ways = self.first_blob_of(Section::Ways)?;
        let relations = self.first_blob_of(Section::Relations)?;

        Ok(Sections {
            nodes: 0..ways,
            ways: ways..relations,
            relations: relations..self.blobs.len(),
        })
    }

    pub fn bytes_in(&self, blobs: Range<usize>) -> u64 {
        self.blobs[blobs].iter().map(|blob| blob.data_size as u64).sum()
    }

    pub fn fingerprint(&self) -> PlanetFingerprint {
        PlanetFingerprint {
            file_size: self.file_size,
            blob_count: self.blobs.len() as u64,
            replication_timestamp: self.header.replication_timestamp.unwrap_or(0),
        }
    }
}

/// What ties a sidecar index, or a harvest, to the exact file it was built from.
///
/// Not a checksum of 94 GB, which no run can afford. The replication timestamp names the minute of
/// OSM history exactly, and the size and blob count catch a truncated or re-dumped file carrying
/// the same timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanetFingerprint {
    pub file_size: u64,
    pub blob_count: u64,
    pub replication_timestamp: i64,
}

/// Every blob's id span, so a lookup by id reads the blobs that can hold it and no others.
///
/// The spans come free. Passes one and two inflate every blob in the node and way sections anyway,
/// and recording the first and last id of each costs nothing beyond the sixteen bytes it takes. The
/// passes that follow, fetching a multipolygon's member ways and then their nodes, are otherwise
/// two more full scans of those same sections for a few thousand entities.
#[derive(Debug, Clone, PartialEq)]
pub struct IdIndex {
    fingerprint: PlanetFingerprint,
    spans: Vec<Option<IdSpan>>,
}

impl IdIndex {
    pub fn empty(fingerprint: PlanetFingerprint, blob_count: usize) -> Self {
        Self {
            fingerprint,
            spans: vec![None; blob_count],
        }
    }

    pub fn fingerprint(&self) -> PlanetFingerprint {
        self.fingerprint
    }

    pub fn record(&mut self, blob: usize, span: IdSpan) {
        if let Some(slot) = self.spans.get_mut(blob) {
            *slot = Some(span);
        }
    }

    pub fn span_of(&self, blob: usize) -> Option<IdSpan> {
        self.spans.get(blob).copied().flatten()
    }

    pub fn recorded(&self) -> usize {
        self.spans.iter().filter(|span| span.is_some()).count()
    }

    pub fn covers(&self, blobs: Range<usize>) -> bool {
        !blobs.is_empty() && self.spans[blobs].iter().all(Option::is_some)
    }

    /// The blobs in `blobs` that could hold any of `wanted`, which must be sorted and deduplicated.
    ///
    /// A blob with no recorded span is kept, because "not indexed" cannot be told from "holds
    /// nothing" and dropping it would silently lose geometry.
    pub fn blobs_holding(&self, blobs: Range<usize>, wanted: &[i64]) -> Vec<usize> {
        blobs
            .filter(|at| match self.span_of(*at) {
                None => true,
                Some(span) => {
                    let from = wanted.partition_point(|id| *id < span.first);
                    let to = wanted.partition_point(|id| *id <= span.last);
                    from < to
                }
            })
            .collect()
    }
}

const INDEX_MAGIC: &[u8; 8] = b"PLANETIX";
const INDEX_VERSION: u32 = 1;
const NO_SECTION: u8 = 0;
const SPANS_AT: usize = 44;
const SPAN_BYTES: usize = 17;

fn section_tag(section: Section) -> u8 {
    match section {
        Section::Empty => NO_SECTION,
        Section::Nodes => 1,
        Section::Ways => 2,
        Section::Relations => 3,
    }
}

fn section_of_tag(tag: u8) -> Option<Section> {
    match tag {
        1 => Some(Section::Nodes),
        2 => Some(Section::Ways),
        3 => Some(Section::Relations),
        _ => None,
    }
}

impl IdIndex {
    /// The index as bytes: fixed width, little endian, no compression.
    ///
    /// Deterministic by construction, so two runs over the same planet file produce the same bytes
    /// and the sidecar can be checksummed like any other published artefact.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SPANS_AT + self.spans.len() * SPAN_BYTES);
        out.extend_from_slice(INDEX_MAGIC);
        out.extend_from_slice(&INDEX_VERSION.to_le_bytes());
        out.extend_from_slice(&self.fingerprint.file_size.to_le_bytes());
        out.extend_from_slice(&self.fingerprint.blob_count.to_le_bytes());
        out.extend_from_slice(&self.fingerprint.replication_timestamp.to_le_bytes());
        out.extend_from_slice(&(self.spans.len() as u64).to_le_bytes());

        for span in &self.spans {
            match span {
                None => {
                    out.push(NO_SECTION);
                    out.extend_from_slice(&0i64.to_le_bytes());
                    out.extend_from_slice(&0i64.to_le_bytes());
                }
                Some(span) => {
                    out.push(section_tag(span.section));
                    out.extend_from_slice(&span.first.to_le_bytes());
                    out.extend_from_slice(&span.last.to_le_bytes());
                }
            }
        }

        out
    }

    /// Reads an index back, or nothing when the bytes are not one.
    ///
    /// The entry count is checked against the buffer before anything is reserved on it, because it
    /// comes out of a file and `with_capacity` on an unchecked number is an allocation a corrupt
    /// sidecar gets to choose.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
            let slice = bytes.get(at..at + 8)?;
            Some(u64::from_le_bytes(
                slice.try_into().expect("a slice of eight bytes"),
            ))
        }

        if bytes.get(0..8)? != INDEX_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
        if version != INDEX_VERSION {
            return None;
        }

        let fingerprint = PlanetFingerprint {
            file_size: u64_at(bytes, 12)?,
            blob_count: u64_at(bytes, 20)?,
            replication_timestamp: u64_at(bytes, 28)? as i64,
        };

        let count = u64_at(bytes, 36)? as usize;
        if bytes.len() < SPANS_AT + count * SPAN_BYTES {
            return None;
        }

        let spans = (0..count)
            .map(|index| {
                let at = SPANS_AT + index * SPAN_BYTES;
                let section = section_of_tag(bytes[at])?;
                Some(IdSpan {
                    section,
                    first: u64_at(bytes, at + 1)? as i64,
                    last: u64_at(bytes, at + 9)? as i64,
                })
            })
            .collect();

        Some(Self { fingerprint, spans })
    }

    pub fn write(&self, path: &Path) -> Result<u64, PbfError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = self.to_bytes();
        std::fs::write(path, &bytes)?;
        Ok(bytes.len() as u64)
    }

    /// Reads a sidecar, or nothing when it is absent, unreadable, or built from another file.
    ///
    /// A mismatched index is treated exactly like a missing one. It is an optimisation, and there is
    /// no reading of it that is better than doing the work again.
    pub fn read_for(path: &Path, fingerprint: PlanetFingerprint) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let index = Self::from_bytes(&bytes)?;
        (index.fingerprint == fingerprint).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint() -> PlanetFingerprint {
        PlanetFingerprint {
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
        }
    }

    fn index_over(spans: &[(usize, Section, i64, i64)], blobs: usize) -> IdIndex {
        let mut index = IdIndex::empty(fingerprint(), blobs);
        for (at, section, first, last) in spans {
            index.record(
                *at,
                IdSpan {
                    section: *section,
                    first: *first,
                    last: *last,
                },
            );
        }
        index
    }

    #[test]
    fn when_an_index_is_written_then_reading_it_back_gives_the_same_thing() {
        let index = index_over(
            &[
                (0, Section::Ways, 37, 3_732_656),
                (1, Section::Ways, 3_732_700, 4_078_000),
            ],
            3,
        );

        assert_eq!(IdIndex::from_bytes(&index.to_bytes()), Some(index));
    }

    /// The sidecar is an artefact anyone may re-derive, so two runs over the same file have to
    /// agree byte for byte or a checksum over it says nothing.
    #[test]
    fn when_the_same_index_is_written_twice_then_the_bytes_are_identical() {
        let index = index_over(&[(1, Section::Nodes, 1, 513_655)], 4);

        assert_eq!(index.to_bytes(), index.to_bytes());
    }

    #[test]
    fn when_the_bytes_are_not_an_index_then_nothing_comes_back_rather_than_a_misreading() {
        assert_eq!(IdIndex::from_bytes(b"this is not an index at all"), None);
    }

    #[test]
    fn when_only_some_blobs_have_been_seen_then_the_rest_stay_unrecorded() {
        let index = index_over(&[(1, Section::Nodes, 10, 20)], 3);

        assert_eq!(
            (index.recorded(), index.span_of(0), index.span_of(2)),
            (1, None, None)
        );
    }

    /// The whole point: a handful of ids should read a handful of blobs, not the section.
    #[test]
    fn when_ids_are_wanted_then_only_the_blobs_whose_span_holds_one_are_read() {
        let index = index_over(
            &[
                (0, Section::Ways, 1, 100),
                (1, Section::Ways, 101, 200),
                (2, Section::Ways, 201, 300),
                (3, Section::Ways, 301, 400),
            ],
            4,
        );

        assert_eq!(index.blobs_holding(0..4, &[150, 350]), vec![1, 3]);
    }

    #[test]
    fn when_a_wanted_id_falls_on_a_span_edge_then_that_blob_is_still_read() {
        let index = index_over(&[(0, Section::Ways, 1, 100), (1, Section::Ways, 101, 200)], 2);

        assert_eq!(index.blobs_holding(0..2, &[100, 101]), vec![0, 1]);
    }

    #[test]
    fn when_no_id_falls_in_any_span_then_nothing_is_read() {
        let index = index_over(&[(0, Section::Ways, 1, 100), (1, Section::Ways, 101, 200)], 2);

        assert!(index.blobs_holding(0..2, &[900]).is_empty());
    }

    /// An unrecorded blob cannot be told from one holding nothing, and skipping it would drop real
    /// geometry with nothing to show for it.
    #[test]
    fn when_a_blob_has_no_recorded_span_then_it_is_read_rather_than_skipped() {
        let index = index_over(&[(0, Section::Ways, 1, 100)], 2);

        assert_eq!(index.blobs_holding(0..2, &[900]), vec![1]);
    }

    #[test]
    fn when_every_blob_in_a_range_has_a_span_then_the_range_is_covered() {
        let index = index_over(&[(0, Section::Nodes, 1, 10), (1, Section::Nodes, 11, 20)], 3);

        assert!(index.covers(0..2));
        assert!(!index.covers(0..3));
    }

    #[test]
    fn when_a_sidecar_was_built_from_another_file_then_it_is_refused() {
        let path = std::env::temp_dir().join("planetrs-index-mismatch.bin");
        index_over(&[(0, Section::Nodes, 1, 10)], 2).write(&path).unwrap();

        let other = PlanetFingerprint {
            file_size: 1,
            blob_count: 2,
            replication_timestamp: 3,
        };

        assert!(IdIndex::read_for(&path, other).is_none());
        assert!(IdIndex::read_for(&path, fingerprint()).is_some());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn when_a_sidecar_is_absent_then_nothing_comes_back_rather_than_an_error() {
        let missing = std::env::temp_dir().join("planetrs-index-that-is-not-there.bin");

        assert!(IdIndex::read_for(&missing, fingerprint()).is_none());
    }
}
