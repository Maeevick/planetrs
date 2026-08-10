//! Filing every address, road and hamlet under the communes that contain it.
//!
//! Pure: the passes have read the planet, and this decides only which commune each point belongs
//! to and which street it is on. A door's street is its own `addr:street` when it has one, then the
//! `associatedStreet` relation naming it, then its `addr:place`, which is how a rural French address
//! with no street at all is written.

use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;

pub use crate::assemble::NodePositions;
pub use crate::filing::CommuneOutlines;
use crate::harvest::{AssociatedStreet, RawAddress, RawBuilding, RawHighway, RawPlace};
use crate::street_index::{AddressPoint, Street};
use crate::text::{FRENCH, HouseNumber, canonical_form, comparison_key, parse_house_number};

/// Everything the passes found that can become a street or a door.
pub struct StreetSources<'a> {
    pub addresses: &'a [RawAddress],
    pub places: &'a [RawPlace],
    pub highways: &'a [RawHighway],
    pub buildings: &'a [RawBuilding],
    pub associated: &'a [AssociatedStreet],
    pub positions: &'a NodePositions,
}

impl<'a> StreetSources<'a> {
    #[cfg(test)]
    pub(crate) fn empty(positions: &'a NodePositions) -> Self {
        Self {
            addresses: &[],
            places: &[],
            highways: &[],
            buildings: &[],
            associated: &[],
            positions,
        }
    }
}

/// What filing could and could not do, so a gap is reported rather than discovered.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FilingReport {
    pub addresses_filed: usize,
    pub addresses_without_a_street: usize,
    pub addresses_outside_every_commune: usize,
    pub addresses_with_an_unreadable_number: usize,
    pub buildings_without_nodes: usize,
    pub highways_filed: usize,
    pub places_filed: usize,
}

enum Door {
    Filed(Vec<(i64, Street)>),
    WithoutStreet,
    UnreadableNumber,
    Outside,
    NoPosition,
}

fn door_on_street(
    position: Option<[f64; 2]>,
    housenumber: &str,
    street: &str,
    related: Option<&String>,
    place: &str,
    communes: &CommuneOutlines,
) -> Door {
    let Some(position) = position else {
        return Door::NoPosition;
    };
    let Some(number): Option<HouseNumber> = parse_house_number(housenumber, &FRENCH) else {
        return Door::UnreadableNumber;
    };
    let named = [street, related.map(String::as_str).unwrap_or(""), place]
        .into_iter()
        .map(|candidate| comparison_key(candidate, &FRENCH))
        .find(|candidate| !candidate.is_empty());
    let Some(name) = named else {
        return Door::WithoutStreet;
    };
    let containing = communes.containing(position);
    if containing.is_empty() {
        return Door::Outside;
    }
    Door::Filed(
        containing
            .into_iter()
            .map(|commune| {
                let door = AddressPoint {
                    number: number.clone(),
                    position,
                };
                (
                    commune,
                    Street {
                        name: name.clone(),
                        addresses: vec![door],
                        path: vec![],
                    },
                )
            })
            .collect(),
    )
}

fn middle(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    let [x, y] = points.iter().fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
        [x + longitude, y + latitude]
    });
    Some([x / count, y / count])
}

