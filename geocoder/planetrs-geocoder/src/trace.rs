//! Every decision the lookup makes for one site, written out for a person to read.
//!
//! The answer at the end is the one [`locate`] itself gives, not a second computation of it: the
//! trace reports the evidence around the decision, the communes considered, the street matches and
//! their near misses, the doors recorded, the shops, so that when they disagree with the answer it
//! is the trace that is wrong and not the geocoder that is hidden.

use std::collections::HashMap;

use crate::gazetteer::locate_by_town_and_postcode;
pub use crate::locate::{Gazetteer, ShopIndex, Site, StreetIndex, locate};
use crate::locate::{exact_reading, street_queries, street_readings};
use crate::loose::unique_street_match;
use crate::shops::metres_between;
use crate::similarity::similarity;
use crate::street::best_street_match;
use crate::street_index::Street;
use crate::text::{FRENCH, comparison_key};

const NEAR_MISSES: usize = 5;
const DOORS_SHOWN: usize = 60;
const SAME_NAME_ELSEWHERE: usize = 6;

fn middle(street: &Street) -> Option<[f64; 2]> {
    let points: Vec<[f64; 2]> = if street.addresses.is_empty() {
        street.path.clone()
    } else {
        street.addresses.iter().map(|door| door.position).collect()
    };
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    let [x, y] = points.iter().fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
        [x + longitude, y + latitude]
    });
    Some([x / count, y / count])
}

