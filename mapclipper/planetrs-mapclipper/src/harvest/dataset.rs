//! A harvest taken from a dataset's map indexes rather than from the planet: for any area inside the
//! countries indexed, the very harvest the planet gives, read from the few row groups around it.
//! An area reaching beyond them is refused, since the dataset would lack what the planet holds there.
//!
//! The five passes of a harvest are replayed over the elements near the area: the nodes inside the
//! window, tested on the planet's own grid; the ways touching them; the multipolygons touching those;
//! their member ways; and the members' nodes. The map index holds each member way whole, so nothing
//! has to be fetched by id.

use planetrs_dataset::map_index::{Element, Grid, Line, Position, Scope};
use planetrs_dataset::{Dataset, DatasetError, ForReadingFiles};
use planetrs_geo::territory::Coverage;
use planetrs_geo::{Area, BoundingBox, Window};

use super::{Harvest, Member, NamedNode, Node, Provenance, Relation, Way};
use crate::error::ClipError;

/// Everything the map indexes of `dataset` hold for one area, as the passes over the planet take it,
/// or [`ClipError::OutsideTheDataset`] when the area reaches beyond the countries they cover.
pub fn harvest_from_dataset<F: ForReadingFiles>(
    dataset: &Dataset<F>,
    area: &Area,
) -> Result<Harvest, ClipError> {
    let collected = area.collected();
    let summary = dataset.map_summary().map_err(|failure| match failure {
        DatasetError::NoMapIndex => ClipError::NoMapIndex,
        other => ClipError::Dataset(other),
    })?;
    let grid = summary.grid;
    let window = Window::of(
        &collected,
        grid.granularity,
        grid.latitude_offset,
        grid.longitude_offset,
    );
    let ground = ground_of(&window, grid);
    if !Coverage::cover(std::slice::from_ref(&summary.coverage), &ground) {
        return Err(ClipError::OutsideTheDataset {
            countries: match summary.scope {
                Scope::Countries(countries) => countries,
                Scope::Planet => Vec::new(),
            },
        });
    }
    let inside = |at: &Position| {
        window.holds(
            (at.latitude - grid.latitude_offset) / grid.granularity,
            (at.longitude - grid.longitude_offset) / grid.granularity,
        )
    };
    let bounds = [ground.west, ground.south, ground.east, ground.north];
    let mut elements = Vec::new();
    for tile in dataset.map_tiles_near(bounds)? {
        elements.extend(dataset.elements_near(&tile, bounds)?);
    }

    let mut nodes: Vec<Node> = Vec::new();
    let mut named_nodes: Vec<NamedNode> = Vec::new();
    let vertices_inside = |line: &Line, nodes: &mut Vec<Node>| {
        nodes.extend(
            line.refs
                .iter()
                .zip(&line.positions)
                .filter_map(|(id, at)| at.filter(|at| inside(at)).map(|at| node(*id, at))),
        );
    };
    for element in &elements {
        match element {
            Element::Node { id, at, tags } if inside(at) => {
                nodes.push(node(*id, *at));
                if tags.iter().any(|(key, _)| key == "name") {
                    named_nodes.push(NamedNode {
                        id: *id,
                        latitude: at.latitude,
                        longitude: at.longitude,
                        tags: tags.clone(),
                    });
                }
            }
            Element::Way(line) => vertices_inside(line, &mut nodes),
            Element::Multipolygon { ways, .. } => {
                for line in ways {
                    vertices_inside(line, &mut nodes);
                }
            }
            Element::Node { .. } => {}
        }
    }
    let node_ids = sorted_distinct(nodes.iter().map(|node| node.id));

    let held: Vec<Way> = elements
        .iter()
        .filter_map(|element| match element {
            Element::Way(line) if line.refs.iter().any(|id| holds(&node_ids, *id)) => Some(way_of(line)),
            _ => None,
        })
        .collect();
    let held_ids = sorted_distinct(held.iter().map(|way| way.id));

    let found: Vec<(Relation, &[Line])> = elements
        .iter()
        .filter_map(|element| match element {
            Element::Multipolygon {
                id,
                tags,
                members,
                ways,
            } if members.iter().any(|(way, _)| holds(&held_ids, *way)) => Some((
                Relation {
                    id: *id,
                    tags: tags.clone(),
                    members: members
                        .iter()
                        .map(|(way, role)| Member {
                            way: *way,
                            role: role.clone(),
                        })
                        .collect(),
                },
                ways.as_slice(),
            )),
            _ => None,
        })
        .collect();

    let member_lines: Vec<&Line> = found.iter().flat_map(|(_, ways)| ways.iter()).collect();
    let fetched: Vec<Way> = member_lines
        .iter()
        .filter(|line| !holds(&held_ids, line.id))
        .map(|line| way_of(line))
        .collect();
    let member_nodes: Vec<Node> = member_lines
        .iter()
        .flat_map(|line| line.refs.iter().zip(&line.positions))
        .filter(|(id, _)| !holds(&node_ids, **id))
        .filter_map(|(id, at)| at.map(|at| node(*id, at)))
        .collect();

    let planet = &dataset.manifest().planet;
    Ok(Harvest {
        provenance: Provenance {
            source_name: planet.source_name.clone(),
            file_size: planet.file_size,
            blob_count: planet.blob_count,
            replication_timestamp: planet.replication_timestamp,
            replication_sequence: planet.replication_sequence,
            writing_program: planet.writing_program.clone(),
        },
        collected,
        nodes: [nodes, member_nodes].concat(),
        named_nodes,
        ways: [held, fetched].concat(),
        relations: found.into_iter().map(|(relation, _)| relation).collect(),
    }
    .sorted())
}

/// The ground whose positions a window holds, in degrees: rounded outward from the area collected, it
/// is what the dataset must cover.
fn ground_of(window: &Window, grid: Grid) -> BoundingBox {
    let degrees = |units: i64, offset: i64| (units * grid.granularity + offset) as f64 / 1e9;
    BoundingBox {
        west: degrees(window.west, grid.longitude_offset),
        south: degrees(window.south, grid.latitude_offset),
        east: degrees(window.east, grid.longitude_offset),
        north: degrees(window.north, grid.latitude_offset),
    }
}

fn node(id: i64, at: Position) -> Node {
    Node {
        id,
        latitude: at.latitude,
        longitude: at.longitude,
    }
}

fn way_of(line: &Line) -> Way {
    Way {
        id: line.id,
        tags: line.tags.clone(),
        refs: line.refs.clone(),
    }
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
