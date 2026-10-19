//! The scene as a picture, at a fixed pixel size and in whatever theme was asked for.
//!
//! Web Mercator (EPSG:3857), which is what a reader has seen on every other map they have opened.
//!
//! The term that matters is handled before this: a degree of longitude is `1/cos(latitude)` shorter
//! than a degree of latitude, and the frame avoids that by being built from kilometres. What
//! Mercator adds is the second-order stretch inside one frame, measured at 0.018 px at Abidjan and
//! 0.57 px on a 40 km frame at Rotterdam. Invisible, and worth having so that this and a
//! `geoMercator` over the same GeoJSON agree exactly rather than nearly.

use planetrs_geo::BoundingBox;

use crate::error::ClipError;
use crate::scene::{Canvas, LabelClass, Point, Scene, Shape};
use crate::theme::Theme;

/// A halo behind the glyphs, as a share of the type size, so a label stays readable over water.
const HALO: f64 = 0.22;

/// Roughly what one character of a mixed-case sans costs, as a share of the type size.
///
/// An estimate on purpose: measuring would mean loading the font, and this is only used to keep two
/// labels off each other. Being a little generous errs towards dropping a label rather than
/// overlapping one, which is the right way to be wrong.
const WIDTH_PER_CHARACTER: f64 = 0.55;

/// What is drawn at the point of interest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Mark {
    /// Nothing.
    #[default]
    None,
    /// A dot, or the icon when one is given.
    Dot,
    /// The coordinates themselves, which are what a contract states and the only thing true about
    /// a point without anyone having to name it.
    Coordinates,
    /// These words.
    Named(String),
}

/// An icon lifted out of an SVG file, ready to be placed on the map.
///
/// Only the drawing is kept, never the `<svg>` wrapper: its `width`, `height` and `stroke` belong
/// to the file, not to where the icon is going. Lucide's paths carry no stroke of their own, so
/// they take whatever the group placing them sets, which is what lets one file serve any size and
/// any palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    /// The drawing's own coordinates: x, y, width and height, as its `viewBox` states them.
    pub view_box: (f64, f64, f64, f64),
    /// The elements inside the `<svg>`, as written.
    pub body: String,
}

/// The icon an SVG file's text draws, or nothing when it holds no drawing.
pub fn icon_from_svg(text: &str) -> Option<Icon> {
    let opened = text.find("<svg")?;
    let after = text[opened..].find('>')? + opened + 1;
    let closed = text.rfind("</svg>")?;
    if closed <= after {
        return None;
    }

    let head = &text[opened..after];
    let view_box = head
        .find("viewBox=\"")
        .and_then(|at| {
            let rest = &head[at + 9..];
            let end = rest.find('"')?;
            let numbers: Vec<f64> = rest[..end]
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect();
            match numbers[..] {
                [x, y, width, height] if width > 0.0 && height > 0.0 => Some((x, y, width, height)),
                _ => None,
            }
        })
        .unwrap_or((0.0, 0.0, 24.0, 24.0));

    let body = text[after..closed].trim().to_string();
    (!body.is_empty()).then_some(Icon { view_box, body })
}

/// The icon the SVG file at `path` draws.
pub fn icon_at(path: &std::path::Path) -> Result<Icon, ClipError> {
    let text = std::fs::read_to_string(path).map_err(|error| ClipError::reading(path, error))?;

    icon_from_svg(&text).ok_or_else(|| ClipError::NotAnIcon {
        path: path.to_path_buf(),
    })
}

/// The point of interest and how it is marked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marker {
    /// What is drawn there.
    pub mark: Mark,
    /// Latitude and longitude of the point, which need not be the middle of the frame.
    pub at: (f64, f64),
    /// An icon drawn in place of the dot.
    pub icon: Option<Icon>,
    /// Pixels. `None` scales from the canvas, so one command works for a 570 pixel hero and a
    /// 208 pixel band.
    pub icon_size: Option<f64>,
    /// The dot's radius in pixels, scaled from the canvas when `None`.
    pub dot_size: Option<f64>,
    /// The type size of the mark's words in pixels, scaled from the canvas when `None`.
    pub label_size: Option<f64>,
}

