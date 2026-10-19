//! The streets of every commune, with their house numbers, and placing a site on one of them.
//!
//! A street is filed under every commune whose outline contains it, and searched only within the
//! communes the previous step settled on. That is what makes the fuzzy tiers of
//! [`crate::street::best_street_match`] safe: against the few hundred streets of one town they
//! forgive a typo, against a whole country they would join two towns' `Rue de la Gare`.

use std::collections::HashMap;

use crate::loose::unique_street_match;
use crate::rarity::WordRarity;
use crate::street::{Quality, StreetMatch, best_street_match};
use crate::text::{HouseNumber, TextProfile, comparison_key};

/// One door: the number it carries and where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct AddressPoint {
    /// The number on the door, with its suffix.
    pub number: HouseNumber,
    /// Longitude and latitude.
    pub position: [f64; 2],
}

/// One street of one commune: its name as a comparison key, its doors, and the points of the road
/// itself where OpenStreetMap draws one.
#[derive(Debug, Clone, PartialEq)]
pub struct Street {
    /// The name as a comparison key, `ROUTE TILLEULS`.
    pub name: String,
    /// The doors recorded on it.
    pub addresses: Vec<AddressPoint>,
    /// Longitude and latitude of points along the road. A written index keeps only their middle.
    pub path: Vec<[f64; 2]>,
}

impl Street {
    /// Where the street is, when no door on it answers.
    ///
    /// Its doors if it has any, because that is where the buildings are; the road itself otherwise.
    /// A long road through a large commune can run kilometres past its last house.
    fn middle(&self) -> Option<[f64; 2]> {
        let points: Vec<[f64; 2]> = if self.addresses.is_empty() {
            self.path.clone()
        } else {
            self.addresses.iter().map(|door| door.position).collect()
        };
        mean(&points)
    }
}

fn mean(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    let [longitude, latitude] = points.iter().fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
        [x + longitude, y + latitude]
    });
    Some([longitude / count, latitude / count])
}

/// How far down the street a site could be placed, closest first, so that comparing two placements
/// says which is nearer the building.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placed {
    HouseNumber,
    /// The asked door is not recorded, so the door whose number is nearest to it answers.
    NearestHouseNumber,
    Street,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnStreet {
    /// Longitude and latitude.
    pub position: [f64; 2],
    pub placed: Placed,
    /// The street's name as recorded, which may differ from the one asked for.
    pub street: String,
    pub quality: Quality,
}

/// Every commune's streets, keyed by the commune's relation, and how rare each word of their names is.
#[derive(Debug, Default)]
pub struct StreetIndex {
    by_commune: HashMap<i64, Vec<Street>>,
    rarity: WordRarity,
}

impl StreetIndex {
    /// Streets filed under communes. The same name filed twice under one commune, from a road and
    /// from its addresses, is merged into one street.
    pub fn of(filed: Vec<(i64, Street)>) -> Self {
        let mut by_commune: HashMap<i64, HashMap<String, Street>> = HashMap::new();
        for (commune, street) in filed {
            let streets = by_commune.entry(commune).or_default();
            match streets.get_mut(&street.name) {
                Some(known) => {
                    known.addresses.extend(street.addresses);
                    known.path.extend(street.path);
                }
                None => {
                    streets.insert(street.name.clone(), street);
                }
            }
        }
        let by_commune: HashMap<i64, Vec<Street>> = by_commune
            .into_iter()
            .map(|(commune, streets)| {
                let mut streets: Vec<Street> = streets.into_values().collect();
                streets.sort_by(|left, right| left.name.cmp(&right.name));
                (commune, streets)
            })
            .collect();
        let rarity = WordRarity::of(by_commune.values().flatten().map(|street| street.name.as_str()));
        Self { by_commune, rarity }
    }

    /// The same streets weighed against another count of words: a country's own, when only some of
    /// its communes are held, or the millions of names of a real index in a test.
    pub(crate) fn with_rarity(self, rarity: WordRarity) -> Self {
        Self { rarity, ..self }
    }

