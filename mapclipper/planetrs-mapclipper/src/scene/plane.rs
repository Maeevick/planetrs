//! A point on a plane, x then y.
//!
//! Every function in this module and its neighbours works on an abstract plane, which is what lets
//! the same clipping and simplification run on lon/lat and on projected pixels without knowing
//! which it has. In map space x is longitude and y is latitude, in that order, because that is how
//! GeoJSON orders a position and reversing it puts Abidjan in the Indian Ocean.

/// A point, x then y: longitude then latitude on the ground, across then down on a canvas.
pub type Point = [f64; 2];

/// The corners a shape spans, or nothing when it has no points.
pub fn bounds(points: &[Point]) -> Option<(Point, Point)> {
    let (mut west, mut south) = (f64::MAX, f64::MAX);
    let (mut east, mut north) = (f64::MIN, f64::MIN);

    for [x, y] in points {
        west = west.min(*x);
        east = east.max(*x);
        south = south.min(*y);
        north = north.max(*y);
    }

    (west <= east).then_some(([west, south], [east, north]))
}

/// The bounding box of a shape, as width and height.
pub fn extent(points: &[Point]) -> (f64, f64) {
    match bounds(points) {
        Some(([west, south], [east, north])) => (east - west, north - south),
        None => (0.0, 0.0),
    }
}

/// Twice the signed area of a ring, positive when it is wound anticlockwise.
pub fn signed_area(ring: &[Point]) -> f64 {
    let mut sum = 0.0;
    for index in 0..ring.len() {
        let [x1, y1] = ring[index];
        let [x2, y2] = ring[(index + 1) % ring.len()];
        sum += x1 * y2 - x2 * y1;
    }
    sum / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_shape_is_measured_then_its_extent_is_its_bounding_box() {
        assert_eq!(extent(&[[0.0, 0.0], [3.0, 0.0], [3.0, 2.0]]), (3.0, 2.0));
    }

    #[test]
    fn when_a_shape_has_no_points_then_it_has_no_extent() {
        assert_eq!(extent(&[]), (0.0, 0.0));
        assert_eq!(bounds(&[]), None);
    }

    #[test]
    fn when_a_ring_is_measured_then_its_area_does_not_depend_on_which_way_it_was_wound() {
        let anticlockwise = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let clockwise = [[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]];

        assert_eq!(signed_area(&anticlockwise).abs(), signed_area(&clockwise).abs());
    }

    #[test]
    fn when_a_ring_is_wound_anticlockwise_then_its_signed_area_is_positive() {
        assert!(signed_area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]) > 0.0);
    }
}