/// Web Mercator's y, in radians of the projected plane.
fn mercator(latitude: f64) -> f64 {
    (std::f64::consts::FRAC_PI_4 + latitude.to_radians() / 2.0)
        .tan()
        .ln()
}

struct Projection {
    canvas: Canvas,
    frame: BoundingBox,
    north: f64,
    span: f64,
}

impl Projection {
    fn of(canvas: Canvas, frame: BoundingBox) -> Self {
        let north = mercator(frame.north);
        Self {
            canvas,
            frame,
            north,
            span: north - mercator(frame.south),
        }
    }

    fn place(&self, [longitude, latitude]: Point) -> (f64, f64) {
        let x = (longitude - self.frame.west) / (self.frame.east - self.frame.west) * self.canvas.width;
        let y = (self.north - mercator(latitude)) / self.span * self.canvas.height;
        (x, y)
    }
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A coordinate with its trailing zeros gone, so 5.27 stays 5.27 and Dakar's 14.6699 keeps its four.
///
/// Padding the first to four decimals would claim a precision the source does not have.
fn decimal(value: f64) -> String {
    let written = format!("{value:.4}");
    let trimmed = written.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The words a coordinates mark carries, `lat 5.269 : lon -4.012`.
pub fn coordinates_label(latitude: f64, longitude: f64) -> String {
    format!("lat {} : lon {}", decimal(latitude), decimal(longitude))
}

fn ring_of(projection: &Projection, points: &[Point], closed: bool) -> String {
    let mut out = String::with_capacity(points.len() * 12);
    for (index, point) in points.iter().enumerate() {
        let (x, y) = projection.place(*point);
        out.push_str(if index == 0 { "M" } else { "L" });
        out.push_str(&format!("{x:.1} {y:.1}"));
        if index + 1 < points.len() {
            out.push(' ');
        }
    }
    if closed {
        out.push('Z');
    }
    out
}

/// One path per feature, with any holes as further subpaths.
///
/// The even-odd rule on the group is what turns an inner ring into a hole rather than a second
/// filled blob. Drawing the island in the land colour instead would be wrong the moment anything is
/// drawn under the water.
fn path_of(projection: &Projection, shape: &Shape) -> String {
    let mut out = ring_of(projection, &shape.points, shape.closed);
    for hole in &shape.holes {
        out.push(' ');
        out.push_str(&ring_of(projection, hole, true));
    }
    out
}

/// The scene drawn in the theme, with its marker and the names of the classes asked for that fit,
/// the same text for the same scene every time.
pub fn write(scene: &Scene, theme: &Theme, marker: &Marker, labels: &[LabelClass]) -> String {
    let projection = Projection::of(scene.canvas, scene.frame);
    let (width, height) = (scene.canvas.width, scene.canvas.height);
    let title = scene.place.any_name().unwrap_or("map").to_string();
    let mut out = String::with_capacity(scene.features.len() * 128);

    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width:.0} {height:.0}\" \
         width=\"{width:.0}\" height=\"{height:.0}\" role=\"img\" aria-label=\"{}\">\n",
        escaped(&title)
    ));
    out.push_str(&format!("<title>{}</title>\n", escaped(&title)));
    out.push_str(&format!("<desc>{}</desc>\n", escaped(crate::credit::DATA_NOTICE)));
    out.push_str(&format!(
        "<rect width=\"{width:.0}\" height=\"{height:.0}\" fill=\"{}\"/>\n",
        escaped(&theme.background)
    ));

    for category in &theme.order {
        let drawn: Vec<_> = scene.of_category(*category).collect();
        if drawn.is_empty() {
            continue;
        }

        let ink = theme.ink_of(*category);
        out.push_str(&format!(
            "<g fill=\"{}\" fill-rule=\"evenodd\" stroke=\"{}\" stroke-width=\"{}\" \
             stroke-linejoin=\"round\" stroke-linecap=\"round\">\n",
            escaped(ink.fill.as_deref().unwrap_or("none")),
            escaped(ink.stroke.as_deref().unwrap_or("none")),
            ink.width
        ));
        for feature in drawn {
            // An open run is a line, and SVG closes a path implicitly before filling it. Letting
            // one take the group's fill draws a wedge following its own course, which is how a
            // two-metre brook came out wider than the pond beside it.
            let surface = if feature.shape.closed {
                ""
            } else {
                " fill=\"none\""
            };
            out.push_str(&format!(
                "<path{surface} d=\"{}\"/>\n",
                path_of(&projection, &feature.shape)
            ));
        }
        out.push_str("</g>\n");
    }

    let (credit, credit_claim) = credited(&projection.canvas, theme);
    let mut taken: Vec<Box> = vec![credit_claim];
    let drawn_marker = marked(&projection, theme, marker, &mut taken);
    out.push_str(&named(&projection, theme, scene, labels, &mut taken));
    out.push_str(&drawn_marker);
    out.push_str(&credit);

    out.push_str("</svg>\n");
    out
}

