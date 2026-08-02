//! The communes of the planet, and finding the ones a town name can mean.
//!
//! A town name on its own is ambiguous in two ways. Across borders, `SAINT DENIS` is in France,
//! Réunion and Québec, which is why only communes inside the asked country are considered. Within a
//! country, four communes are called `MARIGNY`, and with nothing but the name the honest answer is
//! the point between them. Telling those apart is what the postcode is for, and it is a later step.

use std::collections::HashMap;

use crate::boundary::Country;
use crate::similarity::edit_distance_within;
use crate::text::{FRENCH, comparison_key, strip_arrondissement};

/// The administrative levels that mean "municipality" in most of the world, and in France the
/// commune and the delegated commune a merger left behind.
pub const MUNICIPALITY_LEVELS: [u8; 2] = [8, 9];

/// A municipality as the gazetteer holds it: its names, where its centre is, and which country
/// contains that centre.
#[derive(Debug, Clone, PartialEq)]
pub struct Commune {
    /// The OpenStreetMap relation of its boundary, which identifies it everywhere in the index.
    pub relation: i64,
    /// Its `admin_level`: 8 for a commune, 9 for a delegated commune a merger left behind.
    pub level: u8,
    /// Every name it answers to, as comparison keys.
    pub names: Vec<String>,
    /// The postcodes its boundary carries.
    pub postcodes: Vec<String>,
    /// Longitude and latitude of the relation's `admin_centre` or `label` node.
    pub centre: Option<[f64; 2]>,
    /// ISO 3166-1 code of the country whose outline contains the centre.
    pub country: Option<String>,
}

/// How much of a town name had to be forgiven for a commune to answer to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMatch {
    Exact,
    ContainedWords,
    Misspelled,
}

/// What settled the commune an answer was searched in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Basis {
    /// The town name alone, averaged over every commune answering to it.
    NameOnly,
    /// A commune that answers to the name and carries the postcode.
    NameAndPostcode,
    /// The communes the postcode covers, when no commune answers to the town name.
    PostcodeOnly,
}

/// Where a site points, and how many communes had to be averaged to say so.
#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    /// Longitude and latitude.
    pub position: [f64; 2],
    pub communes: usize,
    /// The relations of the communes the answer was drawn from, so the next step can search them.
    pub relations: Vec<i64>,
    pub name_match: Option<NameMatch>,
    pub basis: Basis,
}

const TIERS: [NameMatch; 3] = [NameMatch::Exact, NameMatch::ContainedWords, NameMatch::Misspelled];

pub struct Gazetteer {
    communes: Vec<Commune>,
    by_name: HashMap<String, Vec<usize>>,
    by_country: HashMap<String, Vec<usize>>,
    by_postcode: HashMap<(String, String), Vec<usize>>,
    countries: Vec<Country>,
    /// Each former town a merger left behind, by relation, to the commune that now contains it.
    parents: HashMap<i64, i64>,
    /// Each outlined commune's area in square kilometres, by relation.
    areas: HashMap<i64, f64>,
}

