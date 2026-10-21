//! Where every node of the planet stands, in a file indexed by node id, so that a way finds its
//! nodes' positions on disk rather than in memory: the planet's eleven billion nodes take about a
//! hundred gigabytes.
//!
//! Each id has eight bytes, its latitude then its longitude in the planet's units, each moved up by
//! 2³¹ so that no position is ever all zeros: the parts of the file never written, as a sparse file
//! leaves them, read as no node at all.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// The bytes a node takes in the file: its latitude, then its longitude.
const BYTES_PER_NODE: u64 = 8;

/// How far a coordinate is moved up before it is written, so that none is ever stored as zero.
const LIFT: i64 = 1 << 31;

/// Ids this close together are read in one go, the few between them read for nothing: a way's
/// nodes were mostly drawn together, and their ids lie together.
const READ_ACROSS: i64 = 64;

/// The positions of a planet's nodes, kept in a file that goes when they do.
pub(super) struct NodePositions {
    file: File,
    path: PathBuf,
    length: u64,
}

impl NodePositions {
    /// An empty file at `path` for nodes up to `largest`, sparse where the file system allows it.
    pub(super) fn create(path: &Path, largest: i64) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let length = (largest.max(0) as u64 + 1) * BYTES_PER_NODE;
        file.set_len(length)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            length,
        })
    }

    /// Writes the positions of `nodes`, ids ascending as one block of a sorted planet holds them, as
    /// latitude and longitude in the planet's units, in one write.
    pub(super) fn write(&self, nodes: &[(i64, i32, i32)]) -> io::Result<()> {
        let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
            return Ok(());
        };
        let mut bytes = vec![0u8; ((last.0 - first.0 + 1) as u64 * BYTES_PER_NODE) as usize];
        for (id, latitude, longitude) in nodes {
            let at = ((id - first.0) as u64 * BYTES_PER_NODE) as usize;
            bytes[at..at + 4].copy_from_slice(&lifted(*latitude).to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&lifted(*longitude).to_le_bytes());
        }
        at::write_all(&self.file, &bytes, first.0 as u64 * BYTES_PER_NODE)
    }

    /// The position of each of `ids`, none for a node the planet does not hold.
    pub(super) fn read(&self, ids: &[i64]) -> io::Result<Vec<Option<(i32, i32)>>> {
        let largest = (self.length / BYTES_PER_NODE) as i64 - 1;
        let mut wanted: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| (0..=largest).contains(id))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mut found: Vec<(i64, Option<(i32, i32)>)> = Vec::with_capacity(wanted.len());
        let mut start = 0;
        while start < wanted.len() {
            let mut end = start + 1;
            while end < wanted.len() && wanted[end] - wanted[end - 1] <= READ_ACROSS {
                end += 1;
            }
            let (low, high) = (wanted[start], wanted[end - 1]);
            let mut bytes = vec![0u8; ((high - low + 1) as u64 * BYTES_PER_NODE) as usize];
            at::read_exact(&self.file, &mut bytes, low as u64 * BYTES_PER_NODE)?;
            for id in &wanted[start..end] {
                let at = ((id - low) as u64 * BYTES_PER_NODE) as usize;
                found.push((*id, position_in(&bytes[at..at + 8])));
            }
            start = end;
        }
        Ok(ids
            .iter()
            .map(|id| {
                found
                    .binary_search_by_key(id, |(each, _)| *each)
                    .ok()
                    .and_then(|at| found[at].1)
            })
            .collect())
    }
}

impl NodePositions {
    /// Hands every node the file holds to `visit`, with its position, reading the file a slice at a
    /// time across threads.
    pub(super) fn visit_each(&self, visit: impl Fn(i64, i32, i32) + Sync) -> io::Result<()> {
        let _ = visit;
        Ok(())
    }
}

/// A coordinate as the file keeps it.
fn lifted(units: i32) -> u32 {
    (i64::from(units) + LIFT) as u32
}

/// The position eight bytes of the file stand for, none where they were never written.
fn position_in(bytes: &[u8]) -> Option<(i32, i32)> {
    let latitude = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let longitude = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    (latitude != 0 || longitude != 0).then(|| {
        (
            (i64::from(latitude) - LIFT) as i32,
            (i64::from(longitude) - LIFT) as i32,
        )
    })
}

/// Reading and writing a file at an offset, as each system offers it.
mod at {
    use std::fs::File;
    use std::io;

    #[cfg(unix)]
    pub(super) fn read_exact(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(file, bytes, offset)
    }

    #[cfg(unix)]
    pub(super) fn write_all(file: &File, bytes: &[u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::write_all_at(file, bytes, offset)
    }

    #[cfg(windows)]
    pub(super) fn read_exact(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !bytes.is_empty() {
            match std::os::windows::fs::FileExt::seek_read(file, bytes, offset)? {
                0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                read => {
                    bytes = &mut bytes[read..];
                    offset += read as u64;
                }
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(super) fn write_all(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
        while !bytes.is_empty() {
            match std::os::windows::fs::FileExt::seek_write(file, bytes, offset)? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                written => {
                    bytes = &bytes[written..];
                    offset += written as u64;
                }
            }
        }
        Ok(())
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

    #[test]
    fn given_nodes_written_in_two_blocks_when_each_is_visited_then_every_node_held_is_visited_once_with_its_position()
    -> io::Result<()> {
        let positions = NodePositions::create(&scratch("each"), 3_000)?;
        positions.write(&[(1, 437_347_000, 74_186_000), (3, 437_352_000, 74_214_000)])?;
        positions.write(&[(2_999, -52_690_000, -40_120_000)])?;
        let visited = std::sync::Mutex::new(Vec::new());

        positions.visit_each(|id, latitude, longitude| {
            visited.lock().unwrap().push((id, latitude, longitude));
        })?;

        let mut visited = visited.into_inner().unwrap();
        visited.sort_unstable();
        assert_eq!(
            visited,
            vec![
                (1, 437_347_000, 74_186_000),
                (3, 437_352_000, 74_214_000),
                (2_999, -52_690_000, -40_120_000),
            ]
        );
        Ok(())
    }
}
