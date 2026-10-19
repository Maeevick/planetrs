//! Where planets and the index are kept between runs.
//!
//! One directory holds both, found in this order: `--data-dir`, then `PLANETRS_DATA_DIR`, then
//! `$XDG_DATA_HOME/planetrs`, then `~/.local/share/planetrs`, on macOS as on Linux so that the path
//! is the same to type everywhere. Inside it:
//!
//! ```text
//! planets/planet-YYMMDD.osm.pbf     what `download` fetches
//! geocoder/index/                   what `index` builds and every lookup reads
//! ```

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// The environment variable that moves the data directory.
pub const VARIABLE: &str = "PLANETRS_DATA_DIR";

/// What decided the data directory, so `status` can say how to move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `--data-dir`.
    Flag,
    /// `PLANETRS_DATA_DIR`.
    Variable,
    /// `XDG_DATA_HOME`.
    XdgDataHome,
    /// The home directory, by default.
    Home,
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Flag => "from --data-dir",
            Self::Variable => "from PLANETRS_DATA_DIR",
            Self::XdgDataHome => "from XDG_DATA_HOME",
            Self::Home => "the default",
        })
    }
}

/// The data directory and what chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    /// The directory itself, which may not exist yet.
    pub path: PathBuf,
    /// What chose it.
    pub source: Source,
}

/// No data directory can be found: there is no home directory and nothing names one.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("no data directory: there is no HOME. Name one with --data-dir DIR or PLANETRS_DATA_DIR")]
pub struct NoDataDir;

impl DataDir {
    /// Resolves the data directory from the flag, then the environment as `variable` reads it.
    pub fn resolve(
        flag: Option<&Path>,
        variable: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, NoDataDir> {
        let set = |name: &str| {
            variable(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        if let Some(path) = flag {
            return Ok(Self::of(path.to_path_buf(), Source::Flag));
        }
        if let Some(path) = set(VARIABLE) {
            return Ok(Self::of(path, Source::Variable));
        }
        if let Some(base) = set("XDG_DATA_HOME").filter(|base| base.is_absolute()) {
            return Ok(Self::of(base.join("planetrs"), Source::XdgDataHome));
        }
        let home = set("HOME").ok_or(NoDataDir)?;
        Ok(Self::of(
            home.join(".local").join("share").join("planetrs"),
            Source::Home,
        ))
    }

    fn of(path: PathBuf, source: Source) -> Self {
        Self { path, source }
    }

    /// Where downloaded planets go.
    pub fn planets(&self) -> PathBuf {
        self.path.join("planets")
    }

    /// Where the dataset is built and read, unless `--dataset` or `PLANETRS_DATASET` says otherwise.
    pub fn dataset(&self) -> PathBuf {
        self.path.join("dataset")
    }

    /// Where the ranges read of a remote dataset are kept, so that a second run reads none again.
    pub fn cache(&self) -> PathBuf {
        self.path.join("cache")
    }

    /// The newest planet in [`DataDir::planets`], if any.
    pub fn newest_planet(&self) -> Option<PathBuf> {
        let entries = std::fs::read_dir(self.planets()).ok()?;
        newest_planet(entries.filter_map(Result::ok).map(|entry| entry.path()))
    }
}

/// The newest of the planet files among `paths`: `planet-YYMMDD.osm.pbf`, the latest date winning.
pub fn newest_planet(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| planet_date(&path).map(|date| (date, path)))
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, path)| path)
}

/// The `YYMMDD` of a planet file's name, `260907` for `planet-260907.osm.pbf`.
pub fn planet_date(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let digits = name.strip_prefix("planet-")?.strip_suffix(".osm.pbf")?;
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |wanted| {
            pairs
                .iter()
                .find(|(key, _)| key == wanted)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn given_the_flag_and_the_variable_when_resolved_then_the_flag_wins() {
        let found = DataDir::resolve(
            Some(Path::new("/flag")),
            environment(&[(VARIABLE, "/variable"), ("HOME", "/home/me")]),
        );

        assert_eq!(found, Ok(DataDir::of("/flag".into(), Source::Flag)));
    }

    #[test]
    fn given_the_variable_and_xdg_when_resolved_then_the_variable_wins() {
        let found = DataDir::resolve(
            None,
            environment(&[
                (VARIABLE, "/variable"),
                ("XDG_DATA_HOME", "/xdg"),
                ("HOME", "/home/me"),
            ]),
        );

        assert_eq!(found, Ok(DataDir::of("/variable".into(), Source::Variable)));
    }

    #[test]
    fn given_xdg_data_home_when_resolved_then_planetrs_is_a_directory_inside_it() {
        let found = DataDir::resolve(
            None,
            environment(&[("XDG_DATA_HOME", "/xdg"), ("HOME", "/home/me")]),
        );

        assert_eq!(
            found,
            Ok(DataDir::of("/xdg/planetrs".into(), Source::XdgDataHome))
        );
    }

    #[test]
    fn given_a_relative_xdg_data_home_when_resolved_then_it_is_ignored_as_the_specification_says() {
        let found = DataDir::resolve(
            None,
            environment(&[("XDG_DATA_HOME", "xdg"), ("HOME", "/home/me")]),
        );

        assert_eq!(found.map(|found| found.source), Ok(Source::Home));
    }

    #[test]
    fn given_only_a_home_when_resolved_then_it_is_under_local_share() {
        let found = DataDir::resolve(None, environment(&[("HOME", "/home/me"), (VARIABLE, "")]));

        assert_eq!(
            found,
            Ok(DataDir::of("/home/me/.local/share/planetrs".into(), Source::Home))
        );
    }

    #[test]
    fn given_no_home_and_nothing_set_when_resolved_then_it_says_how_to_name_one() {
        assert_eq!(DataDir::resolve(None, environment(&[])), Err(NoDataDir));
    }

    #[test]
    fn given_a_data_directory_when_asked_then_planets_the_dataset_and_the_cache_have_their_own_places() {
        let data = DataDir::of("/data".into(), Source::Flag);

        assert_eq!(
            (data.planets(), data.dataset(), data.cache()),
            (
                PathBuf::from("/data/planets"),
                PathBuf::from("/data/dataset"),
                PathBuf::from("/data/cache")
            )
        );
    }

    #[test]
    fn given_several_planets_and_other_files_when_the_newest_is_asked_then_the_latest_date_wins() {
        let found = newest_planet(
            [
                "planet-260907.osm.pbf",
                "planet-261005.osm.pbf",
                "planet-261005.osm.pbf.part",
                "notes.txt",
            ]
            .map(PathBuf::from),
        );

        assert_eq!(found, Some(PathBuf::from("planet-261005.osm.pbf")));
    }

    #[test]
    fn given_no_planet_when_the_newest_is_asked_then_there_is_none() {
        assert_eq!(newest_planet([PathBuf::from("planet-latest.osm.pbf")]), None);
    }
}
