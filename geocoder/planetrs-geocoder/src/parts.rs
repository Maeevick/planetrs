//! The geocoder's parts of a dataset: its index written as the gazetteer and one geocoder index per
//! country, with every value exactly as the lookups have always read it.

use std::collections::{BTreeMap, HashMap};

use planetrs_dataset::{gazetteer, geocoder_index};

use crate::boundary::Country;
use crate::gazetteer::{Commune, Gazetteer};
use crate::rarity::WordRarity;
use crate::shops::{Shop, ShopIndex, metres_between};
use crate::street_index::{AddressPoint, Street, StreetIndex};
use crate::text::HouseNumber;

/// A position kept to the seven decimals of a degree OpenStreetMap records it at, about a
/// centimetre, which is what every index has stored and every lookup has been tuned on.
pub(crate) fn at_osm_precision(degrees: f64) -> f64 {
    format!("{degrees:.7}").parse().unwrap_or(degrees)
}

/// An area kept to the thousandth of a square kilometre, a thousand square metres.
fn to_a_thousandth(square_kilometres: f64) -> f64 {
    format!("{square_kilometres:.3}")
        .parse()
        .unwrap_or(square_kilometres)
}

fn present(texts: &[String]) -> Vec<String> {
    texts.iter().filter(|text| !text.is_empty()).cloned().collect()
}

/// Every commune and country as the gazetteer's areas, sorted by relation, with each commune's
/// area and absorbing commune when it has one, and each country under the lowest relation drawing
/// it.
pub(crate) fn gazetteer_of(
    communes: &[Commune],
    countries: &[Country],
    country_relations: &HashMap<String, i64>,
    parents: &HashMap<i64, i64>,
    areas: &HashMap<i64, f64>,
) -> Vec<gazetteer::Area> {
    let as_areas = communes.iter().map(|commune| gazetteer::Area {
        relation: commune.relation,
        level: commune.level,
        country: commune.country.clone(),
        names: present(&commune.names),
        postcodes: present(&commune.postcodes),
        square_kilometres: areas.get(&commune.relation).copied().map(to_a_thousandth),
        absorbed_by: parents.get(&commune.relation).copied(),
        centre: commune
            .centre
            .map(|[longitude, latitude]| [at_osm_precision(longitude), at_osm_precision(latitude)]),
    });
    let as_countries = countries.iter().filter_map(|country| {
        country_relations
            .get(&country.code)
            .map(|relation| gazetteer::Area {
                relation: *relation,
                level: COUNTRY_LEVEL,
                country: Some(country.code.clone()),
                names: present(&country.names),
                postcodes: Vec::new(),
                square_kilometres: None,
                absorbed_by: None,
                centre: None,
            })
    });
    let mut every: Vec<gazetteer::Area> = as_areas.chain(as_countries).collect();
    every.sort_by_key(|area| area.relation);
    every
}

/// The `admin_level` of a country.
pub(crate) const COUNTRY_LEVEL: u8 = 2;

/// The geocoder index of one country: how rare each word of its street names is, then its
/// communes' doors, middles and shops in the order the lookups have always read them.
pub(crate) fn geocoder_index_of(
    country: &str,
    communes: &[Commune],
    streets: &StreetIndex,
    shops: &ShopIndex,
) -> Vec<geocoder_index::Entry> {
    let in_country: BTreeMap<i64, ()> = communes
        .iter()
        .filter(|commune| commune.country.as_deref() == Some(country))
        .map(|commune| (commune.relation, ()))
        .collect();
    let mut filed: Vec<i64> = streets
        .communes()
        .map(|(commune, _)| *commune)
        .chain(shops.communes().map(|(commune, _)| *commune))
        .filter(|commune| in_country.contains_key(commune))
        .collect();
    filed.sort_unstable();
    filed.dedup();
    let rarity = WordRarity::of(
        filed
            .iter()
            .flat_map(|commune| streets.streets_of(*commune))
            .map(|street| street.name.as_str()),
    );
    let counts = std::iter::once(geocoder_index::Entry::Names {
        count: rarity.names(),
    })
    .chain(
        rarity
            .words()
            .into_iter()
            .map(|(word, count)| geocoder_index::Entry::Word {
                word: word.to_string(),
                count: u64::from(count),
            }),
    );
    let entries = filed.iter().flat_map(|commune| {
        streets_of(*commune, streets)
            .into_iter()
            .chain(shops_of(*commune, shops))
    });
    counts.chain(entries).collect()
}

