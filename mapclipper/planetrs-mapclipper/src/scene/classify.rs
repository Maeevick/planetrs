//! What a feature is drawn as, derived once from its tags.
//!
//! The renderer styles by category and never re-learns OSM's tagging, which is what lets a theme be
//! a table of colours rather than a second copy of the rules below.

use crate::harvest::{Tags, tag};

/// The fixed vocabulary a theme assigns ink to.
///
/// Fixed on purpose. A theme that could invent categories would have to carry tag rules too, and
/// then two themes could disagree about what a quay is rather than about what colour it should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// The open sea, filled from the coastline.
    Sea,
    /// Where land meets the sea, as `natural=coastline` draws it.
    Coastline,
    /// Lakes, ponds, reservoirs, docks and marinas.
    Water,
    /// Rivers, streams, canals and ditches, drawn as lines.
    Waterway,
    /// Marshes, swamps and mangroves.
    Wetland,
    /// Sand, shingle and shoals.
    Beach,
    /// Woods, parks, meadows, gardens and pitches.
    Green,
    /// Built-up land: homes, shops and offices.
    Urban,
    /// Industrial land, ports and quarries.
    Industrial,
    /// Buildings.
    Building,
    /// Piers, quays, jetties, breakwaters and groynes.
    Pier,
    /// Motorways, trunk and primary roads.
    RoadMajor,
    /// Secondary and tertiary roads.
    RoadMinor,
    /// Railway lines.
    Rail,
    /// Runways and taxiways.
    Runway,
}

/// Every category, in the order a theme lists them.
pub const EVERY_CATEGORY: [Category; 15] = [
    Category::Sea,
    Category::Coastline,
    Category::Water,
    Category::Waterway,
    Category::Wetland,
    Category::Beach,
    Category::Green,
    Category::Urban,
    Category::Industrial,
    Category::Building,
    Category::Pier,
    Category::RoadMajor,
    Category::RoadMinor,
    Category::Rail,
    Category::Runway,
];

impl Category {
    /// The category's name, as a theme file and the GeoJSON write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sea => "sea",
            Self::Coastline => "coastline",
            Self::Water => "water",
            Self::Waterway => "waterway",
            Self::Wetland => "wetland",
            Self::Beach => "beach",
            Self::Green => "green",
            Self::Urban => "urban",
            Self::Industrial => "industrial",
            Self::Building => "building",
            Self::Pier => "pier",
            Self::RoadMajor => "road-major",
            Self::RoadMinor => "road-minor",
            Self::Rail => "rail",
            Self::Runway => "runway",
        }
    }

    /// The category of this name, if one has it.
    pub fn named(name: &str) -> Option<Self> {
        EVERY_CATEGORY
            .into_iter()
            .find(|category| category.name() == name)
    }

    /// Whether a closed way of this category encloses an area or merely returns to its start.
    ///
    /// A ring road and a harbour basin are both closed ways. One is a line and the other a surface,
    /// and only the tags tell them apart.
    pub fn is_area(self) -> bool {
        !matches!(
            self,
            Self::Coastline | Self::Waterway | Self::RoadMajor | Self::RoadMinor | Self::Rail | Self::Runway
        )
    }
}

