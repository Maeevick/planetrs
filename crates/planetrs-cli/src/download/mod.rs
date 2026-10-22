//! Fetching a planet file: its checksum first, then the file in ranges, resumable, and verified
//! before it takes its name.
//!
//! A planet is about 90 GB, so a download is expected to break at least once. Bytes go to
//! `<name>.part`, with `<name>.part.meta` saying which file of which source they belong to; a later
//! run resumes from the end of the partial file when the meta still matches, and starts over when it
//! does not. Only a file whose md5 matches the published one is renamed into place, with its
//! checksum written beside it. The planet published under its `latest` alias then takes the date
//! its header states, `planet-261019.osm.pbf`, as the server names each week's planet.

pub mod checksum;
pub mod http;
pub mod ranges;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use self::checksum::{NotAChecksum, md5_of_file};
use self::ranges::{PartMeta, file_name_of, named_by_checksum, next_range};
use crate::files::Day;

/// Where the planet is published: a link the server moves to each week's planet.
pub const DEFAULT_SOURCE: &str = "https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf";

/// The name each week's planet is published under, on planet.openstreetmap.org and its mirrors alike.
const LATEST_PLANET: &str = "planet-latest.osm.pbf";

/// What a server says about a file before it is fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// The file's length in bytes.
    pub length: u64,
    /// A strong ETag or a Last-Modified date, which a range request carries so that it is refused
    /// once the file changes.
    pub validator: Option<String>,
}

/// How a server answered a request for some bytes of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// The bytes asked for, written into the sink as they arrived.
    Range,
    /// The whole file instead, because the server serves no ranges or the file changed. Nothing
    /// was written.
    Whole,
    /// Nothing: the range starts at the end of the file or past it.
    PastTheEnd,
}

/// Where planet files come from: HTTP in a run, a fake in the tests.
pub trait Source {
    /// The text at `url`: a checksum file.
    fn text(&self, url: &str) -> io::Result<String>;

    /// The length and validator of the file at `url`.
    fn describe(&self, url: &str) -> io::Result<Remote>;

    /// Writes bytes `from..to` of the file at `url` into `sink` as they arrive, provided it is still
    /// the file `validator` names. On an error midway, what was written is a correct prefix.
    fn range(
        &self,
        url: &str,
        from: u64,
        to: u64,
        validator: Option<&str>,
        sink: &mut dyn Write,
    ) -> io::Result<Reply>;

    /// Writes the whole file at `url` into `sink`, for a server that serves no ranges.
    fn whole(&self, url: &str, sink: &mut dyn Write) -> io::Result<()>;
}

/// How a download paces itself.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Bytes asked for in one request.
    pub step: u64,
    /// Requests in a row that fail without a byte before giving up; the partial file stays for the
    /// next run. A request that brings bytes before failing starts the count again.
    pub attempts: u32,
    /// The pause after the first failure, doubled after each one that follows.
    pub first_pause: Duration,
    /// Free space to leave on the disk beyond the bytes still to come.
    pub margin: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            step: 64 * 1024 * 1024,
            attempts: 6,
            first_pause: Duration::from_secs(1),
            margin: 1_000_000_000,
        }
    }
}

/// What happens during a download, for whoever reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The file to fetch, and how much of it a previous run left.
    Starting {
        /// Where it is fetched from.
        url: String,
        /// Its length.
        length: u64,
        /// Bytes already there from a previous run.
        resumed_at: u64,
    },
    /// Bytes arrived.
    Fetched {
        /// Bytes of the file there so far.
        done: u64,
        /// Its length.
        length: u64,
    },
    /// A request failed and is tried again after a pause.
    Retrying {
        /// Failures in a row so far.
        failures: u32,
        /// Why.
        reason: String,
    },
    /// The file is being checked against its published md5.
    Verifying {
        /// Bytes checked so far.
        done: u64,
        /// Its length.
        length: u64,
    },
}

/// What a download needs besides its source, each piece replaceable in a test.
pub struct Surroundings<'a> {
    /// Free bytes on the disk holding a directory.
    pub free_space: &'a dyn Fn(&Path) -> io::Result<u64>,
    /// Waits between two attempts.
    pub pause: &'a dyn Fn(Duration),
    /// Hears every [`Event`].
    pub report: &'a dyn Fn(&Event),
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The file was fetched, verified and put in place.
    Downloaded(PathBuf),
    /// The file was already there, verified by an earlier run.
    AlreadyThere(PathBuf),
}

