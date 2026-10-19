//! Small planet files written in tests, so that whatever reads a planet can be tested on a handful of
//! nodes rather than on ninety gigabytes.
//!
//! A fixture is what a real planet is, only shorter: a header blob declaring `DenseNodes` and
//! `Sort.Type_then_ID`, then zlib-compressed blocks of dense nodes, ways and relations in that order,
//! with granularity 100 and no offset, as planet.openstreetmap.org writes them.

use std::io::{self, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::entity::to_zigzag;
use crate::header::{DENSE_NODES, SORT_TYPE_THEN_ID};

/// The program a fixture names in its header, so a harvest or an index built from one says so.
pub const FIXTURE_PROGRAM: &str = "planetrs fixture";

/// Tags in the order they are written, duplicates kept, as a planet may hold them.
pub type Tags = Vec<(String, String)>;

/// A node of a fixture, placed on the planet's grid of 100 nanodegrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: i64,
    /// Nanodegrees, a multiple of the planet's granularity.
    pub latitude: i64,
    pub longitude: i64,
    pub tags: Tags,
}

impl Node {
    /// A node at a position given in degrees, rounded onto the planet's 100-nanodegree grid.
    pub fn at(id: i64, latitude: f64, longitude: f64) -> Self {
        Self {
            id,
            latitude: on_the_planet_grid(latitude),
            longitude: on_the_planet_grid(longitude),
            tags: Vec::new(),
        }
    }

    /// The same node with one more tag, after the ones it already has.
    pub fn tagged(mut self, key: &str, value: &str) -> Self {
        self.tags.push((key.to_string(), value.to_string()));
        self
    }
}

/// A way of a fixture: the nodes it runs through, in order, and its tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Way {
    pub id: i64,
    pub tags: Tags,
    pub refs: Vec<i64>,
}

impl Way {
    /// A way through the given nodes, which need not exist: a planet may hold refs to missing nodes.
    pub fn through(id: i64, refs: impl IntoIterator<Item = i64>) -> Self {
        Self {
            id,
            tags: Vec::new(),
            refs: refs.into_iter().collect(),
        }
    }

    /// The same way with one more tag, after the ones it already has.
    pub fn tagged(mut self, key: &str, value: &str) -> Self {
        self.tags.push((key.to_string(), value.to_string()));
        self
    }
}

/// What a relation member is, with the codes the format gives each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Node,
    Way,
    Relation,
}

/// One member of a fixture relation: its kind, its id and its role, which may be empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub kind: MemberKind,
    pub id: i64,
    pub role: String,
}

/// A relation of a fixture: its tags and its members, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: i64,
    pub tags: Tags,
    pub members: Vec<Member>,
}

impl Relation {
    /// A relation with no tags and no members yet.
    pub fn of(id: i64) -> Self {
        Self {
            id,
            tags: Vec::new(),
            members: Vec::new(),
        }
    }

    /// The same relation with one more tag, after the ones it already has.
    pub fn tagged(mut self, key: &str, value: &str) -> Self {
        self.tags.push((key.to_string(), value.to_string()));
        self
    }

    /// The same relation with one more member, after the ones it already has.
    pub fn with_member(mut self, kind: MemberKind, id: i64, role: &str) -> Self {
        self.members.push(Member {
            kind,
            id,
            role: role.to_string(),
        });
        self
    }
}

fn on_the_planet_grid(degrees: f64) -> i64 {
    (degrees * 1e7).round() as i64 * GRANULARITY
}

/// Nanodegrees per unit of a block's coordinates, the planet's own.
const GRANULARITY: i64 = 100;

/// A planet file to be written: its header and its entities.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanetFile {
    writing_program: String,
    nodes: Vec<Node>,
    ways: Vec<Way>,
    relations: Vec<Relation>,
}

impl PlanetFile {
    /// A planet declaring `Sort.Type_then_ID`, as every planet file does.
    pub fn sorted() -> Self {
        Self {
            writing_program: FIXTURE_PROGRAM.to_string(),
            nodes: Vec::new(),
            ways: Vec::new(),
            relations: Vec::new(),
        }
    }

    /// The same planet with one more node.
    pub fn with_node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    /// The same planet with one more way.
    pub fn with_way(mut self, way: Way) -> Self {
        self.ways.push(way);
        self
    }

