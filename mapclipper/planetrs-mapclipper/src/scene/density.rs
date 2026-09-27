//! How built-up the ground is, measured rather than tagged.
//!
//! `landuse=residential` says people live there. It says nothing about how densely, and over a city
//! it covers nearly everything, which is why colouring it wholesale paints the whole metro area one
//! shade and tells the reader nothing. Heavy density is a different question, and a harvest already
//! holds what answers it: a hundred and fifty thousand building footprints.
//!
//! Buildings are summed into a coarse grid and the built-up share of each cell is what a land use
//! polygon is then judged on, so the shapes stay the natural ones from the data and only their
//! colour depends on the measurement.

use planetrs_geo::{BoundingBox, metres_per_degree};

use super::plane::{Point, signed_area};

/// Ring area in square metres, with degrees scaled at this latitude.
pub fn area_sq_m(points: &[Point], latitude: f64) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let (per_latitude, per_longitude) = metres_per_degree(latitude);

    signed_area(points).abs() * per_latitude * per_longitude
}

pub struct Density {
    frame: BoundingBox,
    columns: usize,
    rows: usize,
    /// Built-up share per cell, 0 to 1.
    share: Vec<f64>,
}

impl Density {
    /// Measured over every building the harvest holds, including the ones too small to draw.
    ///
    /// A dense block is thousands of two-metre footprints, and filtering those out before measuring
    /// would report the densest ground in the city as empty.
    pub fn of<'a>(
        buildings: impl Iterator<Item = &'a [Point]>,
        frame: &BoundingBox,
        cell_metres: f64,
    ) -> Self {
        let middle = frame.middle_latitude();
        let (per_latitude, per_longitude) = metres_per_degree(middle);
        let columns = (((frame.east - frame.west) * per_longitude / cell_metres).ceil() as usize).max(1);
        let rows = (((frame.north - frame.south) * per_latitude / cell_metres).ceil() as usize).max(1);

        let mut built = vec![0.0; columns * rows];
        for ring in buildings {
            let Some(centre) = centroid(ring) else { continue };
            let Some(cell) = cell_of(frame, columns, rows, centre) else {
                continue;
            };
            built[cell] += area_sq_m(ring, middle);
        }

        let cell_area = ((frame.east - frame.west) * per_longitude / columns as f64)
            * ((frame.north - frame.south) * per_latitude / rows as f64);

        Self {
            frame: *frame,
            columns,
            rows,
            share: built
                .into_iter()
                .map(|area| (area / cell_area).min(1.0))
                .collect(),
        }
    }

    /// The mean built-up share over the cells a shape's bounding box touches.
    pub fn over(&self, low: Point, high: Point) -> f64 {
        let (Some(first), Some(last)) = (
            cell_coords(&self.frame, self.columns, self.rows, low),
            cell_coords(&self.frame, self.columns, self.rows, high),
        ) else {
            return 0.0;
        };

        let (left, right) = (first.0.min(last.0), first.0.max(last.0));
        let (top, bottom) = (first.1.min(last.1), first.1.max(last.1));

        let mut sum = 0.0;
        let mut seen = 0usize;
        for row in top..=bottom {
            for column in left..=right {
                sum += self.share[row * self.columns + column];
                seen += 1;
            }
        }

        if seen == 0 { 0.0 } else { sum / seen as f64 }
    }

    /// The shares of every cell holding anything, sorted, so a threshold can be read off the data
    /// rather than guessed at.
    pub fn distribution(&self) -> Vec<f64> {
        let mut built: Vec<f64> = self.share.iter().copied().filter(|share| *share > 0.0).collect();
        built.sort_by(f64::total_cmp);
        built
    }

    /// The share at a percentile, for reporting what the ground actually looks like.
    pub fn percentile(&self, share: f64) -> Option<f64> {
        let sorted = self.distribution();
        let last = sorted.len().checked_sub(1)?;
        sorted.get((last as f64 * share) as usize).copied()
    }
}

fn centroid(ring: &[Point]) -> Option<Point> {
    if ring.is_empty() {
        return None;
    }
    let (mut x, mut y) = (0.0, 0.0);
    for [px, py] in ring {
        x += px;
        y += py;
    }
    Some([x / ring.len() as f64, y / ring.len() as f64])
}

fn cell_coords(frame: &BoundingBox, columns: usize, rows: usize, [x, y]: Point) -> Option<(usize, usize)> {
    if !frame.holds(x, y) {
        return None;
    }
    let column = (((x - frame.west) / (frame.east - frame.west)) * columns as f64) as usize;
    let row = (((frame.north - y) / (frame.north - frame.south)) * rows as f64) as usize;
    Some((column.min(columns - 1), row.min(rows - 1)))
}

fn cell_of(frame: &BoundingBox, columns: usize, rows: usize, point: Point) -> Option<usize> {
    cell_coords(frame, columns, rows, point).map(|(column, row)| row * columns + column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use planetrs_geo::Area;

    fn frame() -> BoundingBox {
        Area::around(5.27, -4.01, 10.0, 10.0, 0.0).unwrap().frame()
    }

    /// A square of a thousandth of a degree at Abidjan is about 111 m across.
    #[test]
    fn when_a_ring_is_measured_then_its_area_is_in_square_metres() {
        let square = [[-4.01, 5.27], [-4.009, 5.27], [-4.009, 5.271], [-4.01, 5.271]];

        let area = area_sq_m(&square, 5.27);

        assert!((area - 12_300.0).abs() < 200.0, "about 111 m square, got {area}");
    }

    #[test]
    fn when_a_ring_is_wound_the_other_way_then_its_area_is_still_positive() {
        let clockwise = [[-4.01, 5.27], [-4.01, 5.271], [-4.009, 5.271], [-4.009, 5.27]];
        let widdershins = [[-4.01, 5.27], [-4.009, 5.27], [-4.009, 5.271], [-4.01, 5.271]];

        assert!((area_sq_m(&clockwise, 5.27) - area_sq_m(&widdershins, 5.27)).abs() < 1.0);
    }

    #[test]
    fn when_nothing_is_built_then_every_cell_is_empty() {
        let density = Density::of(std::iter::empty(), &frame(), 250.0);

        assert_eq!(density.over([-4.02, 5.26], [-4.0, 5.28]), 0.0);
        assert_eq!(density.percentile(0.5), None);
    }

    /// The point of the measurement: the same land use tag over two places, one dense and one not.
    #[test]
    fn when_one_area_is_built_up_and_another_is_not_then_their_shares_differ() {
        let dense: Vec<Vec<Point>> = (0..40)
            .map(|index| {
                let offset = index as f64 * 0.0001;
                vec![
                    [-4.015 + offset, 5.272],
                    [-4.0145 + offset, 5.272],
                    [-4.0145 + offset, 5.2725],
                    [-4.015 + offset, 5.2725],
                ]
            })
            .collect();

        let density = Density::of(dense.iter().map(|ring| ring.as_slice()), &frame(), 250.0);

        let built = density.over([-4.015, 5.272], [-4.011, 5.2725]);
        let empty = density.over([-4.005, 5.262], [-4.004, 5.263]);

        assert!(built > empty, "built {built} should exceed empty {empty}");
        assert_eq!(empty, 0.0);
    }

    #[test]
    fn when_a_shape_sits_outside_the_frame_then_it_has_no_measurement() {
        let density = Density::of(std::iter::empty(), &frame(), 250.0);

        assert_eq!(density.over([10.0, 10.0], [11.0, 11.0]), 0.0);
    }
}
