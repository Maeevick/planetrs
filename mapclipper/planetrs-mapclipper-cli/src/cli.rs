//! The command line, which is two verbs and a convenience.
//!
//! The split is the design, not an optimisation. **Harvesting** reads the planet and is minutes;
//! **rendering** draws what was harvested and is milliseconds. Keeping them apart is what lets a
//! framing be tried six times instead of once, and it is what lets a harvest be handed to somebody
//! who has no planet file at all.
//!
//! `clip` does both in one go, for the first time you look at a place.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use planetrs_mapclipper::error::ClipError;
use planetrs_mapclipper::scene::{Canvas, LabelClass, Place, Rules};
use planetrs_mapclipper::theme::Theme;

#[derive(Parser, Debug)]
#[command(
    name = "planetrs-mapclipper",
    version,
    about = "Cut a small illustrated map out of the OpenStreetMap planet file, offline",
    long_about = "Cut a small illustrated map out of the OpenStreetMap planet file, offline.\n\n\
                  No tile service, no API, no network. Map data is © OpenStreetMap contributors, \
                  under the ODbL 1.0: every map shows the credit and the address of \
                  https://www.openstreetmap.org/copyright, and every data file names the licence.",
    after_help = "Examples:\n  \
        planetrs-mapclipper clip planet.osm.pbf --lat 5.269 --lon -4.012 --width-km 29 --height-km 16.53 \\\n    \
            --width-px 1000 --height-px 570 --theme linework --png abidjan.png\n\n  \
        planetrs-mapclipper harvest planet.osm.pbf --lat 5.269 --lon -4.012 --width-km 40 --height-km 40 \\\n    \
            --out abidjan.mapclip\n\n  \
        planetrs-mapclipper render abidjan.mapclip --lat 5.269 --lon -4.012 --width-km 29 --height-km 16.53 \\\n    \
            --width-px 1000 --height-px 570 --svg abidjan.svg\n\n\
        Exit codes: 0 success, 1 failure, 2 bad arguments."
)]
/// The whole command line.
pub struct Cli {
    /// Settings every command shares.
    #[command(flatten)]
    pub global: Global,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Settings every command shares.
#[derive(Args, Debug)]
pub struct Global {
    /// Print nothing on stderr but errors and the attribution
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

/// What to do.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Read the planet for one area and keep it, so drawing never reads the planet again
    Harvest(HarvestArgs),

    /// Draw a harvest, at any framing it covers, in any theme
    Render(RenderArgs),

    /// Harvest and draw in one go
    Clip(ClipArgs),

    /// Report what a planet file or a harvest holds
    Inspect(InspectArgs),

    /// List the themes built in
    Themes(ThemesArgs),
}

/// What `harvest` reads, and where it keeps what it read.
#[derive(Args, Debug)]
pub struct HarvestArgs {
    /// The planet file, as published at planet.openstreetmap.org/pbf/
    pub planet: PathBuf,

    /// What ground the map is of.
    #[command(flatten)]
    pub framing: Framing,

    /// Where the harvest goes
    #[arg(long, short = 'o', value_name = "FILE")]
    pub out: PathBuf,
}

/// Which harvest `render` draws, and how.
#[derive(Args, Debug)]
pub struct RenderArgs {
    /// A harvest, as `planetrs-mapclipper harvest` writes
    pub harvest: PathBuf,

    /// What ground the map is of.
    #[command(flatten)]
    pub framing: Framing,

    /// How the map looks.
    #[command(flatten)]
    pub drawing: Drawing,

    /// Where the map goes.
    #[command(flatten)]
    pub outputs: Outputs,
}

/// What `clip` reads and draws, and whether it keeps the harvest.
#[derive(Args, Debug)]
pub struct ClipArgs {
    /// The planet file, as published at planet.openstreetmap.org/pbf/
    pub planet: PathBuf,

    /// What ground the map is of.
    #[command(flatten)]
    pub framing: Framing,

    /// How the map looks.
    #[command(flatten)]
    pub drawing: Drawing,

    /// Where the map goes.
    #[command(flatten)]
    pub outputs: Outputs,

    /// Keep the harvest here, so drawing it again costs milliseconds
    #[arg(long, value_name = "FILE")]
    pub keep_harvest: Option<PathBuf>,
}

/// The file `inspect` reports on.
#[derive(Args, Debug)]
pub struct InspectArgs {
    /// A planet file or a harvest; which one is read from the file itself
    pub file: PathBuf,

