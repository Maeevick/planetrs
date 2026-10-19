//! Turning a harvest into something drawable, without touching a file.
//!
//! This is the whole of the interesting half and it is a pure function: a harvest, a frame, a
//! canvas and a rule or two go in, and a list of classified, clipped, simplified shapes comes out.
//! Nothing here reads the planet, so all of it is testable with a handful of nodes in memory.
//!
//! Everything is decided in pixels of the canvas being drawn at, because pixels are what say what
//! can be seen. The same harvest drawn at 1000 pixels and at 208 keeps different detail, and it
//! should.

mod classify;
mod density;
mod label;
mod plane;
mod rings;
mod shore;
mod simplify;

use std::collections::BTreeMap;

use planetrs_geo::{Area, BoundingBox};

use crate::error::ClipError;
use crate::harvest::{Harvest, Provenance, Tags, tag};

use classify::category;
use rings::NodeTable;

pub use classify::Category;
pub use label::{Label, LabelClass};
pub use plane::Point;
pub use rings::Shape;

/// Half a pixel of tolerance is under the width of the thinnest stroke a theme draws.
const TOLERANCE_PIXELS: f64 = 0.5;
/// A shed smaller than this is a dot wherever it is put.
const SMALLEST_BUILDING_PIXELS: f64 = 1.5;
/// Features are kept when they reach a frame grown by this much, so nothing pops in at the edge.
const VISIBLE_MARGIN: f64 = 0.02;
/// Built-up share is measured on a grid this coarse, which is a city block.
const DENSITY_CELL_METRES: f64 = 250.0;

/// The picture's size in pixels, which decides what detail is worth keeping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    /// Pixels across.
    pub width: f64,
    /// Pixels down.
    pub height: f64,
}

impl Canvas {
    /// A canvas of at least one pixel each way, refused otherwise.
    pub fn new(width: f64, height: f64) -> Result<Self, ClipError> {
        if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
            return Err(ClipError::CanvasNotPositive);
        }
        Ok(Self { width, height })
    }

    /// Degrees of ground per pixel, taken on the tighter axis so nothing is dropped that the other
    /// axis could still show.
    fn per_pixel(&self, frame: &BoundingBox) -> f64 {
        ((frame.east - frame.west) / self.width).min((frame.north - frame.south) / self.height)
    }

    /// How far the frame's shape differs from the canvas's, as a ratio of one to the other.
    ///
    /// A frame and a canvas that disagree stretch the map. 29 km on 1000 px wants 16.53 km on 570,
    /// not 17.1, and the difference is visible long before it is obvious.
    pub fn stretch(&self, frame: &BoundingBox) -> f64 {
        let ground = frame.width_km() / frame.height_km();
        let canvas = self.width / self.height;
        (ground / canvas).max(canvas / ground)
    }
}

/// What the map is of, in whatever languages the caller has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place {
    /// Language tag to name, so an application can draw the text its own reader wants.
    pub names: BTreeMap<String, String>,
    /// The point the names belong to, as latitude and longitude.
    ///
    /// Not the middle of the picture. The frame is centred for how it looks; the place is where the
    /// thing being described actually is, and those are only the same by coincidence.
    pub at: (f64, f64),
}

impl Place {
    /// One of its names, the first by language tag, when it has any.
    pub fn any_name(&self) -> Option<&str> {
        self.names.values().next().map(String::as_str)
    }

    /// Its name in this language, `en` or `fr`, when it has one.
    pub fn named(&self, language: &str) -> Option<&str> {
        self.names.get(language).map(String::as_str)
    }
}

/// The few decisions that are not derived from the frame and the canvas.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rules {
    /// The built-up share a land use polygon must reach to be drawn as urban.
    ///
    /// `None` draws all of them, which is the default and what the approved renders used. Raising
    /// it is more faithful to the phrase "heavy urban density" and looks worse: it punches holes in
    /// the city and the map loses its articulation. Kept for dense ports where the whole frame
    /// would otherwise go one colour.
    pub urban_density: Option<f64>,
}

