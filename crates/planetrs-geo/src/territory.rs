//! Which parts of the planet a country might occupy, cheap enough to ask of nine billion nodes.
//!
//! A way lists node identifiers and no coordinates, and the nodes come first in the file. So
//! deciding whether a road is in France means remembering, while the nodes stream past, which
//! identifiers fell in France. The exact outline test is too slow to run nine billion times; a
//! raster of the country at 0.05°, conservative at its borders, answers in one lookup.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bounds::BoundingBox;
use crate::outline::Outline;

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

/// How wide a cell of coverage is: a hundredth of a degree, a kilometre or so, so that a box need
/// only keep that far from a border it does not cross.
const COVERAGE_DEGREES: f64 = 0.01;

/// The cells of a hundredth of a degree that lie wholly inside a country indexed, where a box can be
/// cut from a dataset and lack nothing the planet holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// By row of latitude, the runs of covered columns, each as its first and last.
    rows: BTreeMap<i32, Vec<(i32, i32)>>,
}

impl Coverage {
    /// The coverage of each outline of `outlines`, indexed together: the cells inside it that no
    /// border crosses, a border two of them share excepted, since both sides of it are indexed.
    ///
    /// A border is told shared by its edges, which two countries' outlines hold alike when, as in
    /// OpenStreetMap, the countries share the ways drawing it.
    pub fn of_each(outlines: &[&Outline]) -> Vec<Coverage> {
        let crossed = crossed_by_outer_borders(outlines);
        outlines
            .iter()
            .map(|outline| {
                let mut covered: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
                for (low, high) in outline.ring_bounds() {
                    for row in cell_of_coverage(low[1])..=cell_of_coverage(high[1]) {
                        for column in cell_of_coverage(low[0])..=cell_of_coverage(high[0]) {
                            let centre = [
                                (f64::from(column) + 0.5) * COVERAGE_DEGREES,
                                (f64::from(row) + 0.5) * COVERAGE_DEGREES,
                            ];
                            if !crossed.contains(&(row, column)) && outline.contains(centre) {
                                covered.entry(row).or_default().push(column);
                            }
                        }
                    }
                }
                Coverage {
                    rows: covered
                        .into_iter()
                        .map(|(row, mut columns)| {
                            columns.sort_unstable();
                            columns.dedup();
                            (row, runs_of(&columns))
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Whether every cell `box_` touches is covered by one of `coverages`.
    pub fn cover(coverages: &[Coverage], box_: &BoundingBox) -> bool {
        (cell_of_coverage(box_.south)..=cell_of_coverage(box_.north)).all(|row| {
            (cell_of_coverage(box_.west)..=cell_of_coverage(box_.east))
                .all(|column| coverages.iter().any(|coverage| coverage.holds(row, column)))
        })
    }

    /// The coverage as text, as a map index's footer keeps it: the cell size, then each row with
    /// its runs of columns, `0.01;4372:740..743,750..751;-1:-5..-3`.
    pub fn to_text(&self) -> String {
        std::iter::once(COVERAGE_DEGREES.to_string())
            .chain(self.rows.iter().map(|(row, runs)| {
                let runs: Vec<String> = runs
                    .iter()
                    .map(|(first, last)| format!("{first}..{last}"))
                    .collect();
                format!("{row}:{}", runs.join(","))
            }))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The coverage text stands for, or nothing when it is not one, or one of another cell size.
    pub fn from_text(text: &str) -> Option<Coverage> {
        let mut parts = text.split(';');
        if parts.next()?.parse::<f64>().ok()? != COVERAGE_DEGREES {
            return None;
        }
        let rows = parts
            .map(|part| {
                let (row, runs) = part.split_once(':')?;
                let runs = runs
                    .split(',')
                    .map(|run| {
                        let (first, last) = run.split_once("..")?;
                        Some((first.parse().ok()?, last.parse().ok()?))
                    })
                    .collect::<Option<Vec<(i32, i32)>>>()?;
                Some((row.parse().ok()?, runs))
            })
            .collect::<Option<BTreeMap<i32, Vec<(i32, i32)>>>>()?;
        Some(Coverage { rows })
    }

    fn holds(&self, row: i32, column: i32) -> bool {
        self.rows.get(&row).is_some_and(|runs| {
            let after = runs.partition_point(|(first, _)| *first <= column);
            after > 0 && runs[after - 1].1 >= column
        })
    }
}

/// The cell of coverage a longitude or a latitude falls in.
fn cell_of_coverage(degrees: f64) -> i32 {
    (degrees / COVERAGE_DEGREES).floor() as i32
}

/// Sorted, distinct columns as runs of neighbours, each as its first and last.
fn runs_of(columns: &[i32]) -> Vec<(i32, i32)> {
    let mut runs: Vec<(i32, i32)> = Vec::new();
    for column in columns {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == *column => *last = *column,
            _ => runs.push((*column, *column)),
        }
    }
    runs
}

/// Every cell an edge of `outlines` touches, but for the edges two of them hold alike.
fn crossed_by_outer_borders(outlines: &[&Outline]) -> HashSet<(i32, i32)> {
    let key = |[x1, y1, x2, y2]: [f64; 4]| {
        let (one, other) = ((x1.to_bits(), y1.to_bits()), (x2.to_bits(), y2.to_bits()));
        if one <= other { (one, other) } else { (other, one) }
    };
    let mut holders: HashMap<_, Vec<usize>> = HashMap::new();
    for (at, outline) in outlines.iter().enumerate() {
        for edge in outline.edges() {
            let held = holders.entry(key(*edge)).or_default();
            if !held.contains(&at) {
                held.push(at);
            }
        }
    }
    let mut crossed = HashSet::new();
    for outline in outlines {
        for edge in outline.edges() {
            if holders.get(&key(*edge)).is_some_and(|held| held.len() < 2) {
                mark_cells_touched(*edge, &mut crossed);
            }
        }
    }
    crossed
}

/// Marks every cell the edge touches, its sides and corners included.
fn mark_cells_touched([x1, y1, x2, y2]: [f64; 4], crossed: &mut HashSet<(i32, i32)>) {
    for row in cell_of_coverage(y1.min(y2))..=cell_of_coverage(y1.max(y2)) {
        for column in cell_of_coverage(x1.min(x2))..=cell_of_coverage(x1.max(x2)) {
            let (west, south) = (
                f64::from(column) * COVERAGE_DEGREES,
                f64::from(row) * COVERAGE_DEGREES,
            );
            if segment_touches(
                [x1, y1, x2, y2],
                [west, south, west + COVERAGE_DEGREES, south + COVERAGE_DEGREES],
            ) {
                crossed.insert((row, column));
            }
        }
    }
}

/// Whether a segment meets a closed rectangle, by clipping it to the rectangle as Liang and Barsky
/// do.
fn segment_touches([x1, y1, x2, y2]: [f64; 4], [west, south, east, north]: [f64; 4]) -> bool {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let mut entering = 0.0f64;
    let mut leaving = 1.0f64;
    for (towards, room) in [
        (-dx, x1 - west),
        (dx, east - x1),
        (-dy, y1 - south),
        (dy, north - y1),
    ] {
        if towards == 0.0 {
            if room < 0.0 {
                return false;
            }
        } else {
            let at = room / towards;
            if towards < 0.0 {
                entering = entering.max(at);
            } else {
                leaving = leaving.min(at);
            }
        }
    }
    entering <= leaving
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::Outline;

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

    fn box_(west: f64, south: f64, east: f64, north: f64) -> BoundingBox {
        BoundingBox {
            west,
            south,
            east,
            north,
        }
    }

    /// France and Monaco as two squares side by side, sharing the edge at 7.40 east.
    fn riviera() -> (Outline, Outline) {
        let square = |west: f64, south: f64, east: f64, north: f64| {
            Outline::of(vec![vec![
                [west, south],
                [east, south],
                [east, north],
                [west, north],
                [west, south],
            ]])
        };
        (square(7.00, 43.60, 7.40, 43.80), square(7.40, 43.60, 7.45, 43.80))
    }

    #[test]
    fn given_two_countries_sharing_a_border_when_covered_then_boxes_inside_or_across_the_shared_border_are_covered_and_one_across_an_outer_border_is_not()
     {
        let (france, monaco) = riviera();

        let coverages = Coverage::of_each(&[&france, &monaco]);

        assert!(
            Coverage::cover(&coverages, &box_(7.10, 43.65, 7.20, 43.70)),
            "inside France"
        );
        assert!(
            Coverage::cover(&coverages, &box_(7.35, 43.70, 7.43, 43.75)),
            "across the shared border"
        );
        assert!(
            !Coverage::cover(&coverages, &box_(6.95, 43.65, 7.05, 43.70)),
            "across France's outer border"
        );
        assert!(
            !Coverage::cover(&coverages, &box_(7.42, 43.70, 7.47, 43.75)),
            "across Monaco's outer border"
        );
    }

    /// The 49th parallel, between the United States and Canada, runs along the sides of cells, where
    /// rounding decides which side of it a point on it falls.
    #[test]
    fn given_a_border_along_the_sides_of_cells_when_covered_then_the_cells_along_it_inside_are_left_out() {
        let south_of_the_49th = Outline::of(vec![vec![
            [-1.00, 48.50],
            [1.00, 48.50],
            [1.00, 49.00],
            [-1.00, 49.00],
            [-1.00, 48.50],
        ]]);

        let coverages = Coverage::of_each(&[&south_of_the_49th]);

        assert!(
            Coverage::cover(&coverages, &box_(-0.10, 48.90, 0.10, 48.95)),
            "a cell further south"
        );
        assert!(
            !Coverage::cover(&coverages, &box_(-0.10, 48.991, 0.10, 48.999)),
            "the cells along the border"
        );
    }

    #[test]
    fn given_a_coverage_when_written_as_text_and_read_back_then_it_is_the_same_coverage() {
        let (france, monaco) = riviera();
        let west_of_greenwich = Outline::of(vec![vec![
            [-4.10, -0.20],
            [-3.90, -0.20],
            [-3.90, 0.10],
            [-4.10, 0.10],
            [-4.10, -0.20],
        ]]);
        let coverages = Coverage::of_each(&[&france, &monaco, &west_of_greenwich]);

        let read: Vec<Option<Coverage>> = coverages
            .iter()
            .map(|coverage| Coverage::from_text(&coverage.to_text()))
            .collect();

        assert_eq!(read, coverages.into_iter().map(Some).collect::<Vec<_>>());
        assert_eq!(Coverage::from_text("0.05;1:1..2"), None, "another cell size");
    }
}
