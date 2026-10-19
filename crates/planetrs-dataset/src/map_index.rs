//! A country's map index: everything a map of any box inside the country takes from the planet,
//! filed so that a box reads only the few row groups around it.
//!
//! It holds every way with a node in the country, whole, with the position of each node the planet
//! places; every node of the country that carries a name or that no way runs through; and every
//! multipolygon with a member among those ways, followed by each of its member ways whole, so that a
//! map never has to fetch a member by its id. Positions come back to the nanodegree, exactly as the
//! planet stores them.

use planetrs_geo::wkb;

use crate::geoparquet;
use crate::manifest::{self, Provenance};
use rayon::prelude::*;

use crate::tables::{
    self, Column, Footer, TableError, TableWriter, Tail, Values, boxes, bytes, integer_lists, integers,
    optional_integers, small_integer_lists, text_lists, texts,
};

/// How many rows a row group holds before the next element starts another. An element is never
/// split, so a multipolygon and its members always share a row group.
const ROWS_PER_ROW_GROUP: usize = 50_000;

/// The footer key holding the planet's grid, as granularity, latitude offset and longitude offset.
const GRID: &str = "planetrs.grid";

/// The footer key holding the country the file is the map index of.
const COUNTRY: &str = "planetrs.country";

/// How finely the Hilbert curve orders boxes: a cell of 2^16 to a side over the whole planet is
/// about 600 metres, finer than any row group.
const HILBERT_ORDER: u32 = 16;

/// The tags of an element, in the order the planet file states them.
pub type Tags = Vec<(String, String)>;

/// A position as the planet stores it, in nanodegrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Nanodegrees north of the equator.
    pub latitude: i64,
    /// Nanodegrees east of Greenwich.
    pub longitude: i64,
}

/// The planet's grid for positions: what a stored unit is worth, and where the units start.
///
/// A box is tested in these units, as the planet's own blocks are, so a box keeps exactly the nodes
/// a reading of the planet keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// Nanodegrees per unit, 100 in every planet published.
    pub granularity: i64,
    /// Nanodegrees the latitude units start from.
    pub latitude_offset: i64,
    /// Nanodegrees the longitude units start from.
    pub longitude_offset: i64,
}

/// A way as the map index holds it: its tags, every node it runs through, and where each stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The way's OpenStreetMap id.
    pub id: i64,
    /// Its tags.
    pub tags: Tags,
    /// The ids of its nodes, in order.
    pub refs: Vec<i64>,
    /// One per node, none for a node the planet does not place.
    pub positions: Vec<Option<Position>>,
}

/// One element of a map index.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// A node of the country that carries a name, or that no way runs through.
    Node {
        /// The node's OpenStreetMap id.
        id: i64,
        /// Where it stands.
        at: Position,
        /// Its tags, none for a node kept only for its position.
        tags: Tags,
    },
    /// A way with a node in the country.
    Way(Line),
    /// A multipolygon with a member way among the country's, and each of its member ways whole.
    Multipolygon {
        /// The relation's OpenStreetMap id.
        id: i64,
        /// Its tags.
        tags: Tags,
        /// Its way members, as the id of each way and the role it plays.
        members: Vec<(i64, String)>,
        /// Each member way once, in the order the members first name it.
        ways: Vec<Line>,
    },
}

/// The map index of `country` as a GeoParquet file.
///
/// Elements are ordered along a Hilbert curve through the centres of their boxes, so the rows of one
/// box sit in a few row groups, and the row groups carry the bounds of their boxes for a reader to
/// pick them by. The same elements always give the same bytes, whatever order they come in.
pub fn to_parquet(
    country: &str,
    elements: &[Element],
    grid: Grid,
    planet: &Provenance,
) -> Result<Vec<u8>, TableError> {
    to_parquet_from(country, elements.len(), |at| elements[at].clone(), grid, planet)
}

