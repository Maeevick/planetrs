//! Small planet files written in tests, so that whatever reads a planet can be tested on a handful of
//! nodes rather than on ninety gigabytes.
//!
//! A fixture is what a real planet is, only shorter: a header blob declaring `DenseNodes` and
//! `Sort.Type_then_ID`, then zlib-compressed blocks of dense nodes, ways and relations in that order,
//! with granularity 100 and no offset, as planet.openstreetmap.org writes them.

use std::io::{self, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::header::{DENSE_NODES, SORT_TYPE_THEN_ID};

/// The program a fixture names in its header, so a harvest or an index built from one says so.
pub const FIXTURE_PROGRAM: &str = "planetrs fixture";

/// A planet file to be written: its header and, later, its entities.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanetFile {
    writing_program: String,
}

impl PlanetFile {
    /// A planet declaring `Sort.Type_then_ID`, as every planet file does.
    pub fn sorted() -> Self {
        Self {
            writing_program: FIXTURE_PROGRAM.to_string(),
        }
    }

    /// The file's bytes, framing included.
    pub fn to_bytes(&self) -> Vec<u8> {
        let header = Message::new()
            .text(4, "OsmSchema-V0.6")
            .text(4, DENSE_NODES)
            .text(5, SORT_TYPE_THEN_ID)
            .text(16, &self.writing_program)
            .into_bytes();
        framed("OSMHeader", &header)
    }

    /// Writes the file at `path`, replacing whatever was there: fixtures live in temporary places.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_bytes())
    }
}

/// One blob as the framing lays it out: the header's length, the header, then the zlib payload.
fn framed(kind: &str, block: &[u8]) -> Vec<u8> {
    let blob = Message::new()
        .number(2, block.len() as u64)
        .bytes(3, &zlib(block))
        .into_bytes();
    let header = Message::new()
        .text(1, kind)
        .number(3, blob.len() as u64)
        .into_bytes();
    [&(header.len() as u32).to_be_bytes()[..], &header, &blob].concat()
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .and_then(|()| encoder.finish())
        .unwrap_or_default()
}

/// A protobuf message written field by field, in the order the fields are given.
struct Message(Vec<u8>);

impl Message {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn number(mut self, field: u32, value: u64) -> Self {
        put_varint(&mut self.0, u64::from(field) << 3);
        put_varint(&mut self.0, value);
        self
    }

    fn bytes(mut self, field: u32, payload: &[u8]) -> Self {
        put_varint(&mut self.0, u64::from(field) << 3 | 2);
        put_varint(&mut self.0, payload.len() as u64);
        self.0.extend_from_slice(payload);
        self
    }

    fn text(self, field: u32, text: &str) -> Self {
        self.bytes(field, text.as_bytes())
    }

    fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::planet::Planet;

    fn written(planet: &PlanetFile, name: &str) -> io::Result<PathBuf> {
        let path = std::env::temp_dir().join(format!("planetrs-pbf-{name}-{}.osm.pbf", std::process::id()));
        planet.write(&path)?;
        Ok(path)
    }

    #[test]
    fn given_a_fixture_planet_when_opened_then_its_header_names_the_fixture_and_declares_the_sort_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = written(&PlanetFile::sorted(), "header")?;

        let opened = Planet::open(&path)?;

        assert_eq!(opened.header.writing_program.as_deref(), Some(FIXTURE_PROGRAM));
        assert!(opened.header.is_sorted_by_type_then_id());
        assert!(opened.header.requires_dense_nodes());
        Ok(())
    }
}