/// Every street and door, filed under each commune that contains it.
pub fn file_streets(
    sources: &StreetSources<'_>,
    communes: &CommuneOutlines,
) -> (Vec<(i64, Street)>, FilingReport) {
    let mut named_by_relation: HashMap<i64, String> = HashMap::new();
    let mut named_way_by_relation: HashMap<i64, String> = HashMap::new();
    for relation in sources.associated {
        for node in &relation.house_nodes {
            named_by_relation
                .entry(*node)
                .or_insert_with(|| relation.name.clone());
        }
        for way in &relation.house_ways {
            named_way_by_relation
                .entry(*way)
                .or_insert_with(|| relation.name.clone());
        }
    }

    let mut report = FilingReport::default();
    let mut filed: Vec<(i64, Street)> = Vec::new();
    let tally = |door: Door, report: &mut FilingReport, filed: &mut Vec<(i64, Street)>| match door {
        Door::Filed(streets) => {
            report.addresses_filed += 1;
            filed.extend(streets);
        }
        Door::WithoutStreet => report.addresses_without_a_street += 1,
        Door::UnreadableNumber => report.addresses_with_an_unreadable_number += 1,
        Door::Outside => report.addresses_outside_every_commune += 1,
        Door::NoPosition => report.buildings_without_nodes += 1,
    };

    let node_doors: Vec<Door> = sources
        .addresses
        .par_iter()
        .map(|found| {
            door_on_street(
                Some(found.position),
                &found.housenumber,
                &found.street,
                named_by_relation.get(&found.node),
                &found.place,
                communes,
            )
        })
        .collect();
    let building_doors: Vec<Door> = sources
        .buildings
        .par_iter()
        .map(|found| {
            let points: Vec<[f64; 2]> = found
                .references
                .iter()
                .filter_map(|id| sources.positions.get(id).copied())
                .collect();
            door_on_street(
                middle(&points),
                &found.housenumber,
                &found.street,
                named_way_by_relation.get(&found.way),
                &found.place,
                communes,
            )
        })
        .collect();
    for door in node_doors.into_iter().chain(building_doors) {
        tally(door, &mut report, &mut filed);
    }

    let stretches: Vec<Vec<(i64, Street)>> = sources
        .highways
        .par_iter()
        .map(|road| {
            let mut names: Vec<String> = Vec::new();
            let name = comparison_key(&road.name, &FRENCH);
            if !name.is_empty() {
                names.push(name);
            }
            if let Some(read) = FRENCH.road_number {
                for number in &road.road_numbers {
                    if let Some(number) = read(&canonical_form(number, &FRENCH))
                        && !names.contains(&number)
                    {
                        names.push(number);
                    }
                }
            }
            let mut by_commune: BTreeMap<i64, Vec<[f64; 2]>> = BTreeMap::new();
            for point in road
                .references
                .iter()
                .filter_map(|id| sources.positions.get(id).copied())
            {
                for commune in communes.containing(point) {
                    by_commune.entry(commune).or_default().push(point);
                }
            }
            by_commune
                .into_iter()
                .flat_map(|(commune, path)| {
                    names
                        .iter()
                        .map(|name| {
                            (
                                commune,
                                Street {
                                    name: name.clone(),
                                    addresses: vec![],
                                    path: path.clone(),
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .collect();
    for stretch in stretches {
        if !stretch.is_empty() {
            report.highways_filed += 1;
        }
        filed.extend(stretch);
    }

    for place in sources.places {
        let name = comparison_key(&place.name, &FRENCH);
        let containing = communes.containing(place.position);
        if !containing.is_empty() && !name.is_empty() {
            report.places_filed += 1;
        }
        for commune in containing {
            filed.push((
                commune,
                Street {
                    name: name.clone(),
                    addresses: vec![],
                    path: vec![place.position],
                },
            ));
        }
    }

    (filed, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harvest::{AssociatedStreet, RawAddress, RawBuilding, RawHighway, RawPlace};
    use crate::rings::Outline;
    use crate::street_index::StreetIndex;

    fn square(west: f64, south: f64, side: f64) -> Outline {
        Outline::of(vec![vec![
            [west, south],
            [west + side, south],
            [west + side, south + side],
            [west, south + side],
            [west, south],
        ]])
    }

    fn two_communes() -> CommuneOutlines {
        CommuneOutlines::of(vec![(1, square(0.0, 0.0, 1.0)), (2, square(1.0, 0.0, 1.0))])
    }

    fn address(node: i64, x: f64, y: f64, number: &str, street: &str) -> RawAddress {
        RawAddress {
            node,
            position: [x, y],
            housenumber: number.into(),
            street: street.into(),
            place: String::new(),
        }
    }

    fn filed(sources: &StreetSources<'_>) -> StreetIndex {
        StreetIndex::of(file_streets(sources, &two_communes()).0)
    }

    fn empty_positions() -> NodePositions {
        NodePositions::new()
    }

    #[test]
    fn given_an_address_node_with_its_street_when_filed_then_it_is_a_door_of_that_street_in_its_commune() {
        let addresses = [address(10, 0.5, 0.5, "8", "Route des Tilleuls")];
        let positions = empty_positions();
        let sources = StreetSources {
            addresses: &addresses,
            positions: &positions,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);

        assert_eq!(index.streets_of(1)[0].name, "ROUTE TILLEULS");
        assert_eq!(index.streets_of(1)[0].addresses[0].number.number, 8);
    }

    #[test]
    fn given_an_address_whose_street_is_only_on_an_associated_street_relation_then_the_relation_names_it() {
        let addresses = [address(10, 0.5, 0.5, "12", "")];
        let associated = [AssociatedStreet {
            name: "Rue des Lilas".into(),
            house_nodes: vec![10],
            house_ways: vec![],
        }];
        let positions = empty_positions();
        let sources = StreetSources {
            addresses: &addresses,
            associated: &associated,
            ..StreetSources::empty(&positions)
        };

        assert_eq!(filed(&sources).streets_of(1)[0].name, "RUE LILAS");
    }

    #[test]
    fn given_an_address_with_only_a_place_when_filed_then_the_place_stands_for_the_street() {
        let addresses = [RawAddress {
            node: 10,
            position: [0.5, 0.5],
            housenumber: "3".into(),
            street: String::new(),
            place: "La Charmeraie".into(),
        }];
        let positions = empty_positions();
        let sources = StreetSources {
            addresses: &addresses,
            ..StreetSources::empty(&positions)
        };

        assert_eq!(filed(&sources).streets_of(1)[0].name, "CHARMERAIE");
    }

    #[test]
    fn given_a_building_carrying_a_number_when_filed_then_its_door_is_the_middle_of_its_nodes() {
        let buildings = [RawBuilding {
            way: 50,
            housenumber: "4".into(),
            street: "Rue Diderot".into(),
            place: String::new(),
            references: vec![1, 2],
        }];
        let positions = NodePositions::from([(1, [0.2, 0.2]), (2, [0.4, 0.4])]);
        let sources = StreetSources {
            buildings: &buildings,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);
        let door = &index.streets_of(1)[0].addresses[0];
        assert!((door.position[0] - 0.3).abs() < 1e-9 && (door.position[1] - 0.3).abs() < 1e-9);
    }

    /// A road running through two communes belongs to both, and each keeps only its own stretch, so
    /// its middle in one town is not dragged into the next.
    #[test]
    fn given_a_road_crossing_two_communes_when_filed_then_each_keeps_its_own_stretch() {
        let highways = [RawHighway {
            name: "Route des Tilleuls".into(),
            references: vec![1, 2, 3, 4],
            road_numbers: vec![],
        }];
        let positions =
            NodePositions::from([(1, [0.2, 0.5]), (2, [0.4, 0.5]), (3, [1.6, 0.5]), (4, [1.8, 0.5])]);
        let sources = StreetSources {
            highways: &highways,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);

        assert_eq!(index.streets_of(1)[0].path, vec![[0.2, 0.5], [0.4, 0.5]]);
        assert_eq!(index.streets_of(2)[0].path, vec![[1.6, 0.5], [1.8, 0.5]]);
    }

    /// Step eighteen: a road's number is filed as a name of its own, in one canonical form, so that
    /// `RD 620` in a site file finds the road OpenStreetMap tags `ref=D 620`.
    #[test]
    fn given_a_road_with_a_number_when_filed_then_the_number_is_one_of_its_names() {
        let highways = [RawHighway {
            name: "Route de Caen".into(),
            references: vec![1, 2],
            road_numbers: vec!["D 620".into(), "D 513".into()],
        }];
        let positions = NodePositions::from([(1, [0.2, 0.5]), (2, [0.4, 0.5])]);
        let sources = StreetSources {
            highways: &highways,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);

        let names: Vec<&str> = index
            .streets_of(1)
            .iter()
            .map(|street| street.name.as_str())
            .collect();
        assert_eq!(names, vec!["D513", "D620", "ROUTE CAEN"]);
    }

    #[test]
    fn given_a_numbered_road_without_a_name_when_filed_then_its_number_names_it() {
        let highways = [RawHighway {
            name: String::new(),
            references: vec![1, 2],
            road_numbers: vec!["N 7".into()],
        }];
        let positions = NodePositions::from([(1, [0.2, 0.5]), (2, [0.4, 0.5])]);
        let sources = StreetSources {
            highways: &highways,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);

        let names: Vec<&str> = index
            .streets_of(1)
            .iter()
            .map(|street| street.name.as_str())
            .collect();
        assert_eq!(names, vec!["N7"]);
    }

    #[test]
    fn given_a_named_hamlet_when_filed_then_it_is_a_street_of_one_point() {
        let places = [RawPlace {
            position: [1.5, 0.5],
            name: "Les Bruyères".into(),
        }];
        let positions = empty_positions();
        let sources = StreetSources {
            places: &places,
            ..StreetSources::empty(&positions)
        };

        let index = filed(&sources);

        assert_eq!(index.streets_of(2)[0].name, "BRUYERES");
        assert_eq!(index.streets_of(2)[0].path, vec![[1.5, 0.5]]);
    }

    #[test]
    fn given_an_address_outside_every_commune_when_filed_then_it_is_counted_not_dropped_silently() {
        let addresses = [address(10, 9.0, 9.0, "1", "Rue Loin")];
        let positions = empty_positions();
        let sources = StreetSources {
            addresses: &addresses,
            ..StreetSources::empty(&positions)
        };

        let (_, report) = file_streets(&sources, &two_communes());

        assert_eq!(report.addresses_outside_every_commune, 1);
    }

    #[test]
    fn given_an_address_with_no_street_at_all_when_filed_then_it_is_counted_as_unnamed() {
        let addresses = [address(10, 0.5, 0.5, "1", "")];
        let positions = empty_positions();
        let sources = StreetSources {
            addresses: &addresses,
            ..StreetSources::empty(&positions)
        };

        let (_, report) = file_streets(&sources, &two_communes());

        assert_eq!(report.addresses_without_a_street, 1);
    }
}
