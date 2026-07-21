//! What ground is drawn, and what ground is collected to draw it.
//!
//! These are two boxes and they were one type for most of the spike, which cost seven full rescans
//! when a rounded marker coordinate moved the frame and the wrong one was compared against the
//! cache. They are one type again here, but the collected box is *derived* and never stored, so
//! there is no second value that can drift from the first.

use crate::bounds::{BoundingBox, metres_per_degree};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoError {
    LatitudeOutOfRange,
    LongitudeOutOfRange,
    SizeNotPositive,
    MarginNegative,
    CrossesTheAntimeridian,
    ReachesAPole,
}

impl fmt::Display for GeoError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LatitudeOutOfRange => write!(out, "latitude must be between -90 and 90"),
            Self::LongitudeOutOfRange => write!(out, "longitude must be between -180 and 180"),
            Self::SizeNotPositive => write!(out, "width and height must be greater than zero"),
            Self::MarginNegative => write!(out, "margin cannot be negative"),
            Self::CrossesTheAntimeridian => {
                write!(
                    out,
                    "the area crosses 180 degrees, which this tool does not yet cut across"
                )
            }
            Self::ReachesAPole => {
                write!(out, "the area reaches a pole, where Web Mercator has no bottom")
            }
        }
    }
}

impl std::error::Error for GeoError {}

/// A frame to draw, and the margin of extra ground collected around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    frame: BoundingBox,
    margin: f64,
}

impl Area {
    pub fn new(frame: BoundingBox, margin: f64) -> Result<Self, GeoError> {
        if margin < 0.0 || !margin.is_finite() {
            return Err(GeoError::MarginNegative);
        }
        if frame.east <= frame.west || frame.north <= frame.south {
            return Err(GeoError::SizeNotPositive);
        }
        Self { frame, margin }.validated()
    }

    /// A frame of a given size on the ground, centred on a point.
    pub fn around(
        latitude: f64,
        longitude: f64,
        width_km: f64,
        height_km: f64,
        margin: f64,
    ) -> Result<Self, GeoError> {
        if !(-90.0..=90.0).contains(&latitude) || !latitude.is_finite() {
            return Err(GeoError::LatitudeOutOfRange);
        }
        if !(-180.0..=180.0).contains(&longitude) || !longitude.is_finite() {
            return Err(GeoError::LongitudeOutOfRange);
        }
        if !width_km.is_finite() || !height_km.is_finite() || width_km <= 0.0 || height_km <= 0.0 {
            return Err(GeoError::SizeNotPositive);
        }
        if margin < 0.0 || !margin.is_finite() {
            return Err(GeoError::MarginNegative);
        }

        let (per_latitude, per_longitude) = metres_per_degree(latitude);
        if per_longitude <= 0.0 {
            return Err(GeoError::ReachesAPole);
        }
        let half_height = height_km * 500.0 / per_latitude;
        let half_width = width_km * 500.0 / per_longitude;

        Self {
            frame: BoundingBox {
                west: longitude - half_width,
                south: latitude - half_height,
                east: longitude + half_width,
                north: latitude + half_height,
            },
            margin,
        }
        .validated()
    }

    fn validated(self) -> Result<Self, GeoError> {
        let collected = self.collected();
        if collected.west < -180.0 || collected.east > 180.0 {
            return Err(GeoError::CrossesTheAntimeridian);
        }
        if collected.south < -85.0 || collected.north > 85.0 {
            return Err(GeoError::ReachesAPole);
        }
        Ok(self)
    }

    /// The ground that gets drawn.
    pub fn frame(&self) -> BoundingBox {
        self.frame
    }

    /// The ground that gets read, which is the frame plus its margin on every side.
    pub fn collected(&self) -> BoundingBox {
        self.frame.grown(self.margin)
    }

    pub fn margin(&self) -> f64 {
        self.margin
    }

    /// The same frame with a different margin, for asking whether a wider harvest is needed.
    pub fn with_margin(&self, margin: f64) -> Result<Self, GeoError> {
        Self::new(self.frame, margin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abidjan(margin: f64) -> Area {
        Area::around(5.269, -4.012, 29.0, 16.53, margin).unwrap()
    }

    #[test]
    fn when_an_area_is_asked_for_in_kilometres_then_its_frame_measures_that_across() {
        let frame = abidjan(0.1).frame();

        assert!((frame.width_km() - 29.0).abs() < 0.01);
        assert!((frame.height_km() - 16.53).abs() < 0.01);
    }

    #[test]
    fn when_an_area_has_a_margin_then_the_collected_ground_is_larger_than_the_frame() {
        let area = abidjan(0.25);

        assert!((area.collected().width_km() / area.frame().width_km() - 1.5).abs() < 1e-9);
    }

    /// The two boxes cannot drift apart, because only one of them is stored.
    #[test]
    fn when_an_area_has_no_margin_then_the_collected_ground_is_the_frame_itself() {
        let area = abidjan(0.0);

        assert_eq!(area.collected(), area.frame());
    }

    #[test]
    fn when_a_frame_has_no_width_then_the_area_is_refused() {
        assert_eq!(
            Area::around(5.27, -4.01, 0.0, 10.0, 0.1),
            Err(GeoError::SizeNotPositive)
        );
    }

    #[test]
    fn when_a_margin_is_negative_then_the_area_is_refused() {
        assert_eq!(
            Area::around(5.27, -4.01, 10.0, 10.0, -0.1),
            Err(GeoError::MarginNegative)
        );
    }

    #[test]
    fn when_a_latitude_is_not_on_the_globe_then_the_area_is_refused() {
        assert_eq!(
            Area::around(91.0, -4.01, 10.0, 10.0, 0.1),
            Err(GeoError::LatitudeOutOfRange)
        );
    }

    #[test]
    fn when_a_longitude_is_not_on_the_globe_then_the_area_is_refused() {
        assert_eq!(
            Area::around(5.27, 181.0, 10.0, 10.0, 0.1),
            Err(GeoError::LongitudeOutOfRange)
        );
    }

    /// A box that wraps past 180 comes back with west greater than east, which every test
    /// downstream reads as an empty box and every renderer draws as a blank picture.
    #[test]
    fn when_an_area_would_wrap_past_the_antimeridian_then_it_is_refused_rather_than_inverted() {
        assert_eq!(
            Area::around(-16.92, 179.9, 60.0, 60.0, 0.1),
            Err(GeoError::CrossesTheAntimeridian)
        );
    }

    /// Web Mercator sends 90 degrees to infinity, so a frame reaching a pole has no height to
    /// divide by and every projected y comes back as a NaN.
    #[test]
    fn when_an_area_reaches_a_pole_then_it_is_refused_rather_than_projected_to_infinity() {
        assert_eq!(
            Area::around(84.9, 0.0, 100.0, 100.0, 0.25),
            Err(GeoError::ReachesAPole)
        );
    }

    #[test]
    fn when_a_margin_is_widened_then_the_frame_is_unchanged() {
        let wider = abidjan(0.1).with_margin(0.4).unwrap();

        assert_eq!(wider.frame(), abidjan(0.1).frame());
        assert!(wider.collected().width_km() > abidjan(0.1).collected().width_km());
    }
}