/// One thing drawn on the map: a way or a multipolygon, classified, clipped and simplified.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// The OpenStreetMap id of the way or relation it came from.
    pub id: i64,
    /// What it is drawn as.
    pub category: Category,
    /// Its name, when it has one.
    pub name: Option<String>,
    /// Its coordinates, as longitude and latitude.
    pub shape: Shape,
}

/// What building a scene kept, dropped and simplified, so a run can say where the detail went.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneReport {
    /// Features drawn.
    pub kept: u64,
    /// Features left out as off the frame or under a pixel.
    pub dropped: u64,
    /// Vertices before simplification.
    pub vertices_before: u64,
    /// Vertices after it.
    pub vertices_after: u64,
    /// Multipolygon rings closed from their member ways.
    pub rings_closed: u64,
    /// The multipolygons whose members never closed into a ring.
    pub relations_left_open: Vec<i64>,
    /// How many features of each category were drawn.
    pub by_category: Vec<(Category, u64)>,
    /// Names worth drawing.
    pub labels: u64,
    /// p50, p75, p90, p99 of the built-up share, over the cells holding anything.
    pub built_up: Option<[f64; 4]>,
}

/// Everything a map of one frame shows, ready to be written as GeoJSON or drawn as SVG.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The ground the picture shows.
    pub frame: BoundingBox,
    /// The picture's size.
    pub canvas: Canvas,
    /// What the map is of.
    pub place: Place,
    /// What is drawn, back to front as the scene found it.
    pub features: Vec<Feature>,
    /// Every name worth drawing, ranked. Which of them fit is settled at drawing time, because
    /// that is the only place pixels exist, so this list is always complete rather than filtered.
    pub labels: Vec<Label>,
    /// The planet file the harvest was cut from.
    pub provenance: Provenance,
    /// What building the scene kept and dropped.
    pub report: SceneReport,
}

impl Scene {
    /// The features drawn as this category.
    pub fn of_category(&self, category: Category) -> impl Iterator<Item = &Feature> {
        self.features
            .iter()
            .filter(move |feature| feature.category == category)
    }
}

fn named(tags: &Tags) -> Option<String> {
    tag(tags, "name").map(str::to_string)
}

/// Everything the harvest holds that this frame at this canvas can show.
pub fn build(harvest: &Harvest, area: &Area, canvas: Canvas, place: Place, rules: &Rules) -> Scene {
    let frame = area.frame();
    let visible = frame.grown(VISIBLE_MARGIN);
    let per_pixel = canvas.per_pixel(&frame);
    let tolerance = per_pixel * TOLERANCE_PIXELS;
    let smallest = per_pixel * SMALLEST_BUILDING_PIXELS;

    let nodes = NodeTable::of(&harvest.nodes);
    let built = built_up(harvest, &nodes, &visible);

    let mut report = SceneReport {
        built_up: [0.5, 0.75, 0.9, 0.99]
            .iter()
            .map(|at| built.percentile(*at))
            .collect::<Option<Vec<_>>>()
            .and_then(|found| found.try_into().ok()),
        ..SceneReport::default()
    };

    let mut features = Vec::new();
    features.extend(way_features(
        harvest,
        &nodes,
        &visible,
        tolerance,
        smallest,
        rules,
        &built,
        &mut report,
    ));
    features.extend(relation_features(
        harvest,
        &nodes,
        &visible,
        tolerance,
        &mut report,
    ));
    features.extend(sea_features(harvest, &nodes, &frame));

    features.sort_by_key(|feature| (feature.category, feature.id));

    report.kept = features.len() as u64;
    report.by_category = counted(&features);

    let labels = labels_for(harvest, &features, &frame);
    report.labels = labels.len() as u64;

    Scene {
        frame,
        canvas,
        place,
        features,
        labels,
        provenance: harvest.provenance.clone(),
        report,
    }
}

