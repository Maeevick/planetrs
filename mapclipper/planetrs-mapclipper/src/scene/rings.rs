//! Joining ways into chains and rings, and resolving them into coordinates.

use std::collections::HashMap;

use crate::harvest::{Member, Node, Way};

use super::plane::{Point, bounds};

/// A run of coordinates, closed into a ring or left open as a line, with any holes it encloses.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// The points in order.
    pub points: Vec<Point>,
    /// Inner rings: the islands in a lagoon, the courtyard in a block. Empty for everything else.
    pub holes: Vec<Vec<Point>>,
    /// Whether the last point joins the first.
    pub closed: bool,
}

/// Node positions by id, built once so resolving a way's refs is a hash lookup and not a search.
pub struct NodeTable(HashMap<i64, Point>);

impl NodeTable {
    pub fn of(nodes: &[Node]) -> Self {
        Self(
            nodes
                .iter()
                .map(|node| {
                    (
                        node.id,
                        [node.longitude as f64 * 1e-9, node.latitude as f64 * 1e-9],
                    )
                })
                .collect(),
        )
    }

    pub fn at(&self, id: i64) -> Option<Point> {
        self.0.get(&id).copied()
    }
}

/// Resolves a way's refs into one or more runs of coordinates.
///
/// A way is kept when any single node of it falls in the collected box, so its other ends may be
/// missing: a trunk road crossing the frame has thousands of vertices elsewhere in the country.
/// Each unbroken run becomes its own shape rather than being joined across the gap, because joining
/// would draw a straight line between two points the road never went between.
pub fn shapes(refs: &[i64], nodes: &NodeTable) -> Vec<Shape> {
    let closed_way = refs.len() > 3 && refs.first() == refs.last();
    let mut out = Vec::new();
    let mut run: Vec<Point> = Vec::new();
    let mut complete = true;

    for id in refs {
        match nodes.at(*id) {
            Some(point) => run.push(point),
            None => {
                complete = false;
                if run.len() > 1 {
                    out.push(Shape {
                        points: std::mem::take(&mut run),
                        holes: Vec::new(),
                        closed: false,
                    });
                } else {
                    run.clear();
                }
            }
        }
    }

    if run.len() > 1 {
        out.push(Shape {
            points: run,
            holes: Vec::new(),
            closed: closed_way && complete && out.is_empty(),
        });
    }

    out
}

/// Joins ways that share an end node into the longest continuous chains they form.
///
/// A coastline arrives as dozens of separate ways sharing end nodes, and clipping them one by one
/// would cut each at the frame's edge and lose the fact that they continue into one another. Only a
/// joined shore enters and leaves the frame in the places the real coast does.
///
/// Never reverses a way. A coastline's direction is what says which side is water, so flipping one
/// would put the sea on the land.
pub fn join(ways: &[(i64, Vec<i64>)]) -> Vec<Vec<i64>> {
    let usable: Vec<&Vec<i64>> = ways
        .iter()
        .map(|(_, refs)| refs)
        .filter(|refs| refs.len() > 1)
        .collect();

    let mut starts: HashMap<i64, usize> = HashMap::new();
    let mut ends: HashMap<i64, usize> = HashMap::new();
    for (index, refs) in usable.iter().enumerate() {
        starts.insert(refs[0], index);
        ends.insert(refs[refs.len() - 1], index);
    }

    let mut used = vec![false; usable.len()];
    let mut chains = Vec::new();

    for index in 0..usable.len() {
        if used[index] {
            continue;
        }

        let mut first = index;
        let mut walked = vec![false; usable.len()];
        walked[first] = true;
        while let Some(before) = ends.get(&usable[first][0]) {
            if used[*before] || walked[*before] {
                break;
            }
            walked[*before] = true;
            first = *before;
        }

        let mut chain: Vec<i64> = Vec::new();
        let mut at = first;
        loop {
            used[at] = true;
            let refs = usable[at];
            if chain.is_empty() {
                chain.extend_from_slice(refs);
            } else {
                chain.extend_from_slice(&refs[1..]);
            }

            match starts.get(&chain[chain.len() - 1]) {
                Some(next) if !used[*next] => at = *next,
                _ => break,
            }
        }

        chains.push(chain);
    }

    chains
}

