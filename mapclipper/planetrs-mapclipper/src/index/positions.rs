//! Where every node of the planet stands, in a file indexed by node id, so that a way finds its
//! nodes' positions on disk rather than in memory: the planet's eleven billion nodes take about a
//! hundred gigabytes.
//!
//! Each id has eight bytes, its latitude then its longitude in the planet's units, each moved up by
//! 2³¹ so that no position is ever all zeros: the parts of the file never written, as a sparse file
//! leaves them, read as no node at all.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// The positions of a planet's nodes, kept in a file that goes when they do.
pub(super) struct NodePositions {
    file: File,
    path: PathBuf,
}

impl NodePositions {
    /// An empty file at `path` for nodes up to `largest`.
    pub(super) fn create(path: &Path, largest: i64) -> io::Result<Self> {
        let _ = largest;
        Ok(Self {
            file: File::create(path)?,
            path: path.to_path_buf(),
        })
    }

    /// Writes the positions of `nodes`, ids ascending as one block of a sorted planet holds them, as
    /// latitude and longitude in the planet's units.
    pub(super) fn write(&self, nodes: &[(i64, i32, i32)]) -> io::Result<()> {
        let _ = nodes;
        Ok(())
    }

    /// The position of each of `ids`, none for a node the planet does not hold.
    pub(super) fn read(&self, ids: &[i64]) -> io::Result<Vec<Option<(i32, i32)>>> {
        Ok(vec![None; ids.len()])
    }
}

impl Drop for NodePositions {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "planetrs-mapclipper-positions-{name}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn given_two_blocks_of_nodes_written_when_their_positions_are_read_then_each_node_stands_where_it_was_written_and_the_others_are_absent()
    -> io::Result<()> {
        let positions = NodePositions::create(&scratch("blocks"), 2_000)?;
        positions.write(&[(1, 437_347_000, 74_186_000), (3, 437_352_000, 74_214_000)])?;
        positions.write(&[(1_500, -52_690_000, -40_120_000), (2_000, 0, 0)])?;

        let read = positions.read(&[3, 2, 1_500, 2_000, 1, 1_999])?;

        assert_eq!(
            read,
            vec![
                Some((437_352_000, 74_214_000)),
                None,
                Some((-52_690_000, -40_120_000)),
                Some((0, 0)),
                Some((437_347_000, 74_186_000)),
                None,
            ]
        );
        Ok(())
    }
}
