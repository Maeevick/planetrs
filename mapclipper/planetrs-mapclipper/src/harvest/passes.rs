//! Reading the planet for one area.
//!
//! Five passes, and the first four exist because the format is sorted by type: a relation's members
//! cannot be fetched while the ways stream past, because every way has already gone by before the
//! first relation arrives.
//!
//! | Pass | Reads | Why it cannot be merged with another |
//! | ---- | ----- | ------------------------------------ |
//! | nodes inside the box | the node section | nothing else can run first |
//! | ways touching those nodes | the way section | needs the node positions |
//! | multipolygons touching those ways | the relation section | needs the way ids |
//! | member ways, by id | a few way blobs | needs the member lists |
//! | their nodes, by id | a few node blobs | needs the member ways |
//!
//! The last two used to be full scans of the sections they read, which was half the I/O of the
//! whole harvest spent fetching a few thousand entities. They are now pruned by the id index that
//! passes one and two record for free while they inflate those same sections anyway.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use planetrs_geo::{Area, Window};
use planetrs_pbf::{
    Block, IdIndex, IdSpan, PbfError, Planet, Section, Sections, block, dense_group, visit_relations,
    visit_ways,
};
use rayon::prelude::*;

use super::id_filter::IdFilter;
use super::{Harvest, Member, NamedNode, Node, Provenance, Relation, Tags, Way};
use crate::error::ClipError;

const MULTIPOLYGON: &str = "multipolygon";
/// How many blobs pass between two progress lines. A blob is a tenth of a second of work at
/// worst, so this is several times a second at most and costs nothing beside the inflate.
const BLOBS_BETWEEN_PROGRESS_LINES: usize = 512;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What one pass cost, so the report says where the time went rather than that it went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassCost {
    pub elapsed: Duration,
    pub blobs: u64,
    pub inflated_bytes: u64,
    pub kept: u64,
}

#[derive(Debug, Clone, Default)]
pub struct HarvestReport {
    pub nodes: PassCost,
    pub ways: PassCost,
    pub relations: PassCost,
    pub member_ways: PassCost,
    pub member_nodes: PassCost,
    /// Blobs the last two passes would have read without the id index.
    pub blobs_pruned: u64,
}

impl HarvestReport {
    pub fn total_blobs(&self) -> u64 {
        [
            self.nodes,
            self.ways,
            self.relations,
            self.member_ways,
            self.member_nodes,
        ]
        .iter()
        .map(|pass| pass.blobs)
        .sum()
    }

    pub fn total_bytes(&self) -> u64 {
        [
            self.nodes,
            self.ways,
            self.relations,
            self.member_ways,
            self.member_nodes,
        ]
        .iter()
        .map(|pass| pass.inflated_bytes)
        .sum()
    }

    pub fn elapsed(&self) -> Duration {
        [
            self.nodes,
            self.ways,
            self.relations,
            self.member_ways,
            self.member_nodes,
        ]
        .iter()
        .map(|pass| pass.elapsed)
        .sum()
    }
}

/// The lowest and highest id a pass saw in one blob, tracked as it goes.
struct Span {
    lowest: i64,
    highest: i64,
}

impl Default for Span {
    fn default() -> Self {
        Self {
            lowest: i64::MAX,
            highest: i64::MIN,
        }
    }
}

impl Span {
    fn saw(&mut self, id: i64) {
        self.lowest = self.lowest.min(id);
        self.highest = self.highest.max(id);
    }

    fn into_span(self, section: Section) -> Option<IdSpan> {
        (self.lowest <= self.highest).then_some(IdSpan {
            section,
            first: self.lowest,
            last: self.highest,
        })
    }
}

#[derive(Default)]
struct Buffers {
    payload: Vec<u8>,
    inflated: Vec<u8>,
}

/// One thread's share of a pass, with its own buffers so nothing is allocated per blob.
struct Scanned<T> {
    buffers: Buffers,
    kept: Vec<T>,
    /// Nodes that carry a name. Only the node pass fills this; every other pass leaves it empty.
    named: Vec<NamedNode>,
    spans: Vec<(usize, IdSpan)>,
    blobs: u64,
    inflated_bytes: u64,
}

impl<T> Default for Scanned<T> {
    fn default() -> Self {
        Self {
            buffers: Buffers::default(),
            kept: Vec::new(),
            named: Vec::new(),
            spans: Vec::new(),
            blobs: 0,
            inflated_bytes: 0,
        }
    }
}

