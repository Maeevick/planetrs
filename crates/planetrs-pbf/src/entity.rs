//! Nodes, ways and relations, read out of a decoded block without allocating per entity.

use crate::block::Block;
use crate::error::PbfError;
use crate::proto::{Packed, Reader, Undeltaed, Value};

pub const MEMBER_NODE: u64 = 0;
pub const MEMBER_WAY: u64 = 1;

/// A dense node group, still encoded. Nothing is decoded until one of the visitors runs.
pub struct DenseGroup<'a> {
    pub ids: &'a [u8],
    pub latitudes: &'a [u8],
    pub longitudes: &'a [u8],
    /// One flat stream for the whole group: key, value, key, value, then a zero to end that node.
    /// A node with no tags contributes just the zero, so the stream cannot be read without walking
    /// every node in order.
    pub keys_vals: &'a [u8],
}

/// The dense node arrays of a PrimitiveGroup, if it holds any.
///
/// Two levels, and conflating them is silent: the group's own field 2 carries the DenseNodes
/// message, and the parallel arrays are fields 1, 8 and 9 *of that message*. Read at the group
/// level those numbers mean `nodes` and nothing at all, so the arrays come back empty and every
/// node in the file is skipped without an error anywhere.
pub fn dense_group(group: &'_ [u8]) -> Result<Option<DenseGroup<'_>>, PbfError> {
    let mut reader = Reader::new(group);

    while let Some(field) = reader.next_field()? {
        if let (2, Value::Bytes(dense)) = (field.number, field.value) {
            return dense_arrays(dense).map(Some);
        }
    }

    Ok(None)
}

fn dense_arrays(dense: &'_ [u8]) -> Result<DenseGroup<'_>, PbfError> {
    let mut reader = Reader::new(dense);
    let mut group = DenseGroup {
        ids: &[],
        latitudes: &[],
        longitudes: &[],
        keys_vals: &[],
    };

    while let Some(field) = reader.next_field()? {
        match (field.number, field.value) {
            (1, Value::Bytes(bytes)) => group.ids = bytes,
            (8, Value::Bytes(bytes)) => group.latitudes = bytes,
            (9, Value::Bytes(bytes)) => group.longitudes = bytes,
            (10, Value::Bytes(bytes)) => group.keys_vals = bytes,
            _ => {}
        }
    }

    Ok(group)
}

impl DenseGroup<'_> {
    /// Walks the three parallel delta-coded arrays together, handing each node to `on`.
    ///
    /// Coordinates arrive in the block's own units, so the caller applies the block's granularity
    /// and origin to whichever it decides to keep, rather than paying for the conversion nine
    /// billion times over.
    pub fn visit<F: FnMut(i64, i64, i64)>(&self, on: &mut F) -> Result<(), PbfError> {
        let mut ids = Undeltaed::new(self.ids);
        let mut latitudes = Undeltaed::new(self.latitudes);
        let mut longitudes = Undeltaed::new(self.longitudes);

        while let Some(id) = ids.next().transpose()? {
            let latitude = latitudes.next().transpose()?.ok_or(PbfError::Truncated)?;
            let longitude = longitudes.next().transpose()?.ok_or(PbfError::Truncated)?;
            on(id, latitude, longitude);
        }

        Ok(())
    }

    /// Walks the nodes with their tags, reusing one buffer rather than allocating per node.
    pub fn visit_tagged<F: FnMut(i64, i64, i64, &[(u32, u32)])>(&self, on: &mut F) -> Result<(), PbfError> {
        let mut ids = Undeltaed::new(self.ids);
        let mut latitudes = Undeltaed::new(self.latitudes);
        let mut longitudes = Undeltaed::new(self.longitudes);
        let mut pairs = Packed::new(self.keys_vals);
        let tagged = !self.keys_vals.is_empty();
        let mut tags: Vec<(u32, u32)> = Vec::with_capacity(8);

        while let Some(id) = ids.next().transpose()? {
            let latitude = latitudes.next().transpose()?.ok_or(PbfError::Truncated)?;
            let longitude = longitudes.next().transpose()?.ok_or(PbfError::Truncated)?;

            tags.clear();
            if tagged {
                while let Some(key) = pairs.next().transpose()? {
                    if key == 0 {
                        break;
                    }
                    let value = pairs.next().transpose()?.ok_or(PbfError::Truncated)?;
                    tags.push((key as u32, value as u32));
                }
            }

            on(id, latitude, longitude, &tags);
        }

        Ok(())
    }
}

/// One way, with its tag indices and refs left packed.
pub struct RawWay<'a> {
    pub id: i64,
    pub keys: &'a [u8],
    pub values: &'a [u8],
    pub refs: &'a [u8],
}

