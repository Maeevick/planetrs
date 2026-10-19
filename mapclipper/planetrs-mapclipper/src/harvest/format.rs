//! The `.mapclip` file: one harvested area, on disk.
//!
//! Fixed width, little endian, lengths before bytes, no compression. Deliberately dull, because it
//! is meant to be re-implementable from this module alone by anyone who wants to check it.
//!
//! Strings go in a table sorted once and referenced by index. Tags repeat enormously (every shed in
//! a port carries `building=yes`), so the table is most of the saving, and sorting it is what keeps
//! the bytes a function of the data rather than of the order the passes happened to finish in.

use std::collections::BTreeSet;
use std::path::Path;

use planetrs_geo::BoundingBox;

use super::{Harvest, Member, NamedNode, Node, Provenance, Relation, Tags, Way};
use crate::error::ClipError;

const MAGIC: &[u8; 8] = b"MAPCLIP\0";
const VERSION: u32 = 3;

/// A count read out of a file decides an allocation, so it is checked against what is left to read
/// before anything is reserved on it.
fn affordable(count: u64, each: usize, remaining: usize) -> Result<usize, ClipError> {
    let count = count as usize;
    match count.checked_mul(each.max(1)) {
        Some(needed) if needed <= remaining => Ok(count),
        _ => Err(ClipError::HarvestTruncated),
    }
}

#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.0.extend_from_slice(value.as_bytes());
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ClipError> {
        let end = self.at.checked_add(count).ok_or(ClipError::HarvestTruncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(ClipError::HarvestTruncated)?;
        self.at = end;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    fn u32(&mut self) -> Result<u32, ClipError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("a slice of four bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, ClipError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("a slice of eight bytes"),
        ))
    }

    fn i64(&mut self) -> Result<i64, ClipError> {
        self.u64().map(|value| value as i64)
    }

    fn f64(&mut self) -> Result<f64, ClipError> {
        Ok(f64::from_le_bytes(
            self.take(8)?.try_into().expect("a slice of eight bytes"),
        ))
    }

    fn text(&mut self) -> Result<String, ClipError> {
        let length = self.u64()? as usize;
        if length > self.remaining() {
            return Err(ClipError::HarvestTruncated);
        }
        Ok(String::from_utf8_lossy(self.take(length)?).into_owned())
    }
}

/// Every distinct string in a harvest, sorted, so an index into it is stable across runs.
struct StringTable {
    entries: Vec<String>,
}

impl StringTable {
    fn of(harvest: &Harvest) -> Self {
        let tag_strings = harvest
            .ways
            .iter()
            .map(|way| &way.tags)
            .chain(harvest.relations.iter().map(|relation| &relation.tags))
            .chain(harvest.named_nodes.iter().map(|node| &node.tags))
            .flat_map(|tags| {
                tags.iter()
                    .flat_map(|(key, value)| [key.as_str(), value.as_str()])
            });

        let roles = harvest
            .relations
            .iter()
            .flat_map(|relation| relation.members.iter().map(|member| member.role.as_str()));

        let distinct: BTreeSet<&str> = tag_strings.chain(roles).collect();

        Self {
            entries: distinct.into_iter().map(str::to_string).collect(),
        }
    }

    fn index_of(&self, text: &str) -> u32 {
        self.entries
            .binary_search_by(|entry| entry.as_str().cmp(text))
            .expect("a string the table was built from") as u32
    }

    fn at(&self, index: u32) -> Result<String, ClipError> {
        self.entries
            .get(index as usize)
            .cloned()
            .ok_or(ClipError::HarvestTruncated)
    }
}

