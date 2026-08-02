//! Reading a planet file, which is the only part of this crate that touches a disk.
//!
//! Everything a pass yields is plain data the rest of the crate can be tested against without a
//! planet file, so what lives here is the reading and nothing that decides anything.
//!
//! A planet file is sorted nodes, then ways, then relations, and a relation names its members by
//! identifier only. So a place's outline and centre are gathered backwards: the relations say which
//! ways and nodes they need, one pass over the ways fetches those, and one pass over the nodes gives
//! them coordinates.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use planetrs_pbf::{
    Block, PbfError, Planet, block as decode_block, dense_group, inflate, visit_relations, visit_ways,
};
use rayon::prelude::*;

use crate::boundary::{AdministrativeArea, Country, administrative_area_of, centre_node_of, country_of};
use crate::rings::Outline;
use crate::territory::{IdentifierSet, TerritoryMask};

/// An administrative area, with the node standing for its centre and the ways its outline is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestedArea {
    pub relation: i64,
    pub area: AdministrativeArea,
    pub centre_node: Option<i64>,
    pub outline_ways: Vec<i64>,
}

/// A country, with the ways its outline is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestedCountry {
    pub relation: i64,
    pub country: Country,
    pub outline_ways: Vec<i64>,
}

/// What one pass over the relations found, including what it could not read.
///
/// A blob that fails to inflate is counted rather than ignored. A gazetteer quietly missing a
/// thousand communes looks exactly like one that never had them.
/// A street as an `associatedStreet` relation names it, with the houses it names it for.
///
/// The French cadastre import often writes a house number on a bare node and puts the street name
/// on a relation joining those houses to the road. A reader of tags alone sees a number with no
/// street.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociatedStreet {
    pub name: String,
    pub house_nodes: Vec<i64>,
    pub house_ways: Vec<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Harvest {
    pub areas: Vec<HarvestedArea>,
    pub countries: Vec<HarvestedCountry>,
    pub associated_streets: Vec<AssociatedStreet>,
    pub unreadable_blobs: usize,
}

impl Harvest {
    fn absorb(mut self, other: Self) -> Self {
        self.areas.extend(other.areas);
        self.countries.extend(other.countries);
        self.associated_streets.extend(other.associated_streets);
        self.unreadable_blobs += other.unreadable_blobs;
        self
    }
}

/// A pass's findings, and how many blobs it could not read.
pub struct Scanned<T> {
    pub found: T,
    pub unreadable_blobs: usize,
}

