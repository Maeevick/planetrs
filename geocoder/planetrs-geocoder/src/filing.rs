//! Filing a point under every commune whose outline contains it.
//!
//! OpenStreetMap does not say which commune an address or a road belongs to: a French address point
//! almost never states its town, because the town is the boundary around it. So each point is asked
//! which outlines contain it, and a coarse grid keeps that to the handful of communes whose box
//! overlaps the point's cell rather than all thirty-five thousand.

use std::collections::HashMap;

pub use planetrs_geo::Outline;

/// About five kilometres, roughly a commune across, so a cell holds a handful of them.
const CELL_DEGREES: f64 = 0.05;

pub struct CommuneOutlines {
    outlines: Vec<(i64, Outline)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(longitude: f64, latitude: f64) -> (i32, i32) {
    (
        (longitude / CELL_DEGREES).floor() as i32,
        (latitude / CELL_DEGREES).floor() as i32,
    )
}

impl CommuneOutlines {
    /// Outlines keyed by their commune's relation. Filing order follows the relation, so a point in
    /// two communes lists them the same way every run.
    pub fn of(mut outlines: Vec<(i64, Outline)>) -> Self {
        outlines.sort_by_key(|(relation, _)| *relation);
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (at, (_, outline)) in outlines.iter().enumerate() {
            let (low, high) = outline.bounds();
            let (west, south) = cell_of(low[0], low[1]);
            let (east, north) = cell_of(high[0], high[1]);
            for x in west..=east {
                for y in south..=north {
                    grid.entry((x, y)).or_default().push(at as u32);
                }
            }
        }
        Self { outlines, grid }
    }

    /// Every commune whose outline contains the point, by relation.
    pub fn containing(&self, point: [f64; 2]) -> Vec<i64> {
        self.grid
            .get(&cell_of(point[0], point[1]))
            .map(|candidates| {
                candidates
                    .iter()
                    .map(|at| &self.outlines[*at as usize])
                    .filter(|(_, outline)| outline.contains(point))
                    .map(|(relation, _)| *relation)
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn given_a_point_inside_one_commune_when_filed_then_it_is_filed_under_that_commune() {
        let outlines = CommuneOutlines::of(vec![(1, square(0.0, 0.0, 0.1)), (2, square(0.1, 0.0, 0.1))]);

        assert_eq!(outlines.containing([0.05, 0.05]), vec![1]);
    }

    /// A delegated commune sits inside the commune that absorbed it. An address there belongs to
    /// both, and a search settled on either has to find it.
    #[test]
    fn given_a_point_inside_a_commune_and_the_one_it_was_merged_into_then_it_is_filed_under_both() {
        let outlines = CommuneOutlines::of(vec![(1, square(0.0, 0.0, 1.0)), (9, square(0.2, 0.2, 0.1))]);

        assert_eq!(outlines.containing([0.25, 0.25]), vec![1, 9]);
    }

    #[test]
    fn given_a_point_outside_every_commune_when_filed_then_it_is_filed_nowhere() {
        let outlines = CommuneOutlines::of(vec![(1, square(0.0, 0.0, 0.1))]);

        assert!(outlines.containing([5.0, 5.0]).is_empty());
    }

    #[test]
    fn given_a_commune_spanning_many_grid_cells_when_filed_then_a_point_in_its_far_corner_is_found() {
        let outlines = CommuneOutlines::of(vec![(1, square(0.0, 0.0, 0.6))]);

        assert_eq!(outlines.containing([0.59, 0.59]), vec![1]);
    }
}