    /// The same planet with one more relation.
    pub fn with_relation(mut self, relation: Relation) -> Self {
        self.relations.push(relation);
        self
    }

    /// The file's bytes, framing included, every section in id order as `Sort.Type_then_ID` promises.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut nodes = self.nodes.clone();
        nodes.sort_by_key(|node| node.id);
        let mut ways = self.ways.clone();
        ways.sort_by_key(|way| way.id);
        let blocks = [
            (!nodes.is_empty()).then(|| node_block(&nodes)),
            (!ways.is_empty()).then(|| way_block(&ways)),
        ];
        std::iter::once(framed("OSMHeader", &self.header_block()))
            .chain(
                blocks
                    .into_iter()
                    .flatten()
                    .map(|block| framed("OSMData", &block)),
            )
            .flatten()
            .collect()
    }

    fn header_block(&self) -> Vec<u8> {
        Message::new()
            .text(4, "OsmSchema-V0.6")
            .text(4, DENSE_NODES)
            .text(5, SORT_TYPE_THEN_ID)
            .text(16, &self.writing_program)
            .into_bytes()
    }

    /// Writes the file at `path`, replacing whatever was there: fixtures live in temporary places.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_bytes())
    }
}

/// A block of dense nodes: three delta-coded arrays and one flat stream of tags, ended per node by 0.
fn node_block(nodes: &[Node]) -> Vec<u8> {
    let strings = StringTable::of(nodes.iter().flat_map(|node| &node.tags));
    let ids = deltas(nodes.iter().map(|node| node.id));
    let latitudes = deltas(nodes.iter().map(|node| node.latitude / GRANULARITY));
    let longitudes = deltas(nodes.iter().map(|node| node.longitude / GRANULARITY));
    let tagged = nodes.iter().any(|node| !node.tags.is_empty());
    let keys_vals = packed(nodes.iter().flat_map(|node| {
        node.tags
            .iter()
            .flat_map(|(key, value)| [strings.index_of(key), strings.index_of(value)])
            .chain([0])
    }));
    let dense = Message::new()
        .bytes(1, &ids)
        .bytes(8, &latitudes)
        .bytes(9, &longitudes);
    let dense = if tagged {
        dense.bytes(10, &keys_vals)
    } else {
        dense
    };
    let group = Message::new().bytes(2, &dense.into_bytes()).into_bytes();
    primitive_block(&strings, &group)
}

/// A block of ways, each with its tags as two parallel arrays of indices and its refs delta-coded.
fn way_block(ways: &[Way]) -> Vec<u8> {
    let strings = StringTable::of(ways.iter().flat_map(|way| &way.tags));
    let group = ways
        .iter()
        .fold(Message::new(), |group, way| {
            let encoded = Message::new()
                .number(1, way.id as u64)
                .bytes(2, &packed(way.tags.iter().map(|(key, _)| strings.index_of(key))))
                .bytes(
                    3,
                    &packed(way.tags.iter().map(|(_, value)| strings.index_of(value))),
                )
                .bytes(8, &deltas(way.refs.iter().copied()))
                .into_bytes();
            group.bytes(3, &encoded)
        })
        .into_bytes();
    primitive_block(&strings, &group)
}

fn primitive_block(strings: &StringTable, group: &[u8]) -> Vec<u8> {
    Message::new()
        .bytes(1, &strings.to_bytes())
        .bytes(2, group)
        .number(17, GRANULARITY as u64)
        .into_bytes()
}

/// A block's strings, the empty one first as the format reserves index 0, then in order of first use.
struct StringTable(Vec<String>);

impl StringTable {
    fn of<'a>(tags: impl Iterator<Item = &'a (String, String)>) -> Self {
        let mut strings = vec![String::new()];
        for (key, value) in tags {
            for text in [key, value] {
                if !strings.contains(text) {
                    strings.push(text.clone());
                }
            }
        }
        Self(strings)
    }

    fn index_of(&self, text: &str) -> u64 {
        self.0.iter().position(|known| known == text).unwrap_or(0) as u64
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.0
            .iter()
            .fold(Message::new(), |table, text| table.text(1, text))
            .into_bytes()
    }
}

fn packed(values: impl Iterator<Item = u64>) -> Vec<u8> {
    let mut out = Vec::new();
    values.for_each(|value| put_varint(&mut out, value));
    out
}

