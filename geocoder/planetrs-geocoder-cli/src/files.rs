//! Choosing where an answer file goes, and putting files there whole or not at all.
//!
//! An answer file is never written over. When the name asked for is taken, today's date is added,
//! then a counter, and the name finally used is printed. A file appears under its name only once it
//! is complete: it is written beside it first, as `.partial`, and linked into place at the end.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A calendar day, as an answer file's name carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    /// The year, `2026`.
    pub year: i64,
    /// The month, from 1.
    pub month: u32,
    /// The day of the month, from 1.
    pub day: u32,
}

impl Day {
    /// The day a moment falls on, in UTC.
    pub fn of(moment: SystemTime) -> Self {
        let seconds = match moment.duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs() as i64,
            Err(before) => -(before.duration().as_secs() as i64),
        };
        Self::from_days_since_epoch(seconds.div_euclid(86_400))
    }

    /// Howard Hinnant's `civil_from_days`, exact over the whole proleptic Gregorian calendar.
    fn from_days_since_epoch(days: i64) -> Self {
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_from_march = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
        let month = if month_from_march < 10 {
            month_from_march + 3
        } else {
            month_from_march - 9
        } as u32;
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self { year, month, day }
    }
}

impl std::fmt::Display for Day {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The names an answer file falls back on when the one asked for is taken: the same with the day
/// added, then with a counter after the day, `output-2026-10-10.csv`, `output-2026-10-10-2.csv`…
pub fn candidate_names(wanted: &Path, day: Day) -> impl Iterator<Item = PathBuf> {
    let stem = wanted
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = wanted
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let wanted = wanted.to_path_buf();
    (1..).map(move |counter: usize| {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{counter}")
        };
        wanted.with_file_name(format!("{stem}-{day}{suffix}{extension}"))
    })
}

/// The first of [`candidate_names`] not `taken`, with the name asked for itself tried first.
pub fn first_free(wanted: &Path, day: Day, taken: impl Fn(&Path) -> bool) -> PathBuf {
    if !taken(wanted) {
        return wanted.to_path_buf();
    }
    candidate_names(wanted, day)
        .find(|candidate| !taken(candidate))
        .expect("the counter never runs out")
}

/// The file an answer is written to while it is incomplete.
pub fn partial_of(wanted: &Path) -> PathBuf {
    let mut name = wanted
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".partial");
    wanted.with_file_name(name)
}

