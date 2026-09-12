//! Downloading the planet into the data directory, and saying what to do with it.

use std::cell::Cell;
use std::path::Path;

use anyhow::Result;

use super::Context;
use super::status::size_text;
use crate::cli::DownloadArgs;
use crate::data_dir::planet_date;
use crate::download::http::Http;
use crate::download::{Event, Outcome, Settings, Surroundings, download};
use crate::progress::Progress;

fn percent(done: u64, length: u64) -> u64 {
    done.saturating_mul(100).checked_div(length).unwrap_or(100)
}

/// The command that builds the index from a downloaded file.
pub fn next_step(planet: &Path) -> String {
    if planet_date(planet).is_some() {
        "→ next: planetrs-geocoder index".to_string()
    } else {
        format!("→ next: planetrs-geocoder index --planet {}", planet.display())
    }
}

/// Downloads the planet the command line names into the data directory.
pub fn run(context: &Context, arguments: DownloadArgs) -> Result<()> {
    let directory = context.data.planets();
    let progress = Progress::to_stderr(context.quiet);
    let last_percent = Cell::new(None);
    let report = |event: &Event| match event {
        Event::Starting {
            url,
            length,
            resumed_at,
        } => {
            let resuming = if *resumed_at > 0 {
                format!(", resuming at {}", size_text(*resumed_at))
            } else {
                String::new()
            };
            context.say(&format!("downloading {url}, {}{resuming}", size_text(*length)));
        }
        Event::Fetched { done, length } => {
            progress.note(&format!(
                "downloaded {} of {} ({}%)",
                size_text(*done),
                size_text(*length),
                percent(*done, *length)
            ));
        }
        Event::Retrying { failures, reason } => {
            progress.clear();
            context.say(&format!("retrying after failure {failures}: {reason}"));
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
        &Http::new(),
        &arguments.from,
        &directory,
        &Settings::default(),
        &surroundings,
    );
    progress.clear();
    let planet = match outcome? {
        Outcome::Downloaded(planet) => {
            context.say(&format!("downloaded and verified {}", planet.display()));
            planet
        }
        Outcome::AlreadyThere(planet) => {
            context.say(&format!(
                "{} is already downloaded and verified",
                planet.display()
            ));
            planet
        }
    };
    context.say(&next_step(&planet));
    context.attribute();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_dated_planet_when_downloaded_then_index_finds_it_alone() {
        assert_eq!(
            next_step(Path::new("/data/planets/planet-260907.osm.pbf")),
            "→ next: planetrs-geocoder index"
        );
    }

    #[test]
    fn given_an_extract_when_downloaded_then_index_is_told_where_it_is() {
        assert_eq!(
            next_step(Path::new("/data/planets/monaco-latest.osm.pbf")),
            "→ next: planetrs-geocoder index --planet /data/planets/monaco-latest.osm.pbf"
        );
    }

    #[test]
    fn given_bytes_of_a_file_when_counted_then_the_percentage_rounds_down() {
        assert_eq!((percent(0, 200), percent(199, 200), percent(5, 0)), (0, 99, 100));
    }
}