fn streets_of(commune: i64, streets: &StreetIndex) -> Vec<geocoder_index::Entry> {
    streets
        .streets_of(commune)
        .iter()
        .flat_map(|street| {
            let mut doors: Vec<_> = street.addresses.iter().collect();
            doors.sort_by(|left, right| {
                (left.number.number, &left.number.suffix)
                    .cmp(&(right.number.number, &right.number.suffix))
                    .then(left.position[0].total_cmp(&right.position[0]))
                    .then(left.position[1].total_cmp(&right.position[1]))
            });
            let doors = doors.into_iter().map(move |door| geocoder_index::Entry::Door {
                commune,
                street: street.name.clone(),
                number: door.number.number,
                suffix: door.number.suffix.clone(),
                position: [
                    at_osm_precision(door.position[0]),
                    at_osm_precision(door.position[1]),
                ],
            });
            let middle = (!street.path.is_empty()).then(|| {
                let count = street.path.len() as f64;
                let [longitude, latitude] = street
                    .path
                    .iter()
                    .fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
                        [x + longitude, y + latitude]
                    });
                geocoder_index::Entry::Middle {
                    commune,
                    street: street.name.clone(),
                    position: [
                        at_osm_precision(longitude / count),
                        at_osm_precision(latitude / count),
                    ],
                }
            });
            doors.chain(middle)
        })
        .collect()
}

/// How far the middle of any street of `entries` lies from its commune's centre, in metres: a door's
/// street is placed at the mean of its doors, a street without doors at its middle, as `trace`
/// places them.
pub(crate) fn reach_of(entries: &[geocoder_index::Entry], communes: &[Commune]) -> f64 {
    let centres: HashMap<i64, [f64; 2]> = communes
        .iter()
        .filter_map(|commune| {
            commune.centre.map(|[longitude, latitude]| {
                (
                    commune.relation,
                    [at_osm_precision(longitude), at_osm_precision(latitude)],
                )
            })
        })
        .collect();
    let mut doors: BTreeMap<(i64, &str), Vec<[f64; 2]>> = BTreeMap::new();
    let mut middles: BTreeMap<(i64, &str), [f64; 2]> = BTreeMap::new();
    for entry in entries {
        match entry {
            geocoder_index::Entry::Door {
                commune,
                street,
                position,
                ..
            } => doors
                .entry((*commune, street.as_str()))
                .or_default()
                .push(*position),
            geocoder_index::Entry::Middle {
                commune,
                street,
                position,
            } => {
                middles.insert((*commune, street.as_str()), *position);
            }
            _ => {}
        }
    }
    let without_doors: Vec<((i64, &str), [f64; 2])> = middles
        .into_iter()
        .filter(|(street, _)| !doors.contains_key(street))
        .collect();
    let placed = without_doors
        .into_iter()
        .chain(doors.into_iter().map(|(street, positions)| {
            let count = positions.len() as f64;
            let [longitude, latitude] = positions
                .iter()
                .fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
                    [x + longitude, y + latitude]
                });
            (street, [longitude / count, latitude / count])
        }));
    placed
        .filter_map(|((commune, _), middle)| {
            centres
                .get(&commune)
                .map(|centre| metres_between(middle, *centre))
        })
        .fold(0.0, f64::max)
}

/// A commune's shops in the order the index file has always held them: by the text of their line.
fn shops_of(commune: i64, shops: &ShopIndex) -> Vec<geocoder_index::Entry> {
    let mut lines: Vec<(String, geocoder_index::Entry)> = shops
        .shops_of(commune)
        .iter()
        .map(|shop| {
            let position = [
                at_osm_precision(shop.position[0]),
                at_osm_precision(shop.position[1]),
            ];
            let line = format!(
                "{commune}\t{}\t{}\t{:.7}\t{:.7}",
                shop.names.join("|"),
                shop.street,
                shop.position[0],
                shop.position[1]
            );
            let entry = geocoder_index::Entry::Shop {
                commune,
                names: present(&shop.names),
                street: shop.street.clone(),
                position,
            };
            (line, entry)
        })
        .collect();
    lines.sort_by(|left, right| left.0.cmp(&right.0));
    lines.into_iter().map(|(_, entry)| entry).collect()
}

/// The gazetteer read back into the one the lookups settle sites with: its communes, its countries
/// in the order of their codes, and the absorbing commune and the area of whichever communes have
/// them.
pub(crate) fn settling_gazetteer(areas: Vec<gazetteer::Area>) -> Gazetteer {
    let parents = areas
        .iter()
        .filter_map(|area| area.absorbed_by.map(|commune| (area.relation, commune)))
        .collect();
    let surfaces = areas
        .iter()
        .filter_map(|area| area.square_kilometres.map(|surface| (area.relation, surface)))
        .collect();
    let (countries, communes): (Vec<gazetteer::Area>, Vec<gazetteer::Area>) =
        areas.into_iter().partition(|area| area.level == COUNTRY_LEVEL);
    let mut countries: Vec<Country> = countries
        .into_iter()
        .map(|area| Country {
            code: area.country.unwrap_or_default(),
            names: area.names,
        })
        .collect();
    countries.sort_by(|left, right| left.code.cmp(&right.code));
    let communes = communes
        .into_iter()
        .map(|area| Commune {
            relation: area.relation,
            level: area.level,
            names: area.names,
            postcodes: area.postcodes,
            centre: area.centre,
            country: area.country,
        })
        .collect();
    Gazetteer::of(communes, countries)
        .with_parents(parents)
        .with_areas(surfaces)
}