    /// How the report is written: text for a person, JSON for a program
    #[arg(long, short = 'O', value_enum, default_value_t = Shape::Text)]
    pub output: Shape,
}

/// How `themes` lists them.
#[derive(Args, Debug)]
pub struct ThemesArgs {
    /// How the report is written: text for a person, JSON for a program
    #[arg(long, short = 'O', value_enum, default_value_t = Shape::Text)]
    pub output: Shape,
}

/// How a report is written.
#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum Shape {
    /// Lines of label and value, for a person
    Text,
    /// One JSON value, for a program
    Json,
}

/// What ground the map is of.
#[derive(Args, Debug, Clone)]
pub struct Framing {
    /// Latitude of the middle of the frame
    #[arg(long, allow_negative_numbers = true)]
    pub lat: f64,

    /// Longitude of the middle of the frame
    #[arg(long, allow_negative_numbers = true)]
    pub lon: f64,

    /// How much ground the frame covers, east to west
    #[arg(long, value_name = "KM")]
    pub width_km: f64,

    /// How much ground the frame covers, north to south
    #[arg(long, value_name = "KM")]
    pub height_km: f64,

    /// How far beyond the frame to collect, as a share of it.
    ///
    /// A way is kept when one of its nodes falls in the collected box, so drawing it needs the
    /// vertices just outside the frame too, or a quay running off the edge stops dead at it.
    #[arg(long, default_value_t = 0.25)]
    pub margin: f64,
}

/// How the map looks.
#[derive(Args, Debug, Clone)]
pub struct Drawing {
    /// Canvas width in pixels
    #[arg(long, value_name = "PX", default_value_t = 1000.0)]
    pub width_px: f64,

    /// Canvas height in pixels
    #[arg(long, value_name = "PX", default_value_t = 570.0)]
    pub height_px: f64,

    /// A built-in theme, or the path to a theme file
    #[arg(long, default_value = "atlas")]
    pub theme: String,

    /// What is drawn at the point of interest
    #[arg(long, value_enum, default_value_t = MarkKind::None)]
    pub marker: MarkKind,

    /// Where the marker goes, if not the middle of the frame
    #[arg(long, allow_negative_numbers = true, value_name = "LAT")]
    pub poi_lat: Option<f64>,

    /// Where the marker goes, if not the middle of the frame
    #[arg(long, allow_negative_numbers = true, value_name = "LON")]
    pub poi_lon: Option<f64>,

    /// The words `--marker name` draws
    #[arg(long)]
    pub label: Option<String>,

    /// A name for the place, as `--name en="Port of Abidjan"`. May be given once per language.
    #[arg(long = "name", value_name = "LANG=NAME")]
    pub names: Vec<String>,

    /// An SVG drawn in place of the dot. Any icon whose paths carry no stroke of their own works.
    #[arg(long, value_name = "FILE")]
    pub icon: Option<PathBuf>,

    /// Icon height in pixels. Scales with the canvas when unset.
    #[arg(long, value_name = "PX")]
    pub icon_size: Option<f64>,

    /// Dot radius in pixels. Scales with the canvas when unset.
    #[arg(long, value_name = "PX")]
    pub dot_size: Option<f64>,

    /// Label height in pixels. Scales with the canvas when unset.
    #[arg(long, value_name = "PX")]
    pub label_size: Option<f64>,

    /// The built-up share a land use must reach to be drawn as urban. Off by default, which draws
    /// all of them.
    #[arg(long, value_name = "SHARE")]
    pub urban_density: Option<f64>,

    /// Which names to draw on the map, comma separated. Off by default.
    ///
    /// Names always go into the GeoJSON whatever this says, because a reader drawing its own map
    /// places them its own way. This decides only what the picture carries.
    #[arg(long, value_enum, value_delimiter = ',', default_value = "none")]
    pub labels: Vec<LabelKind>,
}

/// Which names the picture carries.
#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum LabelKind {
    /// No name at all
    None,
    /// Towns, villages and hamlets
    Settlements,
    /// Ponds, lakes, rivers and brooks
    Water,
    /// Woods, parks and industrial estates
    Areas,
    /// Every name above
    All,
}