impl<'a> RawWay<'a> {
    /// The node ids this way is built from, undeltaed.
    pub fn refs(&self) -> Undeltaed<'a> {
        Undeltaed::new(self.refs)
    }

    /// The way's tags, resolved through the block's string table.
    ///
    /// Keys and values are two parallel arrays of indices, not pairs: a way with three tags stores
    /// three key indices and three value indices, and reading them as alternating entries of one
    /// array would pair every tag with the wrong value.
    pub fn tags(&self, block: &Block<'a>) -> Vec<(&'a [u8], &'a [u8])> {
        paired_tags(self.keys, self.values, block)
    }
}

fn paired_tags<'a>(keys: &'a [u8], values: &'a [u8], block: &Block<'a>) -> Vec<(&'a [u8], &'a [u8])> {
    let mut values = Packed::new(values);

    Packed::new(keys)
        .filter_map(move |key| {
            let key = key.ok()?;
            let value = values.next()?.ok()?;
            Some((block.string(key as usize), block.string(value as usize)))
        })
        .collect()
}

pub fn visit_ways<F: FnMut(RawWay<'_>) -> Result<(), PbfError>>(
    group: &[u8],
    on: &mut F,
) -> Result<(), PbfError> {
    let mut reader = Reader::new(group);

    while let Some(field) = reader.next_field()? {
        if let (3, Value::Bytes(way)) = (field.number, field.value) {
            let mut inner = Reader::new(way);
            let mut raw = RawWay {
                id: 0,
                keys: &[],
                values: &[],
                refs: &[],
            };
            while let Some(part) = inner.next_field()? {
                match (part.number, part.value) {
                    (1, Value::Varint(id)) => raw.id = id as i64,
                    (2, Value::Bytes(bytes)) => raw.keys = bytes,
                    (3, Value::Bytes(bytes)) => raw.values = bytes,
                    (8, Value::Bytes(bytes)) => raw.refs = bytes,
                    _ => {}
                }
            }
            on(raw)?;
        }
    }

    Ok(())
}

/// One relation, with its member list still packed.
///
/// Member ids are delta coded across the whole list regardless of type, so a relation's ways cannot
/// be read without walking its nodes too. The three arrays, roles, ids and types, are parallel in
/// the same way a way's keys and values are.
pub struct RawRelation<'a> {
    pub id: i64,
    pub keys: &'a [u8],
    pub values: &'a [u8],
    pub roles: &'a [u8],
    pub members: &'a [u8],
    pub types: &'a [u8],
}

impl<'a> RawRelation<'a> {
    pub fn tags(&self, block: &Block<'a>) -> Vec<(&'a [u8], &'a [u8])> {
        paired_tags(self.keys, self.values, block)
    }

    /// Every member, with its kind and role. Ids are delta coded across all kinds together.
    pub fn members(&self, block: &Block<'a>) -> Vec<(i64, u64, &'a [u8])> {
        let mut roles = Packed::new(self.roles);
        let mut types = Packed::new(self.types);

        Undeltaed::new(self.members)
            .map_while(|id| {
                let id = id.ok()?;
                let role = roles.next().and_then(Result::ok).unwrap_or(0);
                let kind = types.next()?.ok()?;
                Some((id, kind, block.string(role as usize)))
            })
            .collect()
    }