/// Why a download stopped.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The checksum file could not be fetched.
    #[error("fetching the checksum {url} failed")]
    Checksum {
        /// The checksum's URL.
        url: String,
        /// Why.
        #[source]
        source: io::Error,
    },
    /// The checksum file holds no checksum.
    #[error(transparent)]
    NotAChecksum(#[from] NotAChecksum),
    /// Neither the checksum nor the URL names a file.
    #[error("{0} names no file to save")]
    NoFileName(String),
    /// A file of that name is there, and no earlier run verified it.
    #[error(
        "{} is already there and was not verified by a download\n  → move it away, or remove it, and run again",
        .0.display()
    )]
    InTheWay(PathBuf),
    /// Another download is writing the same partial file.
    #[error("another download is writing {}", .0.display())]
    Locked(PathBuf),
    /// The disk cannot hold the rest of the file.
    #[error(
        "{} has {available} bytes free and the download needs {needed}, the rest of the file and 1 GB to spare\n  \
         → free some space, or download elsewhere with --data-dir DIR",
        directory.display()
    )]
    NotEnoughSpace {
        /// Where the file goes.
        directory: PathBuf,
        /// Bytes needed.
        needed: u64,
        /// Bytes free.
        available: u64,
    },
    /// The server kept failing.
    #[error("fetching {url} failed {attempts} times in a row; run download again to resume")]
    GaveUp {
        /// What was being fetched.
        url: String,
        /// Attempts made.
        attempts: u32,
        /// The last failure.
        #[source]
        source: io::Error,
    },
    /// The server kept sending the whole file when asked for a part of it.
    #[error("{0} keeps changing while it is downloaded; try again later or with --from another mirror")]
    KeepsChanging(String),
    /// The file arrived whole but not as published.
    #[error(
        "the download does not match its published md5 ({expected}, found {found}) and was removed\n  \
         → run download again"
    )]
    Corrupt {
        /// The published md5.
        expected: String,
        /// The md5 of what arrived.
        found: String,
    },
    /// The disk refused.
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn lock(file: &File, part: &Path) -> Result<(), DownloadError> {
    match fs4::FileExt::try_lock(file) {
        Ok(()) => Ok(()),
        Err(fs4::TryLockError::WouldBlock) => Err(DownloadError::Locked(part.to_path_buf())),
        Err(fs4::TryLockError::Error(error)) => Err(error.into()),
    }
}

fn verified_earlier(sidecar: &Path, md5: &str) -> bool {
    fs::read_to_string(sidecar)
        .ok()
        .and_then(|text| checksum::parse(&text).ok())
        .is_some_and(|published| published.md5 == md5)
}

