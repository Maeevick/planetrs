//! Dropping detail the frame cannot show.
//!
//! An extract carries the map at survey precision: a quay wall is a hundred vertices, a shed is a
//! rectangle two metres across. At 908 pixels for twenty kilometres a pixel is twenty-two metres,
//! so most of that cannot be drawn at all and only makes the file bigger and the browser slower.
//!
//! Both rules here are expressed in pixels rather than in metres, because the frame is what decides
//! what is visible and the same harvest may be drawn at several sizes.

use planetrs_geo::BoundingBox;

use super::plane::{Point, bounds};

/// Ramer–Douglas–Peucker: the polyline through the same ends that never strays further than
/// `tolerance` from the original.
///
/// Iterative rather than recursive: a coastline chain can hold tens of thousands of vertices, and
/// the recursion depth on a degenerate one is its length.
pub fn douglas_peucker(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;

    let mut pending = vec![(0usize, points.len() - 1)];
    while let Some((first, last)) = pending.pop() {
        if last <= first + 1 {
            continue;
        }

        let mut worst = 0.0;
        let mut at = first;
        for index in (first + 1)..last {
            let distance = from_segment(points[index], points[first], points[last]);
            if distance > worst {
                worst = distance;
                at = index;
            }
        }

        if worst > tolerance {
            keep[at] = true;
            pending.push((first, at));
            pending.push((at, last));
        }
    }

    points
        .iter()
        .zip(keep)
        .filter_map(|(point, kept)| kept.then_some(*point))
        .collect()
}

fn from_segment([x, y]: Point, [x1, y1]: Point, [x2, y2]: Point) -> f64 {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let length = dx * dx + dy * dy;

    if length == 0.0 {
        return ((x - x1).powi(2) + (y - y1).powi(2)).sqrt();
    }

    let along = (((x - x1) * dx + (y - y1) * dy) / length).clamp(0.0, 1.0);
    let (nx, ny) = (x1 + along * dx, y1 + along * dy);
    ((x - nx).powi(2) + (y - ny).powi(2)).sqrt()
}

/// Whether a shape could put any ink inside the box, judged on bounding boxes alone.
///
/// Deliberately generous: a shape whose box overlaps but whose ink does not is kept and clipped by
/// the canvas, which costs a path. Being exact here would mean a full intersection test on every
/// one of a hundred and fifty thousand features to save nothing anyone can see.
pub fn touches(points: &[Point], frame: &BoundingBox) -> bool {
    match bounds(points) {
        None => false,
        Some(([west, south], [east, north])) => {
            east >= frame.west && west <= frame.east && north >= frame.south && south <= frame.north
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_points_on_a_straight_line_when_simplified_then_only_the_ends_survive() {
        let line = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];

        assert_eq!(douglas_peucker(&line, 0.1), vec![[0.0, 0.0], [3.0, 0.0]]);
    }

    #[test]
    fn given_a_vertex_straying_further_than_the_tolerance_when_simplified_then_it_is_kept() {
        assert_eq!(
            douglas_peucker(&[[0.0, 0.0], [1.0, 0.5], [2.0, 0.0]], 0.1).len(),
            3
        );
    }

    #[test]
    fn given_a_vertex_staying_within_the_tolerance_when_simplified_then_it_is_dropped() {
        assert_eq!(
            douglas_peucker(&[[0.0, 0.0], [1.0, 0.5], [2.0, 0.0]], 1.0).len(),
            2
        );
    }

    /// The ends anchor the shape: dropping either would move where a quay meets the water.
    #[test]
    fn given_a_wandering_line_when_simplified_then_its_ends_never_move() {
        let wandering: Vec<Point> = (0..200).map(|i| [i as f64, (i as f64 * 0.7).sin()]).collect();

        let simplified = douglas_peucker(&wandering, 0.5);

        assert_eq!(simplified.first(), wandering.first());
        assert_eq!(simplified.last(), wandering.last());
    }

    /// A chain of tens of thousands of vertices is ordinary for a coastline, and a recursive
    /// implementation overflows the stack on one that degenerates into a straight line.
    #[test]
    fn given_a_line_of_a_hundred_thousand_vertices_when_simplified_then_the_stack_is_not_exhausted() {
        let long: Vec<Point> = (0..100_000).map(|i| [i as f64, 0.0]).collect();

        assert_eq!(douglas_peucker(&long, 0.1).len(), 2);
    }

    #[test]
    fn given_a_shape_lying_beyond_the_frame_when_tested_against_it_then_it_is_not_drawn() {
        let frame = BoundingBox {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        };

        assert!(!touches(&[[2.0, 2.0], [3.0, 3.0]], &frame));
        assert!(!touches(&[], &frame));
    }

    #[test]
    fn given_a_shape_crossing_the_frame_with_no_vertex_inside_it_when_tested_against_it_then_it_is_still_drawn()
     {
        let frame = BoundingBox {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        };

        assert!(touches(&[[-1.0, 0.5], [2.0, 0.5]], &frame));
    }
}