/// Every name in the frame, from the settlement nodes and from the shapes that carry one.
fn labels_for(harvest: &Harvest, features: &[Feature], frame: &BoundingBox) -> Vec<Label> {
    let from_nodes = harvest.named_nodes.iter().filter_map(label::from_named_node);

    let from_shapes = features.iter().filter_map(|feature| {
        let text = feature.name.clone()?;
        let (class, rank) = label::class_of(feature.category)?;
        let at = label::anchor(&feature.shape.points, feature.shape.closed)?;
        Some(Label {
            text,
            at,
            class,
            rank,
            osm: feature.id,
        })
    });

    let inside = from_nodes
        .chain(from_shapes)
        .filter(|found| frame.holds(found.at[0], found.at[1]))
        .collect();

    label::strongest(inside)
}

fn counted(features: &[Feature]) -> Vec<(Category, u64)> {
    let mut counts: BTreeMap<Category, u64> = BTreeMap::new();
    for feature in features {
        *counts.entry(feature.category).or_default() += 1;
    }
    let mut ordered: Vec<(Category, u64)> = counts.into_iter().collect();
    ordered.sort_by_key(|(category, count)| (std::cmp::Reverse(*count), *category));
    ordered
}

fn built_up(harvest: &Harvest, nodes: &NodeTable, visible: &BoundingBox) -> density::Density {
    let rings: Vec<Vec<Point>> = harvest
        .ways
        .iter()
        .filter(|way| category(&way.tags) == Some(Category::Building))
        .flat_map(|way| rings::shapes(&way.refs, nodes))
        .map(|shape| shape.points)
        .collect();

    density::Density::of(rings.iter().map(Vec::as_slice), visible, DENSITY_CELL_METRES)
}

#[allow(clippy::too_many_arguments)]
fn way_features(
    harvest: &Harvest,
    nodes: &NodeTable,
    visible: &BoundingBox,
    tolerance: f64,
    smallest: f64,
    rules: &Rules,
    built: &density::Density,
    report: &mut SceneReport,
) -> Vec<Feature> {
    let mut features = Vec::new();

    for way in &harvest.ways {
        let Some(category) = category(&way.tags) else {
            continue;
        };
        let name = named(&way.tags);

        for shape in rings::shapes(&way.refs, nodes) {
            if !simplify::touches(&shape.points, visible) {
                report.dropped += 1;
                continue;
            }

            let (width, height) = plane::extent(&shape.points);
            if category == Category::Building && width < smallest && height < smallest {
                report.dropped += 1;
                continue;
            }

            if category == Category::Urban && !dense_enough(&shape.points, built, rules) {
                report.dropped += 1;
                continue;
            }

            report.vertices_before += shape.points.len() as u64;
            let points = simplify::douglas_peucker(&shape.points, tolerance);
            if points.len() < 2 {
                report.dropped += 1;
                continue;
            }
            report.vertices_after += points.len() as u64;

            features.push(Feature {
                id: way.id,
                category,
                name: name.clone(),
                shape: Shape {
                    points,
                    holes: Vec::new(),
                    closed: shape.closed && category.is_area(),
                },
            });
        }
    }

    features
}

/// Land use says people live here; the buildings say how densely.
fn dense_enough(points: &[Point], built: &density::Density, rules: &Rules) -> bool {
    let Some(threshold) = rules.urban_density else {
        return true;
    };

    plane::bounds(points).is_some_and(|(low, high)| built.over(low, high) >= threshold)
}

