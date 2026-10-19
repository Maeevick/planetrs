//! Turning what the passes read into the gazetteer: centres located, outlines closed, and every
//! commune filed under the country that contains it.
//!
//! Pure: the passes have already done the reading, so this can be tested on a four-node square.

use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;

use crate::boundary::Country;
use crate::gazetteer::Commune;
use crate::harvest::Harvest;
use crate::rings::{Outline, join_into_rings};

/// Each way's node identifiers, as the ways pass read them.
pub type WayNodes = HashMap<i64, Vec<i64>>;

/// Each node's longitude and latitude, as the nodes pass read them.
pub type NodePositions = HashMap<i64, [f64; 2]>;

/// What assembling cost and what it could not do, so a gap is reported rather than discovered.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssemblyReport {
    pub communes: usize,
    pub communes_without_centre: usize,
    pub communes_without_country: usize,
    pub countries: usize,
    pub countries_without_outline: Vec<String>,
    pub broken_chains: usize,
    pub rings_missing_nodes: usize,
}

pub struct Assembled {
    pub communes: Vec<Commune>,
    pub countries: Vec<Country>,
    /// Each country's outline, by ISO code, for whoever needs to test a point against it next.
    pub outlines: Vec<(String, Outline)>,
    pub report: AssemblyReport,
}

/// The gazetteer, from the three passes' findings.
pub fn assemble(harvest: &Harvest, ways: &WayNodes, nodes: &NodePositions) -> Assembled {
    let mut report = AssemblyReport::default();

    let mut ways_of_country: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
    let mut names_of_country: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for harvested in &harvest.countries {
        let code = harvested.country.code.clone();
        let known = names_of_country.entry(code.clone()).or_default();
        for name in &harvested.country.names {
            if !known.contains(name) {
                known.push(name.clone());
            }
        }
        let collected = ways_of_country.entry(code).or_default();
        collected.extend(
            harvested
                .outline_ways
                .iter()
                .filter_map(|id| ways.get(id).cloned()),
        );
    }

    let mut outlines: Vec<(String, Outline)> = Vec::new();
    for (code, country_ways) in &ways_of_country {
        let joined = join_into_rings(country_ways);
        report.broken_chains += joined.broken_chains;
        let mut rings = Vec::new();
        for ring in joined.rings {
            let points: Option<Vec<[f64; 2]>> = ring.iter().map(|id| nodes.get(id).copied()).collect();
            match points {
                Some(points) => rings.push(points),
                None => report.rings_missing_nodes += 1,
            }
        }
        let outline = Outline::of(rings);
        if outline.is_empty() {
            report.countries_without_outline.push(code.clone());
        } else {
            outlines.push((code.clone(), outline));
        }
    }

    let communes: Vec<Commune> = harvest
        .areas
        .par_iter()
        .map(|harvested| {
            let centre = harvested.centre_node.and_then(|id| nodes.get(&id).copied());
            let country = centre.and_then(|point| {
                outlines
                    .iter()
                    .find(|(_, outline)| outline.contains(point))
                    .map(|(code, _)| code.clone())
            });
            Commune {
                relation: harvested.relation,
                level: harvested.area.level,
                names: harvested.area.names.clone(),
                postcodes: harvested.area.postcodes.clone(),
                centre,
                country,
            }
        })
        .collect();

    report.communes = communes.len();
    report.communes_without_centre = communes.iter().filter(|c| c.centre.is_none()).count();
    report.communes_without_country = communes
        .iter()
        .filter(|c| c.centre.is_some() && c.country.is_none())
        .count();
    report.countries = names_of_country.len();

    let countries = names_of_country
        .into_iter()
        .map(|(code, names)| Country { code, names })
        .collect();
    Assembled {
        communes,
        countries,
        outlines,
        report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::{AdministrativeArea, Country};
    use crate::harvest::{Harvest, HarvestedArea, HarvestedCountry};

    fn area(relation: i64, level: u8, name: &str, centre: Option<i64>) -> HarvestedArea {
        HarvestedArea {
            relation,
            area: AdministrativeArea {
                level,
                names: vec![name.into()],
                reference: None,
                postcodes: vec!["89100".into()],
            },
            centre_node: centre,
            outline_ways: vec![],
        }
    }

    fn square_country(code: &str, relation: i64, way: i64) -> HarvestedCountry {
        HarvestedCountry {
            relation,
            country: Country {
                code: code.into(),
                names: vec![code.into()],
            },
            outline_ways: vec![way],
        }
    }

    fn fixture() -> (Harvest, WayNodes, NodePositions) {
        let harvest = Harvest {
            areas: vec![
                area(1, 8, "SENS", Some(100)),
                area(2, 8, "NOWHERE", Some(101)),
                area(3, 8, "NO CENTRE", None),
            ],
            countries: vec![square_country("FR", 10, 500)],
            associated_streets: vec![],
        };
        let ways = HashMap::from([(500, vec![1, 2, 3, 4, 1])]);
        let nodes = HashMap::from([
            (1, [0.0, 40.0]),
            (2, [10.0, 40.0]),
            (3, [10.0, 50.0]),
            (4, [0.0, 50.0]),
            (100, [3.28, 48.20]),
            (101, [-60.0, 10.0]),
        ]);
        (harvest, ways, nodes)
    }

    #[test]
    fn given_a_commune_whose_centre_is_inside_a_country_when_assembled_then_it_is_filed_under_it() {
        let (harvest, ways, nodes) = fixture();

        let built = assemble(&harvest, &ways, &nodes);

        let sens = built.communes.iter().find(|c| c.relation == 1).expect("Sens");
        assert_eq!(sens.country.as_deref(), Some("FR"));
        assert_eq!(sens.centre, Some([3.28, 48.20]));
    }

    #[test]
    fn given_a_commune_outside_every_country_when_assembled_then_it_is_filed_under_none() {
        let (harvest, ways, nodes) = fixture();

        let built = assemble(&harvest, &ways, &nodes);

        let nowhere = built.communes.iter().find(|c| c.relation == 2).expect("Nowhere");
        assert_eq!(nowhere.country, None);
    }

    #[test]
    fn given_a_commune_with_no_centre_node_when_assembled_then_it_is_kept_without_a_position() {
        let (harvest, ways, nodes) = fixture();

        let built = assemble(&harvest, &ways, &nodes);

        let without = built
            .communes
            .iter()
            .find(|c| c.relation == 3)
            .expect("No centre");
        assert_eq!(without.centre, None);
        assert_eq!(built.report.communes_without_centre, 1);
    }

    /// A ring missing a node cannot be drawn, and closing it by skipping the node would move the
    /// border. It is dropped and counted, so a country with a gap is visible rather than wrong.
    #[test]
    fn given_an_outline_missing_a_node_when_assembled_then_the_ring_is_dropped_and_counted() {
        let (harvest, ways, mut nodes) = fixture();
        nodes.remove(&3);

        let built = assemble(&harvest, &ways, &nodes);

        assert_eq!(built.report.rings_missing_nodes, 1);
        assert_eq!(built.report.countries_without_outline, vec!["FR".to_string()]);
    }

    #[test]
    fn given_the_harvest_when_assembled_then_every_country_is_kept_for_name_lookup() {
        let (harvest, ways, nodes) = fixture();

        let built = assemble(&harvest, &ways, &nodes);

        assert_eq!(built.countries.len(), 1);
    }
}