/// Each value as its difference from the one before, zigzagged, as ids and coordinates are stored.
fn deltas(values: impl Iterator<Item = i64>) -> Vec<u8> {
    let mut previous = 0;
    packed(values.map(|value| {
        let delta = value - previous;
        previous = value;
        to_zigzag(delta)
    }))
}

/// One blob as the framing lays it out: the header's length, the header, then the zlib payload.
fn framed(kind: &str, block: &[u8]) -> Vec<u8> {
    let blob = Message::new()
        .number(2, block.len() as u64)
        .bytes(3, &zlib(block))
        .into_bytes();
    let header = Message::new()
        .text(1, kind)
        .number(3, blob.len() as u64)
        .into_bytes();
    [&(header.len() as u32).to_be_bytes()[..], &header, &blob].concat()
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .unwrap_or_default()
}

/// A protobuf message written field by field, in the order the fields are given.
struct Message(Vec<u8>);

impl Message {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn number(mut self, field: u32, value: u64) -> Self {
        put_varint(&mut self.0, u64::from(field) << 3);
        put_varint(&mut self.0, value);
        self
    }

    fn bytes(mut self, field: u32, payload: &[u8]) -> Self {
        put_varint(&mut self.0, u64::from(field) << 3 | 2);
        put_varint(&mut self.0, payload.len() as u64);
        self.0.extend_from_slice(payload);
        self
    }

    fn text(self, field: u32, text: &str) -> Self {
        self.bytes(field, text.as_bytes())
    }

    fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::block::block;
    use crate::entity::{MEMBER_NODE, MEMBER_WAY, dense_group, visit_relations, visit_ways};
    use crate::error::PbfError;
    use crate::planet::Planet;

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn nodes_of(planet: &Planet) -> Result<Vec<Node>, PbfError> {
        let mut found = Vec::new();
        let (mut payload, mut inflated) = (Vec::new(), Vec::new());
        for at in 0..planet.blobs.len() {
            planet.inflate_blob(at, &mut payload, &mut inflated)?;
            let decoded = block(&inflated)?;
            for group in &decoded.groups {
                let Some(dense) = dense_group(group)? else {
                    continue;
                };
                dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                    found.push(Node {
                        id,
                        latitude: decoded.nanodegrees_latitude(latitude),
                        longitude: decoded.nanodegrees_longitude(longitude),
                        tags: tags
                            .iter()
                            .map(|(key, value)| {
                                (
                                    text(decoded.string(*key as usize)),
                                    text(decoded.string(*value as usize)),
                                )
                            })
                            .collect(),
                    })
                })?;
            }
        }
        Ok(found)
    }

    fn ways_of(planet: &Planet) -> Result<Vec<Way>, PbfError> {
        let mut found = Vec::new();
        let (mut payload, mut inflated) = (Vec::new(), Vec::new());
        for at in 0..planet.blobs.len() {
            planet.inflate_blob(at, &mut payload, &mut inflated)?;
            let decoded = block(&inflated)?;
            for group in &decoded.groups {
                visit_ways(group, &mut |way| {
                    found.push(Way {
                        id: way.id,
                        tags: way
                            .tags(&decoded)
                            .iter()
                            .map(|(key, value)| (text(key), text(value)))
                            .collect(),
                        refs: way.refs().collect::<Result<_, _>>()?,
                    });
                    Ok(())
                })?;
            }
        }
        Ok(found)
    }

    fn relations_of(planet: &Planet) -> Result<Vec<Relation>, PbfError> {
        let mut found = Vec::new();
        let (mut payload, mut inflated) = (Vec::new(), Vec::new());
        for at in 0..planet.blobs.len() {
            planet.inflate_blob(at, &mut payload, &mut inflated)?;
            let decoded = block(&inflated)?;
            for group in &decoded.groups {
                visit_relations(group, &mut |relation| {
                    found.push(Relation {
                        id: relation.id,
                        tags: relation
                            .tags(&decoded)
                            .iter()
                            .map(|(key, value)| (text(key), text(value)))
                            .collect(),
                        members: relation
                            .members(&decoded)
                            .iter()
                            .map(|(id, kind, role)| Member {
                                kind: match *kind {
                                    MEMBER_NODE => MemberKind::Node,
                                    MEMBER_WAY => MemberKind::Way,
                                    _ => MemberKind::Relation,
                                },
                                id: *id,
                                role: text(role),
                            })
                            .collect(),
                    });
                    Ok(())
                })?;
            }
        }
        Ok(found)
    }

    fn written(planet: &PlanetFile, name: &str) -> io::Result<PathBuf> {
        let path = std::env::temp_dir().join(format!("planetrs-pbf-{name}-{}.osm.pbf", std::process::id()));
        planet.write(&path)?;
        Ok(path)
    }

    #[test]
    fn given_a_fixture_planet_when_opened_then_its_header_names_the_fixture_and_declares_the_sort_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = written(&PlanetFile::sorted(), "header")?;

        let opened = Planet::open(&path)?;

        assert_eq!(opened.header.writing_program.as_deref(), Some(FIXTURE_PROGRAM));
        assert!(opened.header.is_sorted_by_type_then_id());
        assert!(opened.header.requires_dense_nodes());
        Ok(())
    }

    #[test]
    fn given_nodes_when_their_blob_is_read_then_each_comes_back_with_its_position_and_tags()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = PlanetFile::sorted()
            .with_node(Node::at(2, 43.73, 7.42))
            .with_node(
                Node::at(1, 43.7384, 7.4246)
                    .tagged("name", "Monaco")
                    .tagged("place", "city"),
            );
        let path = written(&fixture, "nodes")?;

        let nodes = nodes_of(&Planet::open(&path)?)?;

        assert_eq!(
            nodes,
            vec![
                Node {
                    id: 1,
                    latitude: 43_738_400_000,
                    longitude: 7_424_600_000,
                    tags: vec![
                        ("name".to_string(), "Monaco".to_string()),
                        ("place".to_string(), "city".to_string())
                    ],
                },
                Node {
                    id: 2,
                    latitude: 43_730_000_000,
                    longitude: 7_420_000_000,
                    tags: vec![],
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn given_ways_when_their_blob_is_read_then_each_keeps_its_tags_and_its_refs_in_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = PlanetFile::sorted()
            .with_node(Node::at(1, 43.7384, 7.4246))
            .with_node(Node::at(2, 43.73, 7.42))
            .with_way(
                Way::through(20, [1, 2])
                    .tagged("highway", "primary")
                    .tagged("name", "Avenue de Monte-Carlo"),
            )
            .with_way(Way::through(10, [2, 3, 1]));
        let path = written(&fixture, "ways")?;

        let ways = ways_of(&Planet::open(&path)?)?;

        assert_eq!(
            ways,
            vec![
                Way {
                    id: 10,
                    tags: vec![],
                    refs: vec![2, 3, 1],
                },
                Way {
                    id: 20,
                    tags: vec![
                        ("highway".to_string(), "primary".to_string()),
                        ("name".to_string(), "Avenue de Monte-Carlo".to_string())
                    ],
                    refs: vec![1, 2],
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn given_relations_when_their_blob_is_read_then_each_keeps_its_members_kinds_and_roles_in_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = PlanetFile::sorted()
            .with_node(Node::at(1, 43.7384, 7.4246))
            .with_way(Way::through(10, [1]))
            .with_relation(
                Relation::of(100)
                    .tagged("type", "multipolygon")
                    .with_member(MemberKind::Way, 10, "outer")
                    .with_member(MemberKind::Way, 11, "inner"),
            )
            .with_relation(
                Relation::of(50)
                    .tagged("boundary", "administrative")
                    .with_member(MemberKind::Node, 1, "admin_centre")
                    .with_member(MemberKind::Way, 10, "")
                    .with_member(MemberKind::Relation, 100, "subarea"),
            );
        let path = written(&fixture, "relations")?;

        let relations = relations_of(&Planet::open(&path)?)?;

        assert_eq!(
            relations,
            vec![
                Relation::of(50)
                    .tagged("boundary", "administrative")
                    .with_member(MemberKind::Node, 1, "admin_centre")
                    .with_member(MemberKind::Way, 10, "")
                    .with_member(MemberKind::Relation, 100, "subarea"),
                Relation::of(100)
                    .tagged("type", "multipolygon")
                    .with_member(MemberKind::Way, 10, "outer")
                    .with_member(MemberKind::Way, 11, "inner"),
            ]
        );
        Ok(())
    }
}