/// The credit's type size, and the smallest it shrinks to on a narrow canvas.
const CREDIT_SIZE: f64 = 10.0;
const SMALLEST_CREDIT_SIZE: f64 = 6.0;
const CREDIT_INSET: f64 = 6.0;
const CREDIT_LEADING: f64 = 1.2;

/// The credit in the bottom right corner, and the space it claims so that no name covers it.
///
/// A map is a Produced Work of OpenStreetMap under the ODbL, and once it is a picture the credit can
/// only be read off it: the lines of [`crate::credit::MAP_CREDIT`], the copyright page written out in
/// full, linked where the SVG is read in a browser. On a canvas too narrow for them the type shrinks
/// rather than the credit being cut.
fn credited(canvas: &Canvas, theme: &Theme) -> (String, Box) {
    let lines = crate::credit::MAP_CREDIT;
    let longest = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0) as f64;
    let room = canvas.width - 2.0 * CREDIT_INSET;
    let size = (room / (longest * WIDTH_PER_CHARACTER)).clamp(SMALLEST_CREDIT_SIZE, CREDIT_SIZE);
    let leading = size * CREDIT_LEADING;
    let right = canvas.width - CREDIT_INSET;
    let last_baseline = canvas.height - CREDIT_INSET;
    let first_baseline = last_baseline - leading * (lines.len() - 1) as f64;
    let claim = Box {
        left: right - longest * size * WIDTH_PER_CHARACTER,
        top: first_baseline - size,
        right,
        bottom: last_baseline + size * 0.3,
    };
    let spans: String = lines
        .iter()
        .enumerate()
        .map(|(at, line)| {
            format!(
                "<tspan x=\"{right:.1}\" y=\"{:.1}\">{}</tspan>",
                first_baseline + leading * at as f64,
                escaped(line)
            )
        })
        .collect();
    let text = format!(
        "<a href=\"{}\"><text class=\"attribution\" text-anchor=\"end\" \
         font-family=\"system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif\" \
         font-size=\"{size:.1}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{:.1}\" \
         paint-order=\"stroke\">{spans}</text></a>\n",
        crate::credit::COPYRIGHT_PAGE,
        escaped(&theme.ink),
        escaped(&theme.background),
        size * HALO
    );
    (text, claim)
}

/// A rectangle of canvas a piece of text has claimed.
#[derive(Debug, Clone, Copy)]
struct Box {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Box {
    fn centred(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            left: x - width / 2.0,
            top: y - height / 2.0,
            right: x + width / 2.0,
            bottom: y + height / 2.0,
        }
    }

