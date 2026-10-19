//! Closing an administrative boundary's ways into rings, and asking whether a point is inside.
//!
//! A boundary relation lists its ways in no guaranteed order and with no direction rule, so a ring
//! is recovered by following shared end nodes, reversing a way whenever it arrives backwards. What
//! does not close is counted, never patched: a straight line across a gap would invent territory.
//!
//! Containment uses the even-odd rule over every ring at once, which treats an enclave as a hole
//! without having to know which rings are outer and which inner. That is also why roles are not
//! read at all.

use std::collections::HashMap;

/// Rings recovered from a boundary's ways, as node identifiers, and how many chains failed to close.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JoinedRings {
    pub rings: Vec<Vec<i64>>,
    pub broken_chains: usize,
}

/// Joins ways end to end into closed rings.
pub fn join_into_rings(ways: &[Vec<i64>]) -> JoinedRings {
    let mut by_end: HashMap<i64, Vec<usize>> = HashMap::new();
    for (at, way) in ways.iter().enumerate() {
        if let (Some(first), Some(last)) = (way.first(), way.last()) {
            by_end.entry(*first).or_default().push(at);
            by_end.entry(*last).or_default().push(at);
        }
    }

    let mut used = vec![false; ways.len()];
    let mut joined = JoinedRings::default();

    for start in 0..ways.len() {
        if used[start] || ways[start].len() < 2 {
            continue;
        }
        used[start] = true;
        let mut chain = ways[start].clone();

        while chain.first() != chain.last() {
            let end = *chain.last().expect("a chain is never empty");
            let next = by_end
                .get(&end)
                .and_then(|candidates| candidates.iter().copied().find(|at| !used[*at]));
            let Some(next) = next else { break };
            used[next] = true;
            let way = &ways[next];
            if way.first() == Some(&end) {
                chain.extend_from_slice(&way[1..]);
            } else {
                chain.extend(way.iter().rev().skip(1));
            }
        }

        if chain.len() >= 4 && chain.first() == chain.last() {
            joined.rings.push(chain);
        } else {
            joined.broken_chains += 1;
        }
    }

    joined
}

/// How many latitude bands an outline's edges are filed under.
///
/// A ray cast from a point only ever meets edges that span the point's latitude, so filing edges by
/// band turns a walk over every edge of Russia into a walk over the few that cross one parallel.
/// Several outlines, the United States, Russia and Fiji among them, span the whole width of the
/// map, and without this every municipality on Earth would walk all of them.
const BANDS: usize = 2048;

/// The area enclosed by a commune's rings, in square kilometres, a ring inside another being a hole.
///
/// Measured on a plane through the rings' mean latitude, which at the size of a commune is exact to
/// well under a percent. Which ring is a hole is told by nesting rather than by winding, since
/// OpenStreetMap does not orient a boundary's rings.
pub fn area_square_kilometres(rings: &[Vec<[f64; 2]>]) -> f64 {
    const EARTH_KILOMETRES: f64 = 6_371.008_8;
    let points: Vec<&[f64; 2]> = rings.iter().flatten().collect();
    if points.is_empty() {
        return 0.0;
    }
    let mean_latitude = points.iter().map(|point| point[1]).sum::<f64>() / points.len() as f64;
    let squeeze = mean_latitude.to_radians().cos();
    let flat = |[longitude, latitude]: [f64; 2]| {
        [
            longitude.to_radians() * EARTH_KILOMETRES * squeeze,
            latitude.to_radians() * EARTH_KILOMETRES,
        ]
    };
    let enclosed = |ring: &[[f64; 2]]| {
        let flattened: Vec<[f64; 2]> = ring.iter().map(|point| flat(*point)).collect();
        flattened
            .windows(2)
            .map(|pair| pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1])
            .sum::<f64>()
            .abs()
            / 2.0
    };
    rings
        .iter()
        .enumerate()
        .filter(|(_, ring)| ring.len() >= 4)
        .map(|(at, ring)| {
            let depth = rings
                .iter()
                .enumerate()
                .filter(|(other, around)| *other != at && around.len() >= 4 && ring_contains(around, ring[0]))
                .count();
            if depth % 2 == 0 {
                enclosed(ring)
            } else {
                -enclosed(ring)
            }
        })
        .sum::<f64>()
        .max(0.0)
}