/// The category a way belongs to, or nothing when it is not worth drawing.
///
/// Ordered by what should win when a way carries several: a quay tagged both `man_made=pier` and
/// `landuse=industrial` is a pier, because that is its shape on the water.
pub fn category(tags: &Tags) -> Option<Category> {
    if tag(tags, "natural") == Some("coastline") {
        return Some(Category::Coastline);
    }

    if let Some(value) = tag(tags, "man_made")
        && matches!(value, "pier" | "breakwater" | "groyne" | "quay" | "jetty")
    {
        return Some(Category::Pier);
    }

    // A dock and a riverbank are surfaces in OSM's own model; a river, a stream and a ditch are
    // centrelines a couple of metres wide. Conflating them fills the centreline and draws a brook
    // wider than the pond beside it.
    if let Some(value) = tag(tags, "waterway") {
        return match value {
            "dock" | "riverbank" => Some(Category::Water),
            "river" | "stream" | "canal" | "brook" | "ditch" | "drain" | "tidal_channel" => {
                Some(Category::Waterway)
            }
            _ => None,
        };
    }

    if let Some(value) = tag(tags, "natural") {
        return match value {
            "water" | "bay" | "strait" => Some(Category::Water),
            "wetland" | "mangrove" => Some(Category::Wetland),
            "beach" | "sand" | "shoal" => Some(Category::Beach),
            "wood" | "scrub" | "grassland" | "heath" => Some(Category::Green),
            _ => None,
        };
    }

    if let Some(value) = tag(tags, "landuse") {
        return match value {
            "basin" | "reservoir" | "salt_pond" | "aquaculture" => Some(Category::Water),
            "industrial" | "port" | "harbour" | "quarry" => Some(Category::Industrial),
            "residential" | "commercial" | "retail" | "garages" | "railway" => Some(Category::Urban),
            "forest" | "grass" | "meadow" | "orchard" | "village_green" | "recreation_ground" => {
                Some(Category::Green)
            }
            _ => None,
        };
    }

    if let Some(value) = tag(tags, "leisure") {
        return match value {
            "marina" => Some(Category::Water),
            "park" | "garden" | "golf_course" | "pitch" => Some(Category::Green),
            _ => None,
        };
    }

    if let Some(value) = tag(tags, "highway") {
        return match value {
            "motorway" | "trunk" | "primary" | "motorway_link" | "trunk_link" | "primary_link" => {
                Some(Category::RoadMajor)
            }
            "secondary" | "tertiary" | "secondary_link" | "tertiary_link" => Some(Category::RoadMinor),
            _ => None,
        };
    }

    if let Some(value) = tag(tags, "aeroway")
        && matches!(value, "runway" | "taxiway")
    {
        return Some(Category::Runway);
    }

    if let Some(value) = tag(tags, "railway")
        && matches!(value, "rail" | "light_rail" | "narrow_gauge")
    {
        return Some(Category::Rail);
    }

    if tag(tags, "building").is_some() {
        return Some(Category::Building);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn given_a_way_tagged_as_a_harbour_basin_when_classified_then_it_is_water_rather_than_landuse() {
        assert_eq!(
            category(&tags(&[("waterway", "dock"), ("name", "Bassin")])),
            Some(Category::Water)
        );
    }

    /// A quay is both, and its shape on the water is what a reader sees.
    #[test]
    fn given_a_quay_also_tagged_as_industrial_land_when_classified_then_the_pier_wins() {
        let quay = tags(&[("man_made", "quay"), ("landuse", "industrial")]);

        assert_eq!(category(&quay), Some(Category::Pier));
    }

    #[test]
    fn given_land_tagged_residential_when_classified_then_it_reads_as_urban_rather_than_industrial() {
        assert_eq!(
            category(&tags(&[("landuse", "residential")])),
            Some(Category::Urban)
        );
        assert_eq!(
            category(&tags(&[("landuse", "port")])),
            Some(Category::Industrial)
        );
    }

    #[test]
    fn given_a_way_tagged_only_with_its_source_when_classified_then_it_is_left_out() {
        assert_eq!(category(&tags(&[("source", "survey")])), None);
    }

    /// A ring road closes on itself and is still a line; a basin closes and is a surface.
    #[test]
    fn given_a_major_road_and_water_when_asked_whether_they_enclose_an_area_then_only_the_water_does() {
        assert!(!Category::RoadMajor.is_area());
        assert!(Category::Water.is_area());
    }

    /// A theme names categories in a file, so every name has to survive the round trip or a theme
    /// silently styles nothing.
    #[test]
    fn given_every_category_when_named_then_its_name_finds_it_again() {
        assert!(
            EVERY_CATEGORY
                .into_iter()
                .all(|it| Category::named(it.name()) == Some(it))
        );
    }

    #[test]
    fn given_a_name_belonging_to_no_category_when_looked_up_then_nothing_comes_back() {
        assert_eq!(Category::named("chartreuse"), None);
    }
}

#[cfg(test)]
mod waterway_tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    /// A stream is a centreline, not a surface. OSM stores `Le Ru` as a `waterway=stream` way
    /// a couple of metres wide, and treating it as the same thing as a pond makes a brook read
    /// wider than the pond beside it.
    #[test]
    fn given_a_way_tagged_as_a_stream_when_classified_then_it_is_a_line_rather_than_a_surface() {
        assert_eq!(
            category(&tags(&[("waterway", "stream")])),
            Some(Category::Waterway)
        );
        assert!(!Category::Waterway.is_area());
    }

    #[test]
    fn given_a_way_tagged_as_a_river_or_a_ditch_when_classified_then_it_is_also_a_line() {
        for kind in ["river", "canal", "ditch", "drain", "brook"] {
            assert_eq!(
                category(&tags(&[("waterway", kind)])),
                Some(Category::Waterway),
                "{kind}"
            );
        }
    }

    /// The areal ones stay areal: a dock and a riverbank are surfaces in OSM's own model.
    #[test]
    fn given_a_way_tagged_as_a_dock_or_a_riverbank_when_classified_then_it_is_still_a_surface() {
        assert_eq!(category(&tags(&[("waterway", "dock")])), Some(Category::Water));
        assert_eq!(
            category(&tags(&[("waterway", "riverbank")])),
            Some(Category::Water)
        );
        assert!(Category::Water.is_area());
    }

    #[test]
    fn given_a_way_tagged_as_a_lake_when_classified_then_it_is_a_surface() {
        assert_eq!(category(&tags(&[("natural", "water")])), Some(Category::Water));
    }
}
