//! Reading a set of blobs in parallel, each decoded block handed to a visit, with each blob's id span
//! kept for the passes that fetch by id, and progress said as the passes of a harvest say it.

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use planetrs_pbf::{Block, IdSpan, PbfError, Planet, Section, block};
use rayon::prelude::*;

/// How many blobs pass between two progress lines, as in a harvest.
const BLOBS_BETWEEN_PROGRESS_LINES: usize = 512;

/// What a pass gathered, and the id span of each blob its visit reported one for.
pub(super) struct Scanned<T> {
    pub(super) found: T,
    pub(super) spans: Vec<(usize, IdSpan)>,
}

/// Reads the blobs at `blobs` in parallel, handing each decoded block to `visit`, which returns the
/// block's id span when it walked every id in it.
///
/// A blob that does not read, inflate or decode stops the pass with its offset: an index built
/// around it would quietly lack whatever the blob held.
pub(super) fn scan<T, E, V, M>(
    planet: &Planet,
    blobs: &[usize],
    label: &str,
    progress: &(dyn Fn(&str) + Sync),
    empty: E,
    visit: V,
    merge: M,
) -> Result<Scanned<T>, PbfError>
where
    T: Send,
    E: Fn() -> T + Sync + Send,
    V: Fn(&mut T, &Block<'_>) -> Result<Option<IdSpan>, PbfError> + Sync + Send,
    M: Fn(T, T) -> T + Sync + Send,
{
    let total = blobs.len();
    let seen = AtomicUsize::new(0);
    if total > 0 {
        progress(&blobs_read(label, 0, total));
    }
    blobs
        .par_iter()
        .try_fold(
            || (empty(), Vec::new(), Vec::new(), Vec::new()),
            |(mut found, mut spans, mut payload, mut inflated), at| {
                planet.inflate_blob(*at, &mut payload, &mut inflated)?;
                let offset = planet.blobs[*at].offset;
                {
                    let decoded = block(&inflated).map_err(|error| error.at_blob(offset))?;
                    if let Some(span) = visit(&mut found, &decoded).map_err(|error| error.at_blob(offset))? {
                        spans.push((*at, span));
                    }
                }
                let done = seen.fetch_add(1, Ordering::Relaxed) + 1;
                if done.is_multiple_of(BLOBS_BETWEEN_PROGRESS_LINES) || done == total {
                    progress(&blobs_read(label, done, total));
                }
                Ok::<_, PbfError>((found, spans, payload, inflated))
            },
        )
        .map(|folded| folded.map(|(found, spans, _, _)| Scanned { found, spans }))
        .try_reduce(
            || Scanned {
                found: empty(),
                spans: Vec::new(),
            },
            |left, right| {
                Ok(Scanned {
                    found: merge(left.found, right.found),
                    spans: [left.spans, right.spans].concat(),
                })
            },
        )
}

/// How far a pass has got, as its progress line says it.
fn blobs_read(pass: &str, done: usize, total: usize) -> String {
    format!("{pass}: {done} / {total} blobs")
}

/// Every blob of a section.
pub(super) fn every_blob(range: Range<usize>) -> Vec<usize> {
    range.collect()
}

/// The lowest and highest id a visit saw in one block, tracked as it walks it.
pub(super) struct Span {
    lowest: i64,
    highest: i64,
}

impl Span {
    pub(super) fn new() -> Self {
        Self {
            lowest: i64::MAX,
            highest: i64::MIN,
        }
    }

    pub(super) fn saw(&mut self, id: i64) {
        self.lowest = self.lowest.min(id);
        self.highest = self.highest.max(id);
    }

    pub(super) fn of(self, section: Section) -> Option<IdSpan> {
        (self.lowest <= self.highest).then_some(IdSpan {
            section,
            first: self.lowest,
            last: self.highest,
        })
    }
}
