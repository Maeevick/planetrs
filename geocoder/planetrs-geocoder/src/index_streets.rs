//! Reading every street, door and hamlet of the chosen countries out of the planet.
//!
//! Three more passes after the gazetteer's: the nodes, to learn which fall inside the countries and
//! which carry an address; the ways, for the roads, the addressed buildings and every commune's
//! outline; and the nodes again, for the coordinates of whatever the ways named.
//!
//! Streets are indexed for chosen countries rather than for the planet because a planet of house
//! numbers does not fit in memory on one machine. The gazetteer above them is planet-wide.

use std::collections::{HashMap, HashSet};

use planetrs_pbf::{PbfError, Planet};

use crate::assemble::Assembled;
use crate::file_streets::{FilingReport, StreetSources, file_streets};
use crate::filing::CommuneOutlines;
use crate::gazetteer::MUNICIPALITY_LEVELS;
use crate::harvest::{Harvest, largest_node_id, node_positions, scan_territory_nodes, scan_territory_ways};
use crate::rings::{Outline, join_into_rings};
use crate::shops::{Shop, ShopIndex};
use crate::street_index::StreetIndex;
use crate::territory::{IdentifierSet, TerritoryMask};
use crate::text::{FRENCH, comparison_key};

/// What indexing the streets found, so every gap is a number rather than a surprise.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StreetIndexReport {
    pub address_nodes: usize,
    pub places: usize,
    pub highways: usize,
    pub buildings: usize,
    pub communes_outlined: usize,
    pub former_towns_placed: usize,
    pub shops_read: usize,
    pub shops_filed: usize,
    pub communes_without_outline: usize,
    pub outline_broken_chains: usize,
    pub unreadable_blobs: usize,
    pub filing: FilingReport,
}

/// What indexing the chosen countries produces beside the gazetteer.
pub struct Indexed {
    pub streets: StreetIndex,
    /// Each former town a merger left behind, to the commune that now contains it.
    pub parents: HashMap<i64, i64>,
    /// Each outlined commune's area in square kilometres.
    pub areas: HashMap<i64, f64>,
    pub shops: ShopIndex,
}