    /// The members that are ways, with their roles resolved.
    pub fn ways(&self, block: &Block<'a>) -> Vec<(i64, &'a [u8])> {
        self.members(block)
            .into_iter()
            .filter(|(_, kind, _)| *kind == MEMBER_WAY)
            .map(|(id, _, role)| (id, role))
            .collect()
    }
}

pub fn visit_relations<F: FnMut(RawRelation<'_>) -> Result<(), PbfError>>(
    group: &[u8],
    on: &mut F,
) -> Result<(), PbfError> {
    let mut reader = Reader::new(group);

    while let Some(field) = reader.next_field()? {
        if let (4, Value::Bytes(relation)) = (field.number, field.value) {
            let mut inner = Reader::new(relation);
            let mut raw = RawRelation {
                id: 0,
                keys: &[],
                values: &[],
                roles: &[],
                members: &[],
                types: &[],
            };
            while let Some(part) = inner.next_field()? {
                match (part.number, part.value) {
                    (1, Value::Varint(id)) => raw.id = id as i64,
                    (2, Value::Bytes(bytes)) => raw.keys = bytes,
                    (3, Value::Bytes(bytes)) => raw.values = bytes,
                    (8, Value::Bytes(bytes)) => raw.roles = bytes,
                    (9, Value::Bytes(bytes)) => raw.members = bytes,
                    (10, Value::Bytes(bytes)) => raw.types = bytes,
                    _ => {}
                }
            }
            on(raw)?;
        }
    }

    Ok(())
}

