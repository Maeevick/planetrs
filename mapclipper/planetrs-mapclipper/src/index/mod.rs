//! Filing what the planet holds under the countries asked for, as each country's map index, so that
//! a map of any box inside them is cut from the dataset rather than from the planet.
//!
//! A country's map index holds what passes one to five of a harvest would take out of the planet for
//! any box inside it: every way with a node in the country, whole; every node of the country that
//! carries a name or that no way runs through, the others standing in their ways; and every
//! multipolygon with a member among those ways, each member way whole with every node placed.

mod countries;
mod scan;

use std::path::Path;

use planetrs_dataset::directory::record;
use planetrs_dataset::map_index::{self, Element, Grid, Line, Position, Tags};
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

/// What one country's map index holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryIndexed {
    /// The country's ISO 3166-1 code.
    pub country: String,
    /// Nodes kept for a name or for standing alone.
    pub nodes: usize,
    /// Ways with a node in the country.
    pub ways: usize,
    /// Multipolygons with a member among those ways.
    pub multipolygons: usize,
}

/// Builds the map index of each country of `countries` from the planet file at `planet`, and records
/// each in the dataset in `directory`.
///
/// `progress` hears how many blobs each pass has read, as a harvest's caller does.
pub fn build_map_index(
    planet: &Path,
    directory: &Path,
    countries: &[&str],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Vec<CountryIndexed>, ClipError> {
    let path = planet;
    let planet = Planet::open(path)?;
    let source = Provenance::of(path, &planet);
    let sections = planet.sections()?;
    let territory = countries_in(&planet, &sections, countries, progress)?;
    let codes: Vec<String> = territory.codes().map(str::to_string).collect();
    let mut index = IdIndex::empty(planet.blobs.len());

    let largest_node = largest_id(&planet, sections.nodes.clone())?;
    let members: Vec<IdentifierSet> = codes.iter().map(|_| IdentifierSet::up_to(largest_node)).collect();
    let nodes = scan(
        &planet,
        &every_blob(sections.nodes.clone()),
        "nodes",
        progress,
        NodesFound::default,
        |found, decoded| visit_nodes(found, decoded, &territory, &members),
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
            |found, decoded| visit_country_ways(found, decoded, &members, &referenced),
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
        tags: way.tags.clone(),
        refs: way.refs.clone(),
        positions: way.refs.iter().map(|node| position_of(*node)).collect(),
    };
    codes
        .iter()
        .enumerate()
        .map(|(country, code)| {
            let bit = 1u64 << country;
            let mut elements: Vec<Element> = named
                .iter()
                .filter(|node| node.country == country)
                .map(|node| Element::Node {
                    id: node.id,
                    at: position_on(grid, node.latitude, node.longitude),
                    tags: node.tags.clone(),
                })
                .collect();
            let named_here = sorted_distinct(
                named
                    .iter()
                    .filter(|node| node.country == country)
                    .map(|node| node.id),
            );
            elements.extend(
                positions
                    .iter()
                    .filter(|(id, _, _)| {
                        members[country].contains(*id)
                            && !referenced.contains(*id)
                            && named_here.binary_search(id).is_err()
                    })
                    .map(|(id, latitude, longitude)| Element::Node {
                        id: *id,
                        at: position_on(grid, *latitude, *longitude),
                        tags: Tags::new(),
                    }),
            );
            let nodes = elements.len();
            elements.extend(
                ways.iter()
                    .filter(|way| way.countries & bit != 0)
                    .map(|way| Element::Way(line_of(way))),
            );
            let way_count = elements.len() - nodes;
            elements.extend(
                multipolygons
                    .iter()
                    .filter(|multipolygon| multipolygon.countries & bit != 0)
                    .map(|multipolygon| Element::Multipolygon {
                        id: multipolygon.id,
                        tags: multipolygon.tags.clone(),
                        members: multipolygon.members.clone(),
                        ways: first_named(multipolygon.members.iter().map(|(way, _)| *way))
                            .into_iter()
                            .filter_map(member_way)
                            .map(line_of)
                            .collect(),
                    }),
            );
            let multipolygon_count = elements.len() - nodes - way_count;
            let rows = elements
                .iter()
                .map(|element| match element {
                    Element::Multipolygon { ways, .. } => 1 + ways.len() as u64,
                    _ => 1,
                })
                .sum();
            let bytes = map_index::to_parquet(code, &elements, grid, &source).map_err(ClipError::Encoding)?;
            record(
                directory,
                &source,
                Part::MapIndex {
                    country: code.clone(),
                },
                &format!("map-{code}.parquet"),
                &bytes,
                rows,
            )
            .map_err(|failure| ClipError::Recording {
                directory: directory.to_path_buf(),
                source: failure,
            })?;
            Ok(CountryIndexed {
                country: code.clone(),
                nodes,
                ways: way_count,
                multipolygons: multipolygon_count,
            })
        })
        .collect()
}

/// A position stored on the planet's grid, in its 32-bit units.
type Stored = (i64, i32, i32);

/// A named node of a country, with every tag it carries.
struct Named {
    country: usize,
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

/// A way kept whole, with the countries it touches as one bit each.
struct WayKept {
    id: i64,
    tags: Tags,
    refs: Vec<i64>,
    countries: u64,
}

/// A multipolygon kept with its way members, and the countries its members touch.
struct MultipolygonKept {
    id: i64,
    tags: Tags,
    members: Vec<(i64, String)>,
    countries: u64,
}

fn visit_nodes(
    found: &mut NodesFound,
    decoded: &Block<'_>,
    territory: &Countries,
    members: &[IdentifierSet],
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
            let country = territory.country_of([decoded.longitude(longitude), decoded.latitude(latitude)])?;
            let (Ok(latitude), Ok(longitude)) = (i32::try_from(latitude), i32::try_from(longitude)) else {
                found.out_of_range = true;
                return None;
            };
            members[country].insert(id);
            found.positions.push((id, latitude, longitude));
            Some((country, latitude, longitude))
        };
        let mut named = Vec::new();
        match name_key {
            None => dense.visit(&mut |id, latitude, longitude| {
                keep(id, latitude, longitude);
            })?,
            Some(name_key) => dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                let Some((country, latitude, longitude)) = keep(id, latitude, longitude) else {
                    return;
                };
                if tags.iter().any(|(key, _)| *key == name_key) {
                    named.push(Named {
                        country,
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
    members: &[IdentifierSet],
    referenced: &IdentifierSet,
) -> Result<Option<IdSpan>, PbfError> {
    let mut seen = Span::new();
    for group in &decoded.groups {
        visit_ways(group, &mut |way| {
            seen.saw(way.id);
            let mut countries = 0u64;
            for reference in way.refs() {
                let node = reference?;
                for (country, set) in members.iter().enumerate() {
                    if set.contains(node) {
                        countries |= 1 << country;
                    }
                }
            }
            if countries == 0 {
                return Ok(());
            }
            let refs = way.refs().collect::<Result<Vec<_>, _>>()?;
            for node in &refs {
                referenced.insert(*node);
            }
            found.push(WayKept {
                id: way.id,
                tags: named_tags(way.tags(decoded)),
                refs,
                countries,
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
            let countries = members
                .iter()
                .filter_map(|(way, _)| kept(ways, *way))
                .fold(0, |all, way| all | way.countries);
            if countries != 0 {
                found.push(MultipolygonKept {
                    id: relation.id,
                    tags,
                    members,
                    countries,
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
                    tags: named_tags(way.tags(decoded)),
                    refs: way.refs().collect::<Result<Vec<_>, _>>()?,
                    countries: 0,
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
