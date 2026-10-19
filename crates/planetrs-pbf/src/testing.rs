//! Small planet files written in tests, so that whatever reads a planet can be tested on a handful of
//! nodes rather than on ninety gigabytes.
//!
//! A fixture is what a real planet is, only shorter: a header blob declaring `DenseNodes` and
//! `Sort.Type_then_ID`, then zlib-compressed blocks of dense nodes, ways and relations in that order,
//! with granularity 100 and no offset, as planet.openstreetmap.org writes them.

use std::io;
use std::path::Path;

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
        Vec::new()
    }

    /// Writes the file at `path`, replacing whatever was there: fixtures live in temporary places.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_bytes())
    }
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