/// The harvest as the bytes of a harvest file, the same bytes for the same harvest every time.
pub fn to_bytes(harvest: &Harvest) -> Vec<u8> {
    let table = StringTable::of(harvest);
    let mut out = Bytes::default();

    out.0.extend_from_slice(MAGIC);
    out.u32(VERSION);
    out.text(crate::credit::DATA_NOTICE);

    out.text(&harvest.provenance.source_name);
    out.u64(harvest.provenance.file_size);
    out.u64(harvest.provenance.blob_count);
    out.i64(harvest.provenance.replication_timestamp);
    out.i64(harvest.provenance.replication_sequence);
    out.text(&harvest.provenance.writing_program);

    for edge in [
        harvest.collected.west,
        harvest.collected.south,
        harvest.collected.east,
        harvest.collected.north,
    ] {
        out.f64(edge);
    }

    out.u64(table.entries.len() as u64);
    for entry in &table.entries {
        out.text(entry);
    }

    out.u64(harvest.nodes.len() as u64);
    for node in &harvest.nodes {
        out.i64(node.id);
        out.i64(node.latitude);
        out.i64(node.longitude);
    }

    out.u64(harvest.named_nodes.len() as u64);
    for node in &harvest.named_nodes {
        out.i64(node.id);
        out.i64(node.latitude);
        out.i64(node.longitude);
        write_tags(&mut out, &table, &node.tags);
    }

    out.u64(harvest.ways.len() as u64);
    for way in &harvest.ways {
        out.i64(way.id);
        write_tags(&mut out, &table, &way.tags);
        out.u32(way.refs.len() as u32);
        for reference in &way.refs {
            out.i64(*reference);
        }
    }

    out.u64(harvest.relations.len() as u64);
    for relation in &harvest.relations {
        out.i64(relation.id);
        write_tags(&mut out, &table, &relation.tags);
        out.u32(relation.members.len() as u32);
        for member in &relation.members {
            out.i64(member.way);
            out.u32(table.index_of(&member.role));
        }
    }

    out.0
}

fn write_tags(out: &mut Bytes, table: &StringTable, tags: &Tags) {
    out.u32(tags.len() as u32);
    for (key, value) in tags {
        out.u32(table.index_of(key));
        out.u32(table.index_of(value));
    }
}

fn read_tags(cursor: &mut Cursor<'_>, table: &StringTable) -> Result<Tags, ClipError> {
    let count = affordable(u64::from(cursor.u32()?), 8, cursor.remaining())?;
    (0..count)
        .map(|_| Ok((table.at(cursor.u32()?)?, table.at(cursor.u32()?)?)))
        .collect()
}

/// The harvest these bytes hold, refused when they are not a harvest of the version this build reads.
pub fn from_bytes(bytes: &[u8]) -> Result<Harvest, ClipError> {
    let mut cursor = Cursor { bytes, at: 0 };

    if cursor.take(8)? != MAGIC {
        return Err(ClipError::NotAHarvest);
    }
    let version = cursor.u32()?;
    if version != VERSION {
        return Err(ClipError::HarvestVersion {
            found: version,
            expected: VERSION,
        });
    }
    cursor.text()?;

    let provenance = Provenance {
        source_name: cursor.text()?,
        file_size: cursor.u64()?,
        blob_count: cursor.u64()?,
        replication_timestamp: cursor.i64()?,
        replication_sequence: cursor.i64()?,
        writing_program: cursor.text()?,
    };

    let collected = BoundingBox {
        west: cursor.f64()?,
        south: cursor.f64()?,
        east: cursor.f64()?,
        north: cursor.f64()?,
    };

    let count = affordable(cursor.u64()?, 8, cursor.remaining())?;
    let table = StringTable {
        entries: (0..count).map(|_| cursor.text()).collect::<Result<_, _>>()?,
    };

    let count = affordable(cursor.u64()?, 24, cursor.remaining())?;
    let nodes = (0..count)
        .map(|_| {
            Ok(Node {
                id: cursor.i64()?,
                latitude: cursor.i64()?,
                longitude: cursor.i64()?,
            })
        })
        .collect::<Result<Vec<_>, ClipError>>()?;

    let count = affordable(cursor.u64()?, 28, cursor.remaining())?;
    let named_nodes = (0..count)
        .map(|_| {
            Ok(NamedNode {
                id: cursor.i64()?,
                latitude: cursor.i64()?,
                longitude: cursor.i64()?,
                tags: read_tags(&mut cursor, &table)?,
            })
        })
        .collect::<Result<Vec<_>, ClipError>>()?;

    let count = affordable(cursor.u64()?, 16, cursor.remaining())?;
    let ways = (0..count)
        .map(|_| {
            let id = cursor.i64()?;
            let tags = read_tags(&mut cursor, &table)?;
            let refs = affordable(u64::from(cursor.u32()?), 8, cursor.remaining())?;
            Ok(Way {
                id,
                tags,
                refs: (0..refs).map(|_| cursor.i64()).collect::<Result<_, _>>()?,
            })
        })
        .collect::<Result<Vec<_>, ClipError>>()?;

    let count = affordable(cursor.u64()?, 16, cursor.remaining())?;
    let relations = (0..count)
        .map(|_| {
            let id = cursor.i64()?;
            let tags = read_tags(&mut cursor, &table)?;
            let members = affordable(u64::from(cursor.u32()?), 12, cursor.remaining())?;
            Ok(Relation {
                id,
                tags,
                members: (0..members)
                    .map(|_| {
                        Ok(Member {
                            way: cursor.i64()?,
                            role: table.at(cursor.u32()?)?,
                        })
                    })
                    .collect::<Result<_, ClipError>>()?,
            })
        })
        .collect::<Result<Vec<_>, ClipError>>()?;

    Ok(Harvest {
        provenance,
        collected,
        nodes,
        named_nodes,
        ways,
        relations,
    })
}