fn relation_features(
    harvest: &Harvest,
    nodes: &NodeTable,
    visible: &BoundingBox,
    tolerance: f64,
    report: &mut SceneReport,
) -> Vec<Feature> {
    let refs_of = rings::refs_of(&harvest.ways);
    let rect = shore::Rect::from(*visible);
    let mut features = Vec::new();

    for relation in &harvest.relations {
        let Some(category) = category(&relation.tags) else {
            continue;
        };
        let name = named(&relation.tags);

        let assembled = rings::multipolygon(&relation.members, &refs_of, nodes);
        if assembled.is_empty() {
            report.relations_left_open.push(relation.id);
            continue;
        }
        report.rings_closed += assembled.len() as u64;

        for shape in assembled {
            if !simplify::touches(&shape.points, visible) {
                report.dropped += 1;
                continue;
            }

            let outer = simplify::douglas_peucker(&shore::clip_ring(&shape.points, &rect), tolerance);
            if outer.len() < 4 {
                report.dropped += 1;
                continue;
            }

            let holes: Vec<Vec<Point>> = shape
                .holes
                .iter()
                .map(|hole| simplify::douglas_peucker(&shore::clip_ring(hole, &rect), tolerance))
                .filter(|hole| hole.len() >= 4)
                .collect();

            features.push(Feature {
                id: relation.id,
                category,
                name: name.clone(),
                shape: Shape {
                    points: outer,
                    holes,
                    closed: true,
                },
            });
        }
    }

    features
}

