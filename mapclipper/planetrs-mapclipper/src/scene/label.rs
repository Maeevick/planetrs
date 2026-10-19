//! What the map says things are called.
//!
//! A label is a candidate, not a decision. The scene works out every name worth drawing, where it
//! belongs and how much it matters; **which ones actually fit is settled at drawing time**, because
//! that is the only place pixels exist. The GeoJSON therefore carries all of them and lets a
//! consumer place them its own way, and the SVG carries the ones that did not collide.

use crate::harvest::{NamedNode, tag};

use super::classify::Category;
use super::plane::{Point, bounds};

/// What kind of thing a name belongs to, which decides how it is drawn and what it outranks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LabelClass {
    /// A town, a village, a hamlet.
    Settlement,
    /// A pond, a lake, a river, a brook.
    Water,
    /// A wood, a park, an industrial estate.
    Area,
}

impl LabelClass {
    /// The class's name, as a theme file writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Settlement => "settlement",
            Self::Water => "water",
            Self::Area => "area",
        }
    }

    /// The class of this name, if one has it.
    pub fn named(name: &str) -> Option<Self> {
        [Self::Settlement, Self::Water, Self::Area]
            .into_iter()
            .find(|it| it.name() == name)
    }
}

/// A name, where it belongs, and how hard it should fight for the space.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// The name itself.
    pub text: String,
    /// Longitude and latitude, as everything else in a scene.
    pub at: Point,
    /// What kind of thing it names.
    pub class: LabelClass,
    /// Higher wins when two labels want the same pixels. A city beats a hamlet beats a brook.
    pub rank: u32,
    /// The feature the name came from, so a reader can trace it back.
    pub osm: i64,
}

/// How much a settlement outranks another, from OSM's own `place` vocabulary.
fn place_rank(place: &str) -> Option<u32> {
    Some(match place {
        "city" => 100,
        "borough" => 95,
        "town" => 90,
        "suburb" => 80,
        "village" => 75,
        "quarter" => 70,
        "neighbourhood" => 65,
        "hamlet" => 60,
        "isolated_dwelling" | "farm" => 50,
        "locality" => 45,
        _ => return None,
    })
}

/// A name a `place=*` node carries, if it carries both.
pub fn from_named_node(node: &NamedNode) -> Option<Label> {
    let text = tag(&node.tags, "name")?;
    let rank = place_rank(tag(&node.tags, "place")?)?;

    Some(Label {
        text: text.to_string(),
        at: [node.longitude as f64 * 1e-9, node.latitude as f64 * 1e-9],
        class: LabelClass::Settlement,
        rank,
        osm: node.id,
    })
}

/// Which class a named shape's name belongs to, and what it is worth.
///
/// A hamlet in rural France is usually a named `landuse=residential` way rather than a `place`
/// node, so built ground carries settlement names as often as nodes do.
pub fn class_of(category: Category) -> Option<(LabelClass, u32)> {
    Some(match category {
        Category::Urban => (LabelClass::Settlement, 55),
        Category::Water => (LabelClass::Water, 40),
        Category::Waterway => (LabelClass::Water, 30),
        Category::Green | Category::Wetland | Category::Beach => (LabelClass::Area, 25),
        Category::Industrial => (LabelClass::Area, 20),
        _ => return None,
    })
}

/// Where a name sits on the shape it belongs to.
///
/// The middle of the bounding box for a surface, and the middle vertex for a line: a brook's name
/// belongs on the brook rather than at the centre of the rectangle it happens to span.
pub fn anchor(points: &[Point], closed: bool) -> Option<Point> {
    if points.is_empty() {
        return None;
    }
    if closed {
        let ([west, south], [east, north]) = bounds(points)?;
        return Some([(west + east) / 2.0, (south + north) / 2.0]);
    }
    Some(points[points.len() / 2])
}