/// The street index of the given countries, by ISO code, with a report of what it could not do.
pub fn index_streets(
    planet: &Planet,
    harvest: &Harvest,
    assembled: &Assembled,
    countries: &[&str],
    progress: &dyn Fn(&str),
) -> Result<(Indexed, StreetIndexReport), PbfError> {
    let mut report = StreetIndexReport::default();
    let targets: Vec<&Outline> = assembled
        .outlines
        .iter()
        .filter(|(code, _)| countries.contains(&code.as_str()))
        .map(|(_, outline)| outline)
        .collect();
    let mask = TerritoryMask::of(&targets);
    progress("territory mask drawn");

    let largest = largest_node_id(planet)?;
    let inside = IdentifierSet::up_to(largest);
    let nodes = scan_territory_nodes(planet, &mask, &targets, &inside)?;
    report.address_nodes = nodes.found.addresses.len();
    report.places = nodes.found.places.len();
    report.unreadable_blobs += nodes.unreadable_blobs;
    progress(&format!(
        "nodes: {} addresses, {} hamlets",
        report.address_nodes, report.places
    ));

    let municipalities: Vec<i64> = assembled
        .communes
        .iter()
        .filter(|commune| MUNICIPALITY_LEVELS.contains(&commune.level))
        .filter(|commune| {
            commune
                .country
                .as_deref()
                .is_some_and(|code| countries.contains(&code))
        })
        .map(|commune| commune.relation)
        .collect();
    let wanted_municipalities: HashSet<i64> = municipalities.iter().copied().collect();
    let outline_ways_of: HashMap<i64, &Vec<i64>> = harvest
        .areas
        .iter()
        .filter(|area| wanted_municipalities.contains(&area.relation))
        .map(|area| (area.relation, &area.outline_ways))
        .collect();
    let outline_ways: HashSet<i64> = outline_ways_of
        .values()
        .flat_map(|ways| ways.iter().copied())
        .collect();

    let ways = scan_territory_ways(planet, &inside, &outline_ways)?;
    drop(inside);
    report.highways = ways.found.highways.len();
    report.buildings = ways.found.buildings.len();
    report.unreadable_blobs += ways.unreadable_blobs;
    progress(&format!(
        "ways: {} roads, {} addressed buildings, {} of {} outline ways",
        report.highways,
        report.buildings,
        ways.found.outlines.len(),
        outline_ways.len()
    ));

    let wanted = IdentifierSet::up_to(largest);
    for references in ways
        .found
        .outlines
        .values()
        .chain(ways.found.highways.iter().map(|road| &road.references))
        .chain(ways.found.buildings.iter().map(|building| &building.references))
        .chain(ways.found.shops.iter().map(|shop| &shop.references))
    {
        for id in references {
            wanted.insert(*id);
        }
    }
    let positions = node_positions(planet, &wanted)?;
    drop(wanted);
    report.unreadable_blobs += positions.unreadable_blobs;
    progress(&format!("positions: {} nodes", positions.found.len()));

    let mut outlines = Vec::with_capacity(municipalities.len());
    let mut areas: HashMap<i64, f64> = HashMap::new();
    for relation in &municipalities {
        let Some(way_ids) = outline_ways_of.get(relation) else {
            continue;
        };
        let members: Vec<Vec<i64>> = way_ids
            .iter()
            .filter_map(|id| ways.found.outlines.get(id).cloned())
            .collect();
        let joined = join_into_rings(&members);
        report.outline_broken_chains += joined.broken_chains;
        let rings: Vec<Vec<[f64; 2]>> = joined
            .rings
            .iter()
            .filter_map(|ring| ring.iter().map(|id| positions.found.get(id).copied()).collect())
            .collect();
        let area = crate::rings::area_square_kilometres(&rings);
        let outline = Outline::of(rings);
        if !outline.is_empty() {
            areas.insert(*relation, area);
        }
        if outline.is_empty() {
            report.communes_without_outline += 1;
        } else {
            outlines.push((*relation, outline));
        }
    }
    report.communes_outlined = outlines.len();
    let communes = CommuneOutlines::of(outlines);

    let level_of: HashMap<i64, u8> = assembled
        .communes
        .iter()
        .map(|commune| (commune.relation, commune.level))
        .collect();
    let parents: HashMap<i64, i64> = assembled
        .communes
        .iter()
        .filter(|commune| {
            commune.level == MUNICIPALITY_LEVELS[1] && wanted_municipalities.contains(&commune.relation)
        })
        .filter_map(|town| {
            let centre = town.centre?;
            communes
                .containing(centre)
                .into_iter()
                .find(|relation| {
                    *relation != town.relation && level_of.get(relation) == Some(&MUNICIPALITY_LEVELS[0])
                })
                .map(|commune| (town.relation, commune))
        })
        .collect();
    report.former_towns_placed = parents.len();
    progress(&format!(
        "outlines: {} communes, {} without",
        report.communes_outlined, report.communes_without_outline
    ));

    let sources = StreetSources {
        addresses: &nodes.found.addresses,
        places: &nodes.found.places,
        highways: &ways.found.highways,
        buildings: &ways.found.buildings,
        associated: &harvest.associated_streets,
        positions: &positions.found,
    };
    let (filed, filing) = file_streets(&sources, &communes);
    report.filing = filing;
    let shop_points = nodes
        .found
        .shops
        .iter()
        .map(|shop| (Some(shop.position), &shop.names, &shop.street))
        .chain(ways.found.shops.iter().map(|shop| {
            let points: Vec<[f64; 2]> = shop
                .references
                .iter()
                .filter_map(|id| positions.found.get(id).copied())
                .collect();
            (middle_of(&points), &shop.names, &shop.street)
        }));
    let mut shops = Vec::new();
    for (position, names, street) in shop_points {
        report.shops_read += 1;
        let Some(position) = position else { continue };
        let containing = communes.containing(position);
        if !containing.is_empty() {
            report.shops_filed += 1;
        }
        let shop = Shop {
            names: names.iter().map(|name| comparison_key(name, &FRENCH)).collect(),
            street: comparison_key(street, &FRENCH),
            position,
        };
        shops.extend(containing.into_iter().map(|commune| (commune, shop.clone())));
    }

    Ok((
        Indexed {
            streets: StreetIndex::of(filed),
            parents,
            areas,
            shops: ShopIndex::of(shops),
        },
        report,
    ))
}

fn middle_of(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    let [x, y] = points.iter().fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
        [x + longitude, y + latitude]
    });
    Some([x / count, y / count])
}