/// Reverses [`zigzag`](crate::proto::zigzag) so a test fixture can be written in the values it means.
pub fn to_zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::zigzag;

    fn block_with(strings: Vec<&'static [u8]>) -> Block<'static> {
        Block {
            strings,
            granularity: 100,
            lat_offset: 0,
            lon_offset: 0,
            groups: vec![],
        }
    }

    /// The three arrays are delta coded independently and read in lockstep, so a node's position
    /// depends on every node before it in the block.
    #[test]
    fn when_dense_nodes_are_visited_then_deltas_accumulate_across_the_group() {
        let group = DenseGroup {
            ids: &[0x02, 0x04],
            latitudes: &[0x02, 0x02],
            longitudes: &[0x04, 0x01],
            keys_vals: &[],
        };

        let mut seen = Vec::new();
        group
            .visit(&mut |id, lat, lon| seen.push((id, lat, lon)))
            .unwrap();

        assert_eq!(seen, vec![(1, 1, 2), (3, 2, 1)]);
    }

    #[test]
    fn when_the_arrays_disagree_in_length_then_it_fails_rather_than_pairing_wrongly() {
        let group = DenseGroup {
            ids: &[0x02, 0x04],
            latitudes: &[0x02],
            longitudes: &[0x04],
            keys_vals: &[],
        };

        assert!(group.visit(&mut |_, _, _| {}).is_err());
    }

    /// Built as a PrimitiveGroup, not as a DenseNodes message, because the bug this guards against
    /// was reading the arrays one level too high: every field number still decoded, and the result
    /// was an empty group rather than an error. A 146 GB scan returned zero nodes and no error.
    #[test]
    fn when_a_group_holds_dense_nodes_then_they_are_found_under_the_groups_own_field() {
        let dense = b"\x0a\x02\x02\x04\x42\x02\x02\x02\x4a\x02\x04\x01";
        let mut group = vec![0x12, dense.len() as u8];
        group.extend_from_slice(dense);

        let found = dense_group(&group).unwrap().expect("a dense group");
        let mut seen = Vec::new();
        found
            .visit(&mut |id, lat, lon| seen.push((id, lat, lon)))
            .unwrap();

        assert_eq!(seen, vec![(1, 1, 2), (3, 2, 1)]);
    }

    #[test]
    fn when_a_group_holds_only_ways_then_it_reports_no_dense_nodes() {
        assert!(dense_group(b"\x1a\x02\x08\x07").unwrap().is_none());
    }

    /// The tag stream is shared by the whole group, so a node with no tags still has to consume its
    /// terminating zero or every node after it reads another node's tags.
    #[test]
    fn when_only_some_nodes_are_tagged_then_the_tags_land_on_the_right_ones() {
        let group = DenseGroup {
            ids: &[0x02, 0x04, 0x02],
            latitudes: &[0x02, 0x02, 0x02],
            longitudes: &[0x04, 0x01, 0x02],
            keys_vals: &[0, 1, 2, 3, 4, 0, 5, 6, 0],
        };

        let mut seen = Vec::new();
        group
            .visit_tagged(&mut |id, _, _, tags| seen.push((id, tags.to_vec())))
            .unwrap();

        assert_eq!(
            seen,
            vec![(1, vec![]), (3, vec![(1, 2), (3, 4)]), (4, vec![(5, 6)])]
        );
    }

    /// Keys and values are parallel arrays, so reading them as one alternating array pairs every
    /// tag with its neighbour's value: `highway=residential, name=Rue X` would come back as
    /// `highway=name` and `residential=Rue X`.
    #[test]
    fn when_a_way_has_tags_then_keys_pair_with_their_own_values_not_the_next_key() {
        let block = block_with(vec![b"", b"highway", b"residential", b"name", b"Rue du Port"]);
        let way = RawWay {
            id: 1,
            keys: &[1, 3],
            values: &[2, 4],
            refs: &[],
        };

        assert_eq!(
            way.tags(&block),
            vec![
                (&b"highway"[..], &b"residential"[..]),
                (&b"name"[..], &b"Rue du Port"[..])
            ]
        );
    }

    #[test]
    fn when_the_value_array_is_short_then_the_unpaired_key_is_dropped_rather_than_mispaired() {
        let block = block_with(vec![b"", b"highway", b"residential", b"name"]);
        let way = RawWay {
            id: 1,
            keys: &[1, 3],
            values: &[2],
            refs: &[],
        };

        assert_eq!(way.tags(&block), vec![(&b"highway"[..], &b"residential"[..])]);
    }

    #[test]
    fn when_a_way_is_read_then_its_refs_are_absolute_node_ids() {
        let way = RawWay {
            id: 7,
            keys: &[],
            values: &[],
            refs: &[0x80, 0x01, 0x02, 0x01],
        };

        let refs: Result<Vec<_>, _> = way.refs().collect();

        assert_eq!(refs.unwrap(), vec![64, 65, 64]);
    }

    /// Members are delta coded across every type at once, so skipping the node members without
    /// accumulating their deltas puts every way id after them wrong.
    #[test]
    fn when_a_relation_mixes_node_and_way_members_then_the_way_ids_still_accumulate() {
        let block = block_with(vec![b"", b"outer", b"inner"]);
        let relation = RawRelation {
            id: 1,
            keys: &[],
            values: &[],
            roles: &[0, 1, 2],
            members: &[0x14, 0x02, 0x04],
            types: &[MEMBER_NODE as u8, MEMBER_WAY as u8, MEMBER_WAY as u8],
        };

        assert_eq!(
            relation.ways(&block),
            vec![(11, &b"outer"[..]), (13, &b"inner"[..])]
        );
    }

    #[test]
    fn when_a_relation_is_read_then_every_member_keeps_its_kind() {
        let block = block_with(vec![b"", b"outer"]);
        let relation = RawRelation {
            id: 1,
            keys: &[],
            values: &[],
            roles: &[0, 1],
            members: &[0x02, 0x02],
            types: &[MEMBER_NODE as u8, MEMBER_WAY as u8],
        };

        let kinds: Vec<u64> = relation
            .members(&block)
            .into_iter()
            .map(|(_, kind, _)| kind)
            .collect();

        assert_eq!(kinds, vec![MEMBER_NODE, MEMBER_WAY]);
    }

    #[test]
    fn when_a_value_is_zigzagged_and_back_then_it_is_unchanged() {
        for value in [0i64, -1, 1, -5000, 14_158_677_401] {
            assert_eq!(zigzag(to_zigzag(value)), value);
        }
    }
}
