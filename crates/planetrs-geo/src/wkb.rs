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
}
