//! What an administrative boundary in OpenStreetMap says about the place it encloses.
//!
//! OpenStreetMap models a town as a boundary relation rather than as a tag repeated on every
//! doorway, which is why an address point almost never states its own commune. The boundary is
//! where the name, the official reference and the postcode live.
//!
//! `admin_level` means a commune in France and something comparable but not identical in fifty
//! other countries, so the level is recorded rather than assumed. What a given level means locally
//! is a question for whoever queries the gazetteer, not for whoever builds it.

use crate::text::{FRENCH, comparison_key};

/// The levels worth indexing for geocoding: below a region and no finer than a neighbourhood.
///
/// Level 2 is a country and level 3 a group of regions; neither answers "which town is this".
const FINEST_USEFUL_LEVEL: u8 = 11;
const COARSEST_USEFUL_LEVEL: u8 = 4;

/// The tags a place can be named by, in no particular order of authority.
///
/// `old_name` is here because it is the one an address file is most likely to carry: France merged
/// thousands of communes from 2015 and postal records still name the commune that was absorbed.
const NAME_KEYS: [&str; 7] = [
    "name",
    "alt_name",
    "old_name",
    "official_name",
    "short_name",
    "name:fr",
    "loc_name",
];

/// The tags carrying an official identifier, most specific first.
///
/// A bare `ref` is included because outside France it is often the only identifier a boundary
/// carries, and excluded from meaning anything on its own because two countries collide on the
/// same digits: an Irish electoral division's `ref` of `13055` is also Marseille's INSEE code.
/// Which tag the code came from therefore travels with it.
const REFERENCE_KEYS: [&str; 3] = ["ref:INSEE", "ref:nuts:3", "ref"];

/// An official identifier together with the tag it came from.
///
/// The scheme is not decoration. `ref:INSEE` identifies a French commune and nothing else; a bare
/// `ref` identifies whatever the local mapping convention says, which is why the two must never be
/// compared as if they were the same namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub scheme: String,
    pub code: String,
}

/// One administrative area, as its boundary relation describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdministrativeArea {
    pub level: u8,
    /// Every spelling this place answers to, as comparison keys, deduplicated.
    pub names: Vec<String>,
    pub reference: Option<Reference>,
    pub postcodes: Vec<String>,
}

/// Reads an administrative area out of a relation's tags, or nothing when they describe something
/// else.
pub fn administrative_area_of(tags: &[(String, String)]) -> Option<AdministrativeArea> {
    let value_of = |wanted: &str| {
        tags.iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    };

    if value_of("boundary") != Some("administrative") {
        return None;
    }

    let level: u8 = value_of("admin_level")?.parse().ok()?;
    if !(COARSEST_USEFUL_LEVEL..FINEST_USEFUL_LEVEL).contains(&level) {
        return None;
    }

    let names = distinct_names(&value_of);
    if names.is_empty() {
        return None;
    }

    Some(AdministrativeArea {
        level,
        names,
        reference: REFERENCE_KEYS.iter().find_map(|scheme| {
            value_of(scheme).map(|code| Reference {
                scheme: (*scheme).into(),
                code: code.into(),
            })
        }),
        postcodes: split_postcodes(value_of("postal_code").or_else(|| value_of("addr:postcode"))),
    })
}

fn distinct_names<'a>(value_of: &impl Fn(&str) -> Option<&'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    for key in NAME_KEYS {
        for spelling in value_of(key).into_iter().flat_map(|value| value.split(';')) {
            let name = comparison_key(spelling, &FRENCH);
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }

    names
}

fn split_postcodes(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| value.split([';', ',']))
        .map(str::trim)
        .filter(|postcode| !postcode.is_empty())
        .map(String::from)
        .collect()
}

/// A country, as its `admin_level=2` boundary names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    /// ISO 3166-1 alpha-2, which is what a boundary without it cannot be trusted to be.
    pub code: String,
    /// Every spelling it answers to, as comparison keys, so `FRANCE` and `République française`
    /// in a site file both reach it.
    pub names: Vec<String>,
}

