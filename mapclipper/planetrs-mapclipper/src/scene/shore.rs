//! Turning a coastline into something that can be filled.
//!
//! OSM does not store the sea. It stores `natural=coastline` as open, directed ways with **land on
//! the left and water on the right**, joined end to end around whole continents. A frame cut out of
//! that has a few line fragments crossing it and nothing that says which side is wet.
//!
//! Recovering the surface means closing those fragments against the frame's own edges: follow a
//! fragment in its own direction, then walk the frame's boundary clockwise until the next fragment
//! starts, and repeat until the loop closes. Clockwise is the whole trick. Walking the other way
//! yields the land instead, which is the same polygon inverted and looks entirely plausible until
//! you notice the sea is inland.

use super::plane::Point;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl Rect {
    fn holds(&self, [x, y]: Point) -> bool {
        x >= self.west && x <= self.east && y >= self.south && y <= self.north
    }

    /// Where a point on the boundary sits along a clockwise walk of it, in [0, 4).
    ///
    /// Clockwise with north up: up the west edge, east along the north edge, down the east edge,
    /// west along the south edge. Corners land on the whole numbers.
    fn along(&self, [x, y]: Point) -> f64 {
        let width = (self.east - self.west).max(f64::MIN_POSITIVE);
        let height = (self.north - self.south).max(f64::MIN_POSITIVE);
        let gap = |a: f64, b: f64| (a - b).abs();

        let candidates = [
            (gap(x, self.west), (y - self.south) / height),
            (gap(y, self.north), 1.0 + (x - self.west) / width),
            (gap(x, self.east), 2.0 + (self.north - y) / height),
            (gap(y, self.south), 3.0 + (self.east - x) / width),
        ];

        candidates
            .iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, position)| *position % 4.0)
            .unwrap_or(0.0)
    }

    fn corner(&self, index: usize) -> Point {
        match index % 4 {
            0 => [self.west, self.south],
            1 => [self.west, self.north],
            2 => [self.east, self.north],
            _ => [self.east, self.south],
        }
    }

    fn on_boundary(&self, point: Point) -> bool {
        let slack = ((self.east - self.west) + (self.north - self.south)) * 1e-9;
        !self.holds(point)
            || (point[0] - self.west).abs() <= slack
            || (self.east - point[0]).abs() <= slack
            || (point[1] - self.south).abs() <= slack
            || (self.north - point[1]).abs() <= slack
    }
}

impl From<planetrs_geo::BoundingBox> for Rect {
    fn from(box_: planetrs_geo::BoundingBox) -> Self {
        Self {
            west: box_.west,
            south: box_.south,
            east: box_.east,
            north: box_.north,
        }
    }
}

/// Liang–Barsky: the part of one segment that lies inside the rectangle, if any.
fn clip_segment(from: Point, to: Point, rect: &Rect) -> Option<(Point, Point, bool, bool)> {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let mut enter = 0.0f64;
    let mut leave = 1.0f64;

    for (direction, distance) in [
        (-dx, from[0] - rect.west),
        (dx, rect.east - from[0]),
        (-dy, from[1] - rect.south),
        (dy, rect.north - from[1]),
    ] {
        if direction == 0.0 {
            if distance < 0.0 {
                return None;
            }
        } else {
            let crossing = distance / direction;
            if direction < 0.0 {
                if crossing > leave {
                    return None;
                }
                enter = enter.max(crossing);
            } else {
                if crossing < enter {
                    return None;
                }
                leave = leave.min(crossing);
            }
        }
    }

    if enter > leave {
        return None;
    }

    Some((
        [from[0] + enter * dx, from[1] + enter * dy],
        [from[0] + leave * dx, from[1] + leave * dy],
        enter > 0.0,
        leave < 1.0,
    ))
}

