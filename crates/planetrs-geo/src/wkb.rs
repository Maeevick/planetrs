//! Geometries as Well-Known Binary, the encoding GeoParquet stores them in.
//!
//! Only what planetrs writes is read back: little-endian, two dimensions, longitude then latitude in
//! degrees on WGS84, which is the axis order of OGC:CRS84 and of every GeoParquet reader.

const LITTLE_ENDIAN: u8 = 1;
const POINT: u32 = 1;
const LINE_STRING: u32 = 2;
/// A byte order, a geometry type and a count.
const LINE_HEADER: usize = 9;

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

/// A line as WKB: a byte order, a geometry type, the count of vertices, then each vertex's longitude
/// and latitude.
pub fn line_string(vertices: &[[f64; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(LINE_HEADER + vertices.len() * 16);
    bytes.push(LITTLE_ENDIAN);
    bytes.extend_from_slice(&LINE_STRING.to_le_bytes());
    bytes.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
    for [longitude, latitude] in vertices {
        bytes.extend_from_slice(&longitude.to_le_bytes());
        bytes.extend_from_slice(&latitude.to_le_bytes());
    }
    bytes
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
    if bytes.len() != 21 || bytes[0] != LITTLE_ENDIAN || bytes[1..5] != POINT.to_le_bytes() {
        return Err(WkbError::NotA { kind: "point" });
    }
    Ok((f64_of(&bytes[5..13]), f64_of(&bytes[13..21])))
}

/// The vertices of a WKB line, each as longitude and latitude.
pub fn read_line_string(bytes: &[u8]) -> Result<Vec<[f64; 2]>, WkbError> {
    let refused = WkbError::NotA { kind: "line string" };
    if bytes.len() < LINE_HEADER || bytes[0] != LITTLE_ENDIAN || bytes[1..5] != LINE_STRING.to_le_bytes() {
        return Err(refused);
    }
    let count = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
    let vertices = &bytes[LINE_HEADER..];
    if vertices.len() != count.saturating_mul(16) {
        return Err(refused);
    }
    let (whole, _) = vertices.as_chunks::<16>();
    Ok(whole
        .iter()
        .map(|vertex| [f64_of(&vertex[..8]), f64_of(&vertex[8..])])
        .collect())
}

fn f64_of(bytes: &[u8]) -> f64 {
    let mut exact = [0u8; 8];
    exact.copy_from_slice(bytes);
    f64::from_le_bytes(exact)
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

    #[test]
    fn given_bytes_that_are_not_a_little_endian_point_when_read_then_they_are_refused() {
        let written = point(3.268_240_2, 48.198_06);
        let big_endian = [[0x00].as_slice(), &written[1..]].concat();
        let line_string = [&written[..1], &2u32.to_le_bytes(), &written[5..]].concat();

        let refused = [&written[..20], &big_endian, &line_string, &[]].map(read_point);

        assert!(
            refused
                .iter()
                .all(|read| *read == Err(WkbError::NotA { kind: "point" }))
        );
    }

    #[test]
    fn given_a_line_when_written_as_wkb_then_it_is_its_vertex_count_then_each_vertex_and_reads_back_to_the_bit()
     {
        let vertices = [[7.418_6, 43.734_7], [7.421_4, 43.735_2], [7.425_9, 43.736_1]];

        let written = line_string(&vertices);

        let expected = [
            [0x01].as_slice(),
            &2u32.to_le_bytes(),
            &3u32.to_le_bytes(),
            &7.418_6f64.to_le_bytes(),
            &43.734_7f64.to_le_bytes(),
            &7.421_4f64.to_le_bytes(),
            &43.735_2f64.to_le_bytes(),
            &7.425_9f64.to_le_bytes(),
            &43.736_1f64.to_le_bytes(),
        ]
        .concat();
        assert_eq!(written, expected);
        assert_eq!(read_line_string(&written), Ok(vertices.to_vec()));
    }
}