fn scan<T, E, V, M>(planet: &Planet, blobs: Range<usize>, empty: E, visit: V, merge: M) -> Scanned<T>
where
    T: Send,
    E: Fn() -> T + Sync + Send,
    V: Fn(&mut T, &Block<'_>) + Sync + Send,
    M: Fn(T, T) -> T + Sync + Send,
{
    let (found, unreadable_blobs) = planet.blobs[blobs]
        .par_iter()
        .fold(
            || (empty(), 0usize),
            |(mut found, mut unreadable), location| {
                let mut payload = Vec::new();
                let mut inflated = Vec::new();
                let readable =
                    planet.read(location, &mut payload).is_ok() && inflate(&payload, &mut inflated).is_ok();
                match readable.then(|| decode_block(&inflated)).and_then(Result::ok) {
                    Some(block) => visit(&mut found, &block),
                    None => unreadable += 1,
                }
                (found, unreadable)
            },
        )
        .reduce(
            || (empty(), 0),
            |(left, a), (right, b)| (merge(left, right), a + b),
        );
    Scanned {
        found,
        unreadable_blobs,
    }
}

fn as_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Every administrative area and every country in the planet file, read from the relations alone.
///
/// Fails when the file does not declare `Sort.Type_then_ID`, because without that guarantee the
/// relations section cannot be found by bisection and this would read the wrong bytes rather than
/// report an error.
pub fn harvest_boundaries(planet: &Planet) -> Result<Harvest, PbfError> {
    let sections = planet.sections()?;
    let scanned = scan(
        planet,
        sections.relations,
        Harvest::default,
        |found, block| {
            for group in &block.groups {
                let _ = visit_relations(group, &mut |relation| {
                    let tags: Vec<(String, String)> = relation
                        .tags(block)
                        .into_iter()
                        .map(|(key, value)| (as_text(key), as_text(value)))
                        .collect();
                    let outline_ways: Vec<i64> = relation.ways(block).into_iter().map(|(id, _)| id).collect();

                    let value_of = |wanted: &str| {
                        tags.iter()
                            .find(|(key, _)| key == wanted)
                            .map(|(_, value)| value.as_str())
                    };
                    if value_of("type") == Some("associatedStreet") {
                        if let Some(name) = value_of("name").filter(|name| !name.is_empty()) {
                            let houses = relation.members(block);
                            let is_house = |role: &[u8]| {
                                role == b"house" || role == b"addr:houselink" || role.is_empty()
                            };
                            found.associated_streets.push(AssociatedStreet {
                                name: name.to_string(),
                                house_nodes: houses
                                    .iter()
                                    .filter(|(_, kind, role)| *kind == 0 && is_house(role))
                                    .map(|(id, _, _)| *id)
                                    .collect(),
                                house_ways: houses
                                    .iter()
                                    .filter(|(_, kind, role)| *kind == 1 && is_house(role))
                                    .map(|(id, _, _)| *id)
                                    .collect(),
                            });
                        }
                        return Ok(());
                    }
                    if let Some(country) = country_of(&tags) {
                        found.countries.push(HarvestedCountry {
                            relation: relation.id,
                            country,
                            outline_ways,
                        });
                    } else if let Some(area) = administrative_area_of(&tags) {
                        let members: Vec<(i64, u64, String)> = relation
                            .members(block)
                            .into_iter()
                            .map(|(id, kind, role)| (id, kind, as_text(role)))
                            .collect();
                        found.areas.push(HarvestedArea {
                            relation: relation.id,
                            area,
                            centre_node: centre_node_of(&members),
                            outline_ways,
                        });
                    }
                    Ok(())
                });
            }
        },
        Harvest::absorb,
    );
    let mut harvest = scanned.found;
    harvest.unreadable_blobs += scanned.unreadable_blobs;
    Ok(harvest)
}

/// The node identifiers of the ways asked for, read from the ways section.
pub fn ways_by_id(
    planet: &Planet,
    wanted: &HashSet<i64>,
) -> Result<Scanned<HashMap<i64, Vec<i64>>>, PbfError> {
    let sections = planet.sections()?;
    Ok(scan(
        planet,
        sections.ways,
        HashMap::new,
        |found, block| {
            for group in &block.groups {
                let _ = visit_ways(group, &mut |way| {
                    if wanted.contains(&way.id) {
                        let references: Vec<i64> = way.refs().filter_map(Result::ok).collect();
                        found.insert(way.id, references);
                    }
                    Ok(())
                });
            }
        },
        |mut left, right| {
            left.extend(right);
            left
        },
    ))
}

/// Longitude and latitude of the nodes asked for, read from the nodes section.
pub fn nodes_by_id(
    planet: &Planet,
    wanted: &HashSet<i64>,
) -> Result<Scanned<HashMap<i64, [f64; 2]>>, PbfError> {
    let sections = planet.sections()?;
    Ok(scan(
        planet,
        sections.nodes,
        HashMap::new,
        |found, block| {
            for group in &block.groups {
                if let Ok(Some(dense)) = dense_group(group) {
                    let _ = dense.visit(&mut |id, latitude, longitude| {
                        if wanted.contains(&id) {
                            found.insert(id, [block.longitude(longitude), block.latitude(latitude)]);
                        }
                    });
                }
            }
        },
        |mut left, right| {
            left.extend(right);
            left
        },
    ))
}

/// The largest node identifier in the file, read from the last blob of the node section.
pub fn largest_node_id(planet: &Planet) -> Result<i64, PbfError> {
    let sections = planet.sections()?;
    let last = sections.nodes.end.checked_sub(1).ok_or(PbfError::Truncated)?;
    let mut payload = Vec::new();
    let mut inflated = Vec::new();
    planet.inflate_blob(last, &mut payload, &mut inflated)?;
    let block = decode_block(&inflated)?;
    Ok(planetrs_pbf::id_span(&block)?.map(|span| span.last).unwrap_or(0))
}

fn tag_values<'a, const N: usize>(
    keys: &[u8],
    values: &[u8],
    block: &Block<'a>,
    wanted: &[Option<u32>; N],
) -> [&'a [u8]; N] {
    let mut found: [&'a [u8]; N] = [b""; N];
    let mut values = planetrs_pbf::proto::Packed::new(values);
    for key in planetrs_pbf::proto::Packed::new(keys) {
        let (Some(Ok(key)), Some(Ok(value))) = (Some(key), values.next()) else {
            break;
        };
        if let Some(at) = wanted.iter().position(|id| *id == Some(key as u32)) {
            found[at] = block.string(value as usize);
        }
    }
    found
}

/// A tagged address node inside a target country.
#[derive(Debug, Clone, PartialEq)]
pub struct RawAddress {
    pub node: i64,
    pub position: [f64; 2],
    pub housenumber: String,
    pub street: String,
    pub place: String,
}

/// A named hamlet, locality or square, which is where a rural French address points when it has no
/// street at all.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPlace {
    pub position: [f64; 2],
    pub name: String,
}