fn ring_contains(ring: &[[f64; 2]], [longitude, latitude]: [f64; 2]) -> bool {
    let mut inside = false;
    for pair in ring.windows(2) {
        let ([x1, y1], [x2, y2]) = (pair[0], pair[1]);
        if (y1 > latitude) != (y2 > latitude) && longitude < x1 + (latitude - y1) / (y2 - y1) * (x2 - x1) {
            inside = !inside;
        }
    }
    inside
}

/// An area's rings as longitude and latitude pairs, with its edges filed by latitude band.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    edges: Vec<[f64; 4]>,
    bands: Vec<Vec<u32>>,
    ring_bounds: Vec<([f64; 2], [f64; 2])>,
    low: [f64; 2],
    high: [f64; 2],
}

impl Outline {
    pub fn of(rings: Vec<Vec<[f64; 2]>>) -> Self {
        let mut low = [f64::MAX; 2];
        let mut high = [f64::MIN; 2];
        for [longitude, latitude] in rings.iter().flatten() {
            low = [low[0].min(*longitude), low[1].min(*latitude)];
            high = [high[0].max(*longitude), high[1].max(*latitude)];
        }

        let ring_bounds = rings
            .iter()
            .map(|ring| {
                ring.iter().fold(
                    ([f64::MAX; 2], [f64::MIN; 2]),
                    |(low, high), [longitude, latitude]| {
                        (
                            [low[0].min(*longitude), low[1].min(*latitude)],
                            [high[0].max(*longitude), high[1].max(*latitude)],
                        )
                    },
                )
            })
            .collect();

        let edges: Vec<[f64; 4]> = rings
            .iter()
            .flat_map(|ring| {
                ring.windows(2)
                    .map(|pair| [pair[0][0], pair[0][1], pair[1][0], pair[1][1]])
            })
            .collect();

        let mut outline = Self {
            edges,
            bands: vec![Vec::new(); BANDS],
            ring_bounds,
            low,
            high,
        };
        for at in 0..outline.edges.len() {
            let [_, y1, _, y2] = outline.edges[at];
            let (from, to) = (outline.band_of(y1.min(y2)), outline.band_of(y1.max(y2)));
            for band in from..=to {
                outline.bands[band].push(at as u32);
            }
        }
        outline
    }

    fn band_of(&self, latitude: f64) -> usize {
        let height = self.high[1] - self.low[1];
        if height <= 0.0 {
            return 0;
        }
        (((latitude - self.low[1]) / height * BANDS as f64) as usize).min(BANDS - 1)
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Every edge, as the longitude and latitude of both ends.
    pub fn edges(&self) -> &[[f64; 4]] {
        &self.edges
    }

    /// Each ring's own box. France's outline as a whole spans from Polynesia to Réunion, so its
    /// overall box excludes almost nothing; its rings' boxes are what is actually small.
    pub fn ring_bounds(&self) -> &[([f64; 2], [f64; 2])] {
        &self.ring_bounds
    }

    /// The smallest box holding every ring, as south-west and north-east corners.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        (self.low, self.high)
    }

