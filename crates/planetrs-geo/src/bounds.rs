//! A box in degrees on WGS84, and what a degree is worth in metres where it sits.

/// Metres per degree of latitude and of longitude at a given latitude, on WGS84.
///
/// A degree of longitude shrinks towards the poles and a degree of latitude grows slightly, so a
/// box asked for in kilometres is not square in degrees. Using 111.32 km for both would make a
/// twelve kilometre frame at Abidjan about 40 m wrong east to west, and would stretch the aspect
/// ratio the illustration is drawn at.
pub fn metres_per_degree(latitude: f64) -> (f64, f64) {
    let phi = latitude.to_radians();
    let north =
        111_132.92 - 559.82 * (2.0 * phi).cos() + 1.175 * (4.0 * phi).cos() - 0.0023 * (6.0 * phi).cos();
    let east = 111_412.84 * phi.cos() - 93.5 * (3.0 * phi).cos() + 0.118 * (5.0 * phi).cos();
    (north, east.max(0.0))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl BoundingBox {
    /// The same box grown by a share of its own size on every side.
    ///
    /// Ways are kept when a node of theirs lands in the frame, and drawing one needs the vertices
    /// just outside it too, or a quay running off the edge stops dead at the boundary instead of
    /// leaving the picture.
    pub fn grown(&self, share: f64) -> Self {
        let horizontal = (self.east - self.west) * share;
        let vertical = (self.north - self.south) * share;

        Self {
            west: self.west - horizontal,
            south: self.south - vertical,
            east: self.east + horizontal,
            north: self.north + vertical,
        }
    }

    pub fn width_km(&self) -> f64 {
        let (_, per_longitude) = metres_per_degree(self.middle_latitude());
        (self.east - self.west) * per_longitude / 1000.0
    }

    pub fn height_km(&self) -> f64 {
        let (per_latitude, _) = metres_per_degree(self.middle_latitude());
        (self.north - self.south) * per_latitude / 1000.0
    }

    pub fn middle_latitude(&self) -> f64 {
        (self.north + self.south) / 2.0
    }

    pub fn middle_longitude(&self) -> f64 {
        (self.east + self.west) / 2.0
    }

    pub fn holds(&self, longitude: f64, latitude: f64) -> bool {
        longitude >= self.west && longitude <= self.east && latitude >= self.south && latitude <= self.north
    }

    /// Whether this box entirely contains another, which is what a cached harvest is judged on.
    pub fn covers(&self, other: &Self) -> bool {
        self.west <= other.west
            && self.south <= other.south
            && self.east >= other.east
            && self.north >= other.north
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.east >= other.west
            && self.west <= other.east
            && self.north >= other.south
            && self.south <= other.north
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abidjan() -> BoundingBox {
        BoundingBox {
            west: -4.07,
            south: 5.258,
            east: -3.95,
            north: 5.282,
        }
    }

    #[test]
    fn when_a_box_is_grown_then_it_keeps_its_centre() {
        let grown = abidjan().grown(0.25);

        assert!((grown.middle_longitude() - abidjan().middle_longitude()).abs() < 1e-12);
        assert!((grown.middle_latitude() - abidjan().middle_latitude()).abs() < 1e-12);
    }

    #[test]
    fn when_a_box_is_grown_by_a_quarter_then_it_measures_half_again_across() {
        let grown = abidjan().grown(0.25);

        assert!((grown.width_km() / abidjan().width_km() - 1.5).abs() < 1e-9);
    }

    /// A degree of longitude is 110.85 km at Abidjan and 71.70 km at Paris, so the same box in
    /// degrees covers far less ground up there.
    #[test]
    fn when_the_latitude_rises_then_a_degree_of_longitude_is_worth_fewer_metres() {
        let (_, at_abidjan) = metres_per_degree(5.27);
        let (_, at_paris) = metres_per_degree(48.85);

        assert!(at_abidjan > at_paris * 1.5);
    }

    #[test]
    fn when_a_cache_reaches_past_what_is_wanted_then_it_can_answer_for_it() {
        let cached = BoundingBox {
            west: -4.1,
            south: 5.2,
            east: -3.9,
            north: 5.3,
        };

        assert!(cached.covers(&BoundingBox {
            west: -4.0,
            south: 5.25,
            east: -3.95,
            north: 5.28
        }));
    }

    #[test]
    fn when_a_cache_stops_short_of_what_is_wanted_then_it_cannot_answer_for_it() {
        let cached = BoundingBox {
            west: -4.1,
            south: 5.2,
            east: -3.9,
            north: 5.3,
        };

        assert!(!cached.covers(&BoundingBox {
            west: -4.2,
            south: 5.25,
            east: -3.95,
            north: 5.28
        }));
    }

    #[test]
    fn when_two_boxes_share_no_ground_then_they_do_not_overlap() {
        let here = BoundingBox {
            west: 0.0,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        };

        assert!(here.overlaps(&BoundingBox {
            west: 0.5,
            south: 0.5,
            east: 2.0,
            north: 2.0
        }));
        assert!(!here.overlaps(&BoundingBox {
            west: 2.0,
            south: 2.0,
            east: 3.0,
            north: 3.0
        }));
    }
}
