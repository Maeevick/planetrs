//! The command line as clap reads it, and nothing else.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

const ABOUT: &str = "A coordinate for a place or a postal address, from OpenStreetMap data alone";

const LONG_ABOUT: &str = "\
A coordinate for a place or a postal address, from OpenStreetMap data alone.

Geocodes one site given as a single argument, or a whole semicolon-separated file, against a dataset
built on this machine from the OpenStreetMap planet: no API, no rate limit, and no network once the
planet is on disk. Every answer says how precise it is, from the door to the commune's centre.";

const SHORT_FOOTER: &str = "\
Examples:
  planetrs-geocoder download
  planetrs-geocoder index
  planetrs-geocoder \"Acme;8 Route des Tilleuls;89100;Sens;France\"
  planetrs-geocoder -f sites.csv -o answers.csv

Exit codes: 0 success, NotFound rows included; 1 failure; 2 bad arguments or a malformed site file.
More with --help.

Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

const LONG_FOOTER: &str = "\
Sites:
  A site is five fields, label;address;postal code;city;country. A site file starts with that
  header, in any order and any case, is UTF-8, and quotes any field holding a semicolon, a double
  quote or a line break. Answers repeat the five fields as written, then lat;lon;precision.

Precision, best first:
  HouseNumber          the asked door of the asked street
  NearestHouseNumber   the recorded door of the asked street closest to the asked number
  Shop                 a shop of the label on the asked street, or alone of its label in the commune
  Street               the middle of the asked street, or of the named hamlet
  Commune              the commune's centre: look before using it, unless the commune is small
  NotFound             not even the commune; lat and lon are left empty

How a site is read:
  Anything between parentheses, in any field, is ignored:
      17 rue des Écoles (LE BOURG)              reads  17 rue des Écoles
  The address is read several ways: whole, before its first spaced dash, after it, split into parts
  at commas, slashes, \"et\" and \"and\", and each part again without the designators that open it.
  An exact match is kept, and a matched door beats a nearest door, which beats a street:
      150 Avenue de l'Europe - Robert Schuman   the whole address, a street of that name
      CC Cap Horizon - Rue Lavandière           Rue Lavandière, after the dash
      ZAC St Martin                             ZAC St Martin, a road named after its zone
      C.C. La Pinsonnière                       La Pinsonnière, without its designator
  Otherwise the parts are matched loosely, and a street written beside a zone wins over the zone:
      ZA du Pont Vieux/Rue Paul Cézanne         Rue Paul Cézanne, not Pont Vieux
  A dash between two door numbers is a range, 86 - 88 Rue Corot; a dash inside a name has no space,
  Saint-Exupéry. The designators are C.C, CC, CCAL, C.Cial, CCial, Ctre Cial, Centre Cial, Centre
  Commercial, Galerie Marchande, Galerie Commerciale, Espace Commercial, Parc Commercial, Parc
  d'Activité(s), ZA, ZAC, ZAE, ZI, ZC, Zone, Zone Artisanale, Industrielle, Commerciale,
  d'Activité(s), d'Aménagement Concerté, Lieu-dit, LD and Quartier. Inside a name they are kept:
  12 Rue du Centre Commercial is searched whole.

Answer files:
  -o never writes over a file. When the name is taken, today's date (UTC) is added, then a counter:
  output-YYYY-MM-DD.csv, output-YYYY-MM-DD-2.csv. The name used is printed.

Data directory:
  --data-dir DIR, else PLANETRS_DATA_DIR, else $XDG_DATA_HOME/planetrs, else ~/.local/share/planetrs.
  It holds planets/, which download fills, and dataset/, which index builds. See status.

Examples:
  planetrs-geocoder download
  planetrs-geocoder index
  planetrs-geocoder \"Acme;8 Route des Tilleuls;89100;Sens;France\"
  planetrs-geocoder -f sites.csv -o answers.csv
  cat sites.csv | planetrs-geocoder -f - -o - | grep NotFound

Exit codes:
  0 success, NotFound rows included; 1 failure; 2 bad arguments or a malformed site file.

Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

const DOWNLOAD_FOOTER: &str = "\
The planet is about 90 GB and goes to planets/ in the data directory. A broken download resumes
where it stopped when run again, and the file takes its name only once its md5 matches the published
one: the Monday its header states, planet-261019.osm.pbf, which index then finds on its own. Asked
again the same week, nothing is fetched. Mirrors carry the same files:
https://wiki.openstreetmap.org/wiki/Planet.osm

