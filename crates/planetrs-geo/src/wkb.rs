//! Geometries as Well-Known Binary, the encoding GeoParquet stores them in.
//!
//! Only what planetrs writes is read back: little-endian, two dimensions, longitude then latitude in
//! degrees on WGS84, which is the axis order of OGC:CRS84 and of every GeoParquet reader.

const LITTLE_ENDIAN: u8 = 1;
const POINT: u32 = 1;

/// A point as WKB: a byte order, a geometry type, then longitude and latitude.
pub fn point(longitude: f64, latitude: f64) -> Vec<u8> {
    [
        [LITTLE_ENDIAN].as_slice(),
        &POINT.to_le_bytes(),
        &longitude.to_le_bytes(),
        &latitude.to_le_bytes(),
    ]
    .concat()
}

/// Why bytes could not be read as the geometry asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WkbError {
    /// The bytes are not little-endian two-dimensional WKB of the kind asked for.
    NotA {
        /// The kind asked for.
        kind: &'static str,
    },
}

impl std::fmt::Display for WkbError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotA { kind } => write!(formatter, "the bytes are not a little-endian WKB {kind}"),
        }
    }
}

impl std::error::Error for WkbError {}

/// The longitude and latitude of a WKB point.
pub fn read_point(bytes: &[u8]) -> Result<(f64, f64), WkbError> {
    let _ = bytes;
    Err(WkbError::NotA { kind: "point" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_point_when_written_as_wkb_then_it_is_the_twenty_one_bytes_the_standard_lays_out() {
        let written = point(7.424_6, 43.738_4);

        let expected = [
            [0x01].as_slice(),
            &1u32.to_le_bytes(),
            &7.424_6f64.to_le_bytes(),
            &43.738_4f64.to_le_bytes(),
        ]
        .concat();
        assert_eq!(written, expected);
    }

    #[test]
    fn given_a_point_written_as_wkb_when_read_back_then_the_same_longitude_and_latitude_come_back_to_the_bit()
    {
        let written = point(3.268_240_2, 48.198_06);

        let read = read_point(&written);

        assert_eq!(read, Ok((3.268_240_2, 48.198_06)));
    }
}
