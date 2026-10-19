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
}

/// Every blob's id span, so a lookup by id reads the blobs that can hold it and no others.
///
/// The spans come free. Passes one and two inflate every blob in the node and way sections anyway,
/// and recording the first and last id of each costs nothing beyond the sixteen bytes it takes. The
/// passes that follow, fetching a multipolygon's member ways and then their nodes, are otherwise
/// two more full scans of those same sections for a few thousand entities.
#[derive(Debug, Clone, PartialEq)]
pub struct IdIndex {
    spans: Vec<Option<IdSpan>>,
}

impl IdIndex {
    /// An index of `blob_count` blobs, none of them recorded yet.
    pub fn empty(blob_count: usize) -> Self {
        Self {
            spans: vec![None; blob_count],
        }
    }

    pub fn record(&mut self, blob: usize, span: IdSpan) {
        if let Some(slot) = self.spans.get_mut(blob) {
            *slot = Some(span);
        }
    }

    pub fn span_of(&self, blob: usize) -> Option<IdSpan> {
        self.spans.get(blob).copied().flatten()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn index_over(spans: &[(usize, Section, i64, i64)], blobs: usize) -> IdIndex {
        let mut index = IdIndex::empty(blobs);
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
    fn when_only_some_blobs_have_been_seen_then_the_rest_stay_unrecorded() {
        let index = index_over(&[(1, Section::Nodes, 10, 20)], 3);

        assert_eq!((index.span_of(0), index.span_of(2)), (None, None));
        assert!(index.span_of(1).is_some());
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
}
