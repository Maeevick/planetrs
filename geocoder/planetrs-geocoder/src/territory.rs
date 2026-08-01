//! Which parts of the planet a country might occupy, cheap enough to ask of nine billion nodes.
//!
//! A way lists node identifiers and no coordinates, and the nodes come first in the file. So
//! deciding whether a road is in France means remembering, while the nodes stream past, which
//! identifiers fell in France. The exact outline test is too slow to run nine billion times; a
//! raster of the country at 0.05°, conservative at its borders, answers in one lookup.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::rings::Outline;

const CELL_DEGREES: f64 = 0.05;
const COLUMNS: usize = (360.0 / CELL_DEGREES) as usize;
const ROWS: usize = (180.0 / CELL_DEGREES) as usize;

/// A raster of the planet, one bit per 0.05° cell, set wherever a country might have territory.
pub struct TerritoryMask {
    bits: Vec<u64>,
}

fn cell_of([longitude, latitude]: [f64; 2]) -> Option<usize> {
    if !(-180.0..=180.0).contains(&longitude) || !(-90.0..=90.0).contains(&latitude) {
        return None;
    }
    let column = (((longitude + 180.0) / CELL_DEGREES) as usize).min(COLUMNS - 1);
    let row = (((latitude + 90.0) / CELL_DEGREES) as usize).min(ROWS - 1);
    Some(row * COLUMNS + column)
}

impl TerritoryMask {
    /// Every cell a border crosses, every neighbour of those, and every cell whose middle is inside.
    pub fn of(outlines: &[&Outline]) -> Self {
        let mut mask = Self {
            bits: vec![0; COLUMNS * ROWS / 64 + 1],
        };
        for outline in outlines {
            for [x1, y1, x2, y2] in outline.edges() {
                let steps =
                    (((x2 - x1).abs().max((y2 - y1).abs())) / (CELL_DEGREES / 2.0)).ceil() as usize + 1;
                for step in 0..=steps {
                    let share = step as f64 / steps as f64;
                    let point = [x1 + (x2 - x1) * share, y1 + (y2 - y1) * share];
                    for dx in [-CELL_DEGREES, 0.0, CELL_DEGREES] {
                        for dy in [-CELL_DEGREES, 0.0, CELL_DEGREES] {
                            mask.set([point[0] + dx, point[1] + dy]);
                        }
                    }
                }
            }
            for (low, high) in outline.ring_bounds() {
                let mut latitude = (low[1] / CELL_DEGREES).floor() * CELL_DEGREES + CELL_DEGREES / 2.0;
                while latitude <= high[1] {
                    let mut longitude = (low[0] / CELL_DEGREES).floor() * CELL_DEGREES + CELL_DEGREES / 2.0;
                    while longitude <= high[0] {
                        if outline.contains([longitude, latitude]) {
                            mask.set([longitude, latitude]);
                        }
                        longitude += CELL_DEGREES;
                    }
                    latitude += CELL_DEGREES;
                }
            }
        }
        mask
    }

    fn set(&mut self, point: [f64; 2]) {
        if let Some(cell) = cell_of(point) {
            self.bits[cell / 64] |= 1 << (cell % 64);
        }
    }

    pub fn might_hold(&self, point: [f64; 2]) -> bool {
        cell_of(point).is_some_and(|cell| self.bits[cell / 64] & (1 << (cell % 64)) != 0)
    }
}

/// A set of node identifiers, one bit each, safe to fill from every thread at once.
///
/// A hash set of the billion-odd nodes inside France's box would cost tens of gigabytes; one bit per
/// possible identifier costs under two.
pub struct IdentifierSet {
    bits: Vec<AtomicU64>,
}

impl IdentifierSet {
    pub fn up_to(largest: i64) -> Self {
        let words = (largest.max(0) as usize) / 64 + 1;
        Self {
            bits: (0..words).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    pub fn insert(&self, identifier: i64) {
        if let Some(word) = usize::try_from(identifier)
            .ok()
            .and_then(|at| self.bits.get(at / 64))
        {
            word.fetch_or(1 << (identifier as usize % 64), Ordering::Relaxed);
        }
    }

    pub fn contains(&self, identifier: i64) -> bool {
        usize::try_from(identifier)
            .ok()
            .and_then(|at| self.bits.get(at / 64))
            .is_some_and(|word| word.load(Ordering::Relaxed) & (1 << (identifier as usize % 64)) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rings::Outline;

    fn square(west: f64, south: f64, side: f64) -> Outline {
        Outline::of(vec![vec![
            [west, south],
            [west + side, south],
            [west + side, south + side],
            [west, south + side],
            [west, south],
        ]])
    }

    #[test]
    fn given_a_point_deep_inside_a_country_when_masked_then_it_might_be_held() {
        let mask = TerritoryMask::of(&[&square(2.0, 45.0, 3.0)]);

        assert!(mask.might_hold([3.5, 46.5]));
    }

    #[test]
    fn given_a_point_far_from_a_country_when_masked_then_it_cannot_be_held() {
        let mask = TerritoryMask::of(&[&square(2.0, 45.0, 3.0)]);

        assert!(!mask.might_hold([-70.0, -30.0]));
    }

    /// A border cell is part inside and part out. The mask is a first filter, and a node dropped here
    /// is lost for good, so a cell the border crosses is always kept.
    #[test]
    fn given_a_point_just_inside_the_border_when_masked_then_it_is_kept() {
        let mask = TerritoryMask::of(&[&square(2.0, 45.0, 3.0)]);

        assert!(mask.might_hold([2.001, 46.0]));
    }

    #[test]
    fn given_an_identifier_set_when_asked_then_it_holds_exactly_what_was_put_in() {
        let set = IdentifierSet::up_to(1_000);
        set.insert(7);
        set.insert(999);

        assert!(set.contains(7) && set.contains(999));
        assert!(!set.contains(8) && !set.contains(1_000_000));
    }
}