Examples:
  planetrs-geocoder download
  planetrs-geocoder download --from https://download.geofabrik.de/europe/monaco-latest.osm.pbf

Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

const INDEX_FOOTER: &str = "\
The planet is read several times; France and Monaco take about four minutes and 28 GB of memory.
The dataset is rebuilt beside the old one, keeping the map indexes built from the same planet, and
replaces it only once complete.

Examples:
  planetrs-geocoder index
  planetrs-geocoder index --planet planet-260907.osm.pbf --countries FR,MC,BE

Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

const TRACE_FOOTER: &str = "\
Examples:
  planetrs-geocoder trace \"Acme;8 Route des Tilleuls;89100;Sens;France\"
  planetrs-geocoder trace -f puzzling.csv

Data © OpenStreetMap contributors, ODbL 1.0: https://www.openstreetmap.org/copyright";

/// The whole command line.
#[derive(Debug, Parser)]
#[command(
    name = "planetrs-geocoder",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_help = SHORT_FOOTER,
    after_long_help = LONG_FOOTER,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Settings every command shares.
    #[command(flatten)]
    pub global: Global,

    /// One site, quoted as one argument: "label;address;postal code;city;country"
    #[arg(value_name = "SITE", conflicts_with = "file")]
    pub site: Option<String>,

    /// A site file, semicolon-separated, header label;address;postal code;city;country; - reads stdin
    #[arg(short, long, value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Where a file's answers go, never written over; - writes to stdout [default: output.csv]
    #[arg(short, long, value_name = "PATH", conflicts_with = "site")]
    pub output: Option<PathBuf>,

    /// What to do instead of geocoding.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Settings every command shares.
#[derive(Debug, Args)]
pub struct Global {
    /// Where planets and the dataset are kept [env: PLANETRS_DATA_DIR]
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// The dataset's directory, to read or to build into with index, instead of the data
    /// directory's [env: PLANETRS_DATASET]
    #[arg(long, global = true, value_name = "DIR")]
    pub dataset: Option<PathBuf>,

    /// Print nothing on stderr but errors and the attribution
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

/// What to do instead of geocoding.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Download the latest planet into the data directory, resuming a broken download
    #[command(after_help = DOWNLOAD_FOOTER)]
    Download(DownloadArgs),
    /// Build the gazetteer and each country's geocoder index from a planet file, into the dataset
    #[command(after_help = INDEX_FOOTER)]
    Index(IndexArgs),
    /// Print every decision the lookup takes, for one site or each site of a file
    #[command(after_help = TRACE_FOOTER)]
    Trace(TraceArgs),
    /// Show the data directory, the planets in it and the dataset
    Status,
}

impl Command {
    /// The command as it is typed.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Download(_) => "download",
            Self::Index(_) => "index",
            Self::Trace(_) => "trace",
            Self::Status => "status",
        }
    }
}

/// Where `download` fetches from.
#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// A planet or an extract, published with its .md5 beside it
    #[arg(long, value_name = "URL", default_value = planetrs_cli::download::DEFAULT_SOURCE)]
    pub from: String,
}

/// What `index` reads, and for which countries.
#[derive(Debug, Args)]
pub struct IndexArgs {
    /// The planet file to read [default: the newest in the data directory]
    #[arg(long, value_name = "PATH")]
    pub planet: Option<PathBuf>,

    /// The countries whose streets, doors and shops are indexed, by ISO code
    #[arg(
        long,
        value_name = "CODES",
        value_delimiter = ',',
        default_values = planetrs_geocoder::DEFAULT_COUNTRIES
    )]
    pub countries: Vec<String>,
}