/// Where the map goes. At least one is needed, or the run produces nothing.
#[derive(Args, Debug, Clone)]
pub struct Outputs {
    /// Write GeoJSON here. Lon/lat, never projected, as RFC 7946 requires.
    #[arg(long, value_name = "FILE")]
    pub geojson: Option<PathBuf>,

    /// Write SVG here
    #[arg(long, value_name = "FILE")]
    pub svg: Option<PathBuf>,

    /// Write PNG here
    #[arg(long, value_name = "FILE")]
    pub png: Option<PathBuf>,

    /// Draw the PNG at this many times the canvas size
    #[arg(long, default_value_t = 2.0)]
    pub scale: f64,
}

/// What is drawn at the point of interest.
#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum MarkKind {
    /// Nothing
    None,
    /// A dot, or the icon given with --icon
    Dot,
    /// The coordinates themselves, which are what a contract states
    LatLon,
    /// The words of --label, else the place's name, else its coordinates
    Name,
}

impl Framing {
    /// The area this framing collects, refused when it is not one.
    pub fn area(&self) -> Result<planetrs_geo::Area, ClipError> {
        Ok(planetrs_geo::Area::around(
            self.lat,
            self.lon,
            self.width_km,
            self.height_km,
            self.margin,
        )?)
    }
}

impl Drawing {
    /// The canvas these sizes describe, refused when it has no pixel.
    pub fn canvas(&self) -> Result<Canvas, ClipError> {
        Canvas::new(self.width_px, self.height_px)
    }

    /// The theme named, built in or read from a file.
    pub fn theme(&self) -> Result<Theme, ClipError> {
        Theme::named_or_at(&self.theme)
    }

    /// The classes of name the picture should carry.
    pub fn label_classes(&self) -> Vec<LabelClass> {
        let mut classes: Vec<LabelClass> = self
            .labels
            .iter()
            .flat_map(|kind| match kind {
                LabelKind::None => Vec::new(),
                LabelKind::Settlements => vec![LabelClass::Settlement],
                LabelKind::Water => vec![LabelClass::Water],
                LabelKind::Areas => vec![LabelClass::Area],
                LabelKind::All => {
                    vec![LabelClass::Settlement, LabelClass::Water, LabelClass::Area]
                }
            })
            .collect();
        classes.sort_unstable();
        classes.dedup();
        classes
    }

    /// The rules the scene follows beyond the frame and the canvas.
    pub fn rules(&self) -> Rules {
        Rules {
            urban_density: self.urban_density,
        }
    }

    /// The names given on the command line, as language and name.
    ///
    /// A `--name` without an `=` is taken as the name in an unnamed language, because writing
    /// `--name "Port of Abidjan"` is what somebody with one language will try first.
    pub fn place(&self, framing: &Framing) -> Place {
        Place {
            names: self
                .names
                .iter()
                .map(|given| match given.split_once('=') {
                    Some((language, name)) => (language.trim().to_string(), name.to_string()),
                    None => ("und".to_string(), given.clone()),
                })
                .collect(),
            at: (
                self.poi_lat.unwrap_or(framing.lat),
                self.poi_lon.unwrap_or(framing.lon),
            ),
        }
    }
}