    /// A copy holding only the given communes, weighed against the same words.
    pub(crate) fn only(&self, communes: impl IntoIterator<Item = i64>) -> StreetIndex {
        Self {
            by_commune: communes
                .into_iter()
                .filter_map(|commune| {
                    self.by_commune
                        .get(&commune)
                        .map(|streets| (commune, streets.clone()))
                })
                .collect(),
            rarity: self.rarity.clone(),
        }
    }

    /// The same index holding the communes of `other` too, still weighed against its own words.
    pub(crate) fn absorb(&mut self, other: StreetIndex) {
        self.by_commune.extend(other.by_commune);
    }

    pub(crate) fn rarity(&self) -> &WordRarity {
        &self.rarity
    }

    /// The streets of a commune, by its relation, sorted by name.
    pub fn streets_of(&self, commune: i64) -> &[Street] {
        self.by_commune.get(&commune).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every commune that has streets, with them, in no particular order.
    pub fn communes(&self) -> impl Iterator<Item = (&i64, &Vec<Street>)> {
        self.by_commune.iter()
    }
}

/// Where the door nearest in number to any of the asked ones is, when none of them is recorded.
///
/// The asked door's own side of the street is searched first, odd numbers among odd and even among
/// even, and the other side only when it has no door or its door is much nearer in number. The two sides of a French street
/// are numbered separately and need not line up: on Faubourg Saint-Honoré doors 203 and 205 stand
/// 250 m from 204, and 202 stands 20 m from it.
///
/// Numbering runs along the street, so the nearest number is the nearest recorded door in all but
/// the oddest streets. Two doors equally near, 6 and 10 for an asked 8, stand either side of it, and
/// the point between them is a better guess than either.
fn nearest_door(doors: &[&AddressPoint], asked: &[u32]) -> Option<[f64; 2]> {
    let either_side = nearest_among(doors, asked, false)?;
    match nearest_among(doors, asked, true) {
        Some(own_side) if own_side.0 <= either_side.0 + CROSSING_THE_STREET => Some(own_side.1),
        _ => Some(either_side.1),
    }
}

/// How many numbers nearer the other side's door must be before the street is crossed for it.
///
/// Measured rather than chosen: replayed over sites answered by a nearest door, staying on the asked
/// side unless the other side is more than two numbers nearer kept every site as close to reference
/// doors at a kilometre as crossing freely did, and brought more of them within 100 m. Never crossing
/// cost some at a kilometre: a side can jump from 115 to 2587.
const CROSSING_THE_STREET: u32 = 2;

/// The recorded doors nearest in number to any asked one, averaged when several tie, searched on
/// the asked door's own side of the street when `same_side` holds, with how far off in number they
/// are.
fn nearest_among(doors: &[&AddressPoint], asked: &[u32], same_side: bool) -> Option<(u32, [f64; 2])> {
    let gap_of = |door: &&AddressPoint| {
        asked
            .iter()
            .filter(|number| !same_side || *number % 2 == door.number.number % 2)
            .map(|number| door.number.number.abs_diff(*number))
            .min()
    };
    let gap = doors.iter().filter_map(gap_of).min()?;
    let nearest: Vec<[f64; 2]> = doors
        .iter()
        .filter(|door| gap_of(door) == Some(gap))
        .map(|door| door.position)
        .collect();
    mean(&nearest).map(|position| (gap, position))
}

/// How far a street name may be from the asked one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The tiers of [`crate::street::best_street_match`], exact down to street type.
    Tiers,
    /// Step fifteen's rules of [`crate::loose::unique_street_match`], which answer only with the one
    /// street of the communes that fits, for when the tiers found nothing in any part of the address.
    UniqueStreet,
}

