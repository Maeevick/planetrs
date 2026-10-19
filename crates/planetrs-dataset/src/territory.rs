//! Which country a point of the planet belongs to, as every part of a dataset decides it.
//!
//! A country is the outline of its `admin_level=2` boundary relation, closed into rings from the
//! relation's ways. Both tools file what they index under these outlines, so the geocoder's communes
//! and the map's nodes fall in the same country however close to a border they lie.

use planetrs_geo::outline::{Outline, join_into_rings};

/// The countries a dataset is built for when none are named: France and Monaco, the first two the
/// dataset was proven on.
pub const DEFAULT_COUNTRIES: [&str; 2] = ["FR", "MC"];

/// The `admin_level` a country's boundary carries.
const COUNTRY_LEVEL: &str = "2";

/// The ISO 3166-1 code of the country a relation's tags describe, or nothing when they describe
/// something else.
///
/// A level-2 boundary with no ISO code is left out on purpose. Disputed and historical areas are
/// mapped at that level too, and treating one as a country would let it claim territory.
pub fn country_code(tags: &[(String, String)]) -> Option<String> {
    let value_of = |wanted: &str| {
        tags.iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    };
    if value_of("boundary") != Some("administrative") || value_of("admin_level") != Some(COUNTRY_LEVEL) {
        return None;
    }
    value_of("ISO3166-1")
        .or_else(|| value_of("ISO3166-1:alpha2"))
        .map(str::to_string)
}

/// A country's outline, closed from its boundary's ways, and what could not be closed.
#[derive(Debug, Clone, PartialEq)]
pub struct CountryOutline {
    /// Every ring that closed and whose nodes all have a position, as one outline.
    pub outline: Outline,
    /// Chains of ways that never closed into a ring, which are left out rather than patched.
    pub broken_chains: usize,
    /// Rings that closed but lack a node's position, which are left out too.
    pub rings_missing_nodes: usize,
}

/// Closes a country's boundary ways, each a list of node identifiers, into rings, and places each
/// ring's nodes with `position_of`.
///
/// What does not close, or cannot be placed, is counted and never patched: a straight line across a
/// gap would invent territory.
pub fn outline_of(ways: &[Vec<i64>], position_of: impl Fn(i64) -> Option<[f64; 2]>) -> CountryOutline {
    let joined = join_into_rings(ways);
    let mut rings = Vec::new();
    let mut rings_missing_nodes = 0;
    for ring in joined.rings {
        let points: Option<Vec<[f64; 2]>> = ring.iter().map(|id| position_of(*id)).collect();
        match points {
            Some(points) => rings.push(points),
            None => rings_missing_nodes += 1,
        }
    }
    CountryOutline {
        outline: Outline::of(rings),
        broken_chains: joined.broken_chains,
        rings_missing_nodes,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn given_a_country_boundary_when_its_code_is_read_then_it_is_its_iso_code() {
        let code = country_code(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("name", "France"),
            ("ISO3166-1", "FR"),
        ]));

        assert_eq!(code.as_deref(), Some("FR"));
    }

    #[test]
    fn given_a_country_boundary_stating_only_the_alpha_two_key_when_its_code_is_read_then_that_is_its_code() {
        let code = country_code(&tags(&[
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("ISO3166-1:alpha2", "MC"),
        ]));

        assert_eq!(code.as_deref(), Some("MC"));
    }

    #[test]
    fn given_a_boundary_below_country_level_when_its_code_is_read_then_it_is_no_country() {
        let code = country_code(&tags(&[
            ("boundary", "administrative"),
            ("admin_level", "4"),
            ("name", "Bretagne"),
            ("ISO3166-2", "FR-BRE"),
        ]));

        assert_eq!(code, None);
    }

    #[test]
    fn given_a_country_level_boundary_with_no_iso_code_when_its_code_is_read_then_it_is_no_country() {
        let code = country_code(&tags(&[
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("name", "Somewhere Disputed"),
        ]));

        assert_eq!(code, None);
    }

    /// A square of four ways, each running between two corners, as a boundary is often drawn.
    fn square_ways() -> Vec<Vec<i64>> {
        vec![vec![1, 2], vec![2, 3], vec![4, 3], vec![4, 1]]
    }

    fn corners() -> HashMap<i64, [f64; 2]> {
        HashMap::from([
            (1, [7.40, 43.72]),
            (2, [7.44, 43.72]),
            (3, [7.44, 43.75]),
            (4, [7.40, 43.75]),
        ])
    }

    #[test]
    fn given_ways_closing_a_square_in_any_direction_when_outlined_then_a_point_inside_is_contained() {
        let corners = corners();

        let outlined = outline_of(&square_ways(), |id| corners.get(&id).copied());

        assert!(outlined.outline.contains([7.42, 43.73]));
        assert!(!outlined.outline.contains([7.50, 43.73]));
        assert_eq!((outlined.broken_chains, outlined.rings_missing_nodes), (0, 0));
    }

    #[test]
    fn given_a_ring_with_a_node_of_no_position_when_outlined_then_it_is_left_out_and_counted() {
        let mut corners = corners();
        corners.remove(&3);

        let outlined = outline_of(&square_ways(), |id| corners.get(&id).copied());

        assert!(outlined.outline.is_empty());
        assert_eq!(outlined.rings_missing_nodes, 1);
    }

    #[test]
    fn given_ways_that_never_close_when_outlined_then_the_chain_is_left_out_and_counted() {
        let corners = corners();

        let outlined = outline_of(&[vec![1, 2], vec![2, 3]], |id| corners.get(&id).copied());

        assert!(outlined.outline.is_empty());
        assert_eq!(outlined.broken_chains, 1);
    }
}
