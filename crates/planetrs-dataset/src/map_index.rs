//! A country's map index: everything a map of any box inside the country takes from the planet,
//! filed so that a box reads only the few row groups around it.
//!
//! It holds every way with a node in the country, whole, with the position of each node the planet
//! places; every node of the country that carries a name or that no way runs through; and every
//! multipolygon with a member among those ways, followed by each of its member ways whole, so that a
//! map never has to fetch a member by its id. Positions come back to the nanodegree, exactly as the
//! planet stores them.

use crate::manifest::Provenance;
use crate::tables::TableError;

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
pub fn to_parquet(
    country: &str,
    elements: &[Element],
    grid: Grid,
    planet: &Provenance,
) -> Result<Vec<u8>, TableError> {
    let _ = (country, elements, grid, planet);
    Ok(Vec::new())
}

/// The elements of a map index file, with the grid its positions were read on.
pub fn from_parquet(bytes: &[u8]) -> Result<(Grid, Vec<Element>), TableError> {
    let _ = bytes;
    Ok((
        Grid {
            granularity: 0,
            latitude_offset: 0,
            longitude_offset: 0,
        },
        Vec::new(),
    ))
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
}