    /// The space a line of text centred on a point would take.
    fn around(x: f64, y: f64, text: &str, size: f64) -> Self {
        let half_width = text.chars().count() as f64 * size * WIDTH_PER_CHARACTER / 2.0;
        let half_height = size * 0.6;

        Self {
            left: x - half_width,
            top: y - half_height,
            right: x + half_width,
            bottom: y + half_height,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }

    fn within(&self, canvas: &Canvas) -> bool {
        self.left >= 0.0 && self.top >= 0.0 && self.right <= canvas.width && self.bottom <= canvas.height
    }
}

/// Draws the names that fit, strongest first.
///
/// Greedy and deliberately simple: a name is drawn when nothing already drawn is in its way, and
/// dropped when something is. The scene has already ranked them, so what survives a crowded corner
/// is the village rather than the brook. A name whose text would run off the canvas is dropped too,
/// because half a word at the edge reads as a mistake.
fn named(
    projection: &Projection,
    theme: &Theme,
    scene: &Scene,
    classes: &[LabelClass],
    taken: &mut Vec<Box>,
) -> String {
    if classes.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    for label in scene.labels.iter().filter(|label| classes.contains(&label.class)) {
        let ink = theme.label_ink(label.class);
        let (x, y) = projection.place(label.at);
        let claim = Box::around(x, y, &label.text, ink.size);

        if !claim.within(&projection.canvas) || taken.iter().any(|placed| placed.overlaps(&claim)) {
            continue;
        }
        taken.push(claim);

        out.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" \
             font-family=\"system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif\" \
             font-size=\"{:.1}\" font-weight=\"{}\" fill=\"{}\" stroke=\"{}\" \
             stroke-width=\"{:.1}\" paint-order=\"stroke\">{}</text>\n",
            y + ink.size * 0.35,
            ink.size,
            if label.class == LabelClass::Settlement {
                600
            } else {
                400
            },
            escaped(ink.colour.as_deref().unwrap_or(&theme.ink)),
            escaped(ink.halo.as_deref().unwrap_or(&theme.background)),
            ink.size * HALO,
            escaped(&label.text)
        ));
    }

    if out.is_empty() {
        out
    } else {
        format!("<g>\n{out}</g>\n")
    }
}

