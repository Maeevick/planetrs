//! Geocoding one site to stdout, or a whole file to a new answer file.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::Result;
use planetrs_geocoder::{Answer, Geocoder, Site, refusal_of};

use super::{Context, UsageError, read_input};
use crate::files::{Day, write_new};
use crate::progress::Progress;
use crate::sites::{ANSWER_HEADER, NOT_FOUND, Row, answer_line, read_one_site, read_site_file};

const DEFAULT_OUTPUT: &str = "output.csv";
const PRECISIONS: [&str; 6] = [
    "HouseNumber",
    "NearestHouseNumber",
    "Shop",
    "Street",
    "Commune",
    NOT_FOUND,
];

/// Geocodes the site given, or the file given, as the command line asks.
pub fn run(
    context: &Context,
    site: Option<String>,
    file: Option<PathBuf>,
    output: Option<PathBuf>,
) -> Result<()> {
    match (site, file) {
        (Some(site), _) => one_site(context, &site),
        (None, Some(file)) => whole_file(context, &file, output.as_deref()),
        (None, None) => Err(UsageError(
            "give a site, \"label;address;postal code;city;country\", or a site file with -f PATH".into(),
        )
        .into()),
    }
}

fn one_site(context: &Context, text: &str) -> Result<()> {
    let row = read_one_site(text)?;
    let geocoder = context.open_geocoder()?;
    let site = row.site();
    let answer = geocoder.locate(&site);
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{ANSWER_HEADER}")?;
    writeln!(stdout, "{}", answer_line(&row, answer.as_ref()))?;
    stdout.flush()?;
    if let Some(reason) = refusal_of(&site, answer.as_ref()) {
        context.say(&format!("→ look before using it: {reason}"));
    }
    context.attribute();
    Ok(())
}

/// How many sites got each precision, and how many a person should look at.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    by_precision: BTreeMap<&'static str, usize>,
    to_look_at: usize,
    sites: usize,
}

impl Tally {
    fn count(&mut self, site: &Site, answer: Option<&Answer>) {
        let precision = answer.map_or(NOT_FOUND, |answer| answer.precision.as_str());
        *self.by_precision.entry(precision).or_default() += 1;
        self.to_look_at += usize::from(refusal_of(site, answer).is_some());
        self.sites += 1;
    }

    fn lines(&self) -> Vec<String> {
        PRECISIONS
            .iter()
            .filter_map(|precision| {
                self.by_precision
                    .get(precision)
                    .map(|count| format!("  {precision:<20}{count:>8}"))
            })
            .collect()
    }
}

fn geocode_rows(
    geocoder: &Geocoder,
    rows: &[Row],
    byte_order_mark: bool,
    progress: &Progress,
    out: &mut impl Write,
) -> Result<Tally> {
    let mut tally = Tally::default();
    if byte_order_mark {
        out.write_all("\u{feff}".as_bytes())?;
    }
    writeln!(out, "{ANSWER_HEADER}")?;
    for (done, row) in rows.iter().enumerate() {
        let site = row.site();
        let answer = geocoder.locate(&site);
        tally.count(&site, answer.as_ref());
        writeln!(out, "{}", answer_line(row, answer.as_ref()))?;
        progress.sites(done + 1, rows.len());
    }
    Ok(tally)
}

/// Where answers go when `-o` names a directory, or nothing at all.
fn wanted_output(output: Option<&Path>) -> PathBuf {
    match output {
        Some(directory) if directory.is_dir() => directory.join(DEFAULT_OUTPUT),
        Some(file) => file.to_path_buf(),
        None => PathBuf::from(DEFAULT_OUTPUT),
    }
}

fn whole_file(context: &Context, file: &Path, output: Option<&Path>) -> Result<()> {
    let sites = read_site_file(&read_input(file)?)?;
    let geocoder = context.open_geocoder()?;
    let progress = Progress::to_stderr(context.quiet);
    let started = Instant::now();
    let to_stdout = output == Some(Path::new("-"));
    let (destination, tally) = if to_stdout {
        let mut stdout = std::io::stdout().lock();
        let tally = geocode_rows(
            &geocoder,
            &sites.rows,
            sites.had_byte_order_mark,
            &progress,
            &mut stdout,
        )?;
        stdout.flush()?;
        ("stdout".to_string(), tally)
    } else {
        let wanted = wanted_output(output);
        let (written, tally) = write_new(&wanted, Day::of(SystemTime::now()), |out| {
            geocode_rows(&geocoder, &sites.rows, sites.had_byte_order_mark, &progress, out)
        })?;
        let written = std::path::absolute(&written).unwrap_or(written);
        (written.display().to_string(), tally)
    };
    progress.clear();
    context.say(&format!(
        "geocoded {} sites in {:.1}s",
        tally.sites,
        started.elapsed().as_secs_f64()
    ));
    for line in tally.lines() {
        context.say(&line);
    }
    context.say(&format!("written {destination}"));
    if tally.to_look_at > 0 {
        context.say(&format!(
            "→ look at the {} answers at a commune's centre or not found before using them",
            tally.to_look_at
        ));
    }
    context.attribute();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use planetrs_geocoder::Precision;

    fn site() -> Site {
        Site {
            label: String::new(),
            address: "3 Rue Inconnue".into(),
            postcode: "89100".into(),
            town: "SENS".into(),
            country: "FRANCE".into(),
        }
    }

    #[test]
    fn given_sites_with_and_without_an_answer_when_tallied_then_not_found_counts_and_asks_for_a_look() {
        let mut tally = Tally::default();

        tally.count(&site(), None);

        assert_eq!(tally.by_precision.get(NOT_FOUND), Some(&1));
        assert_eq!((tally.to_look_at, tally.sites), (1, 1));
    }

    #[test]
    fn given_a_tally_when_written_then_precisions_come_best_first_and_absent_ones_are_left_out() {
        let tally = Tally {
            by_precision: BTreeMap::from([(NOT_FOUND, 2), ("HouseNumber", 10)]),
            to_look_at: 2,
            sites: 12,
        };

        assert_eq!(
            tally.lines(),
            vec![
                format!("  {:<20}{:>8}", "HouseNumber", 10),
                format!("  {:<20}{:>8}", NOT_FOUND, 2)
            ]
        );
    }

    #[test]
    fn given_no_output_when_chosen_then_the_answers_go_to_output_csv_in_the_current_directory() {
        assert_eq!(wanted_output(None), PathBuf::from("output.csv"));
    }

    #[test]
    fn given_an_output_naming_a_directory_when_chosen_then_the_answers_go_to_output_csv_inside_it() {
        let directory = std::env::temp_dir();

        assert_eq!(wanted_output(Some(&directory)), directory.join("output.csv"));
    }

    #[test]
    fn given_every_precision_when_listed_then_the_tally_knows_each_name() {
        let names: Vec<&str> = [
            Precision::HouseNumber,
            Precision::NearestHouseNumber,
            Precision::Shop,
            Precision::Street,
            Precision::Commune,
        ]
        .iter()
        .map(|precision| precision.as_str())
        .collect();

        assert_eq!(names, PRECISIONS[..5]);
    }
}