/// Step three: the door on the street, or the street itself, within the communes already settled.
///
/// A shop written `5/7` or `130-134` asks for every one of its doors: any that is recorded answers,
/// all of them together when several are, and failing that the recorded door nearest to any of them.
#[cfg(test)]
pub(crate) fn locate_on_street(
    index: &StreetIndex,
    communes: &[i64],
    asked_street: &str,
    asked_numbers: &[HouseNumber],
    profile: &TextProfile,
) -> Option<OnStreet> {
    locate_on_street_reaching(
        index,
        communes,
        asked_street,
        asked_numbers,
        profile,
        Reach::Tiers,
    )
}

/// [`locate_on_street`], matching the street as far as `reach` allows.
pub fn locate_on_street_reaching(
    index: &StreetIndex,
    communes: &[i64],
    asked_street: &str,
    asked_numbers: &[HouseNumber],
    profile: &TextProfile,
    reach: Reach,
) -> Option<OnStreet> {
    let asked = comparison_key(asked_street, profile);
    let streets: Vec<&Street> = communes
        .iter()
        .flat_map(|commune| index.streets_of(*commune))
        .collect();
    let names: Vec<&str> = streets.iter().map(|street| street.name.as_str()).collect();
    let matched: StreetMatch = match reach {
        Reach::Tiers => best_street_match(&asked, names.iter().copied(), profile)?,
        Reach::UniqueStreet => unique_street_match(&asked, &names, profile, &index.rarity)?,
    };
    let on_it: Vec<&Street> = streets
        .into_iter()
        .filter(|street| street.name == matched.name)
        .collect();

    let doors: Vec<&AddressPoint> = on_it.iter().flat_map(|street| street.addresses.iter()).collect();
    if !asked_numbers.is_empty() && !doors.is_empty() {
        let first_door_where = |matches: &dyn Fn(&HouseNumber, &AddressPoint) -> bool| -> Vec<[f64; 2]> {
            asked_numbers
                .iter()
                .filter_map(|asked| {
                    doors
                        .iter()
                        .find(|door| matches(asked, door))
                        .map(|door| door.position)
                })
                .collect()
        };
        let exact = first_door_where(&|asked, door| door.number == *asked);
        let same_number = first_door_where(&|asked, door| door.number.number == asked.number);
        if let Some(position) = mean(&exact).or_else(|| mean(&same_number)) {
            return Some(OnStreet {
                position,
                placed: Placed::HouseNumber,
                street: matched.name.clone(),
                quality: matched.quality,
            });
        }
        let asked: Vec<u32> = asked_numbers.iter().map(|asked| asked.number).collect();
        if let Some(position) = nearest_door(&doors, &asked) {
            return Some(OnStreet {
                position,
                placed: Placed::NearestHouseNumber,
                street: matched.name.clone(),
                quality: matched.quality,
            });
        }
    }

    let middles: Vec<[f64; 2]> = on_it.iter().filter_map(|street| street.middle()).collect();
    mean(&middles).map(|position| OnStreet {
        position,
        placed: Placed::Street,
        street: matched.name.clone(),
        quality: matched.quality,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::HouseNumber;

    fn number(value: u32, suffix: &str) -> HouseNumber {
        HouseNumber {
            number: value,
            suffix: suffix.into(),
        }
    }

    fn door(value: u32, suffix: &str, longitude: f64, latitude: f64) -> AddressPoint {
        AddressPoint {
            number: number(value, suffix),
            position: [longitude, latitude],
        }
    }

    fn sens_index() -> StreetIndex {
        StreetIndex::of(vec![
            (
                1,
                Street {
                    name: "ROUTE TILLEULS".into(),
                    addresses: vec![
                        door(8, "", 3.2652, 48.2000),
                        door(8, "BIS", 3.2653, 48.2002),
                        door(10, "", 3.2655, 48.2004),
                    ],
                    path: vec![[3.26, 48.19], [3.27, 48.21]],
                },
            ),
            (
                1,
                Street {
                    name: "RUE GAMBETTA".into(),
                    addresses: vec![],
                    path: vec![[3.28, 48.19], [3.30, 48.21]],
                },
            ),
            (
                2,
                Street {
                    name: "RUE PARON".into(),
                    addresses: vec![door(3, "", 3.25, 48.17)],
                    path: vec![],
                },
            ),
        ])
    }

    fn placed(communes: &[i64], street: &str, asked: Option<HouseNumber>) -> Option<OnStreet> {
        let asked: Vec<HouseNumber> = asked.into_iter().collect();
        locate_on_street(&sens_index(), communes, street, &asked, &crate::text::FRENCH)
    }

    #[test]
    fn given_the_number_on_the_street_when_placed_then_the_door_is_the_answer() {
        let found = placed(&[1], "ROUTE TILLEULS", Some(number(8, ""))).expect("the door");

        assert_eq!(found.position, [3.2652, 48.2000]);
        assert_eq!(found.placed, Placed::HouseNumber);
    }

    #[test]
    fn given_a_number_with_a_suffix_when_placed_then_the_suffixed_door_is_preferred() {
        let found = placed(&[1], "ROUTE TILLEULS", Some(number(8, "BIS"))).expect("the door");

        assert_eq!(found.position, [3.2653, 48.2002]);
    }

    #[test]
    fn given_a_suffix_the_street_does_not_have_when_placed_then_the_same_number_answers() {
        let found = placed(&[1], "ROUTE TILLEULS", Some(number(10, "TER"))).expect("the door");

        assert_eq!(found.position, [3.2655, 48.2004]);
        assert_eq!(found.placed, Placed::HouseNumber);
    }

    /// 5-7 Rue Gambetta in Cherbourg is not in OpenStreetMap, but door 2 is, across the street. A
    /// neighbouring door is metres away where the street's middle can be kilometres away.
    #[test]
    fn given_a_number_the_street_does_not_have_when_placed_then_the_nearest_number_answers() {
        let found = placed(&[1], "ROUTE TILLEULS", Some(number(44, ""))).expect("a door");

        assert_eq!(found.position, [3.2655, 48.2004]);
        assert_eq!(found.placed, Placed::NearestHouseNumber);
    }

    #[test]
    fn given_two_doors_equally_near_the_asked_number_when_placed_then_the_answer_is_between_them() {
        let index = StreetIndex::of(vec![(
            1,
            Street {
                name: "RUE LILAS".into(),
                addresses: vec![door(6, "", 1.0, 45.0), door(10, "", 1.2, 45.2)],
                path: vec![],
            },
        )]);

        let found = locate_on_street(&index, &[1], "RUE LILAS", &[number(8, "")], &crate::text::FRENCH)
            .expect("a door");

        assert!((found.position[0] - 1.1).abs() < 1e-9 && (found.position[1] - 45.1).abs() < 1e-9);
    }

    #[test]
    fn given_a_street_with_no_doors_at_all_when_a_number_is_asked_then_the_street_average_answers() {
        let found = placed(&[1], "RUE GAMBETTA", Some(number(4, ""))).expect("the street");

        assert_eq!(found.placed, Placed::Street);
    }

    #[test]
    fn given_a_street_with_no_addresses_when_placed_then_its_own_path_is_averaged() {
        let found = placed(&[1], "RUE GAMBETTA", None).expect("the street");

        assert_eq!(found.position, [3.29, 48.20]);
    }

    #[test]
    fn given_a_street_absent_from_the_commune_when_placed_then_nothing_is_returned() {
        assert_eq!(placed(&[1], "RUE PARON", Some(number(3, ""))), None);
    }

    /// Four communes are called Marigny and only one of them has the street asked for. A matched
    /// street is stronger evidence of which town is meant than the name of the town.
    #[test]
    fn given_several_candidate_communes_when_only_one_has_the_street_then_it_settles_the_town() {
        let found = placed(&[1, 2], "RUE PARON", Some(number(3, ""))).expect("the door");

        assert_eq!(found.position, [3.25, 48.17]);
    }

    #[test]
    fn given_an_abbreviated_street_when_placed_then_the_matching_cascade_still_finds_it() {
        let found = placed(&[1], "RTE TILLEULS", Some(number(8, ""))).expect("the door");

        assert_eq!(found.placed, Placed::HouseNumber);
    }

    fn gambetta(doors: Vec<AddressPoint>) -> StreetIndex {
        StreetIndex::of(vec![(
            1,
            Street {
                name: "RUE GAMBETTA".into(),
                addresses: doors,
                path: vec![],
            },
        )])
    }

    fn at_doors(index: &StreetIndex, asked: &[u32]) -> Option<OnStreet> {
        let asked: Vec<HouseNumber> = asked.iter().map(|value| number(*value, "")).collect();
        locate_on_street(index, &[1], "RUE GAMBETTA", &asked, &crate::text::FRENCH)
    }

    /// `5/7` is one shop and either door may be the one recorded.
    #[test]
    fn given_several_asked_doors_when_only_the_first_is_recorded_then_it_answers() {
        let index = gambetta(vec![door(5, "", 1.0, 1.0), door(9, "", 3.0, 3.0)]);

        let found = at_doors(&index, &[5, 7]).expect("a door");

        assert_eq!((found.position, found.placed), ([1.0, 1.0], Placed::HouseNumber));
    }

    #[test]
    fn given_several_asked_doors_all_recorded_when_placed_then_the_answer_is_between_them() {
        let index = gambetta(vec![door(5, "", 1.0, 1.0), door(7, "", 1.2, 1.2)]);

        let found = at_doors(&index, &[5, 7]).expect("a door");

        assert!((found.position[0] - 1.1).abs() < 1e-9);
    }

    /// Crossing the street is right when the other side is much nearer in number. On Route d'Uzès the
    /// asked 115's own side jumps to 2587 while 114 stands opposite, and staying on the side put the
    /// answer 2.8 km away.
    #[test]
    fn given_the_other_side_is_much_nearer_in_number_when_placed_then_the_street_is_crossed() {
        let index = gambetta(vec![door(114, "", 1.0, 1.0), door(2587, "", 3.0, 3.0)]);

        let found = at_doors(&index, &[115]).expect("a door");

        assert_eq!(
            (found.position, found.placed),
            ([1.0, 1.0], Placed::NearestHouseNumber)
        );
    }

    #[test]
    fn given_none_of_the_asked_doors_recorded_when_one_side_is_much_nearer_then_that_side_answers() {
        let index = gambetta(vec![door(2, "", 1.0, 1.0), door(27, "", 3.0, 3.0)]);

        let found = at_doors(&index, &[5, 7]).expect("a door");

        assert_eq!(found.position, [1.0, 1.0]);
    }

    #[test]
    fn given_a_nearer_door_across_the_street_when_placed_then_the_same_side_still_wins() {
        let index = gambetta(vec![
            door(202, "", 2.3053, 48.8751),
            door(203, "", 2.3022, 48.8763),
            door(205, "", 2.3018, 48.8764),
        ]);

        let found = at_doors(&index, &[204]).expect("a door");

        assert_eq!(found.position, [2.3053, 48.8751]);
    }

    #[test]
    fn given_no_door_on_the_asked_side_when_placed_then_the_other_side_answers() {
        let index = gambetta(vec![door(203, "", 1.0, 1.0), door(205, "", 1.2, 1.2)]);

        let found = at_doors(&index, &[204]).expect("a door");

        assert!((found.position[0] - 1.1).abs() < 1e-9);
    }

    /// The same number recorded twice, a node and a building or two streets of one name in a merged
    /// commune, is not two doors of one shop. Averaging them once moved a single-door site 1.6 km, so
    /// one recorded door answers for each asked number, as before multiple doors were read.
    #[test]
    fn given_one_asked_door_recorded_twice_far_apart_when_placed_then_they_are_not_averaged() {
        let index = gambetta(vec![door(2, "", 1.0, 1.0), door(2, "", 3.0, 3.0)]);

        let found = at_doors(&index, &[2]).expect("a door");

        assert_eq!(found.position, [1.0, 1.0]);
    }
}