fn marked(projection: &Projection, theme: &Theme, marker: &Marker, taken: &mut Vec<Box>) -> String {
    if marker.mark == Mark::None && marker.icon.is_none() {
        return String::new();
    }

    let (x, y) = projection.place([marker.at.1, marker.at.0]);
    let scaled = (projection.canvas.height * 0.042).max(11.0);
    let label = marker.label_size.unwrap_or(scaled);
    let dot = marker.dot_size.unwrap_or((scaled * 0.16).max(2.5));
    let icon_size = marker.icon_size.unwrap_or(scaled);

    let text = match &marker.mark {
        Mark::None | Mark::Dot => None,
        Mark::Coordinates => Some(coordinates_label(marker.at.0, marker.at.1)),
        Mark::Named(name) => Some(name.clone()),
    };

    // Text clears whatever sits at the point: an icon is far taller than a dot, and a label asked
    // for smaller than the canvas would otherwise land on the dot it names.
    let above = if marker.icon.is_some() {
        icon_size * 0.5 + label * 0.35
    } else {
        (label * 0.75).max(dot + label * 0.5)
    };

    let mut out = String::from("<g>\n");

    // The marker claims its own space before any name is placed, so nothing is ever drawn over the
    // thing the marker was put there to mark.
    let claimed = if marker.icon.is_some() {
        icon_size
    } else {
        dot * 2.0
    };
    taken.push(Box::centred(x, y, claimed, claimed));

    if let Some(text) = text {
        taken.push(Box::around(x, y - above, &text, label));
        out.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" \
             font-family=\"system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif\" \
             font-size=\"{label:.1}\" font-weight=\"600\" fill=\"{}\" stroke=\"{}\" \
             stroke-width=\"{:.1}\" paint-order=\"stroke\">{}</text>\n",
            y - above,
            escaped(&theme.ink),
            escaped(&theme.background),
            label * HALO,
            escaped(&text)
        ));
    }

    match &marker.icon {
        None => out.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{dot:.1}\" fill=\"{}\" stroke=\"{}\" \
             stroke-width=\"{:.1}\"/>\n",
            escaped(&theme.ink),
            escaped(&theme.background),
            dot * 0.5
        )),
        Some(icon) => {
            let (vx, vy, vw, vh) = icon.view_box;
            let scale = icon_size / vh.max(vw);
            let placed = format!(
                "translate({x:.1} {y:.1}) scale({scale:.4}) translate({:.3} {:.3})",
                -(vx + vw / 2.0),
                -(vy + vh / 2.0)
            );
            // Twice: a thick pass in the background colour first, so the icon keeps its shape over
            // water as well as over the city, then the icon itself on top. Stroke width stays in
            // the icon's own units so it scales with the drawing, the way the icon was designed.
            for (colour, width) in [(&theme.background, 5.0), (&theme.ink, 2.0)] {
                out.push_str(&format!(
                    "<g transform=\"{placed}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" \
                     stroke-linecap=\"round\" stroke-linejoin=\"round\">{}</g>\n",
                    escaped(colour),
                    icon.body
                ));
            }
        }
    }

    out.push_str("</g>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harvest::Provenance;
    use crate::scene::{Category, Feature, Place, SceneReport};
    use planetrs_geo::Area;

    /// Giverny, where every fixture of this module is drawn.
    const CENTRE: Point = [1.5339, 49.0754];

    fn area() -> Area {
        Area::around(CENTRE[1], CENTRE[0], 5.0, 5.0, 0.25).unwrap()
    }

    fn scene_of(features: Vec<Feature>) -> Scene {
        Scene {
            frame: area().frame(),
            canvas: Canvas::new(1000.0, 1000.0).unwrap(),
            place: Place::default(),
            features,
            labels: Vec::new(),
            provenance: Provenance::default(),
            report: SceneReport::default(),
        }
    }

    fn feature(category: Category, points: Vec<Point>, closed: bool) -> Feature {
        Feature {
            id: 1,
            category,
            name: None,
            shape: Shape {
                points,
                holes: Vec::new(),
                closed,
            },
        }
    }

    fn near_centre(east: f64, north: f64) -> Point {
        [CENTRE[0] + east, CENTRE[1] + north]
    }

    fn brook() -> Vec<Point> {
        vec![
            near_centre(-0.012, -0.002),
            near_centre(-0.007, 0.0),
            near_centre(-0.002, -0.001),
            near_centre(0.003, 0.001),
        ]
    }

    fn pond() -> Vec<Point> {
        vec![
            near_centre(-0.012, -0.002),
            near_centre(-0.007, -0.002),
            near_centre(-0.007, 0.0),
            near_centre(-0.012, 0.0),
            near_centre(-0.012, -0.002),
        ]
    }

    fn filling_water() -> Theme {
        crate::theme::parse(
            "background = \"#fff\"\nink = \"#000\"\norder = [\"water\"]\n\n[water]\nfill = \"#a6cfe4\"\n",
            std::path::Path::new("test.toml"),
        )
        .unwrap()
    }

    fn paths(drawn: &str) -> Vec<String> {
        drawn
            .lines()
            .filter(|line| line.trim_start().starts_with("<path"))
            .map(str::to_string)
            .collect()
    }

    /// The bug that made a brook read wider than the pond beside it. SVG closes an open path
    /// implicitly before filling it, so a stream drawn in a category that has a fill comes out as a
    /// long filled wedge following its own course.
    #[test]
    fn given_an_open_brook_in_a_category_with_a_fill_when_drawn_then_it_is_never_filled() {
        let drawn = write(
            &scene_of(vec![feature(Category::Water, brook(), false)]),
            &filling_water(),
            &Marker::default(),
            &[],
        );

        let path = paths(&drawn).pop().expect("a path");

        assert!(
            path.contains("fill=\"none\""),
            "an open path must not take the group's fill: {path}"
        );
    }

    #[test]
    fn given_a_closed_pond_in_a_category_with_a_fill_when_drawn_then_it_takes_the_fill_its_theme_gives_it() {
        let drawn = write(
            &scene_of(vec![feature(Category::Water, pond(), true)]),
            &filling_water(),
            &Marker::default(),
            &[],
        );

        let path = paths(&drawn).pop().expect("a path");

        assert!(!path.contains("fill=\"none\""), "a ring is a surface: {path}");
        assert!(path.ends_with("Z\"/>"), "and it closes");
    }

    /// A land use polygon whose run leaves the collected box comes back open, so this is not only
    /// about water: any area category can arrive as a line and must not be filled when it does.
    #[test]
    fn given_a_green_area_arriving_as_an_open_run_when_drawn_then_it_is_not_filled_either() {
        let theme = crate::theme::parse(
            "background = \"#fff\"\norder = [\"green\"]\n\n[green]\nfill = \"#e0e8d0\"\n",
            std::path::Path::new("test.toml"),
        )
        .unwrap();

        let drawn = write(
            &scene_of(vec![feature(Category::Green, brook(), false)]),
            &theme,
            &Marker::default(),
            &[],
        );

        assert!(paths(&drawn).pop().expect("a path").contains("fill=\"none\""));
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;
    use crate::harvest::Provenance;
    use crate::scene::{Label, LabelClass, Place, SceneReport};
    use planetrs_geo::Area;

    /// Giverny, where every fixture of this module is drawn.
    const CENTRE: Point = [1.5339, 49.0754];

    fn area() -> Area {
        Area::around(CENTRE[1], CENTRE[0], 5.0, 5.0, 0.25).unwrap()
    }

    fn theme() -> Theme {
        crate::theme::parse(
            "background = \"#fff\"\nink = \"#111\"\norder = [\"water\"]\n\n[water]\nfill = \"#aad\"\n",
            std::path::Path::new("test.toml"),
        )
        .unwrap()
    }

    fn label(text: &str, class: LabelClass, rank: u32, at: Point) -> Label {
        Label {
            text: text.to_string(),
            at,
            class,
            rank,
            osm: 1,
        }
    }

    fn scene_of(labels: Vec<Label>) -> Scene {
        Scene {
            frame: area().frame(),
            canvas: Canvas::new(1000.0, 1000.0).unwrap(),
            place: Place::default(),
            features: Vec::new(),
            labels,
            provenance: Provenance::default(),
            report: SceneReport::default(),
        }
    }

    fn middle() -> Point {
        CENTRE
    }

    fn drawn_names(scene: &Scene, classes: &[LabelClass]) -> Vec<String> {
        write(scene, &theme(), &Marker::default(), classes)
            .split("<text ")
            .skip(1)
            .filter(|part| !part.starts_with("class=\"attribution\""))
            .filter_map(|part| {
                part.split_once('>')
                    .and_then(|(_, rest)| rest.split_once("</text>"))
            })
            .map(|(text, _)| text.to_string())
            .collect()
    }

    /// A map is a Produced Work of OpenStreetMap: once it leaves this tool, whoever looks at the
    /// picture has to be able to read whose data it shows, and a description hidden in the file is
    /// gone the moment it becomes a PNG.
    #[test]
    fn given_an_empty_map_when_written_then_the_openstreetmap_attribution_is_drawn_on_it() {
        let drawn = write(&scene_of(vec![]), &theme(), &Marker::default(), &[]);

        assert!(drawn.contains(">© OpenStreetMap contributors</tspan>"), "{drawn}");
    }

    /// A picture holds no link, so the OpenStreetMap Foundation asks for the copyright page's full
    /// address on it, as on a printed page.
    #[test]
    fn given_an_empty_map_when_written_then_the_copyright_page_is_written_out_in_full() {
        let drawn = write(&scene_of(vec![]), &theme(), &Marker::default(), &[]);

        assert!(
            drawn.contains(">https://www.openstreetmap.org/copyright</tspan>"),
            "{drawn}"
        );
    }

    #[test]
    fn given_an_empty_map_when_written_then_its_credit_links_to_the_copyright_page_where_a_link_works() {
        let drawn = write(&scene_of(vec![]), &theme(), &Marker::default(), &[]);

        assert!(
            drawn.contains("<a href=\"https://www.openstreetmap.org/copyright\">"),
            "{drawn}"
        );
    }

    #[test]
    fn given_a_canvas_two_hundred_pixels_wide_when_credited_then_the_credit_shrinks_to_stay_on_it() {
        let canvas = Canvas::new(200.0, 120.0).unwrap();

        let (_, claim) = credited(&canvas, &theme());

        assert!(claim.within(&canvas), "{claim:?}");
    }

    #[test]
    fn given_a_village_named_where_the_attribution_sits_when_labelled_then_the_name_gives_way() {
        let frame = area().frame();
        let corner = [
            frame.west + (frame.east - frame.west) * 0.95,
            frame.south + (frame.north - frame.south) * 0.015,
        ];
        let scene = scene_of(vec![label("Giverny", LabelClass::Settlement, 75, corner)]);

        assert!(drawn_names(&scene, &[LabelClass::Settlement]).is_empty());
    }

    #[test]
    fn given_a_named_village_when_no_class_is_asked_for_then_no_name_is_drawn() {
        let scene = scene_of(vec![label("Giverny", LabelClass::Settlement, 75, middle())]);

        assert!(drawn_names(&scene, &[]).is_empty());
    }

    #[test]
    fn given_a_named_village_when_its_class_is_asked_for_then_its_name_is_drawn() {
        let scene = scene_of(vec![label("Giverny", LabelClass::Settlement, 75, middle())]);

        assert_eq!(drawn_names(&scene, &[LabelClass::Settlement]), vec!["Giverny"]);
    }

    /// Asking for settlements and getting brooks would make the flag meaningless.
    #[test]
    fn given_a_named_village_and_a_named_brook_when_one_class_is_asked_for_then_the_others_stay_undrawn() {
        let scene = scene_of(vec![
            label("Giverny", LabelClass::Settlement, 75, middle()),
            label("Le Ru", LabelClass::Water, 30, middle()),
        ]);

        assert_eq!(drawn_names(&scene, &[LabelClass::Settlement]), vec!["Giverny"]);
    }

    /// The whole point of ranking: in a crowded corner what survives is the village, not the brook.
    #[test]
    fn given_a_village_and_a_brook_named_on_the_same_pixels_when_labelled_then_the_stronger_one_keeps_them() {
        let scene = scene_of(vec![
            label("Giverny", LabelClass::Settlement, 75, middle()),
            label("Le Ru", LabelClass::Water, 30, middle()),
        ]);

        assert_eq!(
            drawn_names(&scene, &[LabelClass::Settlement, LabelClass::Water]),
            vec!["Giverny"]
        );
    }

    #[test]
    fn given_two_settlements_named_far_apart_when_labelled_then_both_are_drawn() {
        let frame = area().frame();
        let scene = scene_of(vec![
            label(
                "Giverny",
                LabelClass::Settlement,
                75,
                [frame.west + 0.005, frame.south + 0.005],
            ),
            label(
                "Vernon",
                LabelClass::Settlement,
                60,
                [frame.east - 0.005, frame.north - 0.005],
            ),
        ]);

        assert_eq!(drawn_names(&scene, &[LabelClass::Settlement]).len(), 2);
    }

    /// Half a word at the edge of the picture reads as a mistake rather than as a name.
    #[test]
    fn given_a_long_name_against_the_east_edge_when_labelled_then_it_is_dropped_rather_than_clipped() {
        let frame = area().frame();
        let scene = scene_of(vec![label(
            "Sainte-Geneviève-lès-Gasny",
            LabelClass::Settlement,
            75,
            [frame.east - 0.0002, frame.middle_latitude()],
        )]);

        assert!(drawn_names(&scene, &[LabelClass::Settlement]).is_empty());
    }

    /// A name drawn over the dot hides the thing the dot was put there to mark.
    #[test]
    fn given_a_marker_on_a_named_brook_when_labelled_then_no_name_is_drawn_on_top_of_it() {
        let scene = scene_of(vec![label("Le Ru", LabelClass::Water, 30, middle())]);
        let marker = Marker {
            mark: Mark::Dot,
            at: (CENTRE[1], CENTRE[0]),
            ..Marker::default()
        };

        let drawn = write(&scene, &theme(), &marker, &[LabelClass::Water]);

        assert!(!drawn.contains("Le Ru"));
    }

    #[test]
    fn given_a_name_holding_markup_when_written_then_the_svg_stays_well_formed() {
        let scene = scene_of(vec![label(
            "Saint-Louis & <Nord>",
            LabelClass::Settlement,
            75,
            middle(),
        )]);

        let drawn = write(&scene, &theme(), &Marker::default(), &[LabelClass::Settlement]);

        assert!(drawn.contains("Saint-Louis &amp; &lt;Nord&gt;"));
    }
}
