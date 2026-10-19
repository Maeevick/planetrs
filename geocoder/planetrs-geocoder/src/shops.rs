//! The shops of every commune, and finding the one a site's label names.
//!
//! A label is the name over the door, `ACME CITY`, and OpenStreetMap maps chains as `shop=*`
//! with a `name` and a `brand`. A brand repeats, though, and a wrong shop looks exactly as precise as
//! the right one, so a label answers only when nothing else about the site is in doubt: the shop is
//! on the street asked for, or it is the only one of that label in the commune.

use std::collections::HashMap;

use crate::gazetteer::contains_words_in_order;
use crate::street::is_same_street;
use crate::text::TextProfile;
pub use crate::text::comparison_key;

/// Copies of one shop, a point and a building, or one shop filed under a merged commune and its
/// former town, fall within this distance of each other.
const SAME_SHOP_METRES: f64 = 150.0;

/// Step nineteen: shops of one label all within this distance of their middle put the site in the
/// same kilometre-wide grid cell whichever of them it is.
const ONE_PIXEL_METRES: f64 = 1000.0;

/// Where the shop a label names stands, and, when it was the middle of several shops rather than
/// copies of one, how many.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShopFound {
    /// Longitude and latitude.
    pub position: [f64; 2],
    pub averaged: Option<usize>,
}

/// One shop: every name it answers to and the street it states, as comparison keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Shop {
    /// Its `name` and `brand`, as comparison keys.
    pub names: Vec<String>,
    /// Its own `addr:street`, empty when it states none.
    pub street: String,
    /// Longitude and latitude.
    pub position: [f64; 2],
}

/// Every commune's shops, keyed by the commune's relation.
#[derive(Debug, Default)]
pub struct ShopIndex {
    by_commune: HashMap<i64, Vec<Shop>>,
}

impl ShopIndex {
    /// Shops filed under the relation of the commune each stands in.
    pub fn of(filed: Vec<(i64, Shop)>) -> Self {
        let mut by_commune: HashMap<i64, Vec<Shop>> = HashMap::new();
        for (commune, shop) in filed {
            by_commune.entry(commune).or_default().push(shop);
        }
        Self { by_commune }
    }

    /// The same index holding the communes of `other` too.
    pub(crate) fn absorb(&mut self, other: ShopIndex) {
        self.by_commune.extend(other.by_commune);
    }

    /// The shops of a commune, by its relation.
    pub fn shops_of(&self, commune: i64) -> &[Shop] {
        self.by_commune.get(&commune).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every commune that has shops, with them, in no particular order.
    pub fn communes(&self) -> impl Iterator<Item = (&i64, &Vec<Shop>)> {
        self.by_commune.iter()
    }
}

/// Whether a shop carries the label: the same name, or a name adding its place to the brand, as
/// `ACME CITY GAMBETTA` does to `ACME CITY`.
fn carries(shop: &Shop, label: &str) -> bool {
    shop.names
        .iter()
        .any(|name| name == label || contains_words_in_order(name, label))
}

pub(crate) fn metres_between(a: [f64; 2], b: [f64; 2]) -> f64 {
    let ([lon1, lat1], [lon2, lat2]) = (a, b);
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2)
        + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().min(1.0).asin()
}

/// The one shop the candidates are copies of; or, when they are several shops all within a
/// kilometre of their middle, that middle; or nothing when they are spread wider.
fn one_shop(candidates: &[&Shop]) -> Option<ShopFound> {
    let first = candidates.first()?.position;
    let count = candidates.len() as f64;
    let [x, y] = candidates.iter().fold([0.0, 0.0], |[x, y], shop| {
        [x + shop.position[0], y + shop.position[1]]
    });
    let middle = [x / count, y / count];
    let copies = candidates
        .iter()
        .all(|shop| metres_between(first, shop.position) <= SAME_SHOP_METRES);
    if copies {
        return Some(ShopFound {
            position: middle,
            averaged: None,
        });
    }
    candidates
        .iter()
        .all(|shop| metres_between(middle, shop.position) <= ONE_PIXEL_METRES)
        .then_some(ShopFound {
            position: middle,
            averaged: Some(candidates.len()),
        })
}