/// Splits a relation's way members by role.
///
/// An empty role means outer: the multipolygon schema allows it and plenty of data uses it, so
/// treating only the literal `outer` as outer silently loses whole lagoons.
pub fn roles(members: &[Member]) -> (Vec<i64>, Vec<i64>) {
    let mut outer = Vec::new();
    let mut inner = Vec::new();

    for member in members {
        match member.role.as_str() {
            "inner" => inner.push(member.way),
            "outer" | "" => outer.push(member.way),
            _ => {}
        }
    }

    (outer, inner)
}

/// Joins ways into rings, reversing any that run the other way.
///
/// Deliberately separate from [`join`], which must never reverse anything. A multipolygon carries
/// no direction rule: the schema does not require its members to agree, and most large rings mix
/// both, so a ring cannot be assembled without reversing.
pub fn join_rings(ways: &[(i64, Vec<i64>)]) -> Vec<Vec<i64>> {
    let usable: Vec<&Vec<i64>> = ways
        .iter()
        .map(|(_, refs)| refs)
        .filter(|refs| refs.len() > 1)
        .collect();

    let mut at_node: HashMap<i64, Vec<usize>> = HashMap::new();
    for (index, refs) in usable.iter().enumerate() {
        at_node.entry(refs[0]).or_default().push(index);
        at_node.entry(refs[refs.len() - 1]).or_default().push(index);
    }

    let mut used = vec![false; usable.len()];
    let mut chains = Vec::new();

    let next_at = |node: i64, used: &[bool]| -> Option<usize> {
        at_node.get(&node)?.iter().copied().find(|index| !used[*index])
    };

    for start in 0..usable.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = usable[start].clone();

        while chain[0] != chain[chain.len() - 1] {
            let tail = chain[chain.len() - 1];
            let Some(next) = next_at(tail, &used) else { break };
            used[next] = true;
            let refs = usable[next];
            if refs[0] == tail {
                chain.extend_from_slice(&refs[1..]);
            } else {
                chain.extend(refs.iter().rev().skip(1).copied());
            }
        }

        while chain[0] != chain[chain.len() - 1] {
            let head = chain[0];
            let Some(next) = next_at(head, &used) else { break };
            used[next] = true;
            let refs = usable[next];
            let mut ahead: Vec<i64> = if refs[refs.len() - 1] == head {
                refs[..refs.len() - 1].to_vec()
            } else {
                refs.iter().rev().take(refs.len() - 1).copied().collect()
            };
            ahead.extend_from_slice(&chain);
            chain = ahead;
        }

        chains.push(chain);
    }

    chains
}

/// The rings a relation's members form, once the members are all in hand.
pub fn multipolygon(members: &[Member], refs_of: &HashMap<i64, &Vec<i64>>, nodes: &NodeTable) -> Vec<Shape> {
    let assemble = |ids: &[i64]| -> Vec<Vec<Point>> {
        let ways: Vec<(i64, Vec<i64>)> = ids
            .iter()
            .filter_map(|way| refs_of.get(way).map(|refs| (*way, (*refs).clone())))
            .collect();

        join_rings(&ways)
            .iter()
            .flat_map(|chain| shapes(chain, nodes))
            .filter(|shape| shape.closed)
            .map(|shape| shape.points)
            .collect()
    };

    let (outer_ids, inner_ids) = roles(members);
    let outers = assemble(&outer_ids);
    let inners = assemble(&inner_ids);

    outers
        .into_iter()
        .map(|points| with_islands(points, &inners))
        .collect()
}