impl Gazetteer {
    pub fn of(communes: Vec<Commune>, countries: Vec<Country>) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_country: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_postcode: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (at, commune) in communes.iter().enumerate() {
            if !MUNICIPALITY_LEVELS.contains(&commune.level) || commune.centre.is_none() {
                continue;
            }
            for name in &commune.names {
                by_name.entry(name.clone()).or_default().push(at);
            }
            if let Some(country) = &commune.country {
                by_country.entry(country.clone()).or_default().push(at);
                for postcode in &commune.postcodes {
                    by_postcode
                        .entry((country.clone(), postcode.clone()))
                        .or_default()
                        .push(at);
                }
            }
        }
        Self {
            communes,
            by_name,
            by_country,
            by_postcode,
            countries,
            parents: HashMap::new(),
            areas: HashMap::new(),
        }
    }

    pub fn communes(&self) -> &[Commune] {
        &self.communes
    }

    /// The ISO code of the country a site's country field names, in any spelling it is mapped
    /// under.
    pub fn country_code_for(&self, asked: &str) -> Option<&str> {
        let asked = comparison_key(asked, &FRENCH);
        self.countries
            .iter()
            .find(|country| country.names.contains(&asked))
            .map(|country| country.code.as_str())
    }

    fn in_country<'a>(&'a self, indices: &'a [usize], code: &'a str) -> impl Iterator<Item = usize> + 'a {
        indices
            .iter()
            .copied()
            .filter(move |at| self.communes[*at].country.as_deref() == Some(code))
    }

    fn asked_key(town: &str) -> Option<String> {
        let asked = strip_arrondissement(&comparison_key(town, &FRENCH));
        (!asked.is_empty()).then_some(asked)
    }

    fn answers_to(&self, at: usize, asked: &str, tier: NameMatch) -> bool {
        let names = &self.communes[at].names;
        match tier {
            NameMatch::Exact => names.iter().any(|name| name == asked),
            NameMatch::ContainedWords => names
                .iter()
                .any(|name| contains_words_in_order(name, asked) || contains_words_in_order(asked, name)),
            NameMatch::Misspelled => {
                let budget = if asked.len() < 12 { 1 } else { 2 };
                names
                    .iter()
                    .any(|name| edit_distance_within(asked, name, budget).is_some())
            }
        }
    }

    /// The communes of one country answering to a town name, at the strictest tier that finds any.
    pub fn communes_named(&self, town: &str, code: &str) -> Option<(NameMatch, Vec<&Commune>)> {
        let asked = Self::asked_key(town)?;

        let exact: Vec<usize> = self
            .by_name
            .get(&asked)
            .map(|found| self.in_country(found, code).collect())
            .unwrap_or_default();
        if !exact.is_empty() {
            return Some((NameMatch::Exact, self.resolve(exact)));
        }

        let pool = self.by_country.get(code)?;
        TIERS[1..].iter().find_map(|tier| {
            let found: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|at| self.answers_to(*at, &asked, *tier))
                .collect();
            (!found.is_empty()).then(|| (*tier, self.resolve(found)))
        })
    }

    /// Says which commune each former town now belongs to, as the index measured it from outlines.
    pub fn with_parents(mut self, parents: HashMap<i64, i64>) -> Self {
        self.parents = parents;
        self
    }

    /// Says how large each outlined commune is, as the index measured it.
    pub fn with_areas(mut self, areas: HashMap<i64, f64>) -> Self {
        self.areas = areas;
        self
    }

    /// The commune's area in square kilometres, when the index outlined it.
    pub fn area_of(&self, relation: i64) -> Option<f64> {
        self.areas.get(&relation).copied()
    }

    /// The former towns inside the given communes that carry a postcode, by relation.
    ///
    /// A commune nouvelle keeps each town it absorbed as a delegated commune, and each keeps its own
    /// postcode and its own streets. When the postcode names one of them, it says which town a street
    /// name means inside the merged commune. Only towns inside the settled communes count: a postcode
    /// shared with a neighbouring merged commune once sent a site to that neighbour's Rue des Érables,
    /// 12 km away.
    pub fn former_communes_at(&self, postcode: &str, country: &str, within: &[i64]) -> Vec<i64> {
        let Some(code) = self.country_code_for(country) else {
            return Vec::new();
        };
        self.communes_at_postcode(postcode, code)
            .iter()
            .map(|at| &self.communes[*at])
            .filter(|commune| commune.level == MUNICIPALITY_LEVELS[1])
            .filter(|commune| {
                self.parents
                    .get(&commune.relation)
                    .is_some_and(|parent| within.contains(parent))
            })
            .map(|commune| commune.relation)
            .collect()
    }

    /// The communes of one country carrying a postcode.
    pub fn communes_at_postcode(&self, postcode: &str, code: &str) -> &[usize] {
        self.by_postcode
            .get(&(code.to_string(), postcode.trim().to_string()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn resolve(&self, mut indices: Vec<usize>) -> Vec<&Commune> {
        indices.sort_unstable();
        indices.dedup();
        indices.into_iter().map(|at| &self.communes[at]).collect()
    }
}

/// Whether every word of `shorter` appears in `longer`, in the same order.
///
/// `HOSSEGOR` is `SOORTS HOSSEGOR` and `MAREUIL PERIGORD` is `MAREUIL EN PERIGORD`. Testing only the
/// start of a name misses the first, which once turned a real site in the Landes into a reported
/// postcode error.
pub(crate) fn contains_words_in_order(longer: &str, shorter: &str) -> bool {
    let mut wanted = shorter.split_whitespace().peekable();
    if wanted.peek().is_none() {
        return false;
    }
    for word in longer.split_whitespace() {
        if wanted.peek() == Some(&word) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}

/// Step one: where a town name points when nothing but the name and the country is read.
///
/// Every commune of that country answering to the name is averaged, because with the name alone
/// there is no way to prefer one of them, and choosing would be a guess dressed as an answer.
pub fn locate_by_town(gazetteer: &Gazetteer, town: &str, country: &str) -> Option<Located> {
    let code = gazetteer.country_code_for(country)?;
    let (name_match, communes) = gazetteer.communes_named(town, code)?;
    averaged(&communes, Some(name_match), Basis::NameOnly)
}

fn averaged(communes: &[&Commune], name_match: Option<NameMatch>, basis: Basis) -> Option<Located> {
    let placed: Vec<&&Commune> = communes
        .iter()
        .filter(|commune| commune.centre.is_some())
        .collect();
    if placed.is_empty() {
        return None;
    }
    let count = placed.len() as f64;
    let [longitude, latitude] = placed
        .iter()
        .filter_map(|commune| commune.centre)
        .fold([0.0, 0.0], |[x, y], [longitude, latitude]| {
            [x + longitude, y + latitude]
        });
    Some(Located {
        position: [longitude / count, latitude / count],
        communes: placed.len(),
        relations: placed.iter().map(|commune| commune.relation).collect(),
        name_match,
        basis,
    })
}

/// Step two: the postcode picks one commune out of those answering to the town name.
///
/// The postcode is searched first, because it is the narrower of the two: a town name repeats
/// across a country, a postcode rarely spans more than a handful of communes. Within it the strictest
/// name tier wins, so `SENS` at 89100 is Sens and not Saint-Denis-lès-Sens next door.
///
/// When the postcode and the name disagree and the name is a real commune, the name decides, since
/// five mistyped digits are likelier than a wrong town. When the name is no commune at all, a hamlet
/// or a resort, the communes the postcode covers are the answer.
pub fn locate_by_town_and_postcode(
    gazetteer: &Gazetteer,
    town: &str,
    postcode: &str,
    country: &str,
) -> Option<Located> {
    let code = gazetteer.country_code_for(country)?.to_string();
    let pool = gazetteer.communes_at_postcode(postcode, &code);

    if let Some(asked) = Gazetteer::asked_key(town) {
        for tier in TIERS {
            let matched: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|at| gazetteer.answers_to(*at, &asked, tier))
                .collect();
            if !matched.is_empty() {
                let chosen = nearer_subdivision(gazetteer, &matched, pool, &asked);
                let communes: Vec<&Commune> = chosen.iter().map(|at| &gazetteer.communes[*at]).collect();
                return averaged(&communes, Some(tier), Basis::NameAndPostcode);
            }
        }
    }

    if let Some(namesakes) = namesakes_worth_the_postcode(gazetteer, town, postcode, &code, pool.is_empty()) {
        return averaged(&namesakes, Some(NameMatch::Exact), Basis::NameOnly);
    }

    if pool.is_empty() {
        return locate_by_town(gazetteer, town, country);
    }

    let municipalities: Vec<&Commune> = pool
        .iter()
        .map(|at| &gazetteer.communes[*at])
        .filter(|commune| commune.level == MUNICIPALITY_LEVELS[0])
        .collect();
    let communes: Vec<&Commune> = if municipalities.is_empty() {
        pool.iter().map(|at| &gazetteer.communes[*at]).collect()
    } else {
        municipalities
    };
    averaged(&communes, None, Basis::PostcodeOnly)
}

/// How many leading characters of a postcode name its region: the département in France, and
/// something comparable in most European schemes.
const POSTAL_REGION: usize = 2;

/// Communes elsewhere bearing exactly the asked name, when they are more believable than the
/// postcode.
///
/// A name matching exactly is a real commune, and whoever typed it is likelier to mistype five digits than
/// to invent a town, so it wins over the postcode when both point to the same region: 02200
/// CHATEAU THIERRY is Château-Thierry with a slipped digit. It loses when it is in another region,
/// because then the name is more likely a district that happens to share a commune's name: 51100 LA
/// NEUVILLETTE is a part of Reims, not the Neuvillette two départements away. When the postcode does
/// not exist at all there is nothing to weigh it against, and the name stands.
///
/// Only exact names are trusted this far from the postcode. A fuzzy match across a whole country lands
/// anywhere: it once put Agay, a resort of Saint-Raphaël, 706 km away in the Paris suburbs.
fn namesakes_worth_the_postcode<'a>(
    gazetteer: &'a Gazetteer,
    town: &str,
    postcode: &str,
    code: &str,
    postcode_is_unknown: bool,
) -> Option<Vec<&'a Commune>> {
    let (name_match, namesakes) = gazetteer.communes_named(town, code)?;
    if name_match != NameMatch::Exact {
        return None;
    }
    if postcode_is_unknown {
        return Some(namesakes);
    }
    let region = postcode.trim().get(..POSTAL_REGION)?;
    let nearby: Vec<&Commune> = namesakes
        .into_iter()
        .filter(|commune| commune.postcodes.iter().any(|theirs| theirs.starts_with(region)))
        .collect();
    (!nearby.is_empty()).then_some(nearby)
}

