//! Filing what the planet holds under the countries asked for, as each country's map index, so that
//! a map of any box inside them is cut from the dataset rather than from the planet.
//!
//! A country's map index holds what passes one to five of a harvest would take out of the planet for
//! any box inside it: every way with a node in the country, whole; every node of the country that
//! carries a name or that no way runs through, the others standing in their ways; and every
//! multipolygon with a member among those ways, each member way whole with every node placed.

mod countries;
mod scan;

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use planetrs_dataset::directory::{Written, record_all_written, written_beside};
use planetrs_dataset::map_index::{Element, Grid, Line, Position, Scope, Summary, Tags, Tiling};
use planetrs_dataset::{Part, Provenance};
use planetrs_geo::territory::IdentifierSet;
use planetrs_pbf::{
    Block, IdIndex, IdSpan, PbfError, Planet, Section, block, dense_group, id_span, visit_relations,
    visit_ways,
};
use rayon::prelude::*;

use self::countries::{Countries, countries_in};
use self::scan::{Scanned, Span, every_blob, scan};
use crate::error::ClipError;
use crate::harvest::tag;

const MULTIPOLYGON: &str = "multipolygon";

/// The finest grid a map index holds positions on: a stored unit of 100 nanodegrees, as every
/// planet published stores them, keeps a position in 32 bits.
const FINEST_GRANULARITY: i64 = 100;

/// What a map index holds, once built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapIndexed {
    /// The countries it was built for, by ISO 3166-1 code.
    pub countries: Vec<String>,
    /// Nodes kept for a name or for standing alone.
    pub nodes: usize,
    /// Ways with a node in one of the countries.
    pub ways: usize,
    /// Multipolygons with a member among those ways.
    pub multipolygons: usize,
    /// The tiles they were cut into.
    pub tiles: usize,
}