/// An island belongs to the ring that surrounds it.
///
/// Bounding boxes are enough: multipolygon outer rings do not overlap, so no island can be claimed
/// twice.
fn with_islands(points: Vec<Point>, inners: &[Vec<Point>]) -> Shape {
    let Some((low, high)) = bounds(&points) else {
        return Shape {
            points,
            holes: Vec::new(),
            closed: true,
        };
    };

    let holes = inners
        .iter()
        .filter(|hole| {
            bounds(hole).is_some_and(|(hole_low, hole_high)| {
                hole_low[0] >= low[0]
                    && hole_low[1] >= low[1]
                    && hole_high[0] <= high[0]
                    && hole_high[1] <= high[1]
            })
        })
        .cloned()
        .collect();

    Shape {
        points,
        holes,
        closed: true,
    }
}

/// The refs of every way, so a relation's members can be looked up without searching.
pub fn refs_of(ways: &[Way]) -> HashMap<i64, &Vec<i64>> {
    ways.iter().map(|way| (way.id, &way.refs)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> NodeTable {
        NodeTable::of(&[
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
            Node {
                id: 4,
                latitude: 5_273_000_000,
                longitude: -4_013_000_000,
            },
            Node {
                id: 5,
                latitude: 5_274_000_000,
                longitude: -4_014_000_000,
            },
        ])
    }

    fn members(pairs: &[(i64, &str)]) -> Vec<Member> {
        pairs
            .iter()
            .map(|(way, role)| Member {
                way: *way,
                role: (*role).to_string(),
            })
            .collect()
    }

    #[test]
    fn given_a_node_at_abidjan_when_resolved_then_longitude_comes_first_as_geojson_orders_it() {
        let point = table().at(1).unwrap();

        assert!((point[0] - -4.01).abs() < 1e-9, "longitude");
        assert!((point[1] - 5.27).abs() < 1e-9, "latitude");
    }

    #[test]
    fn given_a_way_whose_every_node_is_in_hand_when_resolved_then_one_shape_comes_back() {
        assert_eq!(shapes(&[1, 2], &table()).len(), 1);
    }

    /// The gap is where the way leaves the collected box. Bridging it would draw a straight line
    /// across ground the road never crossed.
    #[test]
    fn given_a_way_that_leaves_the_collected_box_and_returns_when_resolved_then_its_runs_stay_separate() {
        let runs = shapes(&[1, 2, 3, 4, 5], &table());

        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.points.len() == 2));
    }

    #[test]
    fn given_a_closed_way_missing_a_node_when_resolved_then_it_is_not_reported_as_a_ring() {
        assert!(
            shapes(&[1, 2, 3, 4, 1], &table())
                .iter()
                .all(|shape| !shape.closed)
        );
    }

    #[test]
    fn given_a_closed_way_with_every_node_in_hand_when_resolved_then_it_is_a_ring() {
        let mut nodes = vec![
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
            Node {
                id: 3,
                latitude: 5_272_000_000,
                longitude: -4_012_000_000,
            },
            Node {
                id: 4,
                latitude: 5_273_000_000,
                longitude: -4_013_000_000,
            },
        ];
        nodes.sort_by_key(|node| node.id);

        let ring = shapes(&[1, 2, 3, 4, 1], &NodeTable::of(&nodes));

        assert_eq!(ring.len(), 1);
        assert!(ring[0].closed);
    }

    #[test]
    fn given_a_member_with_an_empty_role_when_split_by_role_then_it_counts_as_outer() {
        let split = roles(&members(&[(1, "outer"), (2, ""), (3, "inner"), (4, "label")]));

        assert_eq!(split, (vec![1, 2], vec![3]));
    }

    #[test]
    fn given_three_ways_sharing_end_nodes_when_joined_then_they_become_one_chain() {
        let ways = vec![(1, vec![10, 11, 12]), (2, vec![12, 13, 14]), (3, vec![14, 15])];

        assert_eq!(join(&ways), vec![vec![10, 11, 12, 13, 14, 15]]);
    }

    /// The file gives no order, so the middle of a shore is as likely to come first as its start.
    #[test]
    fn given_ways_arriving_out_of_order_when_joined_then_the_chain_is_still_whole() {
        let ways = vec![(2, vec![12, 13, 14]), (3, vec![14, 15]), (1, vec![10, 11, 12])];

        assert_eq!(join(&ways), vec![vec![10, 11, 12, 13, 14, 15]]);
    }

    /// An island's shore closes on itself. Walking back from any way would loop forever without the
    /// guard, so this is the test that the guard is really there.
    #[test]
    fn given_an_island_shore_closing_on_itself_when_joined_then_the_walk_terminates() {
        let joined = join(&[(1, vec![10, 11, 12]), (2, vec![12, 13, 10])]);

        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].first(), joined[0].last());
    }

    #[test]
    fn given_two_shores_that_do_not_meet_when_joined_then_they_stay_separate() {
        assert_eq!(join(&[(1, vec![10, 11]), (2, vec![20, 21])]).len(), 2);
    }

    /// The case that kept the Ébrié lagoon unfilled with all 166 of its members in hand: one member
    /// runs against the ring, a directed join refuses to turn it round, and the ring never closes.
    #[test]
    fn given_a_ring_member_running_backwards_when_joined_into_rings_then_it_is_turned_round() {
        let ways = vec![
            (1, vec![10, 11, 12]),
            (2, vec![14, 13, 12]),
            (3, vec![14, 15, 10]),
        ];

        let rings = join_rings(&ways);

        assert_eq!(rings, vec![vec![10, 11, 12, 13, 14, 15, 10]]);
    }

    #[test]
    fn given_ring_members_that_all_agree_when_joined_into_rings_then_the_ring_is_unchanged() {
        let ways = vec![(1, vec![10, 11]), (2, vec![11, 12]), (3, vec![12, 10])];

        assert_eq!(join_rings(&ways), vec![vec![10, 11, 12, 10]]);
    }

    #[test]
    fn given_members_forming_two_separate_rings_when_joined_into_rings_then_both_close() {
        let ways = vec![(1, vec![10, 11, 10]), (2, vec![20, 21]), (3, vec![21, 20])];

        let rings = join_rings(&ways);

        assert_eq!(rings.len(), 2);
        assert!(rings.iter().all(|ring| ring.first() == ring.last()));
    }

    /// Reversing is what `join_rings` is for and what `join` must never do.
    #[test]
    fn given_a_shore_running_backwards_when_joined_then_it_is_left_alone_rather_than_reversed() {
        let ways = vec![(1, vec![10, 11, 12]), (2, vec![14, 13, 12])];

        assert_eq!(join(&ways).len(), 2, "a directed join keeps them apart");
        assert_eq!(join_rings(&ways).len(), 1, "an undirected one joins them");
    }

    #[test]
    fn given_a_lagoon_with_an_island_inside_when_assembled_then_the_island_becomes_a_hole_in_it() {
        let nodes = NodeTable::of(&[
            Node {
                id: 1,
                latitude: 0,
                longitude: 0,
            },
            Node {
                id: 2,
                latitude: 0,
                longitude: 10_000_000_000,
            },
            Node {
                id: 3,
                latitude: 10_000_000_000,
                longitude: 10_000_000_000,
            },
            Node {
                id: 4,
                latitude: 10_000_000_000,
                longitude: 0,
            },
            Node {
                id: 5,
                latitude: 4_000_000_000,
                longitude: 4_000_000_000,
            },
            Node {
                id: 6,
                latitude: 4_000_000_000,
                longitude: 6_000_000_000,
            },
            Node {
                id: 7,
                latitude: 6_000_000_000,
                longitude: 6_000_000_000,
            },
        ]);
        let outer = vec![1, 2, 3, 4, 1];
        let inner = vec![5, 6, 7, 5];
        let mut refs = HashMap::new();
        refs.insert(10, &outer);
        refs.insert(11, &inner);

        let rings = multipolygon(&members(&[(10, "outer"), (11, "inner")]), &refs, &nodes);

        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].holes.len(), 1);
    }
}