/// Downloads the file `asked` points to into `directory`, as described in the module.
pub fn download(
    source: &dyn Source,
    asked: &str,
    directory: &Path,
    settings: &Settings,
    surroundings: &Surroundings,
) -> Result<Outcome, DownloadError> {
    let checksum_url = format!("{asked}.md5");
    let text = source
        .text(&checksum_url)
        .map_err(|source| DownloadError::Checksum {
            url: checksum_url.clone(),
            source,
        })?;
    let published = checksum::parse(&text)?;
    let url = match &published.file_name {
        Some(named) => named_by_checksum(asked, named),
        None => asked.to_string(),
    };
    let file_name = published
        .file_name
        .clone()
        .or_else(|| file_name_of(&url).map(str::to_string))
        .ok_or_else(|| DownloadError::NoFileName(url.clone()))?;

    fs::create_dir_all(directory)?;
    let destination = directory.join(&file_name);
    let sidecar = with_suffix(&destination, ".md5");
    if destination.exists() {
        if verified_earlier(&sidecar, &published.md5) {
            return Ok(Outcome::AlreadyThere(destination));
        }
        return Err(DownloadError::InTheWay(destination));
    }

    let part = with_suffix(&destination, ".part");
    let meta_path = with_suffix(&destination, ".part.meta");
    let remote = source.describe(&url)?;
    let meta = PartMeta {
        url: url.clone(),
        md5: published.md5.clone(),
        length: remote.length,
        validator: remote.validator.clone(),
    };
    let resumable = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|text| PartMeta::from_text(&text))
        == Some(meta.clone());
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&part)?;
    lock(&file, &part)?;
    if !resumable || file.metadata()?.len() > remote.length {
        file.set_len(0)?;
    }
    fs::write(&meta_path, meta.to_text())?;

    let mut offset = file.metadata()?.len();
    let needed = remote.length - offset + settings.margin;
    let available = (surroundings.free_space)(directory)?;
    if available < needed {
        return Err(DownloadError::NotEnoughSpace {
            directory: directory.to_path_buf(),
            needed,
            available,
        });
    }
    (surroundings.report)(&Event::Starting {
        url: url.clone(),
        length: remote.length,
        resumed_at: offset,
    });

    let mut out = BufWriter::new(file);
    out.seek(SeekFrom::Start(offset))?;
    let mut failures = 0;
    let mut restarts = 0;
    while let Some((from, to)) = next_range(offset, remote.length, settings.step) {
        let reply = source.range(&url, from, to, remote.validator.as_deref(), &mut out);
        out.flush()?;
        offset = out.get_ref().metadata()?.len();
        match reply {
            Ok(Reply::Range) => failures = 0,
            Ok(Reply::PastTheEnd) => break,
            Ok(Reply::Whole) if from == 0 => {
                source.whole(&url, &mut out)?;
                out.flush()?;
                offset = out.get_ref().metadata()?.len();
            }
            Ok(Reply::Whole) => {
                restarts += 1;
                if restarts > 2 {
                    return Err(DownloadError::KeepsChanging(url));
                }
                out.get_ref().set_len(0)?;
                out.seek(SeekFrom::Start(0))?;
                offset = 0;
            }
            Err(error) => {
                failures = if offset > from { 1 } else { failures + 1 };
                if failures >= settings.attempts {
                    return Err(DownloadError::GaveUp {
                        url,
                        attempts: failures,
                        source: error,
                    });
                }
                (surroundings.report)(&Event::Retrying {
                    failures,
                    reason: error.to_string(),
                });
                (surroundings.pause)(settings.first_pause * 2u32.pow(failures - 1));
                out.seek(SeekFrom::Start(offset))?;
                continue;
            }
        }
        (surroundings.report)(&Event::Fetched {
            done: offset,
            length: remote.length,
        });
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    drop(out);

    let found = md5_of_file(&part, &|done| {
        (surroundings.report)(&Event::Verifying {
            done,
            length: remote.length,
        })
    })?;
    if found != published.md5 {
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(&meta_path);
        return Err(DownloadError::Corrupt {
            expected: published.md5,
            found,
        });
    }
    fs::rename(&part, &destination)?;
    let destination = dated_by_its_header(destination);
    let file_name = file_name_of_path(&destination).unwrap_or(file_name);
    fs::write(
        with_suffix(&destination, ".md5"),
        format!("{}  {file_name}\n", published.md5),
    )?;
    fs::remove_file(&meta_path)?;
    Ok(Outcome::Downloaded(destination))
}

/// The name the planet published under its `latest` alias takes once verified: the Monday its header
/// states, `planet-261019.osm.pbf`, as the server names each week's planet, so that `index` finds the
/// newest planet by its name and next week's lands beside it. Any other file, a planet whose header
/// states no date, or a dated name already taken keeps the name it came with.
fn dated_by_its_header(planet: PathBuf) -> PathBuf {
    if file_name_of_path(&planet).as_deref() != Some(LATEST_PLANET) {
        return planet;
    }
    let Some(seconds) = planetrs_pbf::Planet::open(&planet)
        .ok()
        .and_then(|opened| opened.header.replication_timestamp)
        .and_then(|seconds| u64::try_from(seconds).ok())
    else {
        return planet;
    };
    let day = Day::of(UNIX_EPOCH + Duration::from_secs(seconds));
    let dated = planet.with_file_name(format!(
        "planet-{:02}{:02}{:02}.osm.pbf",
        day.year.rem_euclid(100),
        day.month,
        day.day
    ));
    if dated.exists() || fs::rename(&planet, &dated).is_err() {
        return planet;
    }
    dated
}