    /// Whether a longitude and latitude pair lies inside, by the even-odd rule over every ring.
    pub fn contains(&self, [longitude, latitude]: [f64; 2]) -> bool {
        if self.edges.is_empty()
            || longitude < self.low[0]
            || longitude > self.high[0]
            || latitude < self.low[1]
            || latitude > self.high[1]
        {
            return false;
        }

        let mut inside = false;
        for at in &self.bands[self.band_of(latitude)] {
            let [x1, y1, x2, y2] = self.edges[*at as usize];
            if (y1 > latitude) != (y2 > latitude) {
                let crossing = x1 + (latitude - y1) / (y2 - y1) * (x2 - x1);
                if longitude < crossing {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hundredth of a degree square at 45° north is 0.787 km by 1.113 km.
    #[test]
    fn given_a_small_square_when_measured_then_its_area_is_its_width_times_its_height() {
        let area = area_square_kilometres(&[square(5.0, 45.0, 0.01)]);

        assert!((area - 0.876).abs() < 0.01, "{area}");
    }

    #[test]
    fn given_a_ring_inside_another_when_measured_then_it_is_a_hole() {
        let with_hole = area_square_kilometres(&[square(5.0, 45.0, 0.02), square(5.005, 45.005, 0.01)]);
        let whole = area_square_kilometres(&[square(5.0, 45.0, 0.02)]);

        assert!(
            (with_hole - whole * 0.75).abs() < 0.02,
            "{with_hole} against {whole}"
        );
    }

    #[test]
    fn given_two_separate_rings_when_measured_then_their_areas_add() {
        let two = area_square_kilometres(&[square(5.0, 45.0, 0.01), square(6.0, 45.0, 0.01)]);

        assert!((two - 2.0 * 0.876).abs() < 0.02, "{two}");
    }

    #[test]
    fn given_two_ways_meeting_end_to_end_when_joined_then_they_close_one_ring() {
        let joined = join_into_rings(&[vec![1, 2, 3], vec![3, 4, 1]]);

        assert_eq!(joined.rings, vec![vec![1, 2, 3, 4, 1]]);
    }

    /// Boundary members carry no direction rule, and neighbouring communes routinely share a way
    /// drawn in whichever direction its first mapper chose.
    #[test]
    fn given_a_way_drawn_the_other_way_round_when_joined_then_it_is_reversed_to_fit() {
        let joined = join_into_rings(&[vec![1, 2, 3], vec![1, 4, 3]]);

        assert_eq!(joined.rings, vec![vec![1, 2, 3, 4, 1]]);
    }

    #[test]
    fn given_a_way_that_is_already_closed_when_joined_then_it_is_a_ring_on_its_own() {
        let joined = join_into_rings(&[vec![5, 6, 7, 5]]);

        assert_eq!(joined.rings, vec![vec![5, 6, 7, 5]]);
    }

    #[test]
    fn given_two_separate_loops_when_joined_then_each_becomes_its_own_ring() {
        let joined = join_into_rings(&[vec![1, 2, 3, 1], vec![8, 9, 10, 8]]);

        assert_eq!(joined.rings.len(), 2);
    }

    /// A gap means no ring, and closing it by drawing a straight line would invent territory.
    #[test]
    fn given_a_chain_with_a_gap_when_joined_then_it_is_counted_as_broken_rather_than_closed() {
        let joined = join_into_rings(&[vec![1, 2, 3], vec![4, 5, 6]]);

        assert!(joined.rings.is_empty());
        assert_eq!(joined.broken_chains, 2);
    }

    fn square(west: f64, south: f64, side: f64) -> Vec<[f64; 2]> {
        vec![
            [west, south],
            [west + side, south],
            [west + side, south + side],
            [west, south + side],
            [west, south],
        ]
    }

    #[test]
    fn given_a_point_inside_a_ring_when_tested_then_it_is_inside() {
        let area = Outline::of(vec![square(0.0, 0.0, 10.0)]);

        assert!(area.contains([5.0, 5.0]));
    }

    #[test]
    fn given_a_point_outside_every_ring_when_tested_then_it_is_outside() {
        let area = Outline::of(vec![square(0.0, 0.0, 10.0)]);

        assert!(!area.contains([15.0, 5.0]));
    }

    /// An enclave is an inner ring, and the even-odd rule treats it as a hole without being told
    /// which ring is which: Baarle-Nassau is inside Belgium's outline and not inside Belgium.
    #[test]
    fn given_a_point_inside_a_hole_when_tested_then_it_is_outside() {
        let area = Outline::of(vec![square(0.0, 0.0, 10.0), square(4.0, 4.0, 2.0)]);

        assert!(!area.contains([5.0, 5.0]));
    }

    #[test]
    fn given_a_point_on_a_separate_island_when_tested_then_it_is_inside() {
        let area = Outline::of(vec![square(0.0, 0.0, 10.0), square(20.0, 0.0, 2.0)]);

        assert!(area.contains([21.0, 1.0]));
    }

    #[test]
    fn given_no_rings_at_all_when_tested_then_nothing_is_inside() {
        let area = Outline::of(vec![]);

        assert!(!area.contains([0.0, 0.0]));
    }

    /// Filing edges by band must not change a single answer, so a ring crossing many bands is
    /// tested against the plain even-odd rule it replaces.
    #[test]
    fn given_a_ring_spanning_many_bands_when_tested_then_banding_changes_no_answer() {
        let zigzag: Vec<[f64; 2]> = (0..=40)
            .map(|at| [if at % 2 == 0 { 0.0 } else { 3.0 }, at as f64])
            .chain([[10.0, 40.0], [10.0, 0.0], [0.0, 0.0]])
            .collect();
        let area = Outline::of(vec![zigzag]);

        assert!(area.contains([5.0, 20.5]));
        assert!(
            !area.contains([1.0, 0.9]),
            "left of the zigzag, inside the box, outside the ring"
        );
        assert!(!area.contains([11.0, 20.0]));
        assert!(!area.contains([5.0, 41.0]));
    }
}