/// [`to_parquet`] for `count` elements built by `element_at` when they are needed, twice each,
/// rather than held all at once: a country's map index is larger than memory.
pub fn to_parquet_from(
    country: &str,
    count: usize,
    element_at: impl Fn(usize) -> Element + Sync,
    grid: Grid,
    planet: &Provenance,
) -> Result<Vec<u8>, TableError> {
    written(country, count, element_at, grid, planet, ROWS_PER_ROW_GROUP)
}

/// [`to_parquet`], with row groups of about `rows_per_row_group` rows.
#[cfg(test)]
pub(crate) fn cut_at(
    country: &str,
    elements: &[Element],
    grid: Grid,
    planet: &Provenance,
    rows_per_row_group: usize,
) -> Result<Vec<u8>, TableError> {
    written(
        country,
        elements.len(),
        |at| elements[at].clone(),
        grid,
        planet,
        rows_per_row_group,
    )
}

fn written(
    country: &str,
    count: usize,
    element_at: impl Fn(usize) -> Element + Sync,
    grid: Grid,
    planet: &Provenance,
    rows_per_row_group: usize,
) -> Result<Vec<u8>, TableError> {
    let mut placed: Vec<(u64, (u8, i64), usize, usize)> = (0..count)
        .into_par_iter()
        .map(|at| {
            let element = element_at(at);
            Ok((
                hilbert_key(box_of(&element)?),
                order_of(&element),
                at,
                rows_in(&element),
            ))
        })
        .collect::<Result<_, TableError>>()?;
    placed.par_sort_unstable_by_key(|(key, order, _, _)| (*key, *order));
    let mut writer = TableWriter::new(
        &[
            geoparquet::geo_covered_by_bbox(&["Point", "LineString", "GeometryCollection"]),
            manifest::notice("map index", planet),
            (COUNTRY, country.to_string()),
            (
                GRID,
                format!(
                    "{},{},{}",
                    grid.granularity, grid.latitude_offset, grid.longitude_offset
                ),
            ),
        ],
        &["bbox.xmin", "bbox.ymin", "bbox.xmax", "bbox.ymax"],
    );
    let mut group: Vec<usize> = Vec::new();
    let mut rows = 0;
    let mut groups_written = 0;
    for (_, _, at, element_rows) in placed {
        group.push(at);
        rows += element_rows;
        if rows >= rows_per_row_group {
            write_group(&mut writer, &group, &element_at)?;
            groups_written += 1;
            group.clear();
            rows = 0;
        }
    }
    if !group.is_empty() || groups_written == 0 {
        write_group(&mut writer, &group, &element_at)?;
    }
    writer.finish()
}

/// Builds the elements at `group`, in that order, and writes their rows as one row group.
fn write_group(
    writer: &mut TableWriter,
    group: &[usize],
    element_at: &(impl Fn(usize) -> Element + Sync),
) -> Result<(), TableError> {
    let rows: Vec<Vec<Row>> = group
        .par_iter()
        .map(|at| rows_of(&element_at(*at)))
        .collect::<Result<_, TableError>>()?;
    writer.write(&columns_of(rows.into_iter().flatten().collect()))
}

/// The box of an element's own row: a node's point, a way's placed nodes, a multipolygon's members.
fn box_of(element: &Element) -> Result<[f64; 4], TableError> {
    match element {
        Element::Node { at, .. } => {
            let [longitude, latitude] = degrees(*at);
            Ok([longitude, latitude, longitude, latitude])
        }
        Element::Way(line) => bounds_of(line).ok_or_else(|| unplaced("way", line.id)),
        Element::Multipolygon { id, ways, .. } => ways
            .iter()
            .filter_map(bounds_of)
            .reduce(union)
            .ok_or_else(|| unplaced("multipolygon", *id)),
    }
}

/// How many rows an element is written as: its own, and one per member way of a multipolygon.
fn rows_in(element: &Element) -> usize {
    match element {
        Element::Multipolygon { ways, .. } => 1 + ways.len(),
        _ => 1,
    }
}

/// The elements of a map index file, with the grid its positions were read on.
pub fn from_parquet(bytes: &[u8]) -> Result<(Grid, Vec<Element>), TableError> {
    let Tail::Footer(footer) = tables::footer_from_tail(bytes, bytes.len() as u64)? else {
        return Err(unexpected("footer"));
    };
    let grid = grid_in(&footer).ok_or_else(|| unexpected(GRID))?;
    Ok((grid, elements_of(&tables::read(bytes)?)?))
}