impl Outputs {
    /// Whether any output was asked for.
    pub fn any(&self) -> bool {
        self.geojson.is_some() || self.svg.is_some() || self.png.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn when_the_command_line_is_defined_then_clap_finds_no_fault_in_it() {
        Cli::command().debug_assert();
    }

    fn parsed(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(arguments).unwrap_or_else(|error| panic!("{error}"))
    }

    /// A longitude west of Greenwich starts with a minus, and an argument parser that reads that as
    /// a flag makes half the planet unreachable.
    #[test]
    fn when_a_longitude_is_negative_then_it_is_a_number_and_not_a_flag() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "harvest",
            "planet.osm.pbf",
            "--lat",
            "5.269",
            "--lon",
            "-4.012",
            "--width-km",
            "29",
            "--height-km",
            "16.53",
            "--out",
            "a.mapclip",
        ]);

        let Command::Harvest(HarvestArgs { framing, .. }) = cli.command else {
            panic!("a harvest")
        };
        assert_eq!(framing.lon, -4.012);
    }

    /// Both spellings have to work; punishing a reader for the one they tried is hostile.
    #[test]
    fn when_a_flag_is_written_with_an_equals_then_it_means_the_same_as_with_a_space() {
        let with_space = parsed(&[
            "planetrs-mapclipper",
            "harvest",
            "p.pbf",
            "--lat",
            "5.0",
            "--lon",
            "1.0",
            "--width-km",
            "10",
            "--height-km",
            "10",
            "--out",
            "a.mapclip",
        ]);
        let with_equals = parsed(&[
            "planetrs-mapclipper",
            "harvest",
            "p.pbf",
            "--lat=5.0",
            "--lon=1.0",
            "--width-km=10",
            "--height-km=10",
            "--out=a.mapclip",
        ]);

        assert_eq!(format!("{with_space:?}"), format!("{with_equals:?}"));
    }

    #[test]
    fn when_a_place_is_named_in_several_languages_then_each_is_kept_under_its_own_tag() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "render",
            "a.mapclip",
            "--lat",
            "5.0",
            "--lon",
            "1.0",
            "--width-km",
            "10",
            "--height-km",
            "10",
            "--svg",
            "a.svg",
            "--name",
            "en=Port of Abidjan",
            "--name",
            "fr=Port d'Abidjan",
        ]);

        let Command::Render(RenderArgs { drawing, framing, .. }) = cli.command else {
            panic!("a render")
        };
        let place = drawing.place(&framing);

        assert_eq!(place.named("en"), Some("Port of Abidjan"));
        assert_eq!(place.named("fr"), Some("Port d'Abidjan"));
    }

    /// Somebody with one language will write this, and refusing it would be pedantry.
    #[test]
    fn when_a_name_states_no_language_then_it_is_still_a_name() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "render",
            "a.mapclip",
            "--lat",
            "5.0",
            "--lon",
            "1.0",
            "--width-km",
            "10",
            "--height-km",
            "10",
            "--svg",
            "a.svg",
            "--name",
            "Abidjan",
        ]);

        let Command::Render(RenderArgs { drawing, framing, .. }) = cli.command else {
            panic!("a render")
        };

        assert_eq!(drawing.place(&framing).any_name(), Some("Abidjan"));
    }

    /// The frame is centred for how it looks; the point belongs on the thing being described, and
    /// those are only the same by coincidence.
    #[test]
    fn when_a_point_of_interest_is_given_then_the_marker_leaves_the_middle_of_the_frame() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "render",
            "a.mapclip",
            "--lat",
            "5.269",
            "--lon",
            "-4.012",
            "--width-km",
            "29",
            "--height-km",
            "16.53",
            "--svg",
            "a.svg",
            "--poi-lat",
            "5.27",
            "--poi-lon",
            "-4.01",
        ]);

        let Command::Render(RenderArgs { drawing, framing, .. }) = cli.command else {
            panic!("a render")
        };

        assert_eq!(drawing.place(&framing).at, (5.27, -4.01));
    }

    #[test]
    fn when_no_point_of_interest_is_given_then_the_marker_sits_in_the_middle_of_the_frame() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "render",
            "a.mapclip",
            "--lat",
            "5.269",
            "--lon",
            "-4.012",
            "--width-km",
            "29",
            "--height-km",
            "16.53",
            "--svg",
            "a.svg",
        ]);

        let Command::Render(RenderArgs { drawing, framing, .. }) = cli.command else {
            panic!("a render")
        };

        assert_eq!(drawing.place(&framing).at, (5.269, -4.012));
    }

    #[test]
    fn when_a_required_argument_is_missing_then_it_is_refused_rather_than_guessed() {
        let missing = Cli::try_parse_from(["planetrs-mapclipper", "harvest", "planet.osm.pbf"]);

        assert!(missing.is_err());
    }

    /// Guessing at a near miss is the cheapest kindness an argument parser offers.
    #[test]
    fn when_a_subcommand_is_mistyped_then_the_nearest_one_is_suggested() {
        let error = Cli::try_parse_from(["planetrs-mapclipper", "harvset"]).unwrap_err();

        assert!(error.to_string().contains("harvest"), "got {error}");
    }

    #[test]
    fn when_no_output_is_asked_for_then_the_run_would_produce_nothing() {
        let cli = parsed(&[
            "planetrs-mapclipper",
            "render",
            "a.mapclip",
            "--lat",
            "5.0",
            "--lon",
            "1.0",
            "--width-km",
            "10",
            "--height-km",
            "10",
        ]);

        let Command::Render(RenderArgs { outputs, .. }) = cli.command else {
            panic!("a render")
        };

        assert!(!outputs.any());
    }
}