/// Whether a name reads as a numbered part of a city: the city's name, then an ordinal.
///
/// `PARIS 11E ARRONDISSEMENT` is a part of Paris. `VITRY CLUNY` contains the word Cluny and is a
/// neighbouring village, which is why containing the name is not enough.
fn is_numbered_part_of(name: &str, city: &str) -> bool {
    name.strip_prefix(city)
        .and_then(|rest| rest.strip_prefix(' '))
        .is_some_and(|rest| rest.starts_with(|character: char| character.is_ascii_digit()))
}

/// A city answering by name is replaced by the subdivision of it the postcode names.
///
/// Paris, Lyon and Marseille are one commune each and twenty, nine and sixteen arrondissements, and
/// the arrondissement carrying the asked postcode is several kilometres nearer the site than the
/// city hall is.
fn nearer_subdivision(gazetteer: &Gazetteer, matched: &[usize], pool: &[usize], asked: &str) -> Vec<usize> {
    let finest = MUNICIPALITY_LEVELS[1];
    if matched.iter().any(|at| gazetteer.communes[*at].level == finest) {
        return matched.to_vec();
    }
    let subdivisions: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|at| gazetteer.communes[*at].level == finest)
        .filter(|at| {
            gazetteer.communes[*at]
                .names
                .iter()
                .any(|name| is_numbered_part_of(name, asked))
        })
        .collect();
    if subdivisions.is_empty() {
        matched.to_vec()
    } else {
        subdivisions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Country;

    fn commune(
        relation: i64,
        level: u8,
        names: &[&str],
        longitude: f64,
        latitude: f64,
        country: &str,
    ) -> Commune {
        Commune {
            relation,
            level,
            names: names.iter().map(|name| (*name).to_string()).collect(),
            postcodes: vec![],
            centre: Some([longitude, latitude]),
            country: Some(country.to_string()),
        }
    }

    fn france_and_canada() -> Vec<Country> {
        vec![
            Country {
                code: "FR".into(),
                names: vec!["FRANCE".into(), "REPUBLIQUE FRANCAISE".into()],
            },
            Country {
                code: "CA".into(),
                names: vec!["CANADA".into()],
            },
        ]
    }

    fn located(gazetteer: &Gazetteer, town: &str, country: &str) -> Option<[f64; 2]> {
        locate_by_town(gazetteer, town, country).map(|found| found.position)
    }

    #[test]
    fn given_one_commune_of_that_name_when_located_then_its_centre_is_the_answer() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["SENS"], 3.28, 48.20, "FR")],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "SENS", "FRANCE"), Some([3.28, 48.20]));
    }

    /// This is the limit of knowing only a name: two places answer to it and the honest answer is
    /// the point between them, which is exactly what the next step exists to improve on.
    #[test]
    fn given_two_communes_sharing_a_name_when_located_then_the_answer_is_between_them() {
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, &["MARIGNY"], -1.0, 49.0, "FR"),
                commune(2, 8, &["MARIGNY"], 5.0, 47.0, "FR"),
            ],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "MARIGNY", "FRANCE"), Some([2.0, 48.0]));
    }

    #[test]
    fn given_a_namesake_in_another_country_when_located_then_it_is_left_out() {
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, &["SAINT DENIS"], 2.35, 48.93, "FR"),
                commune(2, 8, &["SAINT DENIS"], -73.6, 45.5, "CA"),
            ],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "ST DENIS", "FRANCE"), Some([2.35, 48.93]));
    }

    #[test]
    fn given_an_exact_name_and_a_longer_one_containing_it_when_located_then_the_exact_one_wins() {
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, &["SENS"], 3.28, 48.20, "FR"),
                commune(2, 8, &["SENS BRETAGNE"], -1.53, 48.33, "FR"),
            ],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "SENS", "FRANCE"), Some([3.28, 48.20]));
    }

    #[test]
    fn given_a_town_named_by_part_of_its_commune_when_located_then_the_commune_is_found() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["SOORTS HOSSEGOR"], -1.40, 43.66, "FR")],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "HOSSEGOR", "FRANCE"), Some([-1.40, 43.66]));
    }

    #[test]
    fn given_a_misspelled_town_when_located_then_a_one_letter_difference_is_forgiven() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["MONTEREAU FAULT YONNE"], 2.95, 48.38, "FR")],
            france_and_canada(),
        );

        assert_eq!(
            located(&gazetteer, "MONTEREAU FAUT YONNE", "FRANCE"),
            Some([2.95, 48.38])
        );
    }

    #[test]
    fn given_an_arrondissement_suffix_when_located_then_the_city_is_found() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["PARIS"], 2.35, 48.86, "FR")],
            france_and_canada(),
        );

        assert_eq!(
            located(&gazetteer, "PARIS 11EME ARRONDISSEMENT", "FRANCE"),
            Some([2.35, 48.86])
        );
    }

    #[test]
    fn given_a_region_sharing_the_towns_name_when_located_then_only_municipalities_count() {
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, &["PARIS"], 2.35, 48.86, "FR"),
                commune(2, 6, &["PARIS"], 2.00, 48.00, "FR"),
            ],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "PARIS", "FRANCE"), Some([2.35, 48.86]));
    }

    #[test]
    fn given_a_commune_with_no_centre_when_located_then_it_cannot_be_averaged_and_is_skipped() {
        let mut without = commune(2, 8, &["MARIGNY"], 0.0, 0.0, "FR");
        without.centre = None;
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["MARIGNY"], -1.0, 49.0, "FR"), without],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "MARIGNY", "FRANCE"), Some([-1.0, 49.0]));
    }

    #[test]
    fn given_a_country_nobody_recognises_when_located_then_nothing_is_returned() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["SENS"], 3.28, 48.20, "FR")],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "SENS", "ATLANTIS"), None);
    }

    #[test]
    fn given_a_town_no_commune_answers_to_when_located_then_nothing_is_returned() {
        let gazetteer = Gazetteer::of(
            vec![commune(1, 8, &["SENS"], 3.28, 48.20, "FR")],
            france_and_canada(),
        );

        assert_eq!(located(&gazetteer, "NULLE PART", "FRANCE"), None);
    }

    #[test]
    fn given_a_countrys_official_name_when_asked_then_it_resolves_to_its_code() {
        let gazetteer = Gazetteer::of(vec![], france_and_canada());

        assert_eq!(gazetteer.country_code_for("République française"), Some("FR"));
    }

    fn with_postcodes(mut commune: Commune, postcodes: &[&str]) -> Commune {
        commune.postcodes = postcodes.iter().map(|code| (*code).to_string()).collect();
        commune
    }

    fn located_with_postcode(
        gazetteer: &Gazetteer,
        town: &str,
        postcode: &str,
        country: &str,
    ) -> Option<[f64; 2]> {
        locate_by_town_and_postcode(gazetteer, town, postcode, country).map(|found| found.position)
    }

    #[test]
    fn given_two_namesakes_when_the_postcode_names_one_then_that_one_is_the_answer() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 9, &["MARIGNY"], -1.2, 49.1, "FR"), &["50570"]),
                with_postcodes(commune(2, 8, &["MARIGNY"], 5.6, 46.7, "FR"), &["39130"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "MARIGNY", "50570", "FRANCE"),
            Some([-1.2, 49.1])
        );
    }

    #[test]
    fn given_an_exact_name_and_a_containing_one_at_the_same_postcode_then_the_exact_one_wins() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["SENS"], 3.28, 48.20, "FR"), &["89100"]),
                with_postcodes(
                    commune(2, 8, &["SAINT DENIS SENS"], 3.28, 48.23, "FR"),
                    &["89100"],
                ),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "SENS", "89100", "FRANCE"),
            Some([3.28, 48.20])
        );
    }

    /// OpenStreetMap files Lyon's postcodes on its arrondissements and none on the city itself, so
    /// the city is only reachable through the arrondissement its postcode names.
    #[test]
    fn given_a_city_whose_postcodes_sit_on_its_arrondissements_then_the_arrondissement_answers() {
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, &["LYON"], 4.83, 45.76, "FR"),
                with_postcodes(
                    commune(2, 9, &["LYON 3E ARRONDISSEMENT"], 4.85, 45.76, "FR"),
                    &["69003"],
                ),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "LYON", "69003", "FRANCE"),
            Some([4.85, 45.76])
        );
    }

    #[test]
    fn given_a_city_and_the_arrondissement_its_postcode_names_then_the_arrondissement_is_nearer() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["PARIS"], 2.35, 48.86, "FR"), &["75001", "75011"]),
                with_postcodes(
                    commune(2, 9, &["PARIS 11E ARRONDISSEMENT"], 2.38, 48.86, "FR"),
                    &["75011"],
                ),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "PARIS", "75011", "FRANCE"),
            Some([2.38, 48.86])
        );
    }

    /// The postcode and the town disagree, and the town is a commune. Whoever typed it is likelier
    /// to have mistyped five digits than to have named the wrong town, so the name decides.
    #[test]
    fn given_a_town_none_of_whose_namesakes_has_the_postcode_then_the_name_alone_decides() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["BOURG BRESSE"], 5.22, 46.20, "FR"), &["01000"]),
                with_postcodes(commune(2, 8, &["OYONNAX"], 5.65, 46.26, "FR"), &["01100"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "BOURG EN BRESSE", "01100", "FRANCE"),
            Some([5.22, 46.20])
        );
    }

    #[test]
    fn given_a_town_that_is_no_commune_when_its_postcode_names_one_then_that_commune_answers() {
        let gazetteer = Gazetteer::of(
            vec![with_postcodes(
                commune(1, 8, &["AGDE"], 3.47, 43.31, "FR"),
                &["34300"],
            )],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "LE CAP D AGDE", "34300", "FRANCE"),
            Some([3.47, 43.31])
        );
    }

    #[test]
    fn given_a_town_that_is_no_commune_when_its_postcode_covers_several_then_they_are_averaged() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["ARBENT"], 5.60, 46.30, "FR"), &["01100"]),
                with_postcodes(commune(2, 8, &["OYONNAX"], 5.70, 46.20, "FR"), &["01100"]),
            ],
            france_and_canada(),
        );

        let found =
            located_with_postcode(&gazetteer, "NULLE PART", "01100", "FRANCE").expect("the postcode area");
        assert!((found[0] - 5.65).abs() < 1e-9 && (found[1] - 46.25).abs() < 1e-9);
    }

    /// 08800 is both the Ardennes and Barcelona.
    #[test]
    fn given_the_same_postcode_in_another_country_when_located_then_it_is_left_out() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["MONTHERME"], 4.73, 49.88, "FR"), &["08800"]),
                with_postcodes(commune(2, 8, &["VILANOVA GELTRU"], 1.72, 41.22, "CA"), &["08800"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "NULLE PART", "08800", "FRANCE"),
            Some([4.73, 49.88])
        );
    }

    /// Vitry-lès-Cluny is a delegated commune next to Cluny and shares its postcode. It contains the
    /// word CLUNY and is not a part of Cluny, and treating it as one moved a site 7 km.
    #[test]
    fn given_a_neighbour_whose_name_contains_the_town_when_located_then_it_is_not_taken_as_a_subdivision() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["CLUNY"], 4.658, 46.434, "FR"), &["71250"]),
                with_postcodes(commune(2, 9, &["VITRY CLUNY"], 4.588, 46.491, "FR"), &["71250"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "CLUNY", "71250", "FRANCE"),
            Some([4.658, 46.434])
        );
    }

    /// La Neuvillette is a district of Reims and also a commune in two other départements. The
    /// postcode is Reims's, so a namesake a hundred kilometres away is not what was meant.
    #[test]
    fn given_a_namesake_in_another_region_when_the_postcode_is_real_then_the_postcode_decides() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(commune(1, 8, &["REIMS"], 4.03, 49.26, "FR"), &["51100"]),
                with_postcodes(commune(2, 8, &["NEUVILLETTE"], 2.90, 50.03, "FR"), &["02390"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "LA NEUVILLETTE", "51100", "FRANCE"),
            Some([4.03, 49.26])
        );
    }

    /// A resort that is no commune must not be matched to whichever commune across the country
    /// happens to be one letter away: that is how Agay once landed 706 km off, in the Paris suburbs.
    #[test]
    fn given_a_town_resembling_only_far_away_communes_when_the_postcode_is_real_then_the_postcode_decides() {
        let gazetteer = Gazetteer::of(
            vec![
                with_postcodes(
                    commune(1, 8, &["SAINT RAPHAEL"], 6.77, 43.42, "FR"),
                    &["83530", "83700"],
                ),
                with_postcodes(commune(2, 8, &["AGNY"], 2.76, 50.26, "FR"), &["62217"]),
            ],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "AGAY", "83530", "FRANCE"),
            Some([6.77, 43.42])
        );
    }

    #[test]
    fn given_a_postcode_that_does_not_exist_when_the_town_is_a_commune_then_the_town_decides() {
        let gazetteer = Gazetteer::of(
            vec![with_postcodes(
                commune(1, 8, &["WATTIGNIES"], 3.04, 50.59, "FR"),
                &["59139"],
            )],
            france_and_canada(),
        );

        assert_eq!(
            located_with_postcode(&gazetteer, "WATTIGNIES", "51139", "FRANCE"),
            Some([3.04, 50.59])
        );
    }
}
