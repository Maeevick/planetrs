//! The outlines of the countries asked for, read from the planet by the rule every part of a dataset
//! follows, so that a node of the map index falls in the country the geocoder would file it under.

use std::collections::{BTreeMap, HashMap, HashSet};

use planetrs_dataset::territory::{country_code, outline_of};
use planetrs_geo::Outline;
use planetrs_geo::territory::{Coverage, TerritoryMask};
use planetrs_pbf::{Planet, Sections, dense_group, visit_relations, visit_ways};

use super::scan::{every_blob, scan};
use super::text;
use crate::error::ClipError;

/// The countries asked for, by code, with what tells cheaply and then exactly whether a point lies
/// in one.
pub(super) struct Countries {
    outlines: Vec<(String, Outline)>,
    mask: TerritoryMask,
}

impl Countries {
    /// The countries' codes, in the order their files are written.
    pub(super) fn codes(&self) -> impl Iterator<Item = &str> {
        self.outlines.iter().map(|(code, _)| code.as_str())
    }

    /// Where a box cut from the countries' map index lacks nothing the planet holds.
    pub(super) fn coverage(&self) -> Coverage {
        let outlines: Vec<&Outline> = self.outlines.iter().map(|(_, outline)| outline).collect();
        Coverage::of(&outlines)
    }

    /// Whether one of the countries holds a longitude and latitude.
    pub(super) fn hold(&self, point: [f64; 2]) -> bool {
        self.mask.might_hold(point) && self.outlines.iter().any(|(_, outline)| outline.contains(point))
    }
}

/// Reads the outlines of `codes` in three passes: the boundary relations, then their ways, then the
/// ways' nodes.
pub(super) fn countries_in(
    planet: &Planet,
    sections: &Sections,
    codes: &[&str],
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Countries, ClipError> {
    let outline_ways: BTreeMap<String, Vec<i64>> = scan(
        planet,
        &every_blob(sections.relations.clone()),
        "countries",
        progress,
        BTreeMap::new,
        |found: &mut BTreeMap<String, Vec<i64>>, decoded| {
            for group in &decoded.groups {
                visit_relations(group, &mut |relation| {
                    let tags: Vec<(String, String)> = relation
                        .tags(decoded)
                        .into_iter()
                        .map(|(key, value)| (text(key), text(value)))
                        .collect();
                    if let Some(code) = country_code(&tags).filter(|code| codes.contains(&code.as_str())) {
                        found
                            .entry(code)
                            .or_default()
                            .extend(relation.ways(decoded).into_iter().map(|(way, _)| way));
                    }
                    Ok(())
                })?;
            }
            Ok(None)
        },
        |mut left, right| {
            for (code, ways) in right {
                left.entry(code).or_default().extend(ways);
            }
            left
        },
    )?
    .found;

    let wanted_ways: HashSet<i64> = outline_ways.values().flatten().copied().collect();
    let refs: HashMap<i64, Vec<i64>> = scan(
        planet,
        &every_blob(sections.ways.clone()),
        "country outlines",
        progress,
        HashMap::new,
        |found: &mut HashMap<i64, Vec<i64>>, decoded| {
            for group in &decoded.groups {
                visit_ways(group, &mut |way| {
                    if wanted_ways.contains(&way.id) {
                        found.insert(way.id, way.refs().collect::<Result<Vec<_>, _>>()?);
                    }
                    Ok(())
                })?;
            }
            Ok(None)
        },
        |mut left, right| {
            left.extend(right);
            left
        },
    )?
    .found;

    let wanted_nodes: HashSet<i64> = refs.values().flatten().copied().collect();
    let positions: HashMap<i64, [f64; 2]> = scan(
        planet,
        &every_blob(sections.nodes.clone()),
        "country corners",
        progress,
        HashMap::new,
        |found: &mut HashMap<i64, [f64; 2]>, decoded| {
            for group in &decoded.groups {
                if let Some(dense) = dense_group(group)? {
                    dense.visit(&mut |id, latitude, longitude| {
                        if wanted_nodes.contains(&id) {
                            found.insert(id, [decoded.longitude(longitude), decoded.latitude(latitude)]);
                        }
                    })?;
                }
            }
            Ok(None)
        },
        |mut left, right| {
            left.extend(right);
            left
        },
    )?
    .found;

    let mut sorted: Vec<&str> = codes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let outlines = sorted
        .into_iter()
        .map(|code| {
            let ways: Vec<Vec<i64>> = outline_ways
                .get(code)
                .into_iter()
                .flatten()
                .filter_map(|way| refs.get(way).cloned())
                .collect();
            let outline = outline_of(&ways, |id| positions.get(&id).copied()).outline;
            if outline.is_empty() {
                return Err(ClipError::UnknownCountry {
                    country: code.to_string(),
                });
            }
            Ok((code.to_string(), outline))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mask = TerritoryMask::of(&outlines.iter().map(|(_, outline)| outline).collect::<Vec<_>>());
    Ok(Countries { outlines, mask })
}