/// A country's geocoder index read back into the street and shop indexes the lookups search.
pub(crate) fn indexes_of(entries: Vec<geocoder_index::Entry>) -> (StreetIndex, ShopIndex) {
    let mut streets = Vec::new();
    let mut shops = Vec::new();
    for entry in entries {
        match entry {
            geocoder_index::Entry::Door {
                commune,
                street,
                number,
                suffix,
                position,
            } => streets.push((
                commune,
                Street {
                    name: street,
                    addresses: vec![AddressPoint {
                        number: HouseNumber { number, suffix },
                        position,
                    }],
                    path: vec![],
                },
            )),
            geocoder_index::Entry::Middle {
                commune,
                street,
                position,
            } => streets.push((
                commune,
                Street {
                    name: street,
                    addresses: vec![],
                    path: vec![position],
                },
            )),
            geocoder_index::Entry::Shop {
                commune,
                names,
                street,
                position,
            } => shops.push((
                commune,
                Shop {
                    names,
                    street,
                    position,
                },
            )),
            geocoder_index::Entry::Names { .. } | geocoder_index::Entry::Word { .. } => {}
        }
    }
    (StreetIndex::of(streets), ShopIndex::of(shops))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commune(relation: i64, country: &str) -> Commune {
        Commune {
            relation,
            level: 8,
            names: vec![format!("COMMUNE {relation}")],
            postcodes: vec![],
            centre: Some([3.28, 48.19]),
            country: Some(country.into()),
        }
    }

    fn street(name: &str) -> Street {
        Street {
            name: name.into(),
            addresses: vec![],
            path: vec![[3.268_208_249_9, 48.197_500_6]],
        }
    }

    #[test]
    fn given_a_position_finer_than_seven_decimals_when_kept_then_it_reads_as_the_index_has_always_stored_it()
    {
        let kept = at_osm_precision(3.268_208_249_9);

        assert_eq!(kept, "3.2682082".parse::<f64>().unwrap_or_default());
    }

    #[test]
    fn given_shops_of_one_commune_when_written_then_they_follow_the_text_of_their_lines() {
        let shop = |name: &str, longitude: f64| {
            (
                1,
                Shop {
                    names: vec![name.into()],
                    street: String::new(),
                    position: [longitude, 48.19],
                },
            )
        };
        let shops = ShopIndex::of(vec![shop("CASINO", 3.3), shop("ALDI", 3.29), shop("ALDI", 3.1)]);

        let entries = geocoder_index_of("FR", &[commune(1, "FR")], &StreetIndex::default(), &shops);

        let written: Vec<(String, f64)> = entries
            .iter()
            .filter_map(|entry| match entry {
                geocoder_index::Entry::Shop { names, position, .. } => Some((names.join("|"), position[0])),
                _ => None,
            })
            .collect();
        assert_eq!(
            written,
            vec![
                ("ALDI".into(), 3.1),
                ("ALDI".into(), 3.29),
                ("CASINO".into(), 3.3)
            ]
        );
    }

    #[test]
    fn given_streets_in_two_countries_when_each_index_is_written_then_its_word_counts_cover_its_own_names_only()
     {
        let streets = StreetIndex::of(vec![
            (1, street("AVENUE GARE")),
            (1, street("RUE GARE")),
            (2, street("BOULEVARD PRINCESSE CHARLOTTE")),
        ]);
        let communes = [commune(1, "FR"), commune(2, "MC")];

        let entries = geocoder_index_of("FR", &communes, &streets, &ShopIndex::default());

        let counts: Vec<&geocoder_index::Entry> = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    geocoder_index::Entry::Names { .. } | geocoder_index::Entry::Word { .. }
                )
            })
            .collect();
        let word = |word: &str, count| geocoder_index::Entry::Word {
            word: word.into(),
            count,
        };
        assert_eq!(
            counts,
            vec![
                &geocoder_index::Entry::Names { count: 2 },
                &word("AVENUE", 1),
                &word("GARE", 2),
                &word("RUE", 1)
            ]
        );
    }

    #[test]
    fn given_streets_of_two_communes_when_their_reach_is_measured_then_it_is_the_farthest_street_middle_from_its_centre()
     {
        let communes = [commune(1, "FR"), commune(2, "FR")];
        let door = |commune, number, position| geocoder_index::Entry::Door {
            commune,
            street: "RUE GARE".into(),
            number,
            suffix: String::new(),
            position,
        };
        let entries = [
            door(1, 1, [3.28, 48.20]),
            door(1, 3, [3.28, 48.22]),
            geocoder_index::Entry::Middle {
                commune: 2,
                street: "AVENUE GARE".into(),
                position: [3.29, 48.19],
            },
        ];

        let reach = reach_of(&entries, &communes);

        let farthest = metres_between([3.28, 48.21], [3.28, 48.19]);
        assert!((reach - farthest).abs() < 2.0, "{reach} against {farthest}");
    }
}