const COUNTRY_LEVEL: &str = "2";
const COUNTRY_NAME_KEYS: [&str; 6] = [
    "name",
    "name:en",
    "name:fr",
    "official_name",
    "int_name",
    "short_name",
];

/// Reads a country out of a relation's tags, or nothing when they describe something else.
///
/// A level-2 boundary with no ISO code is left out on purpose. Disputed and historical areas are
/// mapped at that level too, and treating one as a country would let it claim communes.
pub fn country_of(tags: &[(String, String)]) -> Option<Country> {
    let value_of = |wanted: &str| {
        tags.iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    };

    if value_of("boundary") != Some("administrative") || value_of("admin_level") != Some(COUNTRY_LEVEL) {
        return None;
    }
    let code = value_of("ISO3166-1")
        .or_else(|| value_of("ISO3166-1:alpha2"))?
        .to_string();

    let mut names: Vec<String> = Vec::new();
    for key in COUNTRY_NAME_KEYS {
        for spelling in value_of(key).into_iter().flat_map(|value| value.split(';')) {
            let name = comparison_key(spelling, &FRENCH);
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }

    Some(Country { code, names })
}

const MEMBER_NODE: u64 = 0;

/// The node standing for a place's centre, from the relation's members.
///
/// `admin_centre` is the seat, which for a commune is the town itself. `label` is only where a
/// renderer was asked to print the name, so it is the fallback rather than the choice. Either has to
/// be a node: a way carrying the role says nothing about a single point.
pub fn centre_node_of(members: &[(i64, u64, String)]) -> Option<i64> {
    let node_with = |wanted: &str| {
        members
            .iter()
            .find(|(_, kind, role)| *kind == MEMBER_NODE && role == wanted)
            .map(|(identifier, _, _)| *identifier)
    };
    node_with("admin_centre").or_else(|| node_with("label"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect()
    }

    #[test]
    fn given_a_relation_that_is_not_a_boundary_when_read_then_it_is_not_an_area() {
        let read = administrative_area_of(&tags(&[("type", "multipolygon"), ("name", "Lagune")]));

        assert_eq!(read, None);
    }

    #[test]
    fn given_a_boundary_with_no_admin_level_when_read_then_it_is_not_an_area() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("name", "Nowhere"),
        ]));

        assert_eq!(read, None);
    }

    #[test]
    fn given_a_french_commune_when_read_then_its_level_name_and_insee_reference_are_kept() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Marigny-Le-Lozon"),
            ("ref:INSEE", "50292"),
            ("postal_code", "50570"),
        ]))
        .expect("a commune");

        assert_eq!(read.level, 8);
        assert_eq!(
            read.reference,
            Some(Reference {
                scheme: "ref:INSEE".into(),
                code: "50292".into()
            })
        );
        assert_eq!(read.postcodes, vec!["50570"]);
        assert!(read.names.contains(&"MARIGNY LOZON".to_string()));
    }

    /// The names a place has had matter as much as the one it has. France merged thousands of
    /// communes from 2015 and address files still carry the old names, so `MARIGNY` has to reach
    /// `Marigny-Le-Lozon` or the search answers from another département entirely.
    #[test]
    fn given_a_merged_commune_when_read_then_its_former_name_is_a_name_it_answers_to() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Marigny-Le-Lozon"),
            ("old_name", "Marigny"),
            ("alt_name", "Marigny le Lozon"),
        ]))
        .expect("a commune");

        assert!(read.names.contains(&"MARIGNY".to_string()));
        assert!(read.names.contains(&"MARIGNY LOZON".to_string()));
    }

    #[test]
    fn given_a_boundary_naming_several_postcodes_when_read_then_each_is_kept_separately() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Paris"),
            ("postal_code", "75001;75002;75003"),
        ]))
        .expect("a commune");

        assert_eq!(read.postcodes, vec!["75001", "75002", "75003"]);
    }

    #[test]
    fn given_a_boundary_with_no_name_at_all_when_read_then_it_is_not_an_area() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
        ]));

        assert_eq!(read, None);
    }

    #[test]
    fn given_repeated_spellings_of_one_name_when_read_then_the_duplicates_collapse() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Sens"),
            ("alt_name", "SENS"),
            ("name:fr", "Sens"),
        ]))
        .expect("a commune");

        assert_eq!(read.names, vec!["SENS".to_string()]);
    }

    #[test]
    fn given_an_admin_level_outside_the_range_we_index_when_read_then_it_is_skipped() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("name", "France"),
        ]));

        assert_eq!(read, None);
    }

    /// The bug this guards against is silent and crosses borders. An Irish electoral division
    /// carries a bare `ref` of `13055`, which is also Marseille's INSEE code, so a gazetteer keyed
    /// on the bare tag filed Marseille under an Irish name. 3 668 French codes were contested.
    #[test]
    fn given_a_bare_ref_when_read_then_the_scheme_travels_with_the_code() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "9"),
            ("name", "Ballykilmore ED"),
            ("ref", "13055"),
        ]))
        .expect("an electoral division");

        assert_eq!(
            read.reference,
            Some(Reference {
                scheme: "ref".into(),
                code: "13055".into()
            })
        );
    }

    #[test]
    fn given_an_insee_reference_when_read_then_its_scheme_names_insee() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Marseille"),
            ("ref:INSEE", "13055"),
        ]))
        .expect("a commune");

        assert_eq!(
            read.reference,
            Some(Reference {
                scheme: "ref:INSEE".into(),
                code: "13055".into()
            })
        );
    }

    #[test]
    fn given_both_a_bare_ref_and_an_insee_ref_when_read_then_insee_wins() {
        let read = administrative_area_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "8"),
            ("name", "Quelquepart"),
            ("ref", "99"),
            ("ref:INSEE", "50292"),
        ]))
        .expect("a commune");

        assert_eq!(read.reference.map(|r| r.scheme), Some("ref:INSEE".to_string()));
    }

    fn member(identifier: i64, kind: u64, role: &str) -> (i64, u64, String) {
        (identifier, kind, role.to_string())
    }

    /// The seat is the town itself; the label is only where a renderer chose to print its name.
    #[test]
    fn given_both_an_admin_centre_and_a_label_when_read_then_the_admin_centre_is_the_centre() {
        let members = [
            member(11, 0, "label"),
            member(22, 0, "admin_centre"),
            member(33, 1, "outer"),
        ];

        assert_eq!(centre_node_of(&members), Some(22));
    }

    #[test]
    fn given_only_a_label_node_when_read_then_the_label_is_the_centre() {
        let members = [member(33, 1, "outer"), member(11, 0, "label")];

        assert_eq!(centre_node_of(&members), Some(11));
    }

    #[test]
    fn given_a_way_carrying_the_admin_centre_role_when_read_then_it_is_not_taken_as_a_node() {
        let members = [member(33, 1, "admin_centre")];

        assert_eq!(centre_node_of(&members), None);
    }

    #[test]
    fn given_a_country_boundary_when_read_then_its_code_and_every_name_are_kept() {
        let read = country_of(&tags(&[
            ("type", "boundary"),
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("name", "France"),
            ("name:en", "France"),
            ("official_name", "République française"),
            ("ISO3166-1", "FR"),
        ]))
        .expect("a country");

        assert_eq!(read.code, "FR");
        assert!(read.names.contains(&"FRANCE".to_string()));
        assert!(read.names.contains(&"REPUBLIQUE FRANCAISE".to_string()));
    }

    #[test]
    fn given_a_boundary_below_country_level_when_read_as_a_country_then_it_is_not_one() {
        let read = country_of(&tags(&[
            ("boundary", "administrative"),
            ("admin_level", "4"),
            ("name", "Bretagne"),
            ("ISO3166-2", "FR-BRE"),
        ]));

        assert_eq!(read, None);
    }

    #[test]
    fn given_a_country_level_boundary_with_no_iso_code_when_read_then_it_is_not_a_country() {
        let read = country_of(&tags(&[
            ("boundary", "administrative"),
            ("admin_level", "2"),
            ("name", "Somewhere Disputed"),
        ]));

        assert_eq!(read, None);
    }
}