impl<T> Scanned<T> {
    fn merged(mut self, mut other: Self) -> Self {
        self.kept.append(&mut other.kept);
        self.named.append(&mut other.named);
        self.spans.append(&mut other.spans);
        self.blobs += other.blobs;
        self.inflated_bytes += other.inflated_bytes;
        self
    }
}

/// Reads a set of blobs in parallel, handing each decoded block to `keep`.
///
/// The one place that reads, inflates, decodes, counts and reports progress, so that a pass is only
/// the question of what to take out of a block. A failure anywhere stops the whole pass and is
/// returned: a blob quietly skipped is a hole in the map and an exit code of zero, which is the
/// failure this tool can least afford.
///
/// `keep` returns the id span it saw, if it saw one. A pass that already walks every entity in a
/// block knows its own bounds for nothing, and asking the block for them separately would decode
/// nine billion delta-coded ids a second time.
fn scan<T, Keep>(
    planet: &Planet,
    blobs: &[usize],
    progress: &(dyn Fn(&str) + Sync),
    label: &str,
    keep: Keep,
) -> Result<Scanned<T>, PbfError>
where
    T: Send,
    Keep: Fn(&Block<'_>, &mut Vec<T>, &mut Vec<NamedNode>) -> Result<Option<IdSpan>, PbfError> + Sync + Send,
{
    let seen = AtomicUsize::new(0);
    let total = blobs.len();
    if total > 0 {
        progress(&blobs_read(label, 0, total));
    }

    let scanned = blobs
        .par_iter()
        .try_fold(Scanned::<T>::default, |mut scanned, at| {
            let at = *at;
            let mut buffers = std::mem::take(&mut scanned.buffers);
            planet.inflate_blob(at, &mut buffers.payload, &mut buffers.inflated)?;

            {
                let decoded = block(&buffers.inflated)?;
                if let Some(span) = keep(&decoded, &mut scanned.kept, &mut scanned.named)? {
                    scanned.spans.push((at, span));
                }
            }

            scanned.blobs += 1;
            scanned.inflated_bytes += buffers.inflated.len() as u64;
            scanned.buffers = buffers;
            let done = seen.fetch_add(1, Ordering::Relaxed) + 1;
            if done.is_multiple_of(BLOBS_BETWEEN_PROGRESS_LINES) || done == total {
                progress(&blobs_read(label, done, total));
            }
            Ok::<_, PbfError>(scanned)
        })
        .try_reduce(Scanned::default, |left, right| Ok(left.merged(right)))?;

    Ok(scanned)
}

/// How far a pass has got, as its progress line says it.
fn blobs_read(pass: &str, done: usize, total: usize) -> String {
    format!("{pass}: {done} / {total} blobs")
}

fn cost<T>(scanned: &Scanned<T>, elapsed: Duration) -> PassCost {
    PassCost {
        elapsed,
        blobs: scanned.blobs,
        inflated_bytes: scanned.inflated_bytes,
        kept: scanned.kept.len() as u64,
    }
}

fn record_spans<T>(index: &mut IdIndex, scanned: &Scanned<T>) {
    for (at, span) in &scanned.spans {
        index.record(*at, *span);
    }
}

fn every_blob(range: std::ops::Range<usize>) -> Vec<usize> {
    range.collect()
}

fn sorted_distinct(ids: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut ids: Vec<i64> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn holds(sorted: &[i64], id: i64) -> bool {
    sorted.binary_search(&id).is_ok()
}

fn nodes_inside(
    planet: &Planet,
    sections: &Sections,
    area: &Area,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Scanned<Node>, PbfError> {
    let collected = area.collected();

    scan(
        planet,
        &every_blob(sections.nodes.clone()),
        progress,
        "nodes",
        move |decoded, kept, named| {
            let window = Window::of(
                &collected,
                decoded.granularity,
                decoded.lat_offset,
                decoded.lon_offset,
            );
            let mut seen = Span::default();

            // Resolved once per block, so the test against every node in it is an integer compare.
            // A block whose string table never mentions `name` cannot hold a named node, and is
            // walked without touching the tag stream at all.
            let name_key = decoded.string_id(b"name");

            for group in &decoded.groups {
                let Some(dense) = dense_group(group)? else {
                    continue;
                };

                let Some(name_key) = name_key else {
                    dense.visit(&mut |id, latitude, longitude| {
                        seen.saw(id);
                        if window.holds(latitude, longitude) {
                            kept.push(Node {
                                id,
                                latitude: decoded.nanodegrees_latitude(latitude),
                                longitude: decoded.nanodegrees_longitude(longitude),
                            });
                        }
                    })?;
                    continue;
                };

                dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                    seen.saw(id);
                    if !window.holds(latitude, longitude) {
                        return;
                    }

                    let latitude = decoded.nanodegrees_latitude(latitude);
                    let longitude = decoded.nanodegrees_longitude(longitude);
                    kept.push(Node {
                        id,
                        latitude,
                        longitude,
                    });

                    if tags.iter().any(|(key, _)| *key == name_key) {
                        named.push(NamedNode {
                            id,
                            latitude,
                            longitude,
                            tags: tags
                                .iter()
                                .map(|(key, value)| {
                                    (
                                        text(decoded.string(*key as usize)),
                                        text(decoded.string(*value as usize)),
                                    )
                                })
                                .collect(),
                        });
                    }
                })?;
            }
            Ok(seen.into_span(Section::Nodes))
        },
    )
}

fn ways_touching(
    planet: &Planet,
    blobs: &[usize],
    held: &[i64],
    progress: &(dyn Fn(&str) + Sync),
    label: &str,
) -> Result<Scanned<Way>, PbfError> {
    let filter = IdFilter::of(held.iter(), held.len());

    scan(planet, blobs, progress, label, move |decoded, kept, _named| {
        let mut seen = Span::default();

        for group in &decoded.groups {
            visit_ways(group, &mut |way| {
                seen.saw(way.id);
                let touches = way
                    .refs()
                    .any(|reference| reference.is_ok_and(|id| filter.may_hold(id) && holds(held, id)));
                if !touches {
                    return Ok(());
                }
                kept.push(Way {
                    id: way.id,
                    tags: named_tags(way.tags(decoded)),
                    refs: way.refs().collect::<Result<Vec<_>, _>>()?,
                });
                Ok(())
            })?;
        }
        Ok(seen.into_span(Section::Ways))
    })
}

fn ways_with_id(
    planet: &Planet,
    blobs: &[usize],
    wanted: &[i64],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Scanned<Way>, PbfError> {
    let filter = IdFilter::of(wanted.iter(), wanted.len());

    scan(
        planet,
        blobs,
        progress,
        "member ways",
        move |decoded, kept, _named| {
            for group in &decoded.groups {
                visit_ways(group, &mut |way| {
                    if !filter.may_hold(way.id) || !holds(wanted, way.id) {
                        return Ok(());
                    }
                    kept.push(Way {
                        id: way.id,
                        tags: named_tags(way.tags(decoded)),
                        refs: way.refs().collect::<Result<Vec<_>, _>>()?,
                    });
                    Ok(())
                })?;
            }
            Ok(None)
        },
    )
}

fn nodes_with_id(
    planet: &Planet,
    blobs: &[usize],
    wanted: &[i64],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Scanned<Node>, PbfError> {
    let filter = IdFilter::of(wanted.iter(), wanted.len());

    scan(
        planet,
        blobs,
        progress,
        "member nodes",
        move |decoded, kept, _named| {
            for group in &decoded.groups {
                let Some(dense) = dense_group(group)? else {
                    continue;
                };
                dense.visit(&mut |id, latitude, longitude| {
                    if filter.may_hold(id) && holds(wanted, id) {
                        kept.push(Node {
                            id,
                            latitude: decoded.nanodegrees_latitude(latitude),
                            longitude: decoded.nanodegrees_longitude(longitude),
                        });
                    }
                })?;
            }
            Ok(None)
        },
    )
}

/// Multipolygons at least one of whose member ways the bbox pass already found.
///
/// Filtered on `type=multipolygon` and nothing else. What is worth drawing is the renderer's
/// question, and a harvest that answered it would be a harvest that could not be re-themed.
fn multipolygons_touching(
    planet: &Planet,
    sections: &Sections,
    held_ways: &[i64],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Scanned<Relation>, PbfError> {
    scan(
        planet,
        &every_blob(sections.relations.clone()),
        progress,
        "relations",
        move |decoded, kept, _named| {
            for group in &decoded.groups {
                visit_relations(group, &mut |relation| {
                    let tags = named_tags(relation.tags(decoded));
                    if super::tag(&tags, "type") != Some(MULTIPOLYGON) {
                        return Ok(());
                    }

                    let members: Vec<Member> = relation
                        .ways(decoded)
                        .into_iter()
                        .map(|(way, role)| Member {
                            way,
                            role: text(role),
                        })
                        .collect();

                    if members.iter().any(|member| holds(held_ways, member.way)) {
                        kept.push(Relation {
                            id: relation.id,
                            tags,
                            members,
                        });
                    }
                    Ok(())
                })?;
            }
            Ok(None)
        },
    )
}

fn named_tags(tags: Vec<(&[u8], &[u8])>) -> Tags {
    tags.into_iter()
        .map(|(key, value)| (text(key), text(value)))
        .collect()
}

fn provenance_of(planet: &Planet, source_name: &str) -> Provenance {
    Provenance {
        source_name: source_name.to_string(),
        file_size: planet.file_size,
        blob_count: planet.blobs.len() as u64,
        replication_timestamp: planet.header.replication_timestamp.unwrap_or_default(),
        replication_sequence: planet.header.replication_sequence.unwrap_or_default(),
        writing_program: planet.header.writing_program.clone().unwrap_or_default(),
    }
}

/// Everything the planet holds for one area, and what it cost to read.
pub fn harvest_area(
    planet: &Planet,
    source_name: &str,
    area: &Area,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<(Harvest, HarvestReport), ClipError> {
    let sections = planet.sections()?;
    let mut report = HarvestReport::default();
    let mut index = IdIndex::empty(planet.blobs.len());

    let started = Instant::now();
    let found_nodes = nodes_inside(planet, &sections, area, progress)?;
    report.nodes = cost(&found_nodes, started.elapsed());
    record_spans(&mut index, &found_nodes);

    let node_ids = sorted_distinct(found_nodes.kept.iter().map(|node| node.id));

    let started = Instant::now();
    let found_ways = ways_touching(
        planet,
        &every_blob(sections.ways.clone()),
        &node_ids,
        progress,
        "ways",
    )?;
    report.ways = cost(&found_ways, started.elapsed());
    record_spans(&mut index, &found_ways);

    let held_ways = sorted_distinct(found_ways.kept.iter().map(|way| way.id));

    let started = Instant::now();
    let found_relations = multipolygons_touching(planet, &sections, &held_ways, progress)?;
    report.relations = cost(&found_relations, started.elapsed());

    let member_ways = sorted_distinct(
        found_relations
            .kept
            .iter()
            .flat_map(|relation| relation.members.iter().map(|m| m.way)),
    );
    let missing_ways: Vec<i64> = member_ways
        .iter()
        .copied()
        .filter(|way| !holds(&held_ways, *way))
        .collect();

    let way_blobs = index.blobs_holding(sections.ways.clone(), &missing_ways);
    report.blobs_pruned += (sections.ways.len() - way_blobs.len()) as u64;

    let started = Instant::now();
    let fetched_ways = ways_with_id(planet, &way_blobs, &missing_ways, progress)?;
    report.member_ways = cost(&fetched_ways, started.elapsed());

    // A member the bbox pass already held is not thereby complete: it was kept because one of its
    // nodes fell in the box, and the rest of it can run far outside. One gap means the ring never
    // closes, and the lagoon stays dry while every count in the report looks healthy.
    let missing_nodes = sorted_distinct(
        found_ways
            .kept
            .iter()
            .chain(fetched_ways.kept.iter())
            .filter(|way| holds(&member_ways, way.id))
            .flat_map(|way| way.refs.iter().copied())
            .filter(|node| !holds(&node_ids, *node)),
    );

    let node_blobs = index.blobs_holding(sections.nodes.clone(), &missing_nodes);
    report.blobs_pruned += (sections.nodes.len() - node_blobs.len()) as u64;

    let started = Instant::now();
    let fetched_nodes = nodes_with_id(planet, &node_blobs, &missing_nodes, progress)?;
    report.member_nodes = cost(&fetched_nodes, started.elapsed());

    let harvest = Harvest {
        provenance: provenance_of(planet, source_name),
        collected: area.collected(),
        nodes: [found_nodes.kept, fetched_nodes.kept].concat(),
        named_nodes: found_nodes.named,
        ways: [found_ways.kept, fetched_ways.kept].concat(),
        relations: found_relations.kept,
    }
    .sorted();

    Ok((harvest, report))
}