/// A shop inside a target country, with every name it answers to and the street it states.
#[derive(Debug, Clone, PartialEq)]
pub struct RawShop {
    pub position: [f64; 2],
    pub names: Vec<String>,
    pub street: String,
}

/// A shop mapped as the outline of its building rather than as a point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawShopWay {
    pub names: Vec<String>,
    pub street: String,
    pub references: Vec<i64>,
}

fn shop_names(name: &[u8], brand: &[u8]) -> Vec<String> {
    [name, brand]
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(as_text)
        .collect()
}

#[derive(Debug, Default)]
pub struct TerritoryNodes {
    pub addresses: Vec<RawAddress>,
    pub places: Vec<RawPlace>,
    pub shops: Vec<RawShop>,
}

impl TerritoryNodes {
    fn absorb(mut self, other: Self) -> Self {
        self.addresses.extend(other.addresses);
        self.places.extend(other.places);
        self.shops.extend(other.shops);
        self
    }
}

const PLACES_THAT_STAND_FOR_A_STREET: [&[u8]; 9] = [
    b"hamlet",
    b"locality",
    b"isolated_dwelling",
    b"neighbourhood",
    b"quarter",
    b"suburb",
    b"square",
    b"farm",
    b"village",
];

/// One pass over every node: remember which fall inside the target countries, and keep the ones
/// that carry an address or name a hamlet.
///
/// The raster mask decides membership, because the exact outline test is too slow for nine billion
/// nodes. The exact test is kept for the few million that carry an address, which is what is filed.
pub fn scan_territory_nodes(
    planet: &Planet,
    mask: &TerritoryMask,
    countries: &[&Outline],
    inside: &IdentifierSet,
) -> Result<Scanned<TerritoryNodes>, PbfError> {
    let sections = planet.sections()?;
    let in_a_country = |point: [f64; 2]| countries.iter().any(|outline| outline.contains(point));
    Ok(scan(
        planet,
        sections.nodes,
        TerritoryNodes::default,
        |found, block| {
            let wanted = [
                block.string_id(b"addr:housenumber"),
                block.string_id(b"addr:street"),
                block.string_id(b"addr:place"),
                block.string_id(b"place"),
                block.string_id(b"name"),
                block.string_id(b"shop"),
                block.string_id(b"brand"),
            ];
            for group in &block.groups {
                let Ok(Some(dense)) = dense_group(group) else {
                    continue;
                };
                let _ = dense.visit_tagged(&mut |id, latitude, longitude, tags| {
                    let point = [block.longitude(longitude), block.latitude(latitude)];
                    if !mask.might_hold(point) {
                        return;
                    }
                    inside.insert(id);
                    if tags.is_empty() {
                        return;
                    }
                    let mut values: [&[u8]; 7] = [b""; 7];
                    for (key, value) in tags {
                        if let Some(at) = wanted.iter().position(|wanted| *wanted == Some(*key)) {
                            values[at] = block.string(*value as usize);
                        }
                    }
                    let [housenumber, street, place, kind, name, shop, brand] = values;
                    if !shop.is_empty() && !(name.is_empty() && brand.is_empty()) && in_a_country(point) {
                        found.shops.push(RawShop {
                            position: point,
                            names: shop_names(name, brand),
                            street: as_text(street),
                        });
                    }
                    if !housenumber.is_empty() {
                        if in_a_country(point) {
                            found.addresses.push(RawAddress {
                                node: id,
                                position: point,
                                housenumber: as_text(housenumber),
                                street: as_text(street),
                                place: as_text(place),
                            });
                        }
                    } else if !name.is_empty()
                        && PLACES_THAT_STAND_FOR_A_STREET.contains(&kind)
                        && in_a_country(point)
                    {
                        found.places.push(RawPlace {
                            position: point,
                            name: as_text(name),
                        });
                    }
                });
            }
        },
        TerritoryNodes::absorb,
    ))
}

/// A named road whose first node fell inside a target country.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHighway {
    /// Empty for a road known only by its number.
    pub name: String,
    pub references: Vec<i64>,
    /// Its `ref` values, `D 620` and `D 513` for `ref=D 620;D 513`.
    pub road_numbers: Vec<String>,
}

/// A way carrying a house number, usually a building from the cadastre import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBuilding {
    pub way: i64,
    pub housenumber: String,
    pub street: String,
    pub place: String,
    pub references: Vec<i64>,
}

#[derive(Debug, Default)]
pub struct TerritoryWays {
    pub outlines: HashMap<i64, Vec<i64>>,
    pub highways: Vec<RawHighway>,
    pub buildings: Vec<RawBuilding>,
    pub shops: Vec<RawShopWay>,
}