/// One label per name per class, keeping the strongest claim to it.
///
/// `Le Ru` arrives as a dozen separate ways and `L'Epte` as several more, because OSM splits
/// a watercourse wherever anything about it changes. Drawing each would stack the same word down
/// the length of the brook.
pub fn strongest(mut candidates: Vec<Label>) -> Vec<Label> {
    candidates.sort_by(|left, right| {
        (left.class, &left.text)
            .cmp(&(right.class, &right.text))
            .then(right.rank.cmp(&left.rank))
            .then(left.osm.cmp(&right.osm))
    });
    candidates.dedup_by(|later, kept| later.class == kept.class && later.text == kept.text);

    candidates.sort_by(|left, right| {
        right
            .rank
            .cmp(&left.rank)
            .then(left.class.cmp(&right.class))
            .then(left.osm.cmp(&right.osm))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(place: &str, name: &str) -> NamedNode {
        NamedNode {
            id: 1,
            latitude: 49_075_400_000,
            longitude: 1_533_900_000,
            tags: vec![
                ("place".to_string(), place.to_string()),
                ("name".to_string(), name.to_string()),
            ],
        }
    }

    fn label(text: &str, class: LabelClass, rank: u32, osm: i64) -> Label {
        Label {
            text: text.to_string(),
            at: [0.0, 0.0],
            class,
            rank,
            osm,
        }
    }

    #[test]
    fn when_a_node_is_a_village_then_it_becomes_a_settlement_label() {
        let found = from_named_node(&node("village", "Giverny")).expect("a label");

        assert_eq!(found.text, "Giverny");
        assert_eq!(found.class, LabelClass::Settlement);
    }

    #[test]
    fn when_a_node_names_no_place_then_it_is_not_a_settlement() {
        let shop = NamedNode {
            id: 1,
            latitude: 0,
            longitude: 0,
            tags: vec![
                ("shop".to_string(), "bakery".to_string()),
                ("name".to_string(), "Le Fournil".to_string()),
            ],
        };

        assert_eq!(from_named_node(&shop), None);
    }

    /// A city has to win the space over a hamlet, or a dense frame labels the wrong things.
    #[test]
    fn when_two_settlements_compete_then_the_larger_kind_outranks_the_smaller() {
        let city = from_named_node(&node("city", "Rouen")).unwrap();
        let hamlet = from_named_node(&node("hamlet", "Le Colombier")).unwrap();

        assert!(city.rank > hamlet.rank);
    }

    /// In rural France a hamlet is usually a named `landuse=residential` way, not a `place` node.
    #[test]
    fn when_built_ground_carries_a_name_then_it_reads_as_a_settlement() {
        assert_eq!(
            class_of(Category::Urban).map(|(class, _)| class),
            Some(LabelClass::Settlement)
        );
    }

    #[test]
    fn when_a_brook_carries_a_name_then_it_reads_as_water() {
        assert_eq!(
            class_of(Category::Waterway).map(|(class, _)| class),
            Some(LabelClass::Water)
        );
    }

    #[test]
    fn when_a_road_carries_a_name_then_it_is_not_labelled_at_this_scope() {
        assert_eq!(class_of(Category::RoadMinor), None);
        assert_eq!(class_of(Category::Building), None);
    }

    #[test]
    fn when_a_surface_is_named_then_the_name_sits_in_the_middle_of_it() {
        let ring = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];

        assert_eq!(anchor(&ring, true), Some([1.0, 1.0]));
    }

    /// A brook's name belongs on the brook, not at the centre of the rectangle it spans, which for
    /// a curving watercourse is usually dry land.
    #[test]
    fn when_a_line_is_named_then_the_name_sits_on_the_line_rather_than_in_its_bounding_box() {
        let bend = [[0.0, 0.0], [0.0, 2.0], [2.0, 2.0]];

        assert_eq!(anchor(&bend, false), Some([0.0, 2.0]));
    }

    #[test]
    fn when_a_shape_has_no_points_then_it_has_nowhere_to_put_a_name() {
        assert_eq!(anchor(&[], true), None);
    }

    /// OSM splits a watercourse wherever anything about it changes, so `Le Ru` arrives as
    /// a dozen ways. Drawing each would stack the same word down the length of the brook.
    #[test]
    fn when_one_name_arrives_many_times_then_only_the_strongest_claim_to_it_survives() {
        let kept = strongest(vec![
            label("Le Ru", LabelClass::Water, 30, 101),
            label("Le Ru", LabelClass::Water, 30, 102),
            label("Le Ru", LabelClass::Water, 30, 103),
            label("L'Epte", LabelClass::Water, 30, 201),
        ]);

        assert_eq!(kept.len(), 2);
        assert_eq!(kept.iter().filter(|l| l.text == "Le Ru").count(), 1);
    }

    #[test]
    fn when_the_same_name_belongs_to_a_pond_and_a_brook_then_both_survive_as_one_each() {
        let kept = strongest(vec![
            label("Les Nymphéas", LabelClass::Water, 40, 1),
            label("Les Nymphéas", LabelClass::Area, 25, 2),
        ]);

        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn when_labels_are_gathered_then_the_strongest_comes_first() {
        let kept = strongest(vec![
            label("Le Ru", LabelClass::Water, 30, 2),
            label("Giverny", LabelClass::Settlement, 75, 1),
        ]);

        assert_eq!(kept[0].text, "Giverny");
    }

    /// The order feeds both outputs, so two runs over the same scene have to agree on it.
    #[test]
    fn when_the_same_candidates_are_gathered_twice_then_the_order_is_the_same() {
        let candidates = || {
            vec![
                label("B", LabelClass::Water, 30, 5),
                label("A", LabelClass::Water, 30, 9),
                label("C", LabelClass::Area, 30, 1),
            ]
        };

        assert_eq!(strongest(candidates()), strongest(candidates()));
    }
}