/// One row of the file: an element, or one member way of a multipolygon.
struct Row {
    kind: &'static str,
    id: i64,
    relation: Option<i64>,
    tags: Tags,
    refs: Vec<i64>,
    unresolved: Vec<i32>,
    member_ways: Vec<i64>,
    member_roles: Vec<String>,
    geometry: Vec<u8>,
    bbox: [f64; 4],
}

/// Where an element sorts among those of the same Hilbert key: nodes, then ways, then
/// multipolygons, each by id.
fn order_of(element: &Element) -> (u8, i64) {
    match element {
        Element::Node { id, .. } => (0, *id),
        Element::Way(line) => (1, line.id),
        Element::Multipolygon { id, .. } => (2, *id),
    }
}

/// The rows an element is written as, its own first.
fn rows_of(element: &Element) -> Result<Vec<Row>, TableError> {
    match element {
        Element::Node { id, at, tags } => {
            let [longitude, latitude] = degrees(*at);
            Ok(vec![Row {
                kind: "node",
                id: *id,
                relation: None,
                tags: tags.clone(),
                refs: Vec::new(),
                unresolved: Vec::new(),
                member_ways: Vec::new(),
                member_roles: Vec::new(),
                geometry: wkb::point(longitude, latitude),
                bbox: [longitude, latitude, longitude, latitude],
            }])
        }
        Element::Way(line) => Ok(vec![line_row("way", line, None, box_of(element)?)]),
        Element::Multipolygon {
            id,
            tags,
            members,
            ways,
        } => {
            let bbox = box_of(element)?;
            let own = Row {
                kind: "multipolygon",
                id: *id,
                relation: None,
                tags: tags.clone(),
                refs: Vec::new(),
                unresolved: Vec::new(),
                member_ways: members.iter().map(|(way, _)| *way).collect(),
                member_roles: members.iter().map(|(_, role)| role.clone()).collect(),
                geometry: wkb::empty_collection(),
                bbox,
            };
            Ok(std::iter::once(own)
                .chain(ways.iter().map(|line| {
                    let own_bbox = bounds_of(line).unwrap_or(bbox);
                    line_row("member", line, Some(*id), own_bbox)
                }))
                .collect())
        }
    }
}

fn line_row(kind: &'static str, line: &Line, relation: Option<i64>, bbox: [f64; 4]) -> Row {
    let vertices: Vec<[f64; 2]> = line.positions.iter().flatten().map(|at| degrees(*at)).collect();
    Row {
        kind,
        id: line.id,
        relation,
        tags: line.tags.clone(),
        refs: line.refs.clone(),
        unresolved: line
            .positions
            .iter()
            .enumerate()
            .filter(|(_, at)| at.is_none())
            .map(|(index, _)| index as i32)
            .collect(),
        member_ways: Vec::new(),
        member_roles: Vec::new(),
        geometry: wkb::line_string(&vertices),
        bbox,
    }
}

/// The box around a line's placed nodes, if any is placed.
fn bounds_of(line: &Line) -> Option<[f64; 4]> {
    line.positions
        .iter()
        .flatten()
        .map(|at| {
            let [longitude, latitude] = degrees(*at);
            [longitude, latitude, longitude, latitude]
        })
        .reduce(union)
}

fn union(left: [f64; 4], right: [f64; 4]) -> [f64; 4] {
    [
        left[0].min(right[0]),
        left[1].min(right[1]),
        left[2].max(right[2]),
        left[3].max(right[3]),
    ]
}

/// A position in degrees, longitude first, as WKB and GeoParquet order it.
fn degrees(at: Position) -> [f64; 2] {
    [at.longitude as f64 / 1e9, at.latitude as f64 / 1e9]
}