/// Writes a new file at the first free name for `wanted`, and returns that name.
///
/// `write` fills the file; nothing appears under the final name unless it succeeds. Taking the name
/// links the finished file into place, which fails rather than replaces when another process took
/// the name in the meantime, so the next name is tried instead.
pub fn write_new<T, E: From<io::Error>>(
    wanted: &Path,
    day: Day,
    write: impl FnOnce(&mut BufWriter<fs::File>) -> Result<T, E>,
) -> Result<(PathBuf, T), E> {
    let partial = partial_of(wanted);
    let mut out = BufWriter::new(fs::File::create(&partial)?);
    let written = match write(&mut out).and_then(|written| {
        out.flush()?;
        out.get_ref().sync_all()?;
        Ok(written)
    }) {
        Ok(written) => written,
        Err(error) => {
            drop(out);
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };
    drop(out);
    let published = publish(&partial, wanted, day);
    if published.is_err() {
        let _ = fs::remove_file(&partial);
    }
    Ok((published?, written))
}

fn publish(partial: &Path, wanted: &Path, day: Day) -> io::Result<PathBuf> {
    let mut candidates = std::iter::once(wanted.to_path_buf()).chain(candidate_names(wanted, day));
    loop {
        let candidate = candidates.next().expect("the counter never runs out");
        match fs::hard_link(partial, &candidate) {
            Ok(()) => {
                fs::remove_file(partial)?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) if !candidate.exists() => {
                fs::rename(partial, &candidate)?;
                return Ok(candidate);
            }
            Err(_) => continue,
        }
    }
}

/// Puts a finished directory in place of another, keeping the old one until the new one is there.
///
/// Used for the index: a build happens in a directory beside it and is swapped in only once it is
/// complete, so a crash or a full disk midway never leaves a half-built index where a whole one was.
pub fn replace_directory(finished: &Path, destination: &Path) -> io::Result<()> {
    let previous = {
        let mut name = destination
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        name.push(".previous");
        destination.with_file_name(name)
    };
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
    let had_one = destination.exists();
    if had_one {
        fs::rename(destination, &previous)?;
    }
    if let Err(error) = fs::rename(finished, destination) {
        if had_one {
            let _ = fs::rename(&previous, destination);
        }
        return Err(error);
    }
    if had_one {
        fs::remove_dir_all(&previous)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> Day {
        Day {
            year: 2026,
            month: 10,
            day: 10,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("planetrs-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn given_moments_across_the_calendar_when_dated_then_the_utc_day_is_right() {
        let at = |seconds: u64| Day::of(UNIX_EPOCH + std::time::Duration::from_secs(seconds)).to_string();

        assert_eq!(at(0), "1970-01-01");
        assert_eq!(at(951_782_400), "2000-02-29");
        assert_eq!(at(1_791_590_399), "2026-10-09");
        assert_eq!(at(1_791_590_400), "2026-10-10");
    }

    #[test]
    fn given_a_free_name_when_chosen_then_it_is_the_one_asked_for() {
        assert_eq!(
            first_free(Path::new("output.csv"), day(), |_| false),
            PathBuf::from("output.csv")
        );
    }

    #[test]
    fn given_the_name_asked_for_is_taken_when_chosen_then_the_day_is_added() {
        let taken = |path: &Path| path == Path::new("out/output.csv");

        assert_eq!(
            first_free(Path::new("out/output.csv"), day(), taken),
            PathBuf::from("out/output-2026-10-10.csv")
        );
    }

    #[test]
    fn given_the_dated_name_is_taken_too_when_chosen_then_a_counter_follows_the_day() {
        let taken = |path: &Path| {
            ["output.csv", "output-2026-10-10.csv", "output-2026-10-10-2.csv"]
                .iter()
                .any(|name| path == Path::new(name))
        };

        assert_eq!(
            first_free(Path::new("output.csv"), day(), taken),
            PathBuf::from("output-2026-10-10-3.csv")
        );
    }

    #[test]
    fn given_a_name_without_extension_when_chosen_then_the_day_still_follows_the_stem() {
        let taken = |path: &Path| path == Path::new("answers");

        assert_eq!(
            first_free(Path::new("answers"), day(), taken),
            PathBuf::from("answers-2026-10-10")
        );
    }

    #[test]
    fn given_an_existing_file_when_written_new_then_it_is_untouched_and_the_dated_name_is_used() {
        let directory = scratch("never-overwrite");
        let wanted = directory.join("output.csv");
        fs::write(&wanted, "kept").unwrap();

        let (written, ()) = write_new::<_, io::Error>(&wanted, day(), |out| out.write_all(b"new")).unwrap();

        assert_eq!(fs::read_to_string(&wanted).unwrap(), "kept");
        assert_eq!(written, directory.join("output-2026-10-10.csv"));
        assert_eq!(fs::read_to_string(&written).unwrap(), "new");
        assert!(!partial_of(&wanted).exists());
    }

    #[test]
    fn given_a_write_that_fails_when_written_new_then_no_file_appears_under_any_name() {
        let directory = scratch("failed-write");
        let wanted = directory.join("output.csv");

        let failed = write_new::<(), io::Error>(&wanted, day(), |out| {
            out.write_all(b"half")?;
            Err(io::Error::other("the geocoder stopped"))
        });

        assert!(failed.is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[test]
    fn given_an_existing_directory_when_replaced_then_the_new_one_takes_its_place_and_the_old_is_gone() {
        let directory = scratch("replace");
        let (finished, destination) = (directory.join("index.building"), directory.join("index"));
        fs::create_dir_all(&finished).unwrap();
        fs::write(finished.join("manifest.json"), "new").unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("manifest.json"), "old").unwrap();

        replace_directory(&finished, &destination).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("manifest.json")).unwrap(),
            "new"
        );
        assert!(!finished.exists());
        assert!(!directory.join("index.previous").exists());
    }
}
