//! A box in one block's own integer units, so the test against nine billion nodes is four integer
//! comparisons rather than four multiplications and a conversion.

use crate::bounds::BoundingBox;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub west: i64,
    pub south: i64,
    pub east: i64,
    pub north: i64,
}

impl Window {
    /// Every edge rounds outward, so the window covers the box it came from.
    ///
    /// Not a nicety: a degree is not representable in binary, and `5.3 * 1e9 / 100` lands at
    /// 52 999 999.9999 rather than 53 000 000. Rounding to nearest would put an edge one unit
    /// inside or outside the asked-for box depending on how the value happens to fall, which is a
    /// centimetre on the ground and an off-by-one in every test written against it.
    pub fn of(box_: &BoundingBox, granularity: i64, lat_offset: i64, lon_offset: i64) -> Self {
        let units = |degrees: f64, offset: i64| (degrees * 1e9 - offset as f64) / granularity as f64;

        Self {
            west: units(box_.west, lon_offset).floor() as i64,
            south: units(box_.south, lat_offset).floor() as i64,
            east: units(box_.east, lon_offset).ceil() as i64,
            north: units(box_.north, lat_offset).ceil() as i64,
        }
    }

    pub fn holds(&self, latitude: i64, longitude: i64) -> bool {
        latitude >= self.south && latitude <= self.north && longitude >= self.west && longitude <= self.east
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_() -> BoundingBox {
        BoundingBox {
            west: -4.1,
            south: 5.2,
            east: -3.9,
            north: 5.3,
        }
    }

    #[test]
    fn when_a_window_is_built_then_degrees_become_the_blocks_own_units() {
        let window = Window::of(&box_(), 100, 0, 0);

        assert_eq!((window.west, window.north), (-41_000_000, 53_000_000));
    }

    #[test]
    fn when_a_node_falls_inside_the_window_then_it_is_kept() {
        let window = Window::of(&box_(), 100, 0, 0);

        assert!(window.holds(52_700_000, -40_100_000));
    }

    #[test]
    fn when_a_node_falls_outside_the_window_then_it_is_not_kept() {
        let window = Window::of(&box_(), 100, 0, 0);

        assert!(!window.holds(52_700_000, -38_999_999));
        assert!(!window.holds(53_000_001, -40_100_000));
    }

    /// Rounding to nearest would make this depend on how the value happens to fall in binary.
    #[test]
    fn when_an_edge_lands_between_two_units_then_the_window_covers_the_box_rather_than_clipping_it() {
        let tight = BoundingBox {
            west: -4.05,
            south: 5.25,
            east: -3.95,
            north: 5.35,
        };
        let window = Window::of(&tight, 100, 0, 0);

        assert!(window.west as f64 * 1e-7 <= tight.west);
        assert!(window.south as f64 * 1e-7 <= tight.south);
        assert!(window.east as f64 * 1e-7 >= tight.east);
        assert!(window.north as f64 * 1e-7 >= tight.north);
    }

    /// Blocks are allowed their own origin and step. The planet file uses 100/0/0 throughout, but a
    /// regional extract cut by another tool may not, and reading those wrongly would silently place
    /// every node somewhere else.
    #[test]
    fn when_a_block_declares_its_own_origin_then_the_window_moves_with_it() {
        let shifted = Window::of(&box_(), 1000, 5_000_000_000, -4_000_000_000);

        assert_eq!((shifted.west, shifted.south), (-100_000, 200_000));
    }
}