/// The position degrees stand for, back to the exact nanodegree: an f64 holds a coordinate to well
/// under a millionth of a nanodegree, so rounding recovers it.
fn position_of([longitude, latitude]: [f64; 2]) -> Position {
    Position {
        latitude: (latitude * 1e9).round() as i64,
        longitude: (longitude * 1e9).round() as i64,
    }
}

/// The index along a Hilbert curve of the cell holding a box's centre.
fn hilbert_key([xmin, ymin, xmax, ymax]: [f64; 4]) -> u64 {
    let cells = f64::from((1u32 << HILBERT_ORDER) - 1);
    let cell =
        |value: f64, low: f64, span: f64| (((value - low) / span).clamp(0.0, 1.0) * cells).round() as u32;
    let x = cell((xmin + xmax) / 2.0, -180.0, 360.0);
    let y = cell((ymin + ymax) / 2.0, -90.0, 180.0);
    hilbert(x, y)
}

/// The classic conversion of a cell to its index along the curve, a quadrant at a time.
fn hilbert(mut x: u32, mut y: u32) -> u64 {
    let side = 1u32 << HILBERT_ORDER;
    let mut index = 0u64;
    let mut quadrant = side / 2;
    while quadrant > 0 {
        let right = u32::from(x & quadrant > 0);
        let up = u32::from(y & quadrant > 0);
        index += u64::from(quadrant) * u64::from(quadrant) * u64::from((3 * right) ^ up);
        if up == 0 {
            if right == 1 {
                x = side - 1 - x;
                y = side - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        quadrant /= 2;
    }
    index
}

fn columns_of(rows: Vec<Row>) -> Vec<Column> {
    vec![
        Column::of(
            "kind",
            Values::Texts(rows.iter().map(|row| row.kind.to_string()).collect()),
        ),
        Column::of("id", Values::Integers(rows.iter().map(|row| row.id).collect())),
        Column::of(
            "relation",
            Values::OptionalIntegers(rows.iter().map(|row| row.relation).collect()),
        ),
        Column::of(
            "tag_keys",
            Values::TextLists(
                rows.iter()
                    .map(|row| row.tags.iter().map(|(key, _)| key.clone()).collect())
                    .collect(),
            ),
        ),
        Column::of(
            "tag_values",
            Values::TextLists(
                rows.iter()
                    .map(|row| row.tags.iter().map(|(_, value)| value.clone()).collect())
                    .collect(),
            ),
        ),
        Column::of(
            "refs",
            Values::IntegerLists(rows.iter().map(|row| row.refs.clone()).collect()),
        ),
        Column::of(
            "unresolved",
            Values::SmallIntegerLists(rows.iter().map(|row| row.unresolved.clone()).collect()),
        ),
        Column::of(
            "member_ways",
            Values::IntegerLists(rows.iter().map(|row| row.member_ways.clone()).collect()),
        ),
        Column::of(
            "member_roles",
            Values::TextLists(rows.iter().map(|row| row.member_roles.clone()).collect()),
        ),
        Column::of(
            "geometry",
            Values::Bytes(rows.iter().map(|row| row.geometry.clone()).collect()),
        ),
        Column::of("bbox", Values::Boxes(rows.iter().map(|row| row.bbox).collect())),
    ]
}

/// The elements a file's rows stand for, each multipolygon gathered with the member rows after it.
fn elements_of(columns: &[Column]) -> Result<Vec<Element>, TableError> {
    let kinds = texts(columns, "kind")?;
    let ids = integers(columns, "id")?;
    let relations = optional_integers(columns, "relation")?;
    let tag_keys = text_lists(columns, "tag_keys")?;
    let tag_values = text_lists(columns, "tag_values")?;
    let refs = integer_lists(columns, "refs")?;
    let unresolved = small_integer_lists(columns, "unresolved")?;
    let member_ways = integer_lists(columns, "member_ways")?;
    let member_roles = text_lists(columns, "member_roles")?;
    let geometries = bytes(columns, "geometry")?;
    boxes(columns, "bbox")?;
    let tags_of = |row: usize| -> Tags {
        tag_keys[row]
            .iter()
            .cloned()
            .zip(tag_values[row].iter().cloned())
            .collect()
    };
    let line_of = |row: usize| -> Result<Line, TableError> {
        let mut vertices = wkb::read_line_string(&geometries[row])
            .map_err(|_| unexpected("geometry"))?
            .into_iter();
        let positions = (0..refs[row].len())
            .map(|index| {
                if unresolved[row].contains(&(index as i32)) {
                    Ok(None)
                } else {
                    vertices
                        .next()
                        .map(|vertex| Some(position_of(vertex)))
                        .ok_or_else(|| unexpected("geometry"))
                }
            })
            .collect::<Result<_, TableError>>()?;
        Ok(Line {
            id: ids[row],
            tags: tags_of(row),
            refs: refs[row].clone(),
            positions,
        })
    };
    let mut elements = Vec::new();
    for row in 0..ids.len() {
        match kinds[row].as_str() {
            "node" => {
                let (longitude, latitude) =
                    wkb::read_point(&geometries[row]).map_err(|_| unexpected("geometry"))?;
                elements.push(Element::Node {
                    id: ids[row],
                    at: position_of([longitude, latitude]),
                    tags: tags_of(row),
                });
            }
            "way" => elements.push(Element::Way(line_of(row)?)),
            "multipolygon" => elements.push(Element::Multipolygon {
                id: ids[row],
                tags: tags_of(row),
                members: member_ways[row]
                    .iter()
                    .copied()
                    .zip(member_roles[row].iter().cloned())
                    .collect(),
                ways: Vec::new(),
            }),
            "member" => match elements.last_mut() {
                Some(Element::Multipolygon { id, ways, .. }) if Some(*id) == relations[row] => {
                    ways.push(line_of(row)?);
                }
                _ => return Err(unexpected("member")),
            },
            _ => return Err(unexpected("kind")),
        }
    }
    Ok(elements)
}

/// The grid a map index file's footer states its positions were read on.
pub(crate) fn grid_in(footer: &Footer) -> Option<Grid> {
    footer.metadata_value(GRID).as_deref().and_then(grid_of)
}

fn grid_of(text: &str) -> Option<Grid> {
    let mut values = text.split(',').map(|value| value.parse::<i64>().ok());
    let grid = Grid {
        granularity: values.next()??,
        latitude_offset: values.next()??,
        longitude_offset: values.next()??,
    };
    values.next().is_none().then_some(grid)
}

fn unplaced(kind: &str, id: i64) -> TableError {
    TableError::Unwritable {
        detail: format!("the {kind} {id} has no node the planet places"),
    }
}

fn unexpected(column: &str) -> TableError {
    TableError::Unreadable {
        detail: format!("the map index's {column} is not as written"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planet() -> Provenance {
        Provenance {
            source_name: "planet-260907.osm.pbf".into(),
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 4_321,
            writing_program: "planet-dump-ng 1.2.7".into(),
        }
    }

    fn grid() -> Grid {
        Grid {
            granularity: 100,
            latitude_offset: 0,
            longitude_offset: 0,
        }
    }

    fn at(latitude: i64, longitude: i64) -> Position {
        Position { latitude, longitude }
    }

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    /// Monaco's harbour: a named square, a quay leaving the country through a node the planet does
    /// not place, and a basin drawn by a multipolygon whose inner member lies out of the country.
    fn harbour() -> Vec<Element> {
        let quay = Line {
            id: 10,
            tags: tags(&[("man_made", "quay"), ("name", "Quai Antoine Ier")]),
            refs: vec![1, 2, 99],
            positions: vec![
                Some(at(43_734_700_000, 7_418_600_000)),
                Some(at(43_735_200_000, 7_421_400_000)),
                None,
            ],
        };
        let island = Line {
            id: 12,
            tags: Tags::new(),
            refs: vec![8, 9, 8],
            positions: vec![
                Some(at(43_800_000_000, 7_500_000_000)),
                Some(at(43_810_000_000, 7_510_000_000)),
                Some(at(43_800_000_000, 7_500_000_000)),
            ],
        };
        vec![
            Element::Node {
                id: 3,
                at: at(43_735_000_000, 7_420_000_000),
                tags: tags(&[("name", "Place d'Armes"), ("place", "square")]),
            },
            Element::Node {
                id: 4,
                at: at(43_736_000_000, 7_419_000_000),
                tags: Tags::new(),
            },
            Element::Way(quay.clone()),
            Element::Multipolygon {
                id: 100,
                tags: tags(&[("type", "multipolygon"), ("natural", "water")]),
                members: vec![(10, "outer".into()), (12, "inner".into())],
                ways: vec![quay, island],
            },
        ]
    }

    /// The elements in a fixed order, since a file holds them in the order that keeps a box's rows
    /// together rather than the order they were given in.
    fn by_id(mut elements: Vec<Element>) -> Vec<Element> {
        let key = |element: &Element| match element {
            Element::Node { id, .. } => (0, *id),
            Element::Way(line) => (1, line.id),
            Element::Multipolygon { id, .. } => (2, *id),
        };
        elements.sort_by_key(key);
        elements
    }

    #[test]
    fn given_nodes_a_way_and_a_multipolygon_when_written_then_reading_them_back_gives_the_same_elements_and_grid()
    -> Result<(), TableError> {
        let written = to_parquet("MC", &harbour(), grid(), &planet())?;

        let (read_grid, read) = from_parquet(&written)?;

        assert_eq!(read_grid, grid());
        assert_eq!(by_id(read), by_id(harbour()));
        Ok(())
    }

    #[test]
    fn given_positions_at_the_ends_of_the_earth_when_written_then_they_come_back_to_the_nanodegree()
    -> Result<(), TableError> {
        let corners = [
            at(90_000_000_000, 180_000_000_000),
            at(-90_000_000_000, -180_000_000_000),
            at(89_999_999_900, -179_999_999_900),
            at(-1, 1),
            at(43_738_412_300, 7_424_615_700),
        ];
        let nodes: Vec<Element> = corners
            .iter()
            .enumerate()
            .map(|(id, at)| Element::Node {
                id: id as i64,
                at: *at,
                tags: Tags::new(),
            })
            .collect();

        let (_, read) = from_parquet(&to_parquet("MC", &nodes, grid(), &planet())?)?;

        assert_eq!(by_id(read), nodes);
        Ok(())
    }

    #[test]
    fn given_the_same_elements_in_another_order_when_written_then_the_bytes_are_identical()
    -> Result<(), TableError> {
        let mut reversed = harbour();
        reversed.reverse();

        let once = to_parquet("MC", &harbour(), grid(), &planet())?;
        let again = to_parquet("MC", &reversed, grid(), &planet())?;

        assert_eq!(once, again);
        Ok(())
    }

    #[test]
    fn given_more_rows_than_a_row_group_holds_when_written_then_no_multipolygon_is_parted_from_its_members()
    -> Result<(), TableError> {
        let written = cut_at("MC", &harbour(), grid(), &planet(), 1)?;

        let Tail::Footer(footer) = tables::footer_from_tail(&written, written.len() as u64)? else {
            return Err(unexpected("footer"));
        };
        let groups = footer.row_groups("id");
        let first_kinds = groups
            .iter()
            .map(|group| {
                let range = group.bytes.start as usize..group.bytes.end as usize;
                let columns = footer.read_row_group(group, written[range].to_vec())?;
                Ok(texts(&columns, "kind")?.first().cloned().unwrap_or_default())
            })
            .collect::<Result<Vec<String>, TableError>>()?;
        assert_eq!(groups.len(), 4, "one row group per element");
        assert!(
            !first_kinds.iter().any(|kind| kind == "member"),
            "{first_kinds:?}"
        );
        Ok(())
    }

    #[test]
    fn given_a_map_index_when_its_footer_is_read_then_the_box_alone_carries_statistics()
    -> Result<(), TableError> {
        let written = to_parquet("MC", &harbour(), grid(), &planet())?;

        let with_statistics = tables::columns_with_statistics(&written)?;

        assert_eq!(
            with_statistics,
            vec!["bbox.xmin", "bbox.ymin", "bbox.xmax", "bbox.ymax"]
        );
        Ok(())
    }
}