fn file_name_of_path(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

/// Downloads the planet at `from` into `directory` as both command lines do: saying through `say`
/// where it starts, when it retries and what it got, drawing its progress on stderr unless `quiet`,
/// and giving back where the planet lies, verified.
pub fn fetch_planet(
    from: &str,
    directory: &Path,
    quiet: bool,
    say: &dyn Fn(&str),
) -> anyhow::Result<PathBuf> {
    let progress = crate::Progress::to_stderr(quiet);
    let last_percent = std::cell::Cell::new(None);
    let report = |event: &Event| match event {
        Event::Starting {
            url,
            length,
            resumed_at,
        } => {
            let resuming = if *resumed_at > 0 {
                format!(", resuming at {}", crate::size_text(*resumed_at))
            } else {
                String::new()
            };
            say(&format!(
                "downloading {url}, {}{resuming}",
                crate::size_text(*length)
            ));
        }
        Event::Fetched { done, length } => {
            progress.note(&format!(
                "downloaded {} of {} ({}%)",
                crate::size_text(*done),
                crate::size_text(*length),
                percent(*done, *length)
            ));
        }
        Event::Retrying { failures, reason } => {
            progress.clear();
            say(&format!("retrying after failure {failures}: {reason}"));
        }
        Event::Verifying { done, length } => {
            let now = percent(*done, *length);
            if last_percent.replace(Some(now)) != Some(now) {
                progress.note(&format!("checking the md5: {now}%"));
            }
        }
    };
    let surroundings = Surroundings {
        free_space: &|path| fs4::available_space(path),
        pause: &std::thread::sleep,
        report: &report,
    };
    let outcome = download(
        &http::Http::new(),
        from,
        directory,
        &Settings::default(),
        &surroundings,
    );
    progress.clear();
    Ok(match outcome? {
        Outcome::Downloaded(planet) => {
            say(&format!("downloaded and verified {}", planet.display()));
            planet
        }
        Outcome::AlreadyThere(planet) => {
            say(&format!(
                "{} is already downloaded and verified",
                planet.display()
            ));
            planet
        }
    })
}

/// How much of a file is done, in whole percent rounded down.
fn percent(done: u64, length: u64) -> u64 {
    done.saturating_mul(100).checked_div(length).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;

    #[test]
    fn given_bytes_of_a_file_when_counted_then_the_percentage_rounds_down() {
        assert_eq!((percent(0, 200), percent(199, 200), percent(5, 0)), (0, 99, 100));
    }

    const LATEST: &str = "https://planet.example.org/pbf/planet-latest.osm.pbf";
    const DATED: &str = "https://planet.example.org/pbf/planet-260907.osm.pbf";

    fn content() -> Vec<u8> {
        (0..1_000u32).map(|at| (at % 251) as u8).collect()
    }

    fn md5_of_bytes(bytes: &[u8]) -> String {
        checksum::md5_of(bytes, &|_| {}).unwrap()
    }

    /// A server holding one file, which can drop connections, corrupt a byte or ignore ranges.
    struct Fake {
        content: Vec<u8>,
        checksum: String,
        /// For each request in turn, how many bytes it sends before its connection drops.
        drops: RefCell<VecDeque<u64>>,
        corrupt_at: Option<u64>,
        ignore_ranges: bool,
        asked: RefCell<Vec<(String, u64, u64)>>,
    }

    impl Fake {
        fn healthy() -> Self {
            Self {
                checksum: format!("{}  planet-260907.osm.pbf\n", md5_of_bytes(&content())),
                content: content(),
                drops: RefCell::new(VecDeque::new()),
                corrupt_at: None,
                ignore_ranges: false,
                asked: RefCell::new(Vec::new()),
            }
        }

        /// A server publishing `content` with an md5 naming it `published_as`.
        fn publishing(content: Vec<u8>, published_as: &str) -> Self {
            Self {
                checksum: format!("{}  {published_as}\n", md5_of_bytes(&content)),
                content,
                ..Self::healthy()
            }
        }

        fn starts(&self) -> Vec<u64> {
            self.asked.borrow().iter().map(|(_, from, _)| *from).collect()
        }
    }

    impl Source for Fake {
        fn text(&self, url: &str) -> io::Result<String> {
            assert_eq!(url, format!("{LATEST}.md5"));
            Ok(self.checksum.clone())
        }

        fn describe(&self, _url: &str) -> io::Result<Remote> {
            Ok(Remote {
                length: self.content.len() as u64,
                validator: Some("\"v1\"".into()),
            })
        }

        fn range(
            &self,
            url: &str,
            from: u64,
            to: u64,
            _validator: Option<&str>,
            sink: &mut dyn Write,
        ) -> io::Result<Reply> {
            self.asked.borrow_mut().push((url.to_string(), from, to));
            if self.ignore_ranges {
                return Ok(Reply::Whole);
            }
            if from >= self.content.len() as u64 {
                return Ok(Reply::PastTheEnd);
            }
            let mut bytes = self.content[from as usize..to as usize].to_vec();
            if let Some(at) = self.corrupt_at.filter(|at| (from..to).contains(at)) {
                bytes[(at - from) as usize] ^= 0xff;
            }
            match self.drops.borrow_mut().pop_front() {
                Some(after) => {
                    sink.write_all(&bytes[..(after as usize).min(bytes.len())])?;
                    Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "the connection dropped",
                    ))
                }
                None => {
                    sink.write_all(&bytes)?;
                    Ok(Reply::Range)
                }
            }
        }

        fn whole(&self, _url: &str, sink: &mut dyn Write) -> io::Result<()> {
            sink.write_all(&self.content)
        }
    }

    fn settings() -> Settings {
        Settings {
            step: 300,
            attempts: 3,
            first_pause: Duration::ZERO,
            margin: 10,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("planetrs-download-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn fetched(source: &Fake, directory: &Path) -> Result<Outcome, DownloadError> {
        fetched_with_space(source, directory, u64::MAX)
    }

    fn fetched_with_space(source: &Fake, directory: &Path, free: u64) -> Result<Outcome, DownloadError> {
        let surroundings = Surroundings {
            free_space: &|_| Ok(free),
            pause: &|_| {},
            report: &|_| {},
        };
        download(source, LATEST, directory, &settings(), &surroundings)
    }

    /// A planet whose header says it was dumped on Monday 7 September 2026.
    fn planet_of_monday_7_september() -> Vec<u8> {
        planetrs_pbf::testing::PlanetFile::sorted()
            .with_node(planetrs_pbf::testing::Node::at(1, 43.7384, 7.4246))
            .replicated_at(1_788_739_204, 4_321)
            .to_bytes()
    }

    /// planet.openstreetmap.org publishes each week's planet as planet-latest.osm.pbf, with an md5
    /// naming that alias, so only the planet's own header says which Monday it is.
    #[test]
    fn given_the_planet_published_under_its_latest_alias_when_downloaded_then_it_takes_the_date_its_header_states()
     {
        let directory = scratch("dated-by-header");
        let source = Fake::publishing(planet_of_monday_7_september(), "planet-latest.osm.pbf");

        let outcome = fetched(&source, &directory).unwrap();

        assert_eq!(
            outcome,
            Outcome::Downloaded(directory.join("planet-260907.osm.pbf"))
        );
        assert!(directory.join("planet-260907.osm.pbf.md5").is_file());
        assert!(!directory.join("planet-latest.osm.pbf").exists());
    }

    #[test]
    fn given_a_healthy_source_when_downloaded_then_the_file_is_whole_and_its_checksum_is_kept_beside_it() {
        let directory = scratch("healthy");
        let source = Fake::healthy();

        let outcome = fetched(&source, &directory).unwrap();

        let planet = directory.join("planet-260907.osm.pbf");
        assert_eq!(outcome, Outcome::Downloaded(planet.clone()));
        assert_eq!(fs::read(&planet).unwrap(), content());
        assert!(
            fs::read_to_string(directory.join("planet-260907.osm.pbf.md5"))
                .unwrap()
                .starts_with(&md5_of_bytes(&content()))
        );
        assert!(!directory.join("planet-260907.osm.pbf.part").exists());
        assert!(!directory.join("planet-260907.osm.pbf.part.meta").exists());
    }

    #[test]
    fn given_the_latest_planet_when_downloaded_then_the_dated_planet_its_checksum_names_is_fetched() {
        let directory = scratch("dated");
        let source = Fake::healthy();

        fetched(&source, &directory).unwrap();

        assert!(source.asked.borrow().iter().all(|(url, _, _)| url == DATED));
    }

    #[test]
    fn given_a_connection_dropped_midway_when_downloaded_then_the_rest_is_asked_for_from_where_it_stopped() {
        let directory = scratch("dropped");
        let source = Fake {
            drops: RefCell::new(VecDeque::from([150])),
            ..Fake::healthy()
        };

        fetched(&source, &directory).unwrap();

        assert_eq!(source.starts(), vec![0, 150, 450, 750]);
        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            content()
        );
    }

    #[test]
    fn given_a_source_that_keeps_failing_when_downloaded_then_a_later_run_resumes_from_the_partial_file() {
        let directory = scratch("resumed");
        let failing = Fake {
            drops: RefCell::new(VecDeque::from([100, 0, 0])),
            ..Fake::healthy()
        };

        let first = fetched(&failing, &directory);
        let healthy = Fake::healthy();
        let second = fetched(&healthy, &directory);

        assert!(matches!(first, Err(DownloadError::GaveUp { attempts: 3, .. })));
        assert_eq!(healthy.starts()[0], 100);
        assert!(matches!(second, Ok(Outcome::Downloaded(_))));
        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            content()
        );
    }

    /// A link that drops every request after a few bytes still gets there: each drop brought bytes.
    #[test]
    fn given_every_request_dropping_after_some_bytes_when_downloaded_then_it_still_completes() {
        let directory = scratch("flaky");
        let source = Fake {
            drops: RefCell::new(VecDeque::from([100; 9])),
            ..Fake::healthy()
        };

        fetched(&source, &directory).unwrap();

        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            content()
        );
    }

    #[test]
    fn given_a_partial_file_of_another_planet_when_downloaded_then_it_starts_over() {
        let directory = scratch("other-planet");
        fs::write(directory.join("planet-260907.osm.pbf.part"), vec![7u8; 500]).unwrap();
        let other = PartMeta {
            url: DATED.into(),
            md5: "0".repeat(32),
            length: 1_000,
            validator: Some("\"v1\"".into()),
        };
        fs::write(directory.join("planet-260907.osm.pbf.part.meta"), other.to_text()).unwrap();
        let source = Fake::healthy();

        fetched(&source, &directory).unwrap();

        assert_eq!(source.starts()[0], 0);
        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            content()
        );
    }

    #[test]
    fn given_a_corrupt_byte_when_downloaded_then_the_checksum_fails_and_nothing_is_left_behind() {
        let directory = scratch("corrupt");
        let source = Fake {
            corrupt_at: Some(500),
            ..Fake::healthy()
        };

        let outcome = fetched(&source, &directory);

        assert!(matches!(outcome, Err(DownloadError::Corrupt { .. })));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[test]
    fn given_a_server_ignoring_ranges_when_downloaded_then_the_whole_file_comes_in_one_request() {
        let directory = scratch("no-ranges");
        let source = Fake {
            ignore_ranges: true,
            ..Fake::healthy()
        };

        fetched(&source, &directory).unwrap();

        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            content()
        );
    }

    #[test]
    fn given_the_planet_verified_by_an_earlier_run_when_asked_again_then_nothing_is_fetched() {
        let directory = scratch("already");
        fs::write(directory.join("planet-260907.osm.pbf"), content()).unwrap();
        fs::write(
            directory.join("planet-260907.osm.pbf.md5"),
            format!("{}  planet-260907.osm.pbf\n", md5_of_bytes(&content())),
        )
        .unwrap();
        let source = Fake::healthy();

        let outcome = fetched(&source, &directory).unwrap();

        assert_eq!(
            outcome,
            Outcome::AlreadyThere(directory.join("planet-260907.osm.pbf"))
        );
        assert!(source.asked.borrow().is_empty());
    }

    #[test]
    fn given_a_file_of_that_name_no_download_verified_when_asked_then_it_is_left_alone() {
        let directory = scratch("in-the-way");
        fs::write(directory.join("planet-260907.osm.pbf"), b"mine").unwrap();

        let outcome = fetched(&Fake::healthy(), &directory);

        assert!(matches!(outcome, Err(DownloadError::InTheWay(_))));
        assert_eq!(
            fs::read(directory.join("planet-260907.osm.pbf")).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn given_too_little_free_space_when_downloaded_then_it_stops_before_asking_for_any_byte() {
        let directory = scratch("full-disk");
        let source = Fake::healthy();

        let outcome = fetched_with_space(&source, &directory, 1_009);

        assert!(matches!(
            outcome,
            Err(DownloadError::NotEnoughSpace {
                needed: 1_010,
                available: 1_009,
                ..
            })
        ));
        assert!(source.asked.borrow().is_empty());
    }

    #[test]
    fn given_another_download_writing_the_partial_file_when_downloaded_then_it_says_so() {
        let directory = scratch("locked");
        let part = directory.join("planet-260907.osm.pbf.part");
        let held = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&part)
            .unwrap();
        lock(&held, &part).unwrap();

        let outcome = fetched(&Fake::healthy(), &directory);

        assert!(matches!(outcome, Err(DownloadError::Locked(_))));
    }
}