/// Builds the map index of the countries `countries` from the planet file at `planet`, cut into
/// tiles, and records it in the dataset in `directory` in place of any map index it held.
///
/// Its tiles weigh at most `tile_weight` each (see [`planetrs_dataset::map_index::TILE_WEIGHT`]), but where one
/// place of the finest quadtree weighs more. `progress` hears how many blobs each pass has read, as a
/// harvest's caller does.
pub fn build_map_index(
    planet: &Path,
    directory: &Path,
    countries: &[&str],
    tile_weight: u64,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<MapIndexed, ClipError> {
    let path = planet;
    let planet = Planet::open(path)?;
    let source = Provenance::of(path, &planet);
    let sections = planet.sections()?;
    let territory = countries_in(&planet, &sections, countries, progress)?;
    let codes: Vec<String> = territory.codes().map(str::to_string).collect();
    let coverage = territory.coverage();
    let mut index = IdIndex::empty(planet.blobs.len());

    let largest_node = largest_id(&planet, sections.nodes.clone())?;
    let inside = IdentifierSet::up_to(largest_node);
    let nodes = scan(
        &planet,
        &every_blob(sections.nodes.clone()),
        "nodes",
        progress,
        NodesFound::default,
        |found, decoded| visit_nodes(found, decoded, &territory, &inside),
        NodesFound::absorb,
    )?;
    let NodesFound {
        mut positions,
        named,
        grids,
        out_of_range,
    } = recorded(&mut index, nodes);
    let grid = grid_of(&grids, out_of_range)?;
    positions.par_sort_unstable_by_key(|(id, _, _)| *id);

    let referenced = IdentifierSet::up_to(largest_node);
    let mut ways = recorded(
        &mut index,
        scan(
            &planet,
            &every_blob(sections.ways.clone()),
            "ways",
            progress,
            Vec::new,
            |found, decoded| visit_country_ways(found, decoded, &inside, &referenced),
            concatenated,
        )?,
    );
    ways.par_sort_unstable_by_key(|way| way.id);

    let mut multipolygons = scan(
        &planet,
        &every_blob(sections.relations.clone()),
        "relations",
        progress,
        Vec::new,
        |found, decoded| visit_multipolygons(found, decoded, &ways),
        concatenated,
    )?
    .found;
    multipolygons.sort_unstable_by_key(|multipolygon| multipolygon.id);

    let missing_ways = sorted_distinct(
        multipolygons
            .iter()
            .flat_map(|multipolygon| multipolygon.members.iter().map(|(way, _)| *way))
            .filter(|way| kept(&ways, *way).is_none()),
    );
    let mut fetched = scan(
        &planet,
        &index.blobs_holding(sections.ways.clone(), &missing_ways),
        "member ways",
        progress,
        Vec::new,
        |found, decoded| visit_ways_with_id(found, decoded, &missing_ways),
        concatenated,
    )?
    .found;
    fetched.sort_unstable_by_key(|way| way.id);

    let member_way = |id: i64| kept(&ways, id).or_else(|| kept(&fetched, id));
    let missing_nodes = sorted_distinct(
        multipolygons
            .iter()
            .flat_map(|multipolygon| {
                multipolygon
                    .members
                    .iter()
                    .filter_map(|(way, _)| member_way(*way))
            })
            .flat_map(|way| way.refs.iter().copied())
            .filter(|node| placed(&positions, *node).is_none()),
    );
    let mut elsewhere = scan(
        &planet,
        &index.blobs_holding(sections.nodes.clone(), &missing_nodes),
        "member nodes",
        progress,
        Vec::new,
        |found, decoded| visit_nodes_with_id(found, decoded, &missing_nodes),
        concatenated,
    )?
    .found;
    elsewhere.sort_unstable_by_key(|(id, _, _)| *id);

    let position_of = |id: i64| {
        placed(&positions, id)
            .or_else(|| placed(&elsewhere, id))
            .map(|(latitude, longitude)| position_on(grid, latitude, longitude))
    };
    let line_of = |way: &WayKept| Line {
        id: way.id,
        tags: way.tags.unpacked(),
        refs: way.refs.to_vec(),
        positions: way.refs.iter().map(|node| position_of(*node)).collect(),
    };
    let named_ids = sorted_distinct(named.iter().map(|node| node.id));
    let lone: Vec<&Stored> = positions
        .iter()
        .filter(|(id, _, _)| !referenced.contains(*id) && named_ids.binary_search(id).is_err())
        .collect();
    let member_ways_of = |multipolygon: &MultipolygonKept| -> Vec<&WayKept> {
        first_named(multipolygon.members.iter().map(|(way, _)| *way))
            .into_iter()
            .filter_map(member_way)
            .collect()
    };
    let (nodes, lone_from) = (named.len() + lone.len(), named.len());
    let ways_from = nodes;
    let multipolygons_from = ways_from + ways.len();
    let count = multipolygons_from + multipolygons.len();
    let element_at = |at: usize| -> Element {
        if at < lone_from {
            let node = &named[at];
            Element::Node {
                id: node.id,
                at: position_on(grid, node.latitude, node.longitude),
                tags: node.tags.clone(),
            }
        } else if at < ways_from {
            let (id, latitude, longitude) = *lone[at - lone_from];
            Element::Node {
                id,
                at: position_on(grid, latitude, longitude),
                tags: Tags::new(),
            }
        } else if at < multipolygons_from {
            Element::Way(line_of(&ways[at - ways_from]))
        } else {
            let multipolygon = &multipolygons[at - multipolygons_from];
            Element::Multipolygon {
                id: multipolygon.id,
                tags: multipolygon.tags.clone(),
                members: multipolygon.members.clone(),
                ways: member_ways_of(multipolygon).into_iter().map(line_of).collect(),
            }
        }
    };
    let tiling = Tiling::of(count, element_at, tile_weight).map_err(ClipError::Encoding)?;
    std::fs::create_dir_all(directory).map_err(|failure| ClipError::writing(directory, failure))?;
    let mut written = Vec::new();
    for (which, tile) in tiling.tiles().into_iter().enumerate() {
        let name = tile.file_name();
        let beside = written_beside(directory, &name);
        let writing = |failure| ClipError::writing(&beside, failure);
        let file = File::create(&beside).map_err(writing)?;
        let done = tiling
            .write(which, BufWriter::new(file), element_at, grid)
            .map_err(ClipError::Encoding)?;
        done.out
            .into_inner()
            .map_err(|failure| writing(failure.into_error()))?
            .sync_all()
            .map_err(writing)?;
        written.push(Written {
            part: Part::MapTile {
                level: tile.level,
                x: tile.x,
                y: tile.y,
            },
            name,
            rows: done.rows,
            bbox: Some(done.bbox),
        });
    }
    let tiles = written.len();
    let summary = Summary {
        scope: Scope::Countries(codes.clone()),
        grid,
        coverage,
    };
    let beside = written_beside(directory, SUMMARY);
    std::fs::write(&beside, summary.to_json()).map_err(|failure| ClipError::writing(&beside, failure))?;
    written.insert(
        0,
        Written {
            part: Part::MapSummary,
            name: SUMMARY.into(),
            rows: tiles as u64,
            bbox: None,
        },
    );
    record_all_written(directory, &source, is_the_map_index, written).map_err(|failure| {
        ClipError::Recording {
            directory: directory.to_path_buf(),
            source: failure,
        }
    })?;
    Ok(MapIndexed {
        countries: codes,
        nodes,
        ways: ways.len(),
        multipolygons: multipolygons.len(),
        tiles,
    })
}

/// The name of a map index's summary in a dataset.
const SUMMARY: &str = "map-summary.json";

/// Whether a part of a dataset belongs to the map index, which a build replaces whole.
fn is_the_map_index(part: &Part) -> bool {
    matches!(
        part,
        Part::MapSummary | Part::MapTile { .. } | Part::MapIndex { .. }
    )
}

/// A position stored on the planet's grid, in its 32-bit units.
type Stored = (i64, i32, i32);

/// A named node of a country, with every tag it carries.
struct Named {
    id: i64,
    latitude: i32,
    longitude: i32,
    tags: Tags,
}

/// What the pass over every node found: each node of a country, its position on the grid, and the
/// named ones with their tags.
#[derive(Default)]
struct NodesFound {
    positions: Vec<Stored>,
    named: Vec<Named>,
    grids: Vec<Grid>,
    out_of_range: bool,
}

impl NodesFound {
    fn absorb(mut self, other: Self) -> Self {
        self.positions.extend(other.positions);
        self.named.extend(other.named);
        for grid in other.grids {
            if !self.grids.contains(&grid) {
                self.grids.push(grid);
            }
        }
        self.out_of_range |= other.out_of_range;
        self
    }
}

/// A way kept whole, as small as it can be held: France keeps tens of millions of them at once.
struct WayKept {
    id: i64,
    tags: PackedTags,
    refs: Box<[i64]>,
}

/// Tags as the planet's bytes in one allocation, each key and value as its length on four bytes
/// then its bytes, read as text only when written.
struct PackedTags(Box<[u8]>);

impl PackedTags {
    fn of(tags: &[(&[u8], &[u8])]) -> Self {
        let mut packed =
            Vec::with_capacity(tags.iter().map(|(key, value)| 8 + key.len() + value.len()).sum());
        for text in tags.iter().flat_map(|(key, value)| [key, value]) {
            packed.extend_from_slice(&(text.len() as u32).to_le_bytes());
            packed.extend_from_slice(text);
        }
        Self(packed.into_boxed_slice())
    }

    /// The tags as text, invalid UTF-8 replaced as a harvest replaces it.
    fn unpacked(&self) -> Tags {
        let mut rest: &[u8] = &self.0;
        let mut next_text = || {
            let (length, after) = rest.split_first_chunk::<4>()?;
            let length = u32::from_le_bytes(*length) as usize;
            let (bytes, after) = after.split_at(length.min(after.len()));
            rest = after;
            Some(text(bytes))
        };
        let mut tags = Tags::new();
        while let (Some(key), Some(value)) = (next_text(), next_text()) {
            tags.push((key, value));
        }
        tags
    }
}

/// A multipolygon kept with its way members.
struct MultipolygonKept {
    id: i64,
    tags: Tags,
    members: Vec<(i64, String)>,
}

fn visit_nodes(
    found: &mut NodesFound,
    decoded: &Block<'_>,
    territory: &Countries,
    inside: &IdentifierSet,
) -> Result<Option<IdSpan>, PbfError> {
    let grid = Grid {
        granularity: decoded.granularity,
        latitude_offset: decoded.lat_offset,
        longitude_offset: decoded.lon_offset,
    };
    if !found.grids.contains(&grid) {
        found.grids.push(grid);
    }
    let mut seen = Span::new();
    // A block whose strings never mention `name` holds no named node, and is walked without its tags.
    let name_key = decoded.string_id(b"name");
    for group in &decoded.groups {
        let Some(dense) = dense_group(group)? else {
            continue;
        };
        let mut keep = |id: i64, latitude: i64, longitude: i64| {
            seen.saw(id);
            if !territory.hold([decoded.longitude(longitude), decoded.latitude(latitude)]) {
                return None;
            }
            let (Ok(latitude), Ok(longitude)) = (i32::try_from(latitude), i32::try_from(longitude)) else {
                found.out_of_range = true;
                return None;
            };
            inside.insert(id);
            found.positions.push((id, latitude, longitude));
            Some((latitude, longitude))
        };
        let mut named = Vec::new();
        match name_key {
            None => dense.visit(&mut |id, latitude, longitude| {
                keep(id, latitude, longitude);
            })?,
            Some(name_key) => dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                let Some((latitude, longitude)) = keep(id, latitude, longitude) else {
                    return;
                };
                if tags.iter().any(|(key, _)| *key == name_key) {
                    named.push(Named {
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
            })?,
        }
        found.named.extend(named);
    }
    Ok(seen.of(Section::Nodes))
}

fn visit_country_ways(
    found: &mut Vec<WayKept>,
    decoded: &Block<'_>,
    inside: &IdentifierSet,
    referenced: &IdentifierSet,
) -> Result<Option<IdSpan>, PbfError> {
    let mut seen = Span::new();
    for group in &decoded.groups {
        visit_ways(group, &mut |way| {
            seen.saw(way.id);
            let mut touches = false;
            for reference in way.refs() {
                touches |= inside.contains(reference?);
            }
            if !touches {
                return Ok(());
            }
            let refs = way.refs().collect::<Result<Box<[i64]>, _>>()?;
            for node in &refs {
                referenced.insert(*node);
            }
            found.push(WayKept {
                id: way.id,
                tags: PackedTags::of(&way.tags(decoded)),
                refs,
            });
            Ok(())
        })?;
    }
    Ok(seen.of(Section::Ways))
}

fn visit_multipolygons(
    found: &mut Vec<MultipolygonKept>,
    decoded: &Block<'_>,
    ways: &[WayKept],
) -> Result<Option<IdSpan>, PbfError> {
    for group in &decoded.groups {
        visit_relations(group, &mut |relation| {
            let tags = named_tags(relation.tags(decoded));
            if tag(&tags, "type") != Some(MULTIPOLYGON) {
                return Ok(());
            }
            let members: Vec<(i64, String)> = relation
                .ways(decoded)
                .into_iter()
                .map(|(way, role)| (way, text(role)))
                .collect();
            if members.iter().any(|(way, _)| kept(ways, *way).is_some()) {
                found.push(MultipolygonKept {
                    id: relation.id,
                    tags,
                    members,
                });
            }
            Ok(())
        })?;
    }
    Ok(None)
}

fn visit_ways_with_id(
    found: &mut Vec<WayKept>,
    decoded: &Block<'_>,
    wanted: &[i64],
) -> Result<Option<IdSpan>, PbfError> {
    for group in &decoded.groups {
        visit_ways(group, &mut |way| {
            if wanted.binary_search(&way.id).is_ok() {
                found.push(WayKept {
                    id: way.id,
                    tags: PackedTags::of(&way.tags(decoded)),
                    refs: way.refs().collect::<Result<Box<[i64]>, _>>()?,
                });
            }
            Ok(())
        })?;
    }
    Ok(None)
}

fn visit_nodes_with_id(
    found: &mut Vec<Stored>,
    decoded: &Block<'_>,
    wanted: &[i64],
) -> Result<Option<IdSpan>, PbfError> {
    for group in &decoded.groups {
        if let Some(dense) = dense_group(group)? {
            dense.visit(&mut |id, latitude, longitude| {
                if wanted.binary_search(&id).is_ok()
                    && let (Ok(latitude), Ok(longitude)) = (i32::try_from(latitude), i32::try_from(longitude))
                {
                    found.push((id, latitude, longitude));
                }
            })?;
        }
    }
    Ok(None)
}

/// The pass's findings, its blobs' id spans recorded in the index for the passes that fetch by id.
fn recorded<T>(index: &mut IdIndex, scanned: Scanned<T>) -> T {
    for (at, span) in &scanned.spans {
        index.record(*at, *span);
    }
    scanned.found
}

/// The one grid every node block stores positions on, refused when there are several or when it is
/// finer than a map index holds.
fn grid_of(grids: &[Grid], out_of_range: bool) -> Result<Grid, ClipError> {
    match grids {
        [grid] if grid.granularity >= FINEST_GRANULARITY && !out_of_range => Ok(*grid),
        [grid] => Err(ClipError::Grid {
            detail: format!(
                "a unit of {} nanodegrees is finer than the {FINEST_GRANULARITY} a map index holds",
                grid.granularity
            ),
        }),
        [] => Err(ClipError::Grid {
            detail: "the planet holds no node".into(),
        }),
        _ => Err(ClipError::Grid {
            detail: format!(
                "its node blocks use {} grids, and a map index holds one",
                grids.len()
            ),
        }),
    }
}

fn position_on(grid: Grid, latitude: i32, longitude: i32) -> Position {
    Position {
        latitude: i64::from(latitude) * grid.granularity + grid.latitude_offset,
        longitude: i64::from(longitude) * grid.granularity + grid.longitude_offset,
    }
}

/// The stored position of a node, among positions sorted by id.
fn placed(positions: &[Stored], id: i64) -> Option<(i32, i32)> {
    positions
        .binary_search_by_key(&id, |(node, _, _)| *node)
        .ok()
        .map(|at| (positions[at].1, positions[at].2))
}

/// The kept way of this id, among ways sorted by id.
fn kept(ways: &[WayKept], id: i64) -> Option<&WayKept> {
    ways.binary_search_by_key(&id, |way| way.id)
        .ok()
        .map(|at| &ways[at])
}

/// The largest id of a section, read from its last blob.
fn largest_id(planet: &Planet, section: std::ops::Range<usize>) -> Result<i64, PbfError> {
    let Some(last) = section.end.checked_sub(1).filter(|last| *last >= section.start) else {
        return Ok(0);
    };
    let mut payload = Vec::new();
    let mut inflated = Vec::new();
    planet.inflate_blob(last, &mut payload, &mut inflated)?;
    let decoded = block(&inflated)?;
    Ok(id_span(&decoded)?.map(|span| span.last).unwrap_or(0))
}

fn sorted_distinct(ids: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut ids: Vec<i64> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Each id once, in the order it is first named.
fn first_named(ids: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut seen = Vec::new();
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

fn concatenated<T>(mut left: Vec<T>, right: Vec<T>) -> Vec<T> {
    left.extend(right);
    left
}

fn named_tags(tags: Vec<(&[u8], &[u8])>) -> Tags {
    tags.into_iter()
        .map(|(key, value)| (text(key), text(value)))
        .collect()
}

/// Text from the planet's bytes, as a harvest reads it: invalid UTF-8 replaced rather than refused.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
