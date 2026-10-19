//! The Riviera as a planet: France and Monaco side by side, with named and unnamed nodes in each, a
//! road across their border, a road leaving France for land no index covers, and a lake whose inner
//! ring lies far away.

use planetrs_pbf::testing::{MemberKind, Node, PlanetFile, Relation, Way};

/// A ring of four corners closed on its first node, as one way.
fn ring(way: i64, first_node: i64, south: f64, west: f64, north: f64, east: f64) -> (Vec<Node>, Way) {
    let corners = [(south, west), (south, east), (north, east), (north, west)];
    let nodes = corners
        .iter()
        .zip(first_node..)
        .map(|((latitude, longitude), id)| Node::at(id, *latitude, *longitude))
        .collect();
    (
        nodes,
        Way::through(way, (first_node..first_node + 4).chain([first_node])),
    )
}

fn country(relation: i64, code: &str, outline: i64) -> Relation {
    Relation::of(relation)
        .tagged("type", "boundary")
        .tagged("boundary", "administrative")
        .tagged("admin_level", "2")
        .tagged("ISO3166-1", code)
        .with_member(MemberKind::Way, outline, "outer")
}

pub fn riviera() -> PlanetFile {
    let (france_corners, france) = ring(2001, 1001, 43.60, 7.00, 43.80, 7.40);
    let (monaco_corners, monaco) = ring(2002, 1011, 43.70, 7.40, 43.76, 7.45);
    let planet = PlanetFile::sorted()
        .replicated_at(1_788_739_204, 4_321)
        .with_node(
            Node::at(1, 43.70, 7.26)
                .tagged("name", "Nice")
                .tagged("place", "city"),
        )
        .with_node(Node::at(2, 43.65, 7.10))
        .with_node(
            Node::at(3, 43.73, 7.42)
                .tagged("name", "Monaco")
                .tagged("place", "city"),
        )
        .with_node(Node::at(4, 43.71, 7.35))
        .with_node(Node::at(5, 43.73, 7.43))
        .with_node(Node::at(6, 43.69, 7.30))
        .with_node(Node::at(7, 44.20, 7.30))
        .with_node(Node::at(8, 43.62, 7.05))
        .with_node(Node::at(9, 43.62, 7.08))
        .with_node(Node::at(13, 43.64, 7.08))
        .with_node(Node::at(15, 45.00, 8.00))
        .with_node(Node::at(16, 45.00, 8.01))
        .with_node(Node::at(17, 45.01, 8.01));
    france_corners
        .into_iter()
        .chain(monaco_corners)
        .fold(planet, PlanetFile::with_node)
        .with_way(
            Way::through(10, [4, 5])
                .tagged("highway", "primary")
                .tagged("name", "Basse Corniche"),
        )
        .with_way(Way::through(11, [6, 7]).tagged("highway", "secondary"))
        .with_way(Way::through(12, [8, 9, 13, 8]))
        .with_way(Way::through(14, [15, 16, 17, 15]))
        .with_way(france)
        .with_way(monaco)
        .with_relation(
            Relation::of(100)
                .tagged("type", "multipolygon")
                .tagged("natural", "water")
                .with_member(MemberKind::Way, 12, "outer")
                .with_member(MemberKind::Way, 14, "inner"),
        )
        .with_relation(country(3001, "FR", 2001))
        .with_relation(country(3002, "MC", 2002))
}