/// The runs of a polyline that lie inside the rectangle, each cut at the boundary.
pub fn clip(points: &[Point], rect: &Rect) -> Vec<Vec<Point>> {
    let mut runs = Vec::new();
    let mut run: Vec<Point> = Vec::new();

    for pair in points.windows(2) {
        match clip_segment(pair[0], pair[1], rect) {
            None => {
                if run.len() > 1 {
                    runs.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
            }
            Some((start, end, entered, left)) => {
                if run.is_empty() || entered {
                    if run.len() > 1 {
                        runs.push(std::mem::take(&mut run));
                    } else {
                        run.clear();
                    }
                    run.push(start);
                }
                run.push(end);
                if left {
                    runs.push(std::mem::take(&mut run));
                }
            }
        }
    }

    if run.len() > 1 {
        runs.push(run);
    }

    runs.retain(|run| run.len() > 1);
    runs
}

/// Closes clipped coastline fragments into filled water polygons.
///
/// Every fragment is expected to begin and end on the boundary, which is what clipping to the frame
/// guarantees. A fragment lying wholly inside means the data itself stopped there, because the
/// collection box cut it, and it is dropped rather than closed against a shore that does not exist.
pub fn water(fragments: Vec<Vec<Point>>, rect: &Rect) -> Vec<Vec<Point>> {
    let usable: Vec<Vec<Point>> = fragments
        .into_iter()
        .filter(|fragment| {
            fragment.len() > 1
                && rect.on_boundary(fragment[0])
                && rect.on_boundary(fragment[fragment.len() - 1])
        })
        .collect();

    if usable.is_empty() {
        return Vec::new();
    }

    let entries: Vec<f64> = usable.iter().map(|fragment| rect.along(fragment[0])).collect();
    let mut used = vec![false; usable.len()];
    let mut polygons = Vec::new();

    for start in 0..usable.len() {
        if used[start] {
            continue;
        }

        let mut ring: Vec<Point> = Vec::new();
        let mut at = start;

        loop {
            used[at] = true;
            ring.extend_from_slice(&usable[at]);

            let leaving = rect.along(usable[at][usable[at].len() - 1]);
            let Some(next) = next_clockwise(&entries, &used, leaving, start) else {
                break;
            };
            walk_boundary(&mut ring, rect, leaving, entries[next]);

            if next == start {
                break;
            }
            at = next;
        }

        if ring.len() > 2 {
            polygons.push(ring);
        }
    }

    polygons
}

/// The fragment whose entry comes first when walking clockwise from `leaving`.
///
/// The starting fragment stays eligible so a loop can close on itself; every other used fragment is
/// skipped, which is what stops two rings from claiming the same shore.
fn next_clockwise(entries: &[f64], used: &[bool], leaving: f64, start: usize) -> Option<usize> {
    let ahead = |position: f64| (position - leaving).rem_euclid(4.0);

    (0..entries.len())
        .filter(|index| !used[*index] || *index == start)
        .min_by(|a, b| ahead(entries[*a]).total_cmp(&ahead(entries[*b])))
}

/// Adds the boundary corners crossed between two positions, walking clockwise.
fn walk_boundary(ring: &mut Vec<Point>, rect: &Rect, from: f64, to: f64) {
    let span = (to - from).rem_euclid(4.0);
    let mut step = from.floor() + 1.0;

    while (step - from).rem_euclid(4.0) <= span && (step - from).rem_euclid(4.0) > 0.0 {
        ring.push(rect.corner(step as usize));
        step += 1.0;
        if step - from > 4.0 {
            break;
        }
    }
}

/// Sutherland–Hodgman: a closed ring cut down to the part inside the rectangle.
///
/// Used on multipolygon rings, which can be enormous. The Ébrié lagoon's outer ring runs a hundred
/// and thirty kilometres for a frame twenty across, and clipping is what keeps the output about the
/// frame it claims to describe rather than shipping a whole lagoon to draw a corner of one.
pub fn clip_ring(points: &[Point], rect: &Rect) -> Vec<Point> {
    let inside = |[x, y]: Point, edge: usize| match edge {
        0 => x >= rect.west,
        1 => x <= rect.east,
        2 => y >= rect.south,
        _ => y <= rect.north,
    };

    let cross = |a: Point, b: Point, edge: usize| -> Point {
        let (limit, vertical) = match edge {
            0 => (rect.west, true),
            1 => (rect.east, true),
            2 => (rect.south, false),
            _ => (rect.north, false),
        };
        if vertical {
            let t = (limit - a[0]) / (b[0] - a[0]);
            [limit, a[1] + t * (b[1] - a[1])]
        } else {
            let t = (limit - a[1]) / (b[1] - a[1]);
            [a[0] + t * (b[0] - a[0]), limit]
        }
    };

    let mut ring = points.to_vec();
    if ring.len() > 1 && ring[0] == ring[ring.len() - 1] {
        ring.pop();
    }

    for edge in 0..4 {
        if ring.is_empty() {
            return Vec::new();
        }

        let mut next = Vec::with_capacity(ring.len());
        for index in 0..ring.len() {
            let current = ring[index];
            let previous = ring[(index + ring.len() - 1) % ring.len()];
            let (here, there) = (inside(current, edge), inside(previous, edge));

            if here {
                if !there {
                    next.push(cross(previous, current, edge));
                }
                next.push(current);
            } else if there {
                next.push(cross(previous, current, edge));
            }
        }
        ring = next;
    }

    if ring.len() > 2 {
        ring.push(ring[0]);
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::super::plane::signed_area;
    use super::*;

    fn unit() -> Rect {
        Rect {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        }
    }

    #[test]
    fn given_a_segment_across_the_frame_when_clipped_then_it_is_cut_at_both_edges() {
        let runs = clip(&[[-1.0, 0.5], [2.0, 0.5]], &unit());

        assert_eq!(runs, vec![vec![[0.0, 0.5], [1.0, 0.5]]]);
    }

    #[test]
    fn given_a_line_that_leaves_the_frame_and_comes_back_when_clipped_then_it_becomes_two_runs() {
        let runs = clip(&[[0.5, 0.5], [0.5, 2.0], [0.8, 2.0], [0.8, 0.5]], &unit());

        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn given_a_line_missing_the_frame_entirely_when_clipped_then_nothing_survives() {
        assert!(clip(&[[2.0, 2.0], [3.0, 3.0]], &unit()).is_empty());
    }

    #[test]
    fn given_the_four_corners_of_the_frame_when_measured_along_its_edge_then_they_run_clockwise_from_the_south_west()
     {
        let rect = unit();
        let corners = [
            rect.along([0.0, 0.0]),
            rect.along([0.0, 1.0]),
            rect.along([1.0, 1.0]),
            rect.along([1.0, 0.0]),
        ];

        assert!(
            corners
                .iter()
                .zip([0.0, 1.0, 2.0, 3.0])
                .all(|(got, want)| (got - want).abs() < 1e-9)
        );
    }

    /// The case the whole module exists for. A shore running west to east has land to its north and
    /// water to its south, so the filled polygon must be the southern half and nothing else.
    #[test]
    fn given_a_shore_running_east_when_the_sea_is_drawn_then_the_water_is_the_half_to_its_south() {
        let filled = water(vec![vec![[0.0, 0.5], [1.0, 0.5]]], &unit());

        assert_eq!(filled.len(), 1);
        assert!(
            filled[0].iter().all(|[_, y]| *y <= 0.5 + 1e-9),
            "nothing north of the shore"
        );
        assert!(
            (signed_area(&filled[0]).abs() - 0.5).abs() < 0.01,
            "half the frame"
        );
    }

    /// Same shore, walked the other way: land is now south and the water is the northern half.
    /// Getting this backwards fills the land and drowns the city, which still looks like a map.
    #[test]
    fn given_a_shore_running_west_when_the_sea_is_drawn_then_the_water_is_the_half_to_its_north() {
        let filled = water(vec![vec![[1.0, 0.5], [0.0, 0.5]]], &unit());

        assert_eq!(filled.len(), 1);
        assert!(
            filled[0].iter().all(|[_, y]| *y >= 0.5 - 1e-9),
            "nothing south of the shore"
        );
    }

    #[test]
    fn given_a_shore_cutting_a_corner_when_the_sea_is_drawn_then_only_that_corner_is_filled() {
        let filled = water(vec![vec![[0.0, 0.2], [0.2, 0.0]]], &unit());

        assert!(
            signed_area(&filled[0]).abs() < 0.05,
            "a corner, not half the frame"
        );
    }

    #[test]
    fn given_a_fragment_stopping_in_the_frame_when_the_sea_is_drawn_then_it_is_dropped_rather_than_closed() {
        assert!(water(vec![vec![[0.0, 0.5], [0.4, 0.5]]], &unit()).is_empty());
    }

    #[test]
    fn given_no_shore_at_all_when_the_sea_is_drawn_then_nothing_is_filled() {
        assert!(water(Vec::new(), &unit()).is_empty());
    }

    #[test]
    fn given_a_ring_wholly_inside_the_frame_when_clipped_then_it_is_left_alone() {
        let ring = vec![[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8], [0.2, 0.2]];

        assert!((signed_area(&clip_ring(&ring, &unit())).abs() - 0.36).abs() < 1e-9);
    }

    #[test]
    fn given_a_ring_swallowing_the_frame_when_clipped_then_only_the_frame_is_left() {
        let ring = vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0], [-5.0, -5.0]];

        assert!((signed_area(&clip_ring(&ring, &unit())).abs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn given_a_ring_overlapping_one_corner_when_clipped_then_only_that_corner_survives() {
        let ring = vec![[-1.0, -1.0], [0.5, -1.0], [0.5, 0.5], [-1.0, 0.5], [-1.0, -1.0]];

        assert!((signed_area(&clip_ring(&ring, &unit())).abs() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn given_a_ring_missing_the_frame_when_clipped_then_nothing_survives() {
        assert!(clip_ring(&[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 2.0]], &unit()).is_empty());
    }

    #[test]
    fn given_a_ring_crossing_the_frame_when_clipped_then_it_comes_back_closed() {
        let ring = vec![[-1.0, 0.5], [2.0, 0.5], [2.0, 0.9], [-1.0, 0.9], [-1.0, 0.5]];
        let clipped = clip_ring(&ring, &unit());

        assert_eq!(clipped.first(), clipped.last());
    }
}