/// The sea, which OSM does not store and which has to be recovered from the shore.
fn sea_features(harvest: &Harvest, nodes: &NodeTable, frame: &BoundingBox) -> Vec<Feature> {
    let shoreline: Vec<(i64, Vec<i64>)> = harvest
        .ways
        .iter()
        .filter(|way| category(&way.tags) == Some(Category::Coastline))
        .map(|way| (way.id, way.refs.clone()))
        .collect();

    if shoreline.is_empty() {
        return Vec::new();
    }

    let rect = shore::Rect::from(*frame);
    let fragments: Vec<Vec<Point>> = rings::join(&shoreline)
        .iter()
        .flat_map(|chain| rings::shapes(chain, nodes))
        .flat_map(|shape| shore::clip(&shape.points, &rect))
        .collect();

    shore::water(fragments, &rect)
        .into_iter()
        .map(|ring| Feature {
            id: 0,
            category: Category::Sea,
            name: None,
            shape: Shape {
                points: ring,
                holes: Vec::new(),
                closed: true,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harvest::{Member, Node, Relation, Way};

    const AT: (f64, f64) = (5.27, -4.01);

    fn area() -> Area {
        Area::around(AT.0, AT.1, 10.0, 10.0, 0.1).unwrap()
    }

    fn canvas() -> Canvas {
        Canvas::new(1000.0, 1000.0).unwrap()
    }

    /// Nodes on a square `metres` across, centred on the frame, as ids `from..from+4`.
    fn square(from: i64, metres: f64) -> Vec<Node> {
        let half = metres / 2.0 / 111_320.0;
        [(-half, -half), (half, -half), (half, half), (-half, half)]
            .iter()
            .enumerate()
            .map(|(index, (dx, dy))| Node {
                id: from + index as i64,
                latitude: ((AT.0 + dy) * 1e9) as i64,
                longitude: ((AT.1 + dx) * 1e9) as i64,
            })
            .collect()
    }

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn harvest_of(nodes: Vec<Node>, ways: Vec<Way>, relations: Vec<Relation>) -> Harvest {
        Harvest {
            provenance: Provenance::default(),
            collected: area().collected(),
            nodes,
            named_nodes: vec![],
            ways,
            relations,
        }
        .sorted()
    }

    fn scene_of(harvest: &Harvest, rules: Rules) -> Scene {
        build(harvest, &area(), canvas(), Place::default(), &rules)
    }

    #[test]
    fn given_a_harvest_holding_a_dock_when_the_scene_is_built_then_the_dock_is_drawn_as_water() {
        let harvest = harvest_of(
            square(1, 2000.0),
            vec![Way {
                id: 7,
                tags: tags(&[("waterway", "dock")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        let scene = scene_of(&harvest, Rules::default());

        assert_eq!(scene.features.len(), 1);
        assert_eq!(scene.features[0].category, Category::Water);
    }

    #[test]
    fn given_a_dock_mapped_as_a_closed_way_when_the_scene_is_built_then_it_is_a_surface_rather_than_a_line() {
        let harvest = harvest_of(
            square(1, 2000.0),
            vec![Way {
                id: 7,
                tags: tags(&[("waterway", "dock")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        assert!(scene_of(&harvest, Rules::default()).features[0].shape.closed);
    }

    /// At ten kilometres on a thousand pixels a pixel is ten metres, so a two metre shed is a dot
    /// wherever it is put and drawing it only makes the file bigger.
    #[test]
    fn given_a_building_smaller_than_a_pixel_when_the_scene_is_built_then_it_is_not_drawn() {
        let harvest = harvest_of(
            square(1, 2.0),
            vec![Way {
                id: 7,
                tags: tags(&[("building", "yes")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        let scene = scene_of(&harvest, Rules::default());

        assert!(scene.features.is_empty());
        assert_eq!(scene.report.dropped, 1);
    }

    #[test]
    fn given_a_building_larger_than_a_pixel_when_the_scene_is_built_then_it_is_drawn() {
        let harvest = harvest_of(
            square(1, 200.0),
            vec![Way {
                id: 7,
                tags: tags(&[("building", "yes")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        assert_eq!(scene_of(&harvest, Rules::default()).features.len(), 1);
    }

    #[test]
    fn given_a_road_far_beyond_the_frame_when_the_scene_is_built_then_it_is_not_drawn() {
        let far = vec![
            Node {
                id: 1,
                latitude: 40_000_000_000,
                longitude: 2_000_000_000,
            },
            Node {
                id: 2,
                latitude: 40_100_000_000,
                longitude: 2_100_000_000,
            },
        ];
        let harvest = harvest_of(
            far,
            vec![Way {
                id: 7,
                tags: tags(&[("highway", "primary")]),
                refs: vec![1, 2],
            }],
            vec![],
        );

        assert!(scene_of(&harvest, Rules::default()).features.is_empty());
    }

    /// The scene feeds both the GeoJSON and the SVG, so its order is part of the output and two
    /// runs over the same harvest have to agree on it.
    #[test]
    fn given_a_dock_and_a_forest_when_the_scene_is_built_twice_then_both_come_out_in_the_same_order() {
        let harvest = harvest_of(
            [square(1, 2000.0), square(10, 1500.0)].concat(),
            vec![
                Way {
                    id: 20,
                    tags: tags(&[("landuse", "forest")]),
                    refs: vec![10, 11, 12, 13, 10],
                },
                Way {
                    id: 7,
                    tags: tags(&[("waterway", "dock")]),
                    refs: vec![1, 2, 3, 4, 1],
                },
            ],
            vec![],
        );

        let once = scene_of(&harvest, Rules::default());
        let twice = scene_of(&harvest, Rules::default());

        assert_eq!(once.features, twice.features);
    }

    #[test]
    fn given_a_lagoon_whose_two_ways_close_into_a_ring_when_the_scene_is_built_then_the_lagoon_is_drawn() {
        let harvest = harvest_of(
            square(1, 4000.0),
            vec![
                Way {
                    id: 20,
                    tags: tags(&[]),
                    refs: vec![1, 2, 3],
                },
                Way {
                    id: 21,
                    tags: tags(&[]),
                    refs: vec![3, 4, 1],
                },
            ],
            vec![Relation {
                id: 99,
                tags: tags(&[("type", "multipolygon"), ("natural", "water"), ("name", "Lagune")]),
                members: vec![
                    Member {
                        way: 20,
                        role: "outer".into(),
                    },
                    Member {
                        way: 21,
                        role: "outer".into(),
                    },
                ],
            }],
        );

        let scene = scene_of(&harvest, Rules::default());

        assert_eq!(scene.report.rings_closed, 1);
        assert_eq!(scene.features[0].name.as_deref(), Some("Lagune"));
    }

    /// A relation whose members never close is worth naming rather than dropping quietly: it is how
    /// the Ébrié lagoon stayed dry with every one of its 166 members in hand.
    #[test]
    fn given_an_open_multipolygon_when_the_scene_is_built_then_it_is_reported_rather_than_dropped_quietly() {
        let harvest = harvest_of(
            square(1, 4000.0),
            vec![Way {
                id: 20,
                tags: tags(&[]),
                refs: vec![1, 2, 3],
            }],
            vec![Relation {
                id: 99,
                tags: tags(&[("type", "multipolygon"), ("natural", "water")]),
                members: vec![Member {
                    way: 20,
                    role: "outer".into(),
                }],
            }],
        );

        assert_eq!(
            scene_of(&harvest, Rules::default()).report.relations_left_open,
            vec![99]
        );
    }

    #[test]
    fn given_a_shore_crossing_the_frame_when_the_scene_is_built_then_the_water_behind_it_is_filled() {
        let west = area().frame().west - 0.01;
        let east = area().frame().east + 0.01;
        let nodes = vec![
            Node {
                id: 1,
                latitude: (AT.0 * 1e9) as i64,
                longitude: (west * 1e9) as i64,
            },
            Node {
                id: 2,
                latitude: (AT.0 * 1e9) as i64,
                longitude: (east * 1e9) as i64,
            },
        ];
        let harvest = harvest_of(
            nodes,
            vec![Way {
                id: 7,
                tags: tags(&[("natural", "coastline")]),
                refs: vec![1, 2],
            }],
            vec![],
        );

        let scene = scene_of(&harvest, Rules::default());

        assert_eq!(scene.of_category(Category::Sea).count(), 1);
    }

    #[test]
    fn given_a_building_and_no_shore_when_the_scene_is_built_then_no_sea_is_invented() {
        let harvest = harvest_of(
            square(1, 2000.0),
            vec![Way {
                id: 7,
                tags: tags(&[("building", "yes")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        assert_eq!(
            scene_of(&harvest, Rules::default())
                .of_category(Category::Sea)
                .count(),
            0
        );
    }

    #[test]
    fn given_urban_land_with_nothing_built_on_it_when_a_density_threshold_is_set_then_it_is_not_coloured() {
        let harvest = harvest_of(
            square(1, 2000.0),
            vec![Way {
                id: 7,
                tags: tags(&[("landuse", "residential")]),
                refs: vec![1, 2, 3, 4, 1],
            }],
            vec![],
        );

        let all = scene_of(&harvest, Rules::default());
        let filtered = scene_of(
            &harvest,
            Rules {
                urban_density: Some(0.25),
            },
        );

        assert_eq!(all.features.len(), 1, "with no threshold every land use is drawn");
        assert!(filtered.features.is_empty(), "with one, empty ground is not");
    }

    /// 29 km on 1000 px wants 16.53 km on 570, not 17.1, and the difference is visible long before
    /// it is obvious.
    #[test]
    fn given_a_wide_frame_and_a_square_canvas_when_fitted_together_then_the_stretch_is_measurable() {
        let wide = Area::around(AT.0, AT.1, 29.0, 16.53, 0.1).unwrap();

        let matched = Canvas::new(1000.0, 570.0).unwrap().stretch(&wide.frame());
        let stretched = Canvas::new(1000.0, 1000.0).unwrap().stretch(&wide.frame());

        assert!(
            (matched - 1.0).abs() < 0.01,
            "a canvas of the frame's own shape does not stretch it"
        );
        assert!(stretched > 1.7, "a square canvas on a wide frame does");
    }

    #[test]
    fn given_a_width_of_zero_or_not_a_number_when_a_canvas_is_made_then_it_is_refused() {
        assert!(matches!(
            Canvas::new(0.0, 570.0),
            Err(ClipError::CanvasNotPositive)
        ));
        assert!(matches!(
            Canvas::new(f64::NAN, 570.0),
            Err(ClipError::CanvasNotPositive)
        ));
    }

    #[test]
    fn given_a_place_named_in_english_and_in_french_when_asked_by_language_then_each_name_comes_back() {
        let place = Place {
            names: [
                ("en".to_string(), "Port of Abidjan".to_string()),
                ("fr".to_string(), "Port d'Abidjan".to_string()),
            ]
            .into_iter()
            .collect(),
            at: AT,
        };

        assert_eq!(place.named("fr"), Some("Port d'Abidjan"));
        assert_eq!(place.named("de"), None);
    }
}