impl TerritoryWays {
    fn absorb(mut self, other: Self) -> Self {
        self.outlines.extend(other.outlines);
        self.highways.extend(other.highways);
        self.buildings.extend(other.buildings);
        self.shops.extend(other.shops);
        self
    }
}

/// One pass over every way: the communes' outline ways by identifier, and the named roads and
/// addressed buildings whose first node the node pass placed inside a target country.
pub fn scan_territory_ways(
    planet: &Planet,
    inside: &IdentifierSet,
    outline_ways: &HashSet<i64>,
) -> Result<Scanned<TerritoryWays>, PbfError> {
    let sections = planet.sections()?;
    Ok(scan(
        planet,
        sections.ways,
        TerritoryWays::default,
        |found, block| {
            let wanted = [
                block.string_id(b"highway"),
                block.string_id(b"name"),
                block.string_id(b"addr:housenumber"),
                block.string_id(b"addr:street"),
                block.string_id(b"addr:place"),
                block.string_id(b"shop"),
                block.string_id(b"brand"),
                block.string_id(b"ref"),
            ];
            for group in &block.groups {
                let _ = visit_ways(group, &mut |way| {
                    if outline_ways.contains(&way.id) {
                        found
                            .outlines
                            .insert(way.id, way.refs().filter_map(Result::ok).collect());
                    }
                    let Some(Ok(first)) = way.refs().next() else {
                        return Ok(());
                    };
                    if !inside.contains(first) {
                        return Ok(());
                    }
                    let [
                        highway,
                        name,
                        housenumber,
                        street,
                        place,
                        shop,
                        brand,
                        road_number,
                    ] = tag_values(way.keys, way.values, block, &wanted);
                    if !shop.is_empty() && !(name.is_empty() && brand.is_empty()) {
                        found.shops.push(RawShopWay {
                            names: shop_names(name, brand),
                            street: as_text(street),
                            references: way.refs().filter_map(Result::ok).collect(),
                        });
                    }
                    if !housenumber.is_empty() {
                        found.buildings.push(RawBuilding {
                            way: way.id,
                            housenumber: as_text(housenumber),
                            street: as_text(street),
                            place: as_text(place),
                            references: way.refs().filter_map(Result::ok).collect(),
                        });
                    } else if !highway.is_empty() && !(name.is_empty() && road_number.is_empty()) {
                        found.highways.push(RawHighway {
                            name: as_text(name),
                            references: way.refs().filter_map(Result::ok).collect(),
                            road_numbers: as_text(road_number)
                                .split(';')
                                .map(str::trim)
                                .filter(|number| !number.is_empty())
                                .map(str::to_string)
                                .collect(),
                        });
                    }
                    Ok(())
                });
            }
        },
        TerritoryWays::absorb,
    ))
}

/// Longitude and latitude of every node in an identifier set.
pub fn node_positions(
    planet: &Planet,
    wanted: &IdentifierSet,
) -> Result<Scanned<HashMap<i64, [f64; 2]>>, PbfError> {
    let sections = planet.sections()?;
    Ok(scan(
        planet,
        sections.nodes,
        HashMap::new,
        |found, block| {
            for group in &block.groups {
                if let Ok(Some(dense)) = dense_group(group) {
                    let _ = dense.visit(&mut |id, latitude, longitude| {
                        if wanted.contains(id) {
                            found.insert(id, [block.longitude(longitude), block.latitude(latitude)]);
                        }
                    });
                }
            }
        },
        |mut left, right| {
            left.extend(right);
            left
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(relation: i64) -> HarvestedArea {
        HarvestedArea {
            relation,
            area: AdministrativeArea {
                level: 8,
                names: vec!["SENS".into()],
                reference: None,
                postcodes: vec![],
            },
            centre_node: None,
            outline_ways: vec![],
        }
    }

    #[test]
    fn given_two_partial_harvests_when_absorbed_then_their_areas_are_joined() {
        let left = Harvest {
            areas: vec![area(1)],
            ..Harvest::default()
        };
        let right = Harvest {
            areas: vec![area(2)],
            ..Harvest::default()
        };

        assert_eq!(left.absorb(right).areas, vec![area(1), area(2)]);
    }

    #[test]
    fn given_partial_harvests_that_each_lost_a_blob_when_absorbed_then_the_losses_are_summed() {
        let left = Harvest {
            unreadable_blobs: 2,
            ..Harvest::default()
        };
        let right = Harvest {
            unreadable_blobs: 3,
            ..Harvest::default()
        };

        assert_eq!(left.absorb(right).unreadable_blobs, 5);
    }
}
