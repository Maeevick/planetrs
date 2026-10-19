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
    /// The dataset was built from another planet, and a dataset answers for one.
    OtherPlanet {
        /// The planet the dataset was built from.
        recorded: Box<Provenance>,
        /// The planet the part was built from.
        offered: Box<Provenance>,
    },
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, .. } => write!(formatter, "{} could not be written or read", path.display()),
            Self::Manifest(_) => write!(formatter, "the dataset's manifest could not be read"),
            Self::OtherPlanet { recorded, offered } => write!(
                formatter,
                "the dataset was built from {}, and this part from {}",
                recorded.source_name, offered.source_name
            ),
        }
    }
}

impl std::error::Error for RecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Manifest(source) => Some(source),
            Self::OtherPlanet { .. } => None,
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
    let recorded = recorded_in(directory)?.unwrap_or_else(|| Manifest {
        planet: planet.clone(),
        files: Vec::new(),
    });
    if recorded.planet != *planet {
        return Err(RecordError::OtherPlanet {
            recorded: Box::new(recorded.planet),
            offered: Box::new(planet.clone()),
        });
    }
    write_whole(&directory.join(name), bytes)?;
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

/// Where a part is written beside the dataset in `directory` before [`record_written`] gives it its
/// name: a part too large to hold in memory is written there as it is encoded.
pub fn written_beside(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}.partial"))
}

/// Records the file already written at [`written_beside`] as the file `name` of the dataset in
/// `directory`, as the given part built from `planet`, and returns the manifest that now lists it.
pub fn record_written(
    directory: &Path,
    planet: &Provenance,
    part: Part,
    name: &str,
    rows: u64,
) -> Result<Manifest, RecordError> {
    let _ = (directory, planet, part, name, rows);
    Err(RecordError::Io {
        path: directory.to_path_buf(),
        source: std::io::Error::other("not yet"),
    })
}

/// What carrying a dataset's parts into a rebuild brought in and left behind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CarriedOver {
    /// The parts brought into the rebuild, as the previous dataset held them.
    pub kept: Vec<Part>,
    /// The parts left behind, because the previous dataset answers for another planet.
    pub left: Vec<Part>,
}

/// Brings into the dataset being built in `building` every part of the dataset in `previous` that
/// `others` says another tool builds, so that rebuilding one tool's parts never loses the other's.
///
/// A file is hard-linked into place, or copied where a link cannot be made, and listed in the
/// rebuild's manifest as the previous one listed it. A dataset answers for one planet, so when the
/// previous one was built from another, its parts are left behind and named instead.
pub fn carry_over(
    previous: &Path,
    building: &Path,
    others: impl Fn(&Part) -> bool,
) -> Result<CarriedOver, RecordError> {
    let (Some(before), Some(rebuilt)) = (recorded_in(previous)?, recorded_in(building)?) else {
        return Ok(CarriedOver::default());
    };
    let carried: Vec<Entry> = before
        .files
        .into_iter()
        .filter(|entry| others(&entry.part))
        .collect();
    if before.planet != rebuilt.planet {
        return Ok(CarriedOver {
            kept: Vec::new(),
            left: carried.into_iter().map(|entry| entry.part).collect(),
        });
    }
    for entry in &carried {
        link_or_copy(&previous.join(&entry.file), &building.join(&entry.file))?;
    }
    let kept = carried.iter().map(|entry| entry.part.clone()).collect();
    let mut files: Vec<Entry> = rebuilt.files.into_iter().chain(carried).collect();
    files.sort_by(|left, right| left.part.cmp(&right.part));
    let manifest = Manifest {
        planet: rebuilt.planet,
        files,
    };
    write_whole(&building.join(MANIFEST), manifest.to_json().as_bytes())?;
    Ok(CarriedOver {
        kept,
        left: Vec::new(),
    })
}

/// Puts the file at `from` at `to` too, as a second name for the same bytes where the file system
/// allows it, else as a copy.
fn link_or_copy(from: &Path, to: &Path) -> Result<(), RecordError> {
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ()).map_err(io_at(to))
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