/// Writes the harvest to `path`, replacing whatever is there, and says how many bytes it took.
pub fn write(path: &Path, harvest: &Harvest) -> Result<u64, ClipError> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| ClipError::writing(path, error))?;
    }
    let bytes = to_bytes(harvest);
    std::fs::write(path, &bytes).map_err(|error| ClipError::writing(path, error))?;
    Ok(bytes.len() as u64)
}

/// The harvest in the file at `path`.
pub fn read(path: &Path) -> Result<Harvest, ClipError> {
    let bytes = std::fs::read(path).map_err(|error| ClipError::reading(path, error))?;
    from_bytes(&bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn provenance() -> Provenance {
        Provenance {
            source_name: "planet-260907.osm.pbf".into(),
            file_size: 94_746_421_723,
            blob_count: 52_810,
            replication_timestamp: 1_788_739_204,
            replication_sequence: 4_312,
            writing_program: "planet-dump-ng 1.2.4".into(),
        }
    }

    pub(crate) fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    pub(crate) fn abidjan() -> Harvest {
        Harvest {
            provenance: provenance(),
            collected: BoundingBox {
                west: -4.1,
                south: 5.2,
                east: -3.9,
                north: 5.3,
            },
            nodes: vec![
                Node {
                    id: 1,
                    latitude: 5_270_000_000,
                    longitude: -4_010_000_000,
                },
                Node {
                    id: 2,
                    latitude: 5_271_000_000,
                    longitude: -4_011_000_000,
                },
            ],
            named_nodes: vec![NamedNode {
                id: 4_242,
                latitude: 5_272_000_000,
                longitude: -4_012_000_000,
                tags: tags(&[("place", "village"), ("name", "Treichville")]),
            }],
            ways: vec![Way {
                id: 7,
                tags: tags(&[("waterway", "dock"), ("name", "Bassin d'Abidjan")]),
                refs: vec![1, 2, 1],
            }],
            relations: vec![Relation {
                id: 1_982_728,
                tags: tags(&[("natural", "water"), ("name", "Lagune Ébrié")]),
                members: vec![
                    Member {
                        way: 7,
                        role: "outer".into(),
                    },
                    Member {
                        way: 8,
                        role: "inner".into(),
                    },
                ],
            }],
        }
    }

    #[test]
    fn when_a_harvest_is_written_then_reading_it_back_gives_the_same_thing() {
        assert_eq!(from_bytes(&to_bytes(&abidjan())).unwrap(), abidjan());
    }

    #[test]
    fn when_a_harvest_is_written_then_its_provenance_survives_the_round_trip() {
        assert_eq!(
            from_bytes(&to_bytes(&abidjan())).unwrap().provenance,
            provenance()
        );
    }

    /// The property that makes a harvest publishable: anyone taking the same planet file and the
    /// same area must produce the same bytes, or a checksum over the result proves nothing.
    #[test]
    fn when_the_same_harvest_is_written_twice_then_the_bytes_are_identical() {
        assert_eq!(to_bytes(&abidjan()), to_bytes(&abidjan()));
    }

    /// The passes fold across threads, so the same area harvested twice arrives in a different
    /// order. Sorting before writing is what makes that invisible in the file.
    #[test]
    fn when_two_harvests_hold_the_same_thing_in_another_order_then_they_write_the_same_bytes() {
        let shuffled = Harvest {
            nodes: abidjan().nodes.into_iter().rev().collect(),
            ..abidjan()
        };

        assert_eq!(to_bytes(&shuffled.sorted()), to_bytes(&abidjan().sorted()));
    }

    /// Names carry accents and apostrophes, and a length-prefixed format has to count bytes rather
    /// than characters or every field after the first accent is read from the wrong offset.
    #[test]
    fn when_a_name_is_not_ascii_then_the_lengths_still_line_up() {
        let read = from_bytes(&to_bytes(&abidjan())).unwrap();

        assert_eq!(
            super::super::tag(&read.relations[0].tags, "name"),
            Some("Lagune Ébrié")
        );
    }

    /// Every shed in a port carries `building=yes`, so the table is most of what the format saves.
    #[test]
    fn when_a_tag_repeats_across_entities_then_its_text_is_stored_once() {
        let repeated = Harvest {
            ways: (0..200)
                .map(|id| Way {
                    id,
                    tags: tags(&[("building", "yes")]),
                    refs: vec![1, 2],
                })
                .collect(),
            ..abidjan()
        };

        let grown = to_bytes(&repeated).len() - to_bytes(&abidjan()).len();

        assert!(
            grown < 200 * 40,
            "200 more ways cost {grown} bytes, so the text is being repeated"
        );
    }

    #[test]
    fn when_the_bytes_are_not_a_harvest_then_it_is_refused_rather_than_misread() {
        assert!(matches!(
            from_bytes(b"this is not a harvest at all"),
            Err(ClipError::NotAHarvest)
        ));
    }

    #[test]
    fn when_a_harvest_was_written_by_another_version_then_it_is_refused_by_number() {
        let mut bytes = to_bytes(&abidjan());
        bytes[8] = 99;

        assert!(matches!(
            from_bytes(&bytes).unwrap_err(),
            ClipError::HarvestVersion { found: 99, expected } if expected == VERSION
        ));
    }

    /// A count read out of a file decides an allocation. Truncating anywhere has to end in an
    /// error rather than in a reservation the file got to choose.
    #[test]
    fn when_a_harvest_is_cut_short_anywhere_then_it_fails_rather_than_reserving_on_a_bad_count() {
        let whole = to_bytes(&abidjan());

        for at in (9..whole.len()).step_by(3) {
            assert!(
                from_bytes(&whole[..at]).is_err(),
                "truncating at {at} was accepted"
            );
        }
    }

    #[test]
    fn when_a_harvest_holds_nothing_then_it_still_round_trips() {
        let empty = Harvest {
            nodes: vec![],
            ways: vec![],
            relations: vec![],
            ..abidjan()
        };

        assert_eq!(from_bytes(&to_bytes(&empty)).unwrap(), empty);
    }

    #[test]
    fn when_a_harvest_is_written_to_disk_then_reading_the_path_back_gives_the_same_thing() {
        let path = std::env::temp_dir().join("mapclipper-roundtrip.mapclip");
        write(&path, &abidjan()).unwrap();

        let back = read(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(back, abidjan());
    }
}

#[cfg(test)]
mod named_node_tests {
    use super::tests::*;
    use super::*;

    /// A settlement is a `place=*` node, so without tagged nodes a map cannot say what a village is
    /// called even when every building around it is in hand.
    #[test]
    fn when_a_harvest_holds_a_named_node_then_its_tags_survive_the_round_trip() {
        let read = from_bytes(&to_bytes(&abidjan())).unwrap();

        assert_eq!(read.named_nodes.len(), 1);
        assert_eq!(
            super::super::tag(&read.named_nodes[0].tags, "name"),
            Some("Treichville")
        );
        assert_eq!(
            super::super::tag(&read.named_nodes[0].tags, "place"),
            Some("village")
        );
    }

    #[test]
    fn when_a_named_node_is_read_back_then_it_is_where_it_was() {
        let read = from_bytes(&to_bytes(&abidjan())).unwrap();

        assert_eq!(
            (read.named_nodes[0].latitude, read.named_nodes[0].longitude),
            (5_272_000_000, -4_012_000_000)
        );
    }

    /// Version 1 harvests held no named nodes, so reading one as the current version would take the
    /// way count for a node count and build nonsense out of it.
    #[test]
    fn when_a_version_one_harvest_is_read_then_it_is_refused_by_number() {
        let mut bytes = to_bytes(&abidjan());
        bytes[8] = 1;

        assert!(matches!(
            from_bytes(&bytes),
            Err(ClipError::HarvestVersion { found: 1, expected }) if expected == VERSION
        ));
    }

    /// A harvest is a Derivative Database of OpenStreetMap that can be handed on, so it names its
    /// licence and links to it, as data has to.
    #[test]
    fn when_a_harvest_is_written_then_it_names_and_links_the_licence_its_data_is_under() {
        let bytes = to_bytes(&abidjan());
        let notice = crate::credit::DATA_NOTICE.as_bytes();

        assert!(bytes.windows(notice.len()).any(|window| window == notice));
    }
}