/// The lines describing how one site is looked up, the last one being the answer.
pub fn trace(gazetteer: &Gazetteer, streets: &StreetIndex, shops: &ShopIndex, site: &Site) -> Vec<String> {
    let mut lines = Vec::new();
    let asked = site;
    let site = asked.without_parentheses();
    let readings = street_readings(&site.address);
    let name_of: HashMap<i64, String> = gazetteer
        .communes()
        .iter()
        .map(|commune| {
            (
                commune.relation,
                format!(
                    "{} (relation {}, level {})",
                    commune.names.first().cloned().unwrap_or_default(),
                    commune.relation,
                    commune.level
                ),
            )
        })
        .collect();
    let named = |relations: &[i64]| {
        relations
            .iter()
            .map(|relation| {
                name_of
                    .get(relation)
                    .cloned()
                    .unwrap_or_else(|| relation.to_string())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    lines.push(format!(
        "site      {} | {} | {} {} | {}",
        site.label, site.address, site.postcode, site.town, site.country
    ));
    let Some(code) = gazetteer.country_code_for(&site.country).map(String::from) else {
        lines.push(format!(
            "country   '{}' names no country in the index",
            site.country
        ));
        lines.push("answer    none".into());
        return lines;
    };
    let at_postcode: Vec<i64> = gazetteer
        .communes_at_postcode(&site.postcode, &code)
        .iter()
        .map(|at| gazetteer.communes()[*at].relation)
        .collect();
    lines.push(format!(
        "postcode  {} is carried by: {}",
        site.postcode,
        if at_postcode.is_empty() {
            "no commune".into()
        } else {
            named(&at_postcode)
        }
    ));

    let Some(settled) = locate_by_town_and_postcode(gazetteer, &site.town, &site.postcode, &site.country)
    else {
        lines.push("commune   none settled".into());
        lines.push("answer    none".into());
        return lines;
    };
    lines.push(format!(
        "commune   {} settled by {:?}{}, centre {:.6}, {:.6}",
        named(&settled.relations),
        settled.basis,
        settled
            .name_match
            .map(|found| format!(" ({found:?} name)"))
            .unwrap_or_default(),
        settled.position[1],
        settled.position[0]
    ));
    let former: Vec<i64> = gazetteer
        .former_communes_at(&site.postcode, &site.country, &settled.relations)
        .into_iter()
        .filter(|relation| !settled.relations.contains(relation))
        .collect();
    lines.push(format!(
        "former    towns searched first: {}",
        if former.is_empty() {
            "none".into()
        } else {
            named(&former)
        }
    ));
    let mut searched = settled.relations.clone();
    searched.extend(former.iter().copied());

    lines.push(format!(
        "readings  {}",
        readings
            .iter()
            .map(|(_, street)| format!("'{street}'"))
            .collect::<Vec<_>>()
            .join(" then ")
    ));
    lines.push(
        match exact_reading(streets, &settled.relations, &former, &readings) {
            Some((reading, on)) => format!(
                "exact     '{}' is exactly '{}', placed {:?}: it answers unless the parts below place the site closer",
                reading.1, on.street, on.placed
            ),
            None => "exact     no reading matches a street exactly, so the parts below are searched".into(),
        },
    );
    let parts = street_queries(&site.address);
    lines.push(format!(
        "parts     {}",
        parts
            .iter()
            .map(|(_, street)| format!("'{street}'"))
            .collect::<Vec<_>>()
            .join(" then ")
    ));
    for (at, (doors, street)) in parts.iter().enumerate() {
        let asked_doors: Vec<String> = doors
            .iter()
            .map(|door| format!("{}{}", door.number, door.suffix))
            .collect();
        lines.push(format!(
            "part {}    doors [{}], street '{}'",
            at + 1,
            asked_doors.join(", "),
            street
        ));
        if street.is_empty() {
            continue;
        }
        for (label, pool) in [("former towns", &former), ("settled", &settled.relations)] {
            if pool.is_empty() {
                continue;
            }
            let candidates: Vec<&Street> = pool
                .iter()
                .flat_map(|relation| streets.streets_of(*relation))
                .collect();
            match best_street_match(
                street,
                candidates.iter().map(|candidate| candidate.name.as_str()),
                &FRENCH,
            ) {
                Some(found) => {
                    lines.push(format!(
                        "  {label:<13} match '{}' ({:?}), {} streets searched",
                        found.name,
                        found.quality,
                        candidates.len()
                    ));
                    let mut numbers: Vec<String> = candidates
                        .iter()
                        .filter(|candidate| candidate.name == found.name)
                        .flat_map(|candidate| candidate.addresses.iter())
                        .map(|door| (door.number.number, door.number.suffix.clone()))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .map(|(number, suffix)| format!("{number}{suffix}"))
                        .collect();
                    let total = numbers.len();
                    numbers.truncate(DOORS_SHOWN);
                    lines.push(format!(
                        "    doors recorded: {}{}",
                        if numbers.is_empty() {
                            "none".into()
                        } else {
                            numbers.join(" ")
                        },
                        if total > DOORS_SHOWN {
                            format!(" … ({total} in all)")
                        } else {
                            String::new()
                        }
                    ));
                }
                None => {
                    let mut near: Vec<(f64, &str)> = candidates
                        .iter()
                        .map(|candidate| (similarity(street, &candidate.name), candidate.name.as_str()))
                        .collect();
                    near.sort_by(|left, right| right.0.total_cmp(&left.0));
                    near.dedup_by(|left, right| left.1 == right.1);
                    lines.push(format!(
                        "  {label:<13} no match among {} streets; nearest names: {}",
                        candidates.len(),
                        near.iter()
                            .take(NEAR_MISSES)
                            .map(|(score, name)| format!("{name} {score:.2}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        let mut elsewhere: Vec<(f64, String)> = streets
            .communes()
            .filter(|(relation, _)| !searched.contains(relation))
            .flat_map(|(relation, found)| {
                found
                    .iter()
                    .filter(|candidate| candidate.name == *street)
                    .map(move |candidate| (*relation, candidate))
            })
            .filter_map(|(relation, candidate)| {
                middle(candidate).map(|point| (metres_between(point, settled.position), relation))
            })
            .filter(|(metres, _)| *metres < 20_000.0)
            .map(|(metres, relation)| {
                (
                    metres,
                    name_of
                        .get(&relation)
                        .cloned()
                        .unwrap_or_else(|| relation.to_string()),
                )
            })
            .collect();
        elsewhere.sort_by(|left, right| left.0.total_cmp(&right.0));
        if !elsewhere.is_empty() {
            lines.push(format!(
                "    same street name in other communes within 20 km: {}",
                elsewhere
                    .iter()
                    .take(SAME_NAME_ELSEWHERE)
                    .map(|(metres, name)| format!("{name} at {:.1} km", metres / 1000.0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let label = comparison_key(&site.label, &FRENCH);
    let carrying: Vec<String> = searched
        .iter()
        .flat_map(|relation| shops.shops_of(*relation))
        .filter(|shop| !label.is_empty() && shop.names.iter().any(|name| name.contains(&label)))
        .map(|shop| {
            format!(
                "{} [street '{}'] at {:.6}, {:.6}",
                shop.names.join(" / "),
                shop.street,
                shop.position[1],
                shop.position[0]
            )
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let settled_names: Vec<&str> = settled
        .relations
        .iter()
        .flat_map(|relation| streets.streets_of(*relation))
        .map(|street| street.name.as_str())
        .collect();
    let loosely: Vec<String> = parts
        .iter()
        .filter_map(|(_, street)| {
            unique_street_match(
                &comparison_key(street, &FRENCH),
                &settled_names,
                &FRENCH,
                streets.rarity(),
            )
            .map(|found| {
                format!(
                    "'{street}' is the only street fitting '{}' ({:?})",
                    found.name, found.quality
                )
            })
        })
        .collect();
    lines.push(format!(
        "unique    step fifteen, used only when no part matched above: {}",
        if loosely.is_empty() {
            "no part fits a single street".to_string()
        } else {
            loosely.join("; ")
        }
    ));
    lines.push(format!(
        "shops     carrying '{label}' in the searched communes: {}",
        if carrying.is_empty() {
            "none".into()
        } else {
            format!("{}", carrying.len())
        }
    ));
    lines.extend(carrying.into_iter().take(12).map(|line| format!("    {line}")));

    match locate(gazetteer, streets, shops, asked) {
        Some(answer) => lines.push(format!(
            "answer    {:?} from {:?}{}: {:.6}, {:.6}",
            answer.precision,
            answer.basis,
            answer
                .street
                .map(|(name, quality)| format!(" on '{name}' ({quality:?})"))
                .unwrap_or_default()
                + &answer
                    .rewritten_as
                    .map(|rewriting| format!(", the address rewritten as '{rewriting}'"))
                    .unwrap_or_default(),
            answer.position[1],
            answer.position[0]
        )),
        None => lines.push("answer    none".into()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Country;
    use crate::gazetteer::Commune;
    use crate::street_index::{AddressPoint, Street};
    use crate::text::HouseNumber;

    fn fixture() -> (Gazetteer, StreetIndex, ShopIndex) {
        let gazetteer = Gazetteer::of(
            vec![Commune {
                relation: 1,
                level: 8,
                names: vec!["SENS".into()],
                postcodes: vec!["89100".into()],
                centre: Some([3.283, 48.197]),
                country: Some("FR".into()),
            }],
            vec![Country {
                code: "FR".into(),
                names: vec!["FRANCE".into()],
            }],
        );
        let streets = StreetIndex::of(vec![(
            1,
            Street {
                name: "ROUTE TILLEULS".into(),
                addresses: vec![AddressPoint {
                    number: HouseNumber {
                        number: 6,
                        suffix: String::new(),
                    },
                    position: [3.265, 48.200],
                }],
                path: vec![],
            },
        )]);
        (gazetteer, streets, ShopIndex::default())
    }

    fn site(address: &str) -> Site {
        Site {
            label: "ACME".into(),
            address: address.into(),
            postcode: "89100".into(),
            town: "SENS".into(),
            country: "FRANCE".into(),
        }
    }

    fn traced(address: &str) -> String {
        let (gazetteer, streets, shops) = fixture();
        trace(&gazetteer, &streets, &shops, &site(address)).join("\\n")
    }

    #[test]
    fn given_a_site_when_traced_then_the_settled_commune_is_named() {
        assert!(traced("8 Route des Tilleuls").contains("SENS"));
    }

    #[test]
    fn given_a_site_when_traced_then_the_street_matched_and_its_recorded_doors_are_listed() {
        let written = traced("8 Route des Tilleuls");

        assert!(
            written.contains("ROUTE TILLEULS") && written.contains("doors recorded: 6"),
            "{written}"
        );
    }

    #[test]
    fn given_a_site_when_traced_then_it_ends_with_the_answer_the_lookup_itself_gives() {
        let (gazetteer, streets, shops) = fixture();
        let answer = locate(&gazetteer, &streets, &shops, &site("8 Route des Tilleuls")).expect("an answer");

        let written = trace(&gazetteer, &streets, &shops, &site("8 Route des Tilleuls"));

        assert!(
            written
                .last()
                .expect("a line")
                .contains(&format!("{:.6}, {:.6}", answer.position[1], answer.position[0]))
        );
    }

    /// The trace hands the lookup the site as it was asked, not the one it cleaned for display, so the
    /// answer it ends with is the lookup's own.
    #[test]
    fn given_a_street_named_after_a_spaced_dash_when_traced_then_it_ends_with_the_lookups_answer() {
        let (gazetteer, streets, shops) = fixture();
        let asked = site("Zone des Tanneries - 6 Route des Tilleuls");
        let answer = locate(&gazetteer, &streets, &shops, &asked).expect("an answer");

        let written = trace(&gazetteer, &streets, &shops, &asked);

        assert_eq!(answer.position, [3.265, 48.200]);
        assert!(
            written
                .last()
                .expect("a line")
                .contains(&format!("{:.6}, {:.6}", answer.position[1], answer.position[0])),
            "{written:?}"
        );
    }
}
