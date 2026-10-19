//! Everything read out of the planet for one area, and nothing about how it is drawn.
//!
//! A harvest is the expensive half. Collecting one is minutes of reading a hundred and eighty
//! gigabytes; drawing from one is milliseconds. Keeping them apart is the difference between trying
//! six framings and settling for the first.
//!
//! It is also the half worth publishing. A harvest is a few tens of megabytes, it is a derived
//! database under ODbL, and anyone holding one can render from it without the planet file at all.
//! That is why the format is versioned, carries the provenance of the file it came from, and is
//! byte-deterministic: two runs over the same planet and the same area must produce the same bytes,
//! or a checksum over the result says nothing.

pub mod format;
mod id_filter;
pub mod passes;

use planetrs_geo::BoundingBox;

/// Tags in the order the file stated them, which is stable across runs and carries no meaning.
pub type Tags = Vec<(String, String)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub id: i64,
    /// Nanodegrees, exact and independent of the block the node was read from.
    pub latitude: i64,
    pub longitude: i64,
}

/// A node that carries a name, kept so the map can say what things are called.
///
/// Geometry nodes are kept by the million and store only a position, because a way's shape is all
/// that is wanted from them. A named node is a different thing: a hamlet, a church, a shop. There
/// are a few hundred of them in a frame rather than a few million, so they can afford their tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedNode {
    pub id: i64,
    pub latitude: i64,
    pub longitude: i64,
    pub tags: Tags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Way {
    pub id: i64,
    pub tags: Tags,
    pub refs: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: i64,
    pub tags: Tags,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub way: i64,
    pub role: String,
}

/// Which file a harvest came from, and which moment of OSM history that file is a snapshot of.
///
/// A coordinate that feeds an index calculation has to be defensible after the fact, which means
/// every harvest states the file it was cut from precisely enough to cut it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance {
    pub source_name: String,
    pub file_size: u64,
    pub blob_count: u64,
    pub replication_timestamp: i64,
    pub replication_sequence: i64,
    pub writing_program: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Harvest {
    pub provenance: Provenance,
    /// The ground actually read, which is a frame plus its margin. A harvest answers for any frame
    /// whose own collected box this one covers, and for no other.
    pub collected: BoundingBox,
    pub nodes: Vec<Node>,
    /// Every node in the collected box that carries a `name`, with its tags.
    pub named_nodes: Vec<NamedNode>,
    pub ways: Vec<Way>,
    pub relations: Vec<Relation>,
}

impl Harvest {
    /// The same harvest with every list in ascending id order.
    ///
    /// The passes fold across threads, so the order entities arrive in depends on how work was
    /// stolen and differs between two runs over the same input. Sorting is what makes the written
    /// bytes a function of the data rather than of the schedule.
    pub fn sorted(mut self) -> Self {
        self.nodes.sort_unstable_by_key(|node| node.id);
        self.nodes.dedup_by_key(|node| node.id);
        self.named_nodes.sort_unstable_by_key(|node| node.id);
        self.named_nodes.dedup_by_key(|node| node.id);
        self.ways.sort_unstable_by_key(|way| way.id);
        self.ways.dedup_by_key(|way| way.id);
        self.relations.sort_unstable_by_key(|relation| relation.id);
        self.relations.dedup_by_key(|relation| relation.id);
        self
    }

    /// Whether this harvest can answer for an area, which it can only do if it covers every bit of
    /// ground that area collects.
    ///
    /// Judged on the collected box and never on the drawn frame: a frame inside the harvest but
    /// asking for a wider margin needs ways the passes never looked for.
    pub fn answers_for(&self, collected: &BoundingBox) -> bool {
        self.collected.covers(collected)
    }

    pub fn node(&self, id: i64) -> Option<&Node> {
        self.nodes
            .binary_search_by_key(&id, |node| node.id)
            .ok()
            .map(|at| &self.nodes[at])
    }

    pub fn way(&self, id: i64) -> Option<&Way> {
        self.ways
            .binary_search_by_key(&id, |way| way.id)
            .ok()
            .map(|at| &self.ways[at])
    }
}

pub fn tag<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(id: i64) -> Way {
        Way {
            id,
            tags: vec![],
            refs: vec![],
        }
    }

    fn harvest(nodes: Vec<Node>, ways: Vec<Way>) -> Harvest {
        Harvest {
            provenance: Provenance::default(),
            collected: BoundingBox {
                west: -4.1,
                south: 5.2,
                east: -3.9,
                north: 5.3,
            },
            nodes,
            named_nodes: vec![],
            ways,
            relations: vec![],
        }
    }

    fn node(id: i64) -> Node {
        Node {
            id,
            latitude: 5_270_000_000,
            longitude: -4_010_000_000,
        }
    }

    /// The passes fold across threads, so two runs disagree on order unless this puts them right.
    #[test]
    fn when_a_harvest_is_sorted_then_every_list_runs_in_ascending_id_order() {
        let sorted = harvest(vec![node(9), node(2), node(5)], vec![way(30), way(10)]).sorted();

        assert_eq!(
            sorted.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![2, 5, 9]
        );
        assert_eq!(sorted.ways.iter().map(|w| w.id).collect::<Vec<_>>(), vec![10, 30]);
    }

    /// A member way fetched by the relation pass can already be in hand from the bbox pass, and
    /// writing it twice would put the same geometry on the map twice.
    #[test]
    fn when_the_same_entity_arrives_twice_then_only_one_of_it_survives() {
        let sorted = harvest(vec![node(2), node(2)], vec![way(10), way(10)]).sorted();

        assert_eq!(sorted.nodes.len(), 1);
        assert_eq!(sorted.ways.len(), 1);
    }

    #[test]
    fn when_a_node_is_looked_up_then_the_sorted_list_finds_it() {
        let sorted = harvest(vec![node(9), node(2), node(5)], vec![]).sorted();

        assert_eq!(sorted.node(5).map(|n| n.id), Some(5));
        assert_eq!(sorted.node(7), None);
    }

    #[test]
    fn when_an_area_sits_inside_the_harvested_ground_then_the_harvest_answers_for_it() {
        let held = harvest(vec![], vec![]);

        assert!(held.answers_for(&BoundingBox {
            west: -4.0,
            south: 5.25,
            east: -3.95,
            north: 5.28
        }));
    }

    /// The failure that cost seven full rescans: a frame that still fits but a margin that no
    /// longer does needs ways the passes never looked for.
    #[test]
    fn when_an_area_reaches_past_the_harvested_ground_then_the_harvest_cannot_answer_for_it() {
        let held = harvest(vec![], vec![]);

        assert!(!held.answers_for(&BoundingBox {
            west: -4.2,
            south: 5.25,
            east: -3.95,
            north: 5.28
        }));
    }

    #[test]
    fn when_a_tag_is_asked_for_then_its_own_value_comes_back() {
        let tags = vec![
            ("waterway".into(), "dock".into()),
            ("name".into(), "Bassin".into()),
        ];

        assert_eq!(tag(&tags, "name"), Some("Bassin"));
        assert_eq!(tag(&tags, "highway"), None);
    }
}