fn labelled<'a>(index: &'a ShopIndex, communes: &[i64], label: &str, profile: &TextProfile) -> Vec<&'a Shop> {
    let label = comparison_key(label, profile);
    if label.is_empty() {
        return Vec::new();
    }
    communes
        .iter()
        .flat_map(|commune| index.shops_of(*commune))
        .filter(|shop| carries(shop, &label))
        .collect()
}

/// The shop of that label standing on the asked street, by its own `addr:street`.
///
/// Same street means spelt, spaced or typed differently at most. Sharing words is not enough:
/// Faubourg Saint-Jacques shares three of five with Faubourg Saint-Vincent, across the city.
pub fn shop_on_street(
    index: &ShopIndex,
    communes: &[i64],
    label: &str,
    asked_street: &str,
    profile: &TextProfile,
) -> Option<ShopFound> {
    let asked = comparison_key(asked_street, profile);
    if asked.is_empty() {
        return None;
    }
    let on_it: Vec<&Shop> = labelled(index, communes, label, profile)
        .into_iter()
        .filter(|shop| !shop.street.is_empty() && is_same_street(&asked, &shop.street, profile))
        .collect();
    one_shop(&on_it)
}

/// The shop of that label, when the communes hold exactly one, or the middle of those they hold when
/// all stand within a kilometre of it.
pub fn only_shop_in_communes(
    index: &ShopIndex,
    communes: &[i64],
    label: &str,
    profile: &TextProfile,
) -> Option<ShopFound> {
    one_shop(&labelled(index, communes, label, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FRENCH;

    fn shop(names: &[&str], street: &str, position: [f64; 2]) -> Shop {
        Shop {
            names: names.iter().map(|name| comparison_key(name, &FRENCH)).collect(),
            street: comparison_key(street, &FRENCH),
            position,
        }
    }

    fn index() -> ShopIndex {
        ShopIndex::of(vec![
            (1, shop(&["Acme City", "Acme City"], "Rue Gambetta", [1.0, 1.0])),
            (1, shop(&["Acme Market"], "Route de Paris", [2.0, 2.0])),
            (2, shop(&["Bazar Shop"], "", [5.0, 5.0])),
            (3, shop(&["Primeur"], "", [7.0, 7.0])),
            (3, shop(&["Primeur Gare"], "", [8.0, 8.0])),
        ])
    }

    #[test]
    fn given_a_label_and_a_shop_on_the_asked_street_then_the_shop_answers() {
        let found = shop_on_street(&index(), &[1], "ACME CITY", "RUE GAMBETTA", &FRENCH);

        assert_eq!(found.map(|found| found.position), Some([1.0, 1.0]));
    }

    #[test]
    fn given_a_shop_of_that_label_on_another_street_then_it_does_not_answer_for_this_one() {
        assert_eq!(
            shop_on_street(&index(), &[1], "ACME MARKET", "RUE GAMBETTA", &FRENCH),
            None
        );
    }

    #[test]
    fn given_a_label_naming_the_brand_and_a_shop_adding_its_place_then_they_match() {
        let found = shop_on_street(&index(), &[1], "ACME", "ROUTE PARIS", &FRENCH);

        assert_eq!(found.map(|found| found.position), Some([2.0, 2.0]));
    }

    #[test]
    fn given_the_only_shop_of_that_label_in_the_commune_then_it_answers() {
        assert_eq!(
            only_shop_in_communes(&index(), &[2], "BAZAR SHOP", &FRENCH).map(|found| found.position),
            Some([5.0, 5.0])
        );
    }

    /// Two shops of one brand in one town is a coin toss, and a wrong shop looks as precise as a
    /// right one.
    #[test]
    fn given_two_shops_of_that_label_in_the_commune_then_neither_answers() {
        assert_eq!(only_shop_in_communes(&index(), &[3], "PRIMEUR", &FRENCH), None);
    }

    /// Step nineteen: a supermarket mapped twice, 207 m apart. Two shops of one label
    /// within a kilometre of their middle put the site in the same grid cell whichever it is, so
    /// their middle answers, and says how many it stands for.
    #[test]
    fn given_shops_of_the_label_within_a_kilometre_of_their_middle_then_the_middle_answers() {
        let index = ShopIndex::of(vec![
            (
                4,
                shop(&["Acme Market"], "Boulevard des Mouettes", [1.000000, 49.279449]),
            ),
            (4, shop(&["Acme Market Plage"], "", [1.001581, 49.277898])),
        ]);

        let found = only_shop_in_communes(&index, &[4], "ACME MARKET", &FRENCH).expect("the middle");

        assert!(
            (found.position[0] - 1.0007905).abs() < 1e-6 && (found.position[1] - 49.2786735).abs() < 1e-6
        );
        assert_eq!(found.averaged, Some(2));
    }

    #[test]
    fn given_shops_of_the_label_further_apart_than_a_kilometre_then_neither_answers() {
        let index = ShopIndex::of(vec![
            (4, shop(&["Acme Market"], "", [1.000, 49.279])),
            (4, shop(&["Acme Market"], "", [1.030, 49.279])),
        ]);

        assert_eq!(only_shop_in_communes(&index, &[4], "ACME MARKET", &FRENCH), None);
    }

    #[test]
    fn given_a_label_no_shop_carries_then_nothing_answers() {
        assert_eq!(only_shop_in_communes(&index(), &[2], "VERGER", &FRENCH), None);
    }

    #[test]
    fn given_no_label_at_all_then_nothing_answers() {
        assert_eq!(only_shop_in_communes(&index(), &[2], "", &FRENCH), None);
    }

    /// A shop is often mapped twice, as a point and as the outline of its building, and filed under a
    /// merged commune and its former town alike. Those copies are one shop, not two.
    #[test]
    fn given_one_shop_mapped_twice_a_few_metres_apart_then_it_still_counts_as_one() {
        let index = ShopIndex::of(vec![
            (4, shop(&["Fournil"], "", [3.00000, 45.00000])),
            (4, shop(&["Fournil"], "", [3.00030, 45.00020])),
        ]);

        let found = only_shop_in_communes(&index, &[4], "FOURNIL", &FRENCH).expect("one shop");

        assert!((found.position[0] - 3.00015).abs() < 1e-9);
    }

    /// Rue du Faubourg Saint-Jacques shares three of its five words with Rue du Faubourg
    /// Saint-Vincent and is a different street across the city. A shop's street may be spelt or typed
    /// differently from the one asked for; it may not merely share words with it.
    #[test]
    fn given_a_shop_on_a_street_sharing_words_with_the_asked_one_then_it_is_not_on_the_asked_street() {
        let index = ShopIndex::of(vec![(
            1,
            shop(&["Acme City"], "Rue du Faubourg Saint-Jacques", [2.34, 48.84]),
        )]);

        assert_eq!(
            shop_on_street(&index, &[1], "ACME CITY", "RUE FAUBOURG SAINT VINCENT", &FRENCH),
            None
        );
    }

    #[test]
    fn given_a_shop_on_the_asked_street_spelt_or_typed_differently_then_it_is_still_on_it() {
        let index = ShopIndex::of(vec![(
            1,
            shop(&["Acme City"], "Avenue du Faubourg Saint-Vincent", [2.35, 48.87]),
        )]);

        assert_eq!(
            shop_on_street(&index, &[1], "ACME CITY", "RUE FAUBOURG SAINT VINCENT", &FRENCH)
                .map(|found| found.position),
            Some([2.35, 48.87])
        );
    }
}
