//! What each command does, given a parsed command line.

pub mod download;
pub mod geocode;
pub mod index;
pub mod status;
pub mod trace;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result};
use planetrs_dataset::DirectoryFiles;
use planetrs_geocoder::{Geocoder, OpenError};

use crate::ATTRIBUTION;
use crate::cli::{Cli, Command, Global};
use crate::data_dir::DataDir;

/// A command asked for in a way it cannot be done, which exits 2 like a parsing error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(pub String);

/// What every command knows before it starts: where things are, and how much to say.
pub struct Context {
    /// Where planets and the index are kept.
    pub data: DataDir,
    /// The index `--index` names, when it does.
    pub index: Option<PathBuf>,
    /// Whether to say anything on stderr beyond errors and the attribution.
    pub quiet: bool,
}

impl Context {
    /// Reads the shared settings, the environment standing behind `--data-dir`.
    pub fn of(global: &Global) -> Result<Self> {
        let data = DataDir::resolve(global.data_dir.as_deref(), |name| std::env::var_os(name))?;
        let index = match global.index.as_deref() {
            Some(remote) if remote.starts_with("http://") || remote.starts_with("https://") => {
                return Err(UsageError(format!(
                    "remote indexes are not supported yet: {remote}\n  \
                     → build one locally: planetrs-geocoder download, then planetrs-geocoder index"
                ))
                .into());
            }
            Some(local) => Some(PathBuf::from(local)),
            None => None,
        };
        Ok(Self {
            data,
            index,
            quiet: global.quiet,
        })
    }

    /// The index to read or build: `--index` when given, the data directory's otherwise.
    pub fn index_directory(&self) -> PathBuf {
        self.index.clone().unwrap_or_else(|| self.data.index())
    }

    /// Opens the index, saying how long it took, or says how to get one.
    pub fn open_geocoder(&self) -> Result<Geocoder<DirectoryFiles>> {
        let directory = self.index_directory();
        let started = Instant::now();
        let geocoder = Geocoder::open(&directory).map_err(|error| match error {
            OpenError::NoIndex { directory } => anyhow::anyhow!(
                "no index in {}\n  \
                 → build one:        planetrs-geocoder download, then planetrs-geocoder index\n  \
                 → from a planet:    planetrs-geocoder index --planet PATH\n  \
                 → or read another:  --index DIR",
                directory.display()
            ),
            other => {
                let failure = anyhow::Error::new(other);
                anyhow::anyhow!("{failure:#}\n  → rebuild it: planetrs-geocoder index")
            }
        })?;
        self.say(&format!(
            "loaded the index {} in {:.1}s",
            directory.display(),
            started.elapsed().as_secs_f64()
        ));
        Ok(geocoder)
    }

    /// A line on stderr, unless the run is quiet.
    pub fn say(&self, line: &str) {
        if !self.quiet {
            eprintln!("{line}");
        }
    }

    /// The OpenStreetMap attribution, which every run that reads its data ends with, quiet or not.
    pub fn attribute(&self) {
        eprintln!("{ATTRIBUTION}");
    }
}

/// The bytes of a site file, or of stdin for `-`.
pub fn read_input(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .read_to_end(&mut bytes)
            .context("reading the sites from stdin")?;
    } else {
        bytes = std::fs::read(path).with_context(|| format!("reading the sites from {}", path.display()))?;
    }
    Ok(bytes)
}

/// Runs the command the command line asks for.
pub fn run(cli: Cli) -> Result<()> {
    if let Some(command) = &cli.command
        && (cli.site.is_some() || cli.file.is_some() || cli.output.is_some())
    {
        return Err(UsageError(format!(
            "{} takes no site, -f or -o before it; give those after it, or drop the command to geocode",
            command.name()
        ))
        .into());
    }
    let context = Context::of(&cli.global)?;
    match cli.command {
        Some(Command::Download(arguments)) => download::run(&context, arguments),
        Some(Command::Index(arguments)) => index::run(&context, arguments),
        Some(Command::Trace(arguments)) => trace::run(&context, arguments),
        Some(Command::Status) => status::run(&context),
        None => geocode::run(&context, cli.site, cli.file, cli.output),
    }
}