/// The sites `trace` follows: one, or a file of them.
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct TraceArgs {
    /// One site, quoted as one argument: "label;address;postal code;city;country"
    #[arg(value_name = "SITE")]
    pub site: Option<String>,

    /// A site file, as for geocoding; - reads stdin
    #[arg(short, long, value_name = "PATH")]
    pub file: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    const SITE: &str = "Acme;8 Route des Tilleuls;89100;Sens;France";

    fn parsed(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("planetrs-geocoder").chain(arguments.iter().copied()))
    }

    fn refused(arguments: &[&str]) -> ErrorKind {
        parsed(arguments).expect_err("refused").kind()
    }

    #[test]
    fn given_the_definition_when_checked_then_clap_finds_nothing_inconsistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn given_one_site_when_parsed_then_it_is_the_site_and_there_is_no_command() {
        let cli = parsed(&[SITE]).unwrap();

        assert_eq!((cli.site.as_deref(), cli.command.is_none()), (Some(SITE), true));
    }

    #[test]
    fn given_a_file_and_an_output_when_parsed_then_both_are_kept() {
        let cli = parsed(&["-f", "sites.csv", "-o", "answers.csv"]).unwrap();

        assert_eq!(
            (cli.file, cli.output),
            (
                Some(PathBuf::from("sites.csv")),
                Some(PathBuf::from("answers.csv"))
            )
        );
    }

    #[test]
    fn given_a_site_and_a_file_when_parsed_then_they_conflict() {
        assert_eq!(refused(&[SITE, "-f", "sites.csv"]), ErrorKind::ArgumentConflict);
    }

    /// One site's answer goes to stdout, so an output file beside it would be silently ignored.
    #[test]
    fn given_an_output_with_one_site_when_parsed_then_they_conflict() {
        assert_eq!(refused(&[SITE, "-o", "answers.csv"]), ErrorKind::ArgumentConflict);
    }

    /// Clap lets a command follow a site or a file; [`crate::commands::run`] refuses the mix.
    #[test]
    fn given_a_file_and_a_command_when_parsed_then_both_are_kept_for_the_run_to_refuse() {
        let cli = parsed(&["-f", "sites.csv", "status"]).unwrap();

        assert!(cli.file.is_some() && cli.command.is_some());
    }

    #[test]
    fn given_nothing_when_parsed_then_the_help_is_shown() {
        assert_eq!(refused(&[]), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
    }

    /// A shared setting given before the command must leave the command a command, not a site.
    #[test]
    fn given_the_dataset_option_before_any_command_when_parsed_then_the_command_is_still_a_command() {
        assert!(matches!(
            parsed(&["--dataset", "/dataset", "trace", SITE]).unwrap().command,
            Some(Command::Trace(_))
        ));
        assert!(matches!(
            parsed(&["--data-dir", "/data", "index"]).unwrap().command,
            Some(Command::Index(_))
        ));
    }

    #[test]
    fn given_shared_settings_before_or_after_a_command_when_parsed_then_both_places_work() {
        let before = parsed(&["--data-dir", "/data", "-q", "status"]).unwrap();
        let after = parsed(&["status", "--data-dir", "/data", "--dataset", "/dataset"]).unwrap();

        assert!(matches!(before.command, Some(Command::Status)) && before.site.is_none());
        assert_eq!(before.global.data_dir, Some(PathBuf::from("/data")));
        assert!(before.global.quiet);
        assert!(matches!(after.command, Some(Command::Status)));
        assert_eq!(after.global.dataset, Some(PathBuf::from("/dataset")));
    }

    #[test]
    fn given_download_without_a_source_when_parsed_then_the_planet_server_is_the_default() {
        let Some(Command::Download(arguments)) = parsed(&["download"]).unwrap().command else {
            panic!("a download command");
        };

        assert_eq!(
            arguments.from,
            "https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf"
        );
    }

    #[test]
    fn given_index_without_countries_when_parsed_then_france_and_monaco_are_the_default() {
        let Some(Command::Index(arguments)) = parsed(&["index"]).unwrap().command else {
            panic!("an index command");
        };

        assert_eq!(arguments.countries, vec!["FR", "MC"]);
    }

    #[test]
    fn given_countries_separated_by_commas_when_parsed_then_each_is_one_code() {
        let Some(Command::Index(arguments)) = parsed(&[
            "index",
            "--planet",
            "planet-260907.osm.pbf",
            "--countries",
            "fr,mc,be",
        ])
        .unwrap()
        .command
        else {
            panic!("an index command");
        };

        assert_eq!(arguments.countries, vec!["fr", "mc", "be"]);
        assert_eq!(arguments.planet, Some(PathBuf::from("planet-260907.osm.pbf")));
    }

    #[test]
    fn given_trace_without_a_site_or_a_file_when_parsed_then_one_is_asked_for() {
        assert_eq!(refused(&["trace"]), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn given_trace_with_a_site_and_a_file_when_parsed_then_they_conflict() {
        assert_eq!(
            refused(&["trace", SITE, "-f", "sites.csv"]),
            ErrorKind::ArgumentConflict
        );
    }

    /// A word that is no command can only be a site, and the site rules then refuse it with a pointer
    /// to the commands.
    #[test]
    fn given_a_misspelt_command_when_parsed_then_it_is_left_to_the_site_rules() {
        assert_eq!(
            parsed(&["stauts", "--quiet"]).unwrap().site.as_deref(),
            Some("stauts")
        );
    }
}
