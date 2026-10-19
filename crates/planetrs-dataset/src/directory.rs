//! A dataset as a directory on this machine, where a build records each part it writes.
//!
//! Every file lands whole or not at all, and the manifest is rewritten after each one, so that a
//! dataset interrupted mid-build lists only files that are complete. A dataset answers for one
//! planet: a part built from another is refused rather than mixed in.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::{Entry, Manifest, ManifestError, Part, Provenance};

/// The manifest's file name, the one a dataset is opened by.
pub const MANIFEST: &str = "manifest.json";

/// Why a part could not be recorded.
#[derive(Debug)]
#[non_exhaustive]
pub enum RecordError {
    /// A file of the dataset could not be written or read.
    Io {
        /// The file.
        path: PathBuf,
        /// What failed.
        source: std::io::Error,
    },
    /// The dataset's manifest could not be read.
    Manifest(ManifestError),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, .. } => write!(formatter, "{} could not be written or read", path.display()),
            Self::Manifest(_) => write!(formatter, "the dataset's manifest could not be read"),
        }
    }
}

impl std::error::Error for RecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Manifest(source) => Some(source),
        }
    }
}

/// Records `bytes` as the file `name` of the dataset in `directory`, as the given part built from
/// `planet`, and returns the manifest that now lists it.
pub fn record(
    directory: &Path,
    planet: &Provenance,
    part: Part,
    name: &str,
    bytes: &[u8],
    rows: u64,
) -> Result<Manifest, RecordError> {
    std::fs::create_dir_all(directory).map_err(io_at(directory))?;
    write_whole(&directory.join(name), bytes)?;
    let recorded = recorded_in(directory)?.unwrap_or_else(|| Manifest {
        planet: planet.clone(),
        files: Vec::new(),
    });
    let mut files: Vec<Entry> = recorded
        .files
        .into_iter()
        .filter(|entry| entry.part != part)
        .collect();
    files.push(Entry {
        part,
        file: name.to_string(),
        url: None,
        bytes: bytes.len() as u64,
        sha256: sha256_of(bytes),
        rows,
    });
    files.sort_by(|left, right| left.part.cmp(&right.part));
    let manifest = Manifest {
        planet: recorded.planet,
        files,
    };
    write_whole(&directory.join(MANIFEST), manifest.to_json().as_bytes())?;
    Ok(manifest)
}

/// The manifest already in `directory`, if there is one.
fn recorded_in(directory: &Path) -> Result<Option<Manifest>, RecordError> {
    let path = directory.join(MANIFEST);
    match std::fs::read_to_string(&path) {
        Ok(text) => Manifest::from_json(&text)
            .map(Some)
            .map_err(RecordError::Manifest),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(RecordError::Io { path, source }),
    }
}

/// Writes a file under a temporary name first, so that it appears whole or not at all.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), RecordError> {
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).map_err(io_at(&partial))?;
    std::fs::rename(&partial, path).map_err(io_at(path))
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> RecordError + '_ {
    move |source| RecordError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The SHA-256 of the bytes, in lower-case hexadecimal as `sha256sum` prints it.
fn sha256_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
