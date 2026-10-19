//! What each command does, given a parsed command line.

pub mod download;
pub mod geocode;
pub mod index;
pub mod status;
pub mod trace;

use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, Result};
use planetrs_dataset::directory::MANIFEST;
use planetrs_dataset::{Dataset, DatasetError, ForReadingFiles, Location, Unreachable};
use planetrs_geocoder::{Geocoder, OpenError};

use crate::ATTRIBUTION;
use crate::cli::{Cli, Command, Global};
use crate::data_dir::DataDir;

/// The environment variable naming the dataset when `--dataset` does not.
pub const DATASET_VARIABLE: &str = "PLANETRS_DATASET";

/// A command asked for in a way it cannot be done, which exits 2 like a parsing error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(pub String);

/// What every command knows before it starts: where things are, and how much to say.
pub struct Context {
    /// Where planets and the index are kept.
    pub data: DataDir,
    /// The dataset to read or build: `--dataset`, else `PLANETRS_DATASET`, else the data directory's.
    pub dataset: Location,
    /// Whether to say anything on stderr beyond errors and the attribution.
    pub quiet: bool,
}

impl Context {
    /// Reads the shared settings, the environment standing behind `--data-dir`.
    pub fn of(global: &Global) -> Result<Self> {
        let data = DataDir::resolve(global.data_dir.as_deref(), |name| std::env::var_os(name))?;
        let named = global.dataset.clone().or_else(|| {
            std::env::var(DATASET_VARIABLE)
                .ok()
                .filter(|named| !named.is_empty())
        });
        let dataset = named.map_or_else(
            || Location::Directory(data.dataset()),
            |named| Location::of(&named),
        );
        Ok(Self {
            data,
            dataset,
            quiet: global.quiet,
        })
    }

    /// The dataset's location as a person reads it.
    pub fn dataset_name(&self) -> String {
        match &self.dataset {
            Location::Directory(directory) => directory.display().to_string(),
            Location::Address(address) => address.clone(),
        }
    }

    /// Opens the dataset, saying how long it took, or says how to get one. A remote dataset keeps
    /// what it reads in the data directory's cache.
    pub fn open_geocoder(&self) -> Result<Geocoder<Box<dyn ForReadingFiles>>> {
        let started = Instant::now();
        let files = self.dataset.files().map_err(anyhow::Error::new)?;
        let dataset = Dataset::open(files).map_err(|error| match error {
            DatasetError::Unreachable(Unreachable::Missing { file }) if file == MANIFEST => anyhow::anyhow!(
                "no dataset in {}\n  \
                 → build one:        planetrs-geocoder download, then planetrs-geocoder index\n  \
                 → from a planet:    planetrs-geocoder index --planet PATH\n  \
                 → or read another:  --dataset DIR|URL",
                self.dataset_name()
            ),
            other => anyhow::Error::new(other).context(format!(
                "the dataset in {} could not be read",
                self.dataset_name()
            )),
        })?;
        let dataset = match self.dataset {
            Location::Address(_) => dataset.cached_in(self.data.cache()),
            Location::Directory(_) => dataset,
        };
        let geocoder = Geocoder::over(dataset).map_err(|error| match error {
            OpenError::NoIndex { directory } => anyhow::anyhow!("no dataset in {}", directory.display()),
            other => anyhow::Error::new(other).context("→ rebuild it: planetrs-geocoder index"),
        })?;
        self.say(&format!(
            "opened the dataset {} in {:.1}s",
            self.dataset_name(),
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
