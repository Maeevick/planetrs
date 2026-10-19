//! The whole lookup: the commune, then the street, then the door.
//!
//! Each stage narrows the one before and falls back on it when it cannot, so a site is never placed
//! worse than the commune settled for it: a street not found leaves the commune's centre, and a
//! number not found leaves the street.

pub use crate::gazetteer::Gazetteer;
use crate::gazetteer::{Basis, Located, locate_by_town_and_postcode};
pub use crate::shops::ShopIndex;
use crate::shops::{only_shop_in_communes, shop_on_street};
use crate::street::Quality;
pub use crate::street_index::StreetIndex;
use crate::street_index::{OnStreet, Placed, Reach, locate_on_street_reaching};
use crate::text::{
    FRENCH, HouseNumber, cut_at_spaced_dash, parse_leading_doors, split_leading_house_number,
    without_opening_designators, without_parentheses,
};

/// The fields of a site this geocoder reads, as they were written: nothing needs cleaning first.
///
/// Anything between parentheses, in any field, is an annotation and is ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Site {
    /// The name over the door, `Acme City`, which may name a shop the index holds. May be empty.
    pub label: String,
    /// The street address, door numbers included: `8 Route des Tilleuls`, `5/7 Rue Saint-Exupéry`,
    /// `ZA des Rivières - 46 Rue de Lisbonne`.
    pub address: String,
    /// The postal code, `89100`.
    pub postcode: String,
    /// The town, in any case, with or without accents, hyphens or an arrondissement.
    pub town: String,
    /// The country, by name in a language OpenStreetMap knows it in, or by ISO code.
    pub country: String,
}

impl Site {
    /// The site with every parenthesised annotation removed from every field, which is how step
    /// eight reads it.
    pub(crate) fn without_parentheses(&self) -> Self {
        Self {
            label: without_parentheses(&self.label),
            address: without_parentheses(&self.address),
            postcode: without_parentheses(&self.postcode),
            town: without_parentheses(&self.town),
            country: without_parentheses(&self.country),
        }
    }
}

/// How far down the ladder an answer got, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Precision {
    /// The asked door of the asked street.
    HouseNumber,
    /// The recorded door of the asked street closest to the asked number, when that one is not
    /// recorded.
    NearestHouseNumber,
    /// A shop carrying the site's label, on the asked street or alone of its label in the commune,
    /// or the middle of several within a kilometre of it.
    Shop,
    /// The middle of the asked street, or of the named hamlet, in the settled commune.
    Street,
    /// The centre of the settled commune, or the middle of several when the town names more than one.
    Commune,
}

impl Precision {
    /// The name written in output files: `HouseNumber`, `NearestHouseNumber`, `Shop`, `Street` or
    /// `Commune`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HouseNumber => "HouseNumber",
            Self::NearestHouseNumber => "NearestHouseNumber",
            Self::Shop => "Shop",
            Self::Street => "Street",
            Self::Commune => "Commune",
        }
    }
}

impl std::fmt::Display for Precision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Where a site was placed, how precisely, and on what.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Answer {
    /// Longitude and latitude, in that order; [`Answer::latitude`] and [`Answer::longitude`] say
    /// which is which.
    pub position: [f64; 2],
    /// How far down the ladder the answer got.
    pub precision: Precision,
    /// What settled the commune: its name, its postcode, or both.
    pub basis: Basis,
    /// The communes the answer was searched in, by relation.
    pub communes: Vec<i64>,
    /// The street as recorded, and how much had to be forgiven to match it, when one was.
    pub street: Option<(String, Quality)>,
    /// The address as rewritten to find the street, a word left out or one part kept, when the
    /// address as written found nothing better than the centre.
    pub rewritten_as: Option<String>,
    /// For an answer at a commune's centre, the commune's area in square kilometres, when a single
    /// commune was settled and the index measured it.
    pub commune_area: Option<f64>,
    /// For a shop answer standing for several shops of the label within a kilometre, how many.
    pub shops_averaged: Option<usize>,
}

impl Answer {
    /// The latitude in degrees, north positive.
    pub fn latitude(&self) -> f64 {
        self.position[1]
    }

    /// The longitude in degrees, east positive.
    pub fn longitude(&self) -> f64 {
        self.position[0]
    }
}

/// The parts of an address field worth trying as a street, in the order they were written.
///
/// Commas and slashes separate a retail park from its street, `ZA Les Essarts/Avenue des Pins`, but a
/// slash between the doors a field opens with is a range, `5/7 Rue Gambetta`, and cutting there once
/// threw the 5 away. So a slash is kept when a door number stands on each side of it at the head of
/// the field, and a separator everywhere else: `Grand Ouest 2/30 Avenue des Platanes` is a centre, then a door.
///
/// Step sixteen adds `et` and `and` between two addresses, `5 rue Matisse et 19 rue Degas`, but
/// not between two doors, `8 et 10 Route des Tilleuls`, which is a range.
pub(crate) fn street_candidates(address: &str) -> Vec<&str> {
    address
        .split(',')
        .flat_map(|segment| {
            if slash_joins_doors(segment) {
                vec![segment.trim()]
            } else {
                segment.split('/').map(str::trim).collect()
            }
        })
        .flat_map(split_on_conjunctions)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

/// The words joining two addresses, in French and English.
const CONJUNCTIONS: [&str; 2] = [" et ", " and "];

fn split_on_conjunctions(segment: &str) -> Vec<&str> {
    let bytes = segment.as_bytes();
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let joining = CONJUNCTIONS.iter().find(|conjunction| {
            bytes.len() >= at + conjunction.len()
                && bytes[at..at + conjunction.len()].eq_ignore_ascii_case(conjunction.as_bytes())
        });
        let Some(conjunction) = joining else {
            at += 1;
            continue;
        };
        let before = segment[start..at].trim_end();
        let after = segment[at + conjunction.len()..].trim_start();
        let a_door =
            |word: Option<&str>| word.is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()));
        if a_door(before.split_whitespace().last()) && a_door(after.split_whitespace().next()) {
            at += conjunction.len();
            continue;
        }
        pieces.push(&segment[start..at]);
        start = at + conjunction.len();
        at = start;
    }
    pieces.push(&segment[start..]);
    pieces
}

/// The doors asked for and the street they stand on, as one part of an address field reads them.
pub(crate) type StreetQuery = (Vec<HouseNumber>, String);

/// The doors and street of every part of an address field, in the order they are searched.
///
/// Every part is first searched as it was written. Then, step twelve, each part a designator opens
/// is searched again without it: a zone's name is a place, weaker evidence than a street written
/// beside it, and some roads are named after their zone, so the bare name comes last.
pub(crate) fn street_queries(address: &str) -> Vec<StreetQuery> {
    let as_written = parts_as_written(&[address]);
    let bare = without_designators(&as_written);
    as_written.into_iter().chain(bare).collect()
}

/// The road number each part names, `D712` for `118 RD 712 Les Ponts Neufs`, with its doors.
fn road_numbers(parts: &[StreetQuery]) -> Vec<StreetQuery> {
    let Some(read) = FRENCH.road_number else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|(doors, street)| read(street).map(|number| (doors.clone(), number)))
        .collect()
}

/// Every way step thirteen reads an address field, in the order its exact matches are preferred.
///
/// The field whole, dash included, then the text before its first spaced dash, then the text after
/// it, each split into its parts; then each of those parts again without its opening designators.
/// Reading it whole keeps a street whose name holds the dash, `Avenue de l'Europe - Robert
/// Schuman`, and reading what follows the dash finds the street written after a centre's name, `CC
/// Cap Horizon - Rue Lavandière`.
pub(crate) fn street_readings(address: &str) -> Vec<StreetQuery> {
    let (before, after) = cut_at_spaced_dash(address);
    let texts: Vec<&str> = [Some(address), Some(before.as_str()), after.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let as_written = parts_as_written(&texts);
    let bare = without_designators(&as_written);
    let mut readings: Vec<StreetQuery> = Vec::new();
    for reading in as_written.into_iter().chain(bare) {
        if !readings.contains(&reading) {
            readings.push(reading);
        }
    }
    readings
}

fn parts_as_written(texts: &[&str]) -> Vec<StreetQuery> {
    texts
        .iter()
        .flat_map(|text| street_candidates(text))
        .map(|part| parse_leading_doors(part, &FRENCH))
        .filter(|(_, street)| !street.is_empty())
        .collect()
}

fn without_designators(parts: &[StreetQuery]) -> Vec<StreetQuery> {
    parts
        .iter()
        .filter_map(|(doors, street)| {
            without_opening_designators(street, &FRENCH).map(|bare| (doors.clone(), bare))
        })
        .filter(|(_, street)| !street.is_empty())
        .collect()
}

/// The reading of step thirteen that matches a street exactly, when one does.
///
/// A door beats a nearest door, which beats a street's middle: a door is a building, and two exact
/// readings that disagree are two places the field names. Among equals the earlier reading wins.
pub(crate) fn exact_reading(
    streets: &StreetIndex,
    settled: &[i64],
    former_towns: &[i64],
    readings: &[StreetQuery],
) -> Option<(StreetQuery, OnStreet)> {
    readings
        .iter()
        .enumerate()
        .filter_map(|(order, (doors, street))| {
            on_the_street(streets, settled, former_towns, street, doors, Reach::Tiers)
                .filter(|on| on.quality == Quality::Exact)
                .map(|on| (order, (doors.clone(), street.clone()), on))
        })
        .min_by_key(|(order, _, on)| (on.placed, *order))
        .map(|(_, reading, on)| (reading, on))
}

fn slash_joins_doors(segment: &str) -> bool {
    let Some((before, after)) = segment.split_once('/') else {
        return false;
    };
    let before: Vec<&str> = before.split_whitespace().collect();
    let opens_with_a_door = before
        .first()
        .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()));
    opens_with_a_door && before.len() <= 2 && after.trim_start().starts_with(|c: char| c.is_ascii_digit())
}

/// Step five narrows the street search to the former town the postcode names, inside a merged
/// commune, and widens back to the whole merged commune when that town does not have the street.
///
/// The narrow answer is taken only when it matched at least as strictly as the wide one. A street
/// matching exactly in a neighbouring former town is better evidence than one merely resembling it in
/// the town the postcode names.
fn on_the_street(
    streets: &StreetIndex,
    settled: &[i64],
    former_towns: &[i64],
    street: &str,
    doors: &[HouseNumber],
    reach: Reach,
) -> Option<crate::street_index::OnStreet> {
    let wide = locate_on_street_reaching(streets, settled, street, doors, &FRENCH, reach);
    let narrow = (!former_towns.is_empty())
        .then(|| locate_on_street_reaching(streets, former_towns, street, doors, &FRENCH, reach))
        .flatten();
    match (narrow, wide) {
        (Some(narrow), Some(wide)) if wide.quality < narrow.quality => Some(wide),
        (Some(narrow), _) => Some(narrow),
        (None, wide) => wide,
    }
}

/// Where a site's commune was settled, and the former towns inside it that its postcode names.
pub(crate) struct Settled {
    pub(crate) located: Located,
    pub(crate) former_towns: Vec<i64>,
}

impl Settled {
    /// Every commune whose streets and shops a lookup of the site searches.
    pub(crate) fn searched(&self) -> Vec<i64> {
        self.located
            .relations
            .iter()
            .chain(&self.former_towns)
            .copied()
            .collect()
    }
}

/// Step five settles the commune before any street is looked for, and names the former towns of a
/// merged commune its postcode points at, which is all a lookup needs to know of the gazetteer.
pub(crate) fn settle(gazetteer: &Gazetteer, site: &Site) -> Option<Settled> {
    let site = site.without_parentheses();
    let located = locate_by_town_and_postcode(gazetteer, &site.town, &site.postcode, &site.country)?;
    let former_towns = if located.basis == Basis::NameAndPostcode {
        gazetteer
            .former_communes_at(&site.postcode, &site.country, &located.relations)
            .into_iter()
            .filter(|relation| !located.relations.contains(relation))
            .collect()
    } else {
        Vec::new()
    };
    Some(Settled {
        located,
        former_towns,
    })
}

/// Step nine reads the label once the address has said all it can.
///
/// Step nineteen adds the middle of several shops of the label within a kilometre, and only in
/// place of the commune's centre: every street, rewriting and road number is tried first, since a
/// door on the asked street is a building and the middle of four shops is a guess within a pixel.
///
/// Step thirteen tries every reading of the address, whole, either side of a spaced dash, with and
/// without designators, and keeps an exact match if any reading has one, unless the loose reading
/// already placed the site closer: an exact name is not worth a door already found. The loose reading
/// is the field as written, split into its parts, each matched as loosely as the street tiers allow,
/// then each without its designators; step fourteen stopped cutting it at the dash, since every exact
/// reading either side of the dash has already been tried.
///
/// Step fifteen extends the loose reading once more: when no tier matches any part, the parts are
/// tried again against the rules of [`crate::loose`], which answer only with the one street of the
/// commune that fits. They come after every part has been tried by the tiers, so a street written
/// beside a zone still beats the zone's name, and a street they find at no door gives way to the
/// one shop of the label in the commune.
///
/// An exact door is never second-guessed by a shop. A shop of the label standing on the asked street,
/// by its own `addr:street`, beats a neighbouring door or the street's middle; and when no street was
/// found at all, the only shop of that label in the commune beats the commune's centre. Two shops of
/// one label in one town are left alone: choosing between them would be a guess dressed as a point.
pub fn locate(
    gazetteer: &Gazetteer,
    streets: &StreetIndex,
    shops: &ShopIndex,
    site: &Site,
) -> Option<Answer> {
    let site = &site.without_parentheses();
    let readings = street_readings(&site.address);
    let found = settle(gazetteer, site)?;
    let searched = found.searched();
    let Settled {
        located: settled,
        former_towns,
    } = found;
    let cut = street_queries(&site.address);
    let loose = [Reach::Tiers, Reach::UniqueStreet].into_iter().find_map(|reach| {
        cut.iter().find_map(|(doors, street)| {
            on_the_street(streets, &settled.relations, &former_towns, street, doors, reach)
        })
    });
    let exact = exact_reading(streets, &settled.relations, &former_towns, &readings)
        .filter(|(_, exact)| loose.as_ref().is_none_or(|loose| exact.placed <= loose.placed));
    let parts: Vec<StreetQuery> = exact
        .iter()
        .map(|(reading, _)| reading.clone())
        .chain(cut)
        .collect();

    let on_a_street = exact.map(|(_, on)| on).or(loose).map(|on| {
        let precision = match on.placed {
            Placed::HouseNumber => Precision::HouseNumber,
            Placed::NearestHouseNumber => Precision::NearestHouseNumber,
            Placed::Street => Precision::Street,
        };
        Answer {
            position: on.position,
            precision,
            basis: settled.basis,
            communes: searched.clone(),
            street: Some((on.street, on.quality)),
            rewritten_as: None,
            commune_area: None,
            shops_averaged: None,
        }
    });
    if let Some(answer) = on_a_street
        .as_ref()
        .filter(|answer| answer.precision == Precision::HouseNumber)
    {
        return Some(answer.clone());
    }

    let shop = |found: crate::shops::ShopFound| Answer {
        position: found.position,
        precision: Precision::Shop,
        basis: settled.basis,
        communes: searched.clone(),
        street: None,
        rewritten_as: None,
        commune_area: None,
        shops_averaged: found.averaged,
    };
    if let Some(position) = parts
        .iter()
        .find_map(|(_, street)| shop_on_street(shops, &searched, &site.label, street, &FRENCH))
        .filter(|found| found.averaged.is_none())
    {
        return Some(shop(position));
    }
    let only_shop = || {
        only_shop_in_communes(shops, &searched, &site.label, &FRENCH).filter(|found| found.averaged.is_none())
    };
    if let Some(answer) = on_a_street {
        let found_loosely = answer
            .street
            .as_ref()
            .is_some_and(|(_, quality)| is_loose(*quality));
        if found_loosely && let Some(position) = only_shop() {
            return Some(shop(position));
        }
        return Some(answer);
    }
    if let Some(position) = only_shop() {
        return Some(shop(position));
    }
    if let Some((on, rewriting)) = rewritten(streets, &settled.relations, &former_towns, &site.address) {
        return Some(Answer {
            position: on.position,
            precision: precision_of(on.placed),
            basis: settled.basis,
            communes: searched.clone(),
            street: Some((on.street, on.quality)),
            rewritten_as: Some(rewriting),
            commune_area: None,
            shops_averaged: None,
        });
    }
    if let Some((on, number)) = on_a_numbered_road(streets, &settled.relations, &former_towns, &site.address)
    {
        return Some(Answer {
            position: on.position,
            precision: precision_of(on.placed),
            basis: settled.basis,
            communes: searched.clone(),
            street: Some((on.street, on.quality)),
            rewritten_as: Some(number),
            commune_area: None,
            shops_averaged: None,
        });
    }
    if let Some(found) = only_shop_in_communes(shops, &searched, &site.label, &FRENCH) {
        return Some(shop(found));
    }

    let commune_area = match settled.relations[..] {
        [only] => gazetteer.area_of(only),
        _ => None,
    };
    Some(Answer {
        position: settled.position,
        precision: Precision::Commune,
        basis: settled.basis,
        communes: settled.relations,
        street: None,
        rewritten_as: None,
        commune_area,
        shops_averaged: None,
    })
}

fn precision_of(placed: Placed) -> Precision {
    match placed {
        Placed::HouseNumber => Precision::HouseNumber,
        Placed::NearestHouseNumber => Precision::NearestHouseNumber,
        Placed::Street => Precision::Street,
    }
}

/// Step eighteen: the road whose number the address names, as the last answer before the centre.
///
/// A road's number is weak evidence: a departmental road runs for kilometres, and its middle in a
/// commune may stand far from the site, so a named street, the one shop of the label and every
/// rewriting of the address are tried first. A number is an identifier, so it must match exactly:
/// `D620` is not `D620A`.
fn on_a_numbered_road(
    streets: &StreetIndex,
    settled: &[i64],
    former_towns: &[i64],
    address: &str,
) -> Option<(OnStreet, String)> {
    road_numbers(&parts_as_written(&[address]))
        .into_iter()
        .find_map(|query| {
            on_the_street(streets, settled, former_towns, &query.1, &query.0, Reach::Tiers)
                .filter(|on| on.quality == Quality::Exact)
                .map(|on| (on, written(&query)))
        })
}

/// Step sixteen: the address rewritten, when as written it found nothing better than the centre.
///
/// It is split at every separator a person uses between two addresses or an address and a note,
/// `/`, `,`, `-`, `et`, `and`, and each piece is tried as it is and with each of its words left out
/// in turn, since one wrong word, `Général` for `Maréchal`, hides a street as well as a missing one
/// does. The narrowest answer wins: a door over the nearest door over a street. Between equals, the
/// rewriting that left out the least informative word wins, `COURS GALLIENI` rather than `COURS
/// GENERAL`. Two equally good rewritings on two different streets are a guess, and the centre stays.
fn rewritten(
    streets: &StreetIndex,
    settled: &[i64],
    former_towns: &[i64],
    address: &str,
) -> Option<(OnStreet, String)> {
    let mut tried: Vec<StreetQuery> = Vec::new();
    let mut found: Vec<(Placed, f64, OnStreet, String)> = Vec::new();
    for piece in pieces(address) {
        for (doors, street) in street_queries(&piece) {
            let words: Vec<&str> = street.split_whitespace().collect();
            let mut rewritings: Vec<(String, f64)> = vec![(street.clone(), 0.0)];
            if words.len() >= 2 {
                for left_out in 0..words.len() {
                    let kept: Vec<&str> = words
                        .iter()
                        .enumerate()
                        .filter(|(at, _)| *at != left_out)
                        .map(|(_, word)| *word)
                        .collect();
                    rewritings.push((kept.join(" "), streets.rarity().of_word(words[left_out])));
                }
            }
            for (rewriting, cost) in rewritings {
                let query = (doors.clone(), rewriting);
                if tried.contains(&query) || !names_something(&query.1, streets.rarity()) {
                    continue;
                }
                let on = [Reach::Tiers, Reach::UniqueStreet].into_iter().find_map(|reach| {
                    on_the_street(streets, settled, former_towns, &query.1, &query.0, reach)
                });
                if let Some(on) = on {
                    found.push((on.placed, cost, on, written(&query)));
                }
                tried.push(query);
            }
        }
    }
    let best = found
        .iter()
        .min_by(|left, right| left.0.cmp(&right.0).then(left.1.total_cmp(&right.1)))?;
    let rivals = found
        .iter()
        .filter(|other| other.0 == best.0 && other.1 == best.1 && other.2.street != best.2.street)
        .count();
    (rivals == 0).then(|| (best.2.clone(), best.3.clone()))
}

/// Enough rarity among the words a rewriting keeps that it still names a street: street types, the
/// profile's designator words and words under three letters say nothing, and `ROUTE SAINT` or
/// `AVENUE JEAN`, all that is left once `VIGOR` or `RACINE` is left out, say too little. `GAULLE`,
/// in one street name in 740, is just enough.
const NAMES_SOMETHING: f64 = 6.0;

fn names_something(street: &str, rarity: &crate::rarity::WordRarity) -> bool {
    let designator_words: Vec<&str> = FRENCH
        .designators
        .iter()
        .flat_map(|designator| designator.split_whitespace())
        .collect();
    street
        .split_whitespace()
        .filter(|word| {
            word.chars().count() >= 3
                && !FRENCH.street_types.contains(word)
                && !designator_words.contains(word)
        })
        .map(|word| rarity.of_word(word))
        .sum::<f64>()
        >= NAMES_SOMETHING
}

/// The address field and every piece of it between separators, whole field first.
fn pieces(address: &str) -> Vec<String> {
    let mut pieces: Vec<String> = vec![address.trim().to_string()];
    for part in street_candidates(address) {
        for piece in split_on_dashes(part) {
            let piece = piece.trim().to_string();
            if !piece.is_empty() && !pieces.contains(&piece) {
                pieces.push(piece);
            }
        }
    }
    pieces
}

/// A part cut at every dash with a space on either side, unless it joins two door numbers,
/// `13 - 19`. A dash without a space is part of a name, `Lyon-Saint-Exupéry`.
fn split_on_dashes(part: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (at, character) in part.char_indices() {
        if !matches!(character, '-' | '–' | '—') {
            continue;
        }
        let spaced = part[..at].ends_with(char::is_whitespace)
            || part[at + character.len_utf8()..].starts_with(char::is_whitespace);
        if !spaced {
            continue;
        }
        let before = part[..at].trim_end();
        let after = part[at + character.len_utf8()..].trim_start();
        let joins_doors =
            before.ends_with(|c: char| c.is_ascii_digit()) && after.starts_with(|c: char| c.is_ascii_digit());
        if !joins_doors {
            pieces.push(&part[start..at]);
            start = at + character.len_utf8();
        }
    }
    pieces.push(&part[start..]);
    pieces
}

fn written((doors, street): &StreetQuery) -> String {
    let doors: Vec<String> = doors
        .iter()
        .map(|door| format!("{}{}", door.number, door.suffix))
        .collect();
    if doors.is_empty() {
        street.clone()
    } else {
        format!("{} {street}", doors.join("-"))
    }
}

/// Whether a street was found only by step fifteen's rules, which rank below the one shop of the
/// label in the commune and above its centre.
fn is_loose(quality: Quality) -> bool {
    matches!(
        quality,
        Quality::BothWays | Quality::AskedWithin | Quality::StreetWithin
    )
}

/// A commune no larger than a disc about a kilometre across, whose centre is within a kilometre of
/// wherever in it the site stands (step seventeen).
pub const SMALL_COMMUNE_SQUARE_KILOMETRES: f64 = 3.5;

/// Why an answer should be seen by a person before it is used, or nothing when it need not be.
///
/// An answer that got no further than a commune's centre is refused, because a centre written as a
/// coordinate looks exactly like an address and is not one, and so is a site with no answer at all.
/// Refusing asks a person to look; the coordinate is still given. The reason says what failed, in
/// the site's own words where it can, so the look is short.
///
/// A commune of at most 3.5 km², a disc about a kilometre across, is spared: wherever the site
/// stands in it, the centre is within about a kilometre. The answer still says `Commune`; it is
/// only no longer refused.
pub fn refusal_of(site: &Site, answer: Option<&Answer>) -> Option<String> {
    let site = &site.without_parentheses();
    let Some(answer) = answer else {
        return Some(format!(
            "no commune found for {} {} in {}",
            site.postcode.trim(),
            site.town.trim(),
            site.country.trim()
        ));
    };
    if answer.precision != Precision::Commune {
        return None;
    }
    if answer
        .commune_area
        .is_some_and(|area| area <= SMALL_COMMUNE_SQUARE_KILOMETRES)
    {
        return None;
    }

    let streets: Vec<String> = street_candidates(&site.address)
        .into_iter()
        .map(|part| split_leading_house_number(part, &FRENCH).1)
        .filter(|street| !street.is_empty())
        .collect();
    let mut reasons = vec![if streets.is_empty() {
        "the address names no street".to_string()
    } else {
        format!(
            "no street matching {} in the commune",
            streets
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(" or ")
        )
    }];
    match answer.basis {
        Basis::PostcodeOnly => reasons.push(format!(
            "{} is not a commune at {}, so the postcode's communes were used",
            site.town.trim(),
            site.postcode.trim()
        )),
        Basis::NameOnly => reasons.push(format!(
            "postcode {} does not belong to {}, so the town name was trusted",
            site.postcode.trim(),
            site.town.trim()
        )),
        Basis::NameAndPostcode => {}
    }
    Some(format!("placed at the commune centre: {}", reasons.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Country;
    use crate::gazetteer::Commune;
    use crate::shops::Shop;
    use crate::street_index::{AddressPoint, Street};
    use crate::text::HouseNumber;
    use std::collections::HashMap;

    fn gazetteer() -> Gazetteer {
        Gazetteer::of(
            vec![
                Commune {
                    relation: 1,
                    level: 8,
                    names: vec!["SENS".into()],
                    postcodes: vec!["89100".into()],
                    centre: Some([3.283, 48.197]),
                    country: Some("FR".into()),
                },
                Commune {
                    relation: 2,
                    level: 8,
                    names: vec!["MARIGNY".into()],
                    postcodes: vec!["50570".into()],
                    centre: Some([-1.2, 49.1]),
                    country: Some("FR".into()),
                },
                Commune {
                    relation: 3,
                    level: 8,
                    names: vec!["MARIGNY".into()],
                    postcodes: vec!["39130".into()],
                    centre: Some([5.6, 46.7]),
                    country: Some("FR".into()),
                },
            ],
            vec![Country {
                code: "FR".into(),
                names: vec!["FRANCE".into()],
            }],
        )
    }

    fn streets() -> StreetIndex {
        StreetIndex::of(vec![
            (
                1,
                Street {
                    name: "ROUTE TILLEULS".into(),
                    addresses: vec![AddressPoint {
                        number: HouseNumber {
                            number: 8,
                            suffix: String::new(),
                        },
                        position: [3.2652, 48.2000],
                    }],
                    path: vec![],
                },
            ),
            (
                3,
                Street {
                    name: "RUE FERRONNIERE".into(),
                    addresses: vec![],
                    path: vec![[5.61, 46.71]],
                },
            ),
        ])
    }

    fn site(address: &str, postcode: &str, town: &str) -> Site {
        Site {
            label: String::new(),
            address: address.into(),
            postcode: postcode.into(),
            town: town.into(),
            country: "FRANCE".into(),
        }
    }

    #[test]
    fn given_a_numbered_address_on_a_known_street_when_located_then_the_door_answers() {
        let found = locate(
            &gazetteer(),
            &streets(),
            &ShopIndex::default(),
            &site("8 Route des Tilleuls", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.2652, 48.2000]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    #[test]
    fn given_a_street_the_commune_does_not_have_when_located_then_the_commune_centre_answers() {
        let found = locate(
            &gazetteer(),
            &streets(),
            &ShopIndex::default(),
            &site("3 Rue Inconnue", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.283, 48.197]);
        assert_eq!(found.precision, Precision::Commune);
    }

    /// The postcode says one Marigny and the street says another. The street wins only when the
    /// postcode did not settle the commune; here it did, so the street is looked for there alone.
    #[test]
    fn given_the_postcode_settled_the_commune_when_the_street_is_elsewhere_then_it_is_not_searched_there() {
        let found = locate(
            &gazetteer(),
            &streets(),
            &ShopIndex::default(),
            &site("2 rue de la Ferronnière", "50570", "MARIGNY"),
        )
        .expect("an answer");

        assert_eq!(found.position, [-1.2, 49.1]);
    }

    #[test]
    fn given_no_street_index_when_located_then_it_is_exactly_the_previous_step() {
        let found = locate(
            &gazetteer(),
            &StreetIndex::default(),
            &ShopIndex::default(),
            &site("8 Route des Tilleuls", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.283, 48.197]);
    }

    /// Site files often join a retail park and a street in one field, and only the street can be
    /// matched.
    #[test]
    fn given_an_address_naming_a_retail_park_then_a_street_when_located_then_the_street_part_answers() {
        let found = locate(
            &gazetteer(),
            &streets(),
            &ShopIndex::default(),
            &site("ZA Les Essarts/8 Route des Tilleuls", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::HouseNumber);
    }

    fn answer(precision: Precision, basis: Basis) -> Answer {
        Answer {
            rewritten_as: None,
            commune_area: None,
            shops_averaged: None,
            position: [0.0, 0.0],
            precision,
            basis,
            communes: vec![1],
            street: None,
        }
    }

    #[test]
    fn given_a_site_placed_at_its_door_when_judged_then_it_is_not_refused() {
        let site = site("8 Route des Tilleuls", "89100", "SENS");

        assert_eq!(
            refusal_of(
                &site,
                Some(&answer(Precision::HouseNumber, Basis::NameAndPostcode))
            ),
            None
        );
    }

    #[test]
    fn given_a_site_placed_on_its_street_when_judged_then_it_is_not_refused() {
        let site = site("Route des Tilleuls", "89100", "SENS");

        assert_eq!(
            refusal_of(&site, Some(&answer(Precision::Street, Basis::NameAndPostcode))),
            None
        );
    }

    #[test]
    fn given_a_site_left_at_its_commune_centre_when_judged_then_it_is_refused_naming_the_street_not_found() {
        let site = site("3 Rue Inconnue", "89100", "SENS");

        let refused =
            refusal_of(&site, Some(&answer(Precision::Commune, Basis::NameAndPostcode))).expect("refused");

        assert!(refused.contains("RUE INCONNUE"), "{refused}");
    }

    #[test]
    fn given_a_town_that_is_no_commune_when_judged_then_the_refusal_says_so() {
        let site = site("Bd des Mouettes", "83530", "AGAY");

        let refused =
            refusal_of(&site, Some(&answer(Precision::Commune, Basis::PostcodeOnly))).expect("refused");

        assert!(refused.contains("not a commune"), "{refused}");
    }

    #[test]
    fn given_a_postcode_not_the_towns_when_judged_then_the_refusal_says_so() {
        let site = site("Rue des Écoles", "61000", "FLERS");

        let refused = refusal_of(&site, Some(&answer(Precision::Commune, Basis::NameOnly))).expect("refused");

        assert!(refused.contains("postcode"), "{refused}");
    }

    #[test]
    fn given_no_answer_at_all_when_judged_then_it_is_refused() {
        let site = site("1 Rue des Mésanges", "98000", "MONACO");

        assert!(refusal_of(&site, None).is_some());
    }

    fn merged_commune() -> (Gazetteer, StreetIndex) {
        let commune = |relation: i64, level: u8, name: &str, postcodes: &[&str], centre: [f64; 2]| Commune {
            relation,
            level,
            names: vec![name.into()],
            postcodes: postcodes.iter().map(|code| (*code).to_string()).collect(),
            centre: Some(centre),
            country: Some("FR".into()),
        };
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, "CHERBOURG", &["50100", "50110"], [-1.62, 49.64]),
                commune(2, 9, "CHERBOURG OCTEVILLE", &["50100"], [-1.63, 49.63]),
                commune(3, 9, "TOURLAVILLE", &["50110"], [-1.58, 49.64]),
            ],
            vec![Country {
                code: "FR".into(),
                names: vec!["FRANCE".into()],
            }],
        )
        .with_parents(HashMap::from([(2, 1), (3, 1)]));
        let street = |name: &str, point: [f64; 2]| Street {
            name: name.into(),
            addresses: vec![],
            path: vec![point],
        };
        let streets = StreetIndex::of(vec![
            (1, street("RUE GAMBETTA", [-1.626, 49.638])),
            (2, street("RUE GAMBETTA", [-1.626, 49.638])),
            (1, street("RUE GAMBETTA", [-1.585, 49.643])),
            (3, street("RUE GAMBETTA", [-1.585, 49.643])),
            (1, street("RUE ABBAYE", [-1.640, 49.645])),
            (3, street("RUE ABBAYE", [-1.640, 49.645])),
            (2, street("RUE ABBAYES", [-1.630, 49.630])),
        ]);
        (gazetteer, streets)
    }

    /// Cherbourg-en-Cotentin merged several towns in 2016 and three of them kept a Rue Gambetta.
    /// The postcode names the former town, and its street alone is the answer: averaging the three
    /// would put a site 1.4 km from its door.
    #[test]
    fn given_a_merged_commune_when_the_postcode_names_a_former_town_then_its_street_answers() {
        let (gazetteer, streets) = merged_commune();

        let found = locate(
            &gazetteer,
            &streets,
            &ShopIndex::default(),
            &site("Rue Gambetta", "50100", "CHERBOURG"),
        )
        .expect("an answer");

        assert_eq!(found.position, [-1.626, 49.638]);
    }

    #[test]
    fn given_a_street_only_in_another_former_town_when_located_then_the_whole_merged_commune_answers() {
        let (gazetteer, streets) = merged_commune();

        let found = locate(
            &gazetteer,
            &streets,
            &ShopIndex::default(),
            &site("Rue de l'Abbaye", "50100", "CHERBOURG"),
        )
        .expect("an answer");

        assert_eq!(found.position, [-1.640, 49.645]);
    }

    #[test]
    fn given_the_former_town_only_resembles_the_street_when_another_has_it_exactly_then_the_exact_one_wins() {
        let (gazetteer, streets) = merged_commune();

        let found = locate(
            &gazetteer,
            &streets,
            &ShopIndex::default(),
            &site("Rue de l'Abbaye", "50100", "CHERBOURG"),
        )
        .expect("an answer");

        assert_eq!(found.street.map(|(name, _)| name), Some("RUE ABBAYE".to_string()));
    }

    /// Val Fleuri is no merged commune, but its postcode is shared with a former town of a merged
    /// neighbour that has its own Rue des Érables 12 km away. Only former towns inside the settled
    /// commune may be searched first.
    #[test]
    fn given_a_former_town_of_another_commune_at_the_same_postcode_when_located_then_it_is_not_searched() {
        let commune = |relation: i64, level: u8, name: &str, centre: [f64; 2]| Commune {
            relation,
            level,
            names: vec![name.into()],
            postcodes: vec!["35150".into()],
            centre: Some(centre),
            country: Some("FR".into()),
        };
        let gazetteer = Gazetteer::of(
            vec![
                commune(1, 8, "VAL FLEURI", [-1.58, 47.98]),
                commune(2, 8, "MONTFORT", [-1.50, 47.96]),
                commune(3, 9, "SAINTE COLOMBE", [-1.45, 47.88]),
            ],
            vec![Country {
                code: "FR".into(),
                names: vec!["FRANCE".into()],
            }],
        )
        .with_parents(HashMap::from([(3, 2)]));
        let street = |name: &str, point: [f64; 2]| Street {
            name: name.into(),
            addresses: vec![],
            path: vec![point],
        };
        let streets = StreetIndex::of(vec![
            (1, street("RUE ERABLES", [-1.578, 47.981])),
            (3, street("RUE ERABLES", [-1.450, 47.880])),
            (2, street("RUE ERABLES", [-1.450, 47.880])),
        ]);

        let found = locate(
            &gazetteer,
            &streets,
            &ShopIndex::default(),
            &site("6 Rue des Érables", "35150", "VAL FLEURI"),
        )
        .expect("an answer");

        assert_eq!(found.position, [-1.578, 47.981]);
    }

    fn one_street(name: &str, doors: &[(u32, [f64; 2])]) -> StreetIndex {
        StreetIndex::of(vec![(
            1,
            Street {
                name: name.into(),
                addresses: doors
                    .iter()
                    .map(|(value, position)| AddressPoint {
                        number: HouseNumber {
                            number: *value,
                            suffix: String::new(),
                        },
                        position: *position,
                    })
                    .collect(),
                path: vec![],
            },
        )])
    }

    /// Streets of Sens by their comparison key, as the index files them, each placed at its middle.
    fn streets_of_sens(names: &[(&str, [f64; 2])]) -> StreetIndex {
        StreetIndex::of(
            names
                .iter()
                .map(|(name, middle)| {
                    (
                        1,
                        Street {
                            name: (*name).into(),
                            addresses: vec![],
                            path: vec![*middle],
                        },
                    )
                })
                .collect(),
        )
    }

    /// Step twelve: `C.C. La Pinsonnière` names nothing the index holds until the designator is
    /// skipped, and then the neighbourhood answers instead of the commune's centre.
    #[test]
    fn given_a_designator_before_a_place_the_index_holds_when_located_then_the_place_answers() {
        let streets = streets_of_sens(&[("PINSONNIERE", [3.29, 48.21])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("C.C. La Pinsonnière", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.29, 48.21]);
        assert_eq!(found.precision, Precision::Street);
    }

    /// A zone's name read without its designator is a place, weaker evidence than a street written
    /// in the same field. It is tried only once every part as written has failed.
    #[test]
    fn given_a_designator_part_then_a_street_when_located_then_the_street_answers() {
        let streets = streets_of_sens(&[
            ("CHEMIN PONT VIEUX", [3.30, 48.22]),
            ("RUE PAUL CEZANNE", [3.27, 48.19]),
        ]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("ZA du Pont Vieux/Rue Paul Cézanne", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.27, 48.19]);
    }

    /// Mappers name the road through a zone after the zone, `ZAC Saint Martin`, so the part is first
    /// searched as it was written, and the designator is skipped only when that finds nothing.
    #[test]
    fn given_a_street_named_after_its_zone_when_located_then_the_whole_name_answers() {
        let streets = streets_of_sens(&[
            ("ZONE ARTISANALE SAINT MARTIN", [3.31, 48.23]),
            ("RUE SAINT MARTIN", [3.26, 48.18]),
        ]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("ZAC St Martin", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.31, 48.23]);
    }

    /// A street's comparison key, its middle, and its recorded doors.
    type StreetOfSens<'a> = (&'a str, [f64; 2], &'a [(u32, [f64; 2])]);

    /// Streets of Sens by their comparison key, each with its middle and its recorded doors.
    fn streets_of_sens_with_doors(entries: &[StreetOfSens]) -> StreetIndex {
        StreetIndex::of(
            entries
                .iter()
                .map(|(name, middle, doors)| {
                    (
                        1,
                        Street {
                            name: (*name).into(),
                            addresses: doors
                                .iter()
                                .map(|(number, position)| AddressPoint {
                                    number: HouseNumber {
                                        number: *number,
                                        suffix: String::new(),
                                    },
                                    position: *position,
                                })
                                .collect(),
                            path: vec![*middle],
                        },
                    )
                })
                .collect(),
        )
    }

    fn located_in_sens(streets: &StreetIndex, address: &str) -> Answer {
        locate(
            &gazetteer(),
            streets,
            &ShopIndex::default(),
            &site(address, "89100", "SENS"),
        )
        .expect("an answer")
    }

    /// Step thirteen: the dash of `Avenue de l'Europe - Robert Schuman` is part of the
    /// street's name, so the address read whole matches exactly and is kept.
    #[test]
    fn given_a_spaced_dash_inside_a_street_name_when_the_whole_address_matches_exactly_then_it_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("AVENUE EUROPE", [3.20, 48.10], &[(146, [3.21, 48.11])]),
            (
                "AVENUE EUROPE ROBERT SCHUMAN",
                [3.30, 48.20],
                &[(150, [3.31, 48.21])],
            ),
        ]);

        let found = located_in_sens(&streets, "150 Avenue de l'Europe - Robert Schuman");

        assert_eq!(found.position, [3.31, 48.21]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    /// The street is sometimes written after the dash, `CC Cap Horizon - Rue Lavandière`. An exact match
    /// there beats a street merely resembling the part before it.
    #[test]
    fn given_the_street_after_a_spaced_dash_when_it_matches_exactly_then_it_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE CAP HORIZON", [3.20, 48.10], &[]),
            ("RUE LAVANDIERE", [3.30, 48.20], &[]),
        ]);

        let found = located_in_sens(&streets, "CC Cap Horizon - Rue Lavandière");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    /// Both sides of the dash match exactly, and only the second has the door, which is the better
    /// answer: a door is a building, a zone is somewhere around it.
    #[test]
    fn given_exact_matches_on_both_sides_of_a_dash_when_only_one_has_the_door_then_the_door_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("ZONE ARTISANALE RIVIERES", [3.20, 48.10], &[]),
            ("RUE LISBONNE", [3.30, 48.20], &[(46, [3.31, 48.21])]),
        ]);

        let found = located_in_sens(&streets, "ZA des Rivières - 46 Rue de Lisbonne");

        assert_eq!(found.position, [3.31, 48.21]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    /// Read as written, `ZA du Moulin` only resembles `ZA des Moulins`; without its designator it
    /// is exactly the hamlet of `Moulin`. The exact reading is kept.
    #[test]
    fn given_a_designator_part_when_only_the_bare_name_matches_exactly_then_it_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("ZONE ARTISANALE MOULINS", [3.20, 48.10], &[]),
            ("MOULIN", [3.30, 48.20], &[]),
        ]);

        let found = located_in_sens(&streets, "ZA du Moulin");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    /// An exact name is preferred, but not at the cost of a door already found: `2 Bis Rue Mollière`
    /// only resembles `Rue Molière`, yet its door 2 is a building, and `Vieux Port` is a quarter.
    #[test]
    fn given_a_door_found_on_a_resembling_street_when_another_part_matches_exactly_without_a_door_then_the_door_answers()
     {
        let streets = streets_of_sens_with_doors(&[
            ("RUE MOLIERE", [3.20, 48.10], &[(2, [3.21, 48.11])]),
            ("VIEUX PORT", [3.30, 48.20], &[]),
        ]);

        let found = located_in_sens(&streets, "2 Rue Mollière/74 Vieux Port");

        assert_eq!(found.position, [3.21, 48.11]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    /// Step fourteen: with no exact reading, the field is read loosely as it was written, dash
    /// included. `Route des Saules - Beauregard` names no street exactly on either side, and cut
    /// at the dash it only resembles `Place des Saules`; whole, it shares its words with `Route de
    /// Beauregard`.
    #[test]
    fn given_no_exact_reading_when_located_then_the_loose_reading_keeps_the_text_after_the_dash() {
        let streets = streets_of_sens_with_doors(&[
            ("PLACE SAULES", [3.20, 48.10], &[]),
            ("ROUTE BEAUREGARD", [3.30, 48.20], &[]),
        ]);

        let found = located_in_sens(&streets, "Route des Saules - Beauregard");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    /// Step fifteen: `J` for `JEAN` is beyond every tier, and `RUE JEAN JAURES` is the only street of
    /// Sens it can mean.
    #[test]
    fn given_an_initial_no_tier_forgives_when_located_then_the_only_street_it_can_mean_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE JEAN JAURES", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ]);

        let found = located_in_sens(&streets, "Rue J Jaures");

        assert_eq!(found.position, [3.30, 48.20]);
        assert_eq!(found.street.map(|(_, quality)| quality), Some(Quality::BothWays));
    }

    /// The looser rules come second for the whole field: a part the tiers match beats an earlier part
    /// only step fifteen could, as a street written beside a zone beat the zone's name in step twelve.
    #[test]
    fn given_one_part_matched_by_a_tier_and_an_earlier_one_only_loosely_when_located_then_the_tier_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE JEAN JAURES", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ]);

        let found = located_in_sens(&streets, "Rue J Jaures/Route des Tilleuls");

        assert_eq!(found.position, [3.20, 48.10]);
    }

    /// With every word as rare as in a real index, a centre's name before the street, with no
    /// separator between them, no longer hides the street.
    #[test]
    fn given_a_centres_name_glued_before_the_street_when_located_then_the_street_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE LAVANDIERE", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ])
        .with_rarity(crate::rarity::WordRarity::from_counts(
            3_000_000,
            &[
                ("RUE", 900_000),
                ("ROUTE", 400_000),
                ("CAP", 3_000),
                ("HORIZON", 3_000),
            ],
        ));

        let found = located_in_sens(&streets, "Cap Horizon Rue Lavandière");

        assert_eq!(found.position, [3.30, 48.20]);
        assert_eq!(
            found.street.map(|(_, quality)| quality),
            Some(Quality::StreetWithin)
        );
    }

    /// A street found only by step fifteen's looser rules is weaker evidence than the one shop of the
    /// label in the commune: measured on real site files, trusting the street took sites further from
    /// their door than the shop did.
    #[test]
    fn given_a_street_found_only_loosely_when_the_commune_has_one_shop_of_the_label_then_the_shop_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE JEAN JAURES", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ]);
        let shops = shops(&[(1, "Acme", "Avenue de la Gare", [3.27, 48.21])]);

        let found =
            locate(&gazetteer(), &streets, &shops, &labelled("ACME", "Rue J Jaures")).expect("an answer");

        assert_eq!(found.position, [3.27, 48.21]);
        assert_eq!(found.precision, Precision::Shop);
    }

    #[test]
    fn given_a_street_found_only_loosely_when_the_commune_has_no_shop_of_the_label_then_the_street_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE JEAN JAURES", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &labelled("ACME", "Rue J Jaures"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    /// Step sixteen: `et` joining two addresses separates them as a slash does, and the narrowest
    /// reading answers: the door on one street beats the other street's middle.
    #[test]
    fn given_two_addresses_joined_by_et_when_located_then_the_door_on_either_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE MATISSE", [3.20, 48.10], &[]),
            ("RUE DEGAS", [3.30, 48.20], &[(19, [3.31, 48.21])]),
        ]);

        let found = located_in_sens(&streets, "5 rue Matisse et 19 rue Degas");

        assert_eq!(found.position, [3.31, 48.21]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    #[test]
    fn given_two_addresses_joined_by_and_when_split_then_each_is_a_part() {
        assert_eq!(
            street_candidates("5 Matisse Street and 19 Degas Road"),
            vec!["5 Matisse Street", "19 Degas Road"]
        );
    }

    #[test]
    fn given_et_between_two_door_numbers_when_split_then_it_stays_a_range() {
        assert_eq!(
            street_candidates("8 et 10 Route des Tilleuls"),
            vec!["8 et 10 Route des Tilleuls"]
        );
    }

    /// Step sixteen: when nothing better than the centre was found, each word of the address is left
    /// out in turn. `Général` for `Maréchal` hides the street until `Général` is left out, and the
    /// least informative word is the first one tried without.
    #[test]
    fn given_one_wrong_word_when_left_out_then_the_door_answers_and_the_rewriting_is_named() {
        let streets = streets_of_sens_with_doors(&[
            ("COURS MARECHAL VAUBAN", [3.30, 48.20], &[(13, [3.31, 48.21])]),
            ("RUE MARECHAL VAUBAN", [3.20, 48.10], &[(13, [3.21, 48.11])]),
        ])
        .with_rarity(crate::rarity::WordRarity::from_counts(
            3_000_000,
            &[
                ("COURS", 20_000),
                ("RUE", 900_000),
                ("GENERAL", 60_000),
                ("MARECHAL", 40_000),
                ("VAUBAN", 900),
            ],
        ));

        let found = located_in_sens(&streets, "13 Cours du Général Vauban");

        assert_eq!(found.position, [3.31, 48.21]);
        assert_eq!(found.precision, Precision::HouseNumber);
        assert_eq!(found.rewritten_as.as_deref(), Some("13 COURS VAUBAN"));
    }

    #[test]
    fn given_the_address_matched_when_located_then_it_is_not_rewritten() {
        let found = located_in_sens(
            &one_street("ROUTE TILLEULS", &[(8, [3.26, 48.20])]),
            "8 Route des Tilleuls",
        );

        assert_eq!(found.rewritten_as, None);
    }

    /// Leaving a word out may not trade the centre for a wrong street: two readings placing the site on
    /// two different streets at the same precision are ambiguous, and the centre stays.
    #[test]
    fn given_two_rewritings_on_two_streets_equally_informative_when_located_then_the_centre_stays() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE ALPHA", [3.30, 48.20], &[(3, [3.31, 48.21])]),
            ("RUE BETA", [3.20, 48.10], &[(3, [3.21, 48.11])]),
        ])
        .with_rarity(crate::rarity::WordRarity::from_counts(
            3_000_000,
            &[("ALPHA", 500), ("BETA", 500)],
        ));

        let found = located_in_sens(&streets, "3 Alpha Beta");

        assert_eq!(found.precision, Precision::Commune);
    }

    /// Leaving out `VIGOR` leaves `ROUTE SAINT`, which names nothing in particular: a rewriting
    /// must keep enough rare words to say which street it means.
    #[test]
    fn given_the_only_informative_word_left_out_when_rewritten_then_the_rest_does_not_answer() {
        let streets = streets_of_sens_with_doors(&[("ROUTE SAINT CHRISTOPHE", [3.30, 48.20], &[])])
            .with_rarity(crate::rarity::WordRarity::from_counts(
                3_000_000,
                &[
                    ("ROUTE", 400_000),
                    ("SAINT", 70_000),
                    ("VIGOR", 13),
                    ("CHRISTOPHE", 2_000),
                ],
            ));

        let found = located_in_sens(&streets, "Route de Saint Vigor");

        assert_eq!(found.precision, Precision::Commune);
    }

    /// A dash without a space is part of a name, `Lyon-Saint-Exupéry`: the rewriting cuts only at a dash with
    /// a space on either side, as step eleven does.
    #[test]
    fn given_a_hyphenated_name_when_cut_into_pieces_then_it_stays_whole() {
        assert_eq!(
            pieces("Aéroport de Lyon-Saint-Exupéry Terminal 1"),
            vec!["Aéroport de Lyon-Saint-Exupéry Terminal 1".to_string()]
        );
        assert_eq!(
            pieces("Rue Lavandière - Cap Horizon"),
            vec![
                "Rue Lavandière - Cap Horizon".to_string(),
                "Rue Lavandière".to_string(),
                "Cap Horizon".to_string()
            ]
        );
    }

    /// Step seventeen: in a commune of at most 3.5 km², a disc about a kilometre across, the centre is
    /// within a kilometre of the site wherever it is, so it is an answer and not a refusal.
    #[test]
    fn given_a_small_commune_when_placed_at_its_centre_then_it_is_not_refused() {
        let gazetteer = gazetteer().with_areas(HashMap::from([(1, 2.9)]));

        let found = locate(
            &gazetteer,
            &streets(),
            &ShopIndex::default(),
            &site("3 Rue Inconnue", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::Commune);
        assert_eq!(found.commune_area, Some(2.9));
        assert_eq!(
            refusal_of(&site("3 Rue Inconnue", "89100", "SENS"), Some(&found)),
            None
        );
    }

    #[test]
    fn given_a_large_commune_when_placed_at_its_centre_then_it_is_still_refused() {
        let gazetteer = gazetteer().with_areas(HashMap::from([(1, 44.9)]));

        let found = locate(
            &gazetteer,
            &streets(),
            &ShopIndex::default(),
            &site("3 Rue Inconnue", "89100", "SENS"),
        )
        .expect("an answer");

        assert!(refusal_of(&site("3 Rue Inconnue", "89100", "SENS"), Some(&found)).is_some());
    }

    #[test]
    fn given_a_commune_whose_area_is_unknown_when_placed_at_its_centre_then_it_is_refused() {
        let found = locate(
            &gazetteer(),
            &streets(),
            &ShopIndex::default(),
            &site("3 Rue Inconnue", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(found.commune_area, None);
        assert!(refusal_of(&site("3 Rue Inconnue", "89100", "SENS"), Some(&found)).is_some());
    }

    /// Step eighteen: `RD 620` finds the road filed under its number, `D620`.
    #[test]
    fn given_a_road_number_when_located_then_the_road_filed_under_it_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("D620", [3.30, 48.20], &[]),
            ("ROUTE CAEN", [3.30, 48.20], &[]),
            ("RUE GARE", [3.20, 48.10], &[]),
        ]);

        let found = located_in_sens(&streets, "RD 620");

        assert_eq!(found.position, [3.30, 48.20]);
        assert_eq!(found.precision, Precision::Street);
    }

    #[test]
    fn given_a_road_number_after_a_door_and_before_a_place_when_located_then_the_road_answers() {
        let streets =
            streets_of_sens_with_doors(&[("D712", [3.32, 48.22], &[]), ("RUE GARE", [3.20, 48.10], &[])]);

        let found = located_in_sens(&streets, "118 RD 712 Les Ponts Neufs");

        assert_eq!(found.position, [3.32, 48.22]);
    }

    /// A road's number is weak evidence: a departmental road runs for kilometres, and its middle in
    /// a commune may be far from the site. A named street in the same field answers first.
    #[test]
    fn given_a_named_street_and_a_road_number_when_located_then_the_named_street_answers() {
        let streets =
            streets_of_sens_with_doors(&[("RUE LACS", [3.30, 48.20], &[]), ("D627", [3.20, 48.10], &[])]);

        let found = located_in_sens(&streets, "Route des Lacs/RD 627");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    #[test]
    fn given_a_road_number_when_the_commune_has_one_shop_of_the_label_then_the_shop_answers() {
        let streets =
            streets_of_sens_with_doors(&[("D214", [3.20, 48.10], &[]), ("RUE GARE", [3.30, 48.20], &[])]);
        let shops = shops(&[(1, "Acme", "", [3.27, 48.21])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &shops,
            &labelled("ACME", "Chemin Départementale 214"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::Shop);
    }

    /// A road number is an identifier: `D620` is not `D620A`, however alike they look.
    #[test]
    fn given_a_road_number_only_resembling_a_filed_one_when_located_then_the_centre_stays() {
        let streets =
            streets_of_sens_with_doors(&[("D620A", [3.20, 48.10], &[]), ("RUE GARE", [3.30, 48.20], &[])]);

        let found = located_in_sens(&streets, "RD 620");

        assert_eq!(found.precision, Precision::Commune);
    }

    /// Step nineteen: two shops of the label a few hundred metres apart put the site in one pixel
    /// whichever it is, so their middle answers instead of the commune's centre, and says how many.
    #[test]
    fn given_two_shops_of_the_label_within_a_kilometre_when_no_street_matched_then_their_middle_answers() {
        let shops = shops(&[
            (1, "Acme Market", "", [3.270, 48.210]),
            (1, "Acme Market", "", [3.272, 48.211]),
        ]);

        let found = locate(
            &gazetteer(),
            &streets(),
            &shops,
            &labelled("ACME MARKET", "RD 620"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::Shop);
        assert_eq!(found.shops_averaged, Some(2));
        assert!((found.position[0] - 3.271).abs() < 1e-9 && (found.position[1] - 48.2105).abs() < 1e-9);
    }

    /// The middle of several shops stands in for the commune's centre, nothing better: a door found by
    /// rewriting the address answers first.
    #[test]
    fn given_shops_to_average_when_a_rewriting_finds_the_door_then_the_door_answers() {
        let streets =
            streets_of_sens_with_doors(&[("COURS MARECHAL VAUBAN", [3.30, 48.20], &[(13, [3.31, 48.21])])])
                .with_rarity(crate::rarity::WordRarity::from_counts(
                    3_000_000,
                    &[
                        ("COURS", 20_000),
                        ("GENERAL", 60_000),
                        ("MARECHAL", 40_000),
                        ("VAUBAN", 900),
                    ],
                ));
        let shops = shops(&[
            (1, "Acme City", "", [3.270, 48.210]),
            (1, "Acme City", "", [3.272, 48.211]),
        ]);

        let found = locate(
            &gazetteer(),
            &streets,
            &shops,
            &labelled("ACME CITY", "13 Cours du Général Vauban"),
        )
        .expect("an answer");

        assert_eq!(found.position, [3.31, 48.21]);
        assert_eq!(found.precision, Precision::HouseNumber);
    }

    #[test]
    fn given_shops_to_average_when_a_street_is_found_loosely_then_the_street_answers() {
        let streets = streets_of_sens_with_doors(&[
            ("RUE JEAN JAURES", [3.30, 48.20], &[]),
            ("ROUTE TILLEULS", [3.20, 48.10], &[]),
        ]);
        let shops = shops(&[(1, "Acme", "", [3.270, 48.210]), (1, "Acme", "", [3.272, 48.211])]);

        let found =
            locate(&gazetteer(), &streets, &shops, &labelled("ACME", "Rue J Jaures")).expect("an answer");

        assert_eq!(found.position, [3.30, 48.20]);
    }

    /// Step six read only the 7 of `5/7`, because the slash split the field in two.
    #[test]
    fn given_doors_joined_by_a_slash_when_only_the_first_is_recorded_then_it_answers() {
        let streets = one_street("ROUTE TILLEULS", &[(5, [3.26, 48.20]), (9, [3.27, 48.21])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("5/7 Route des Tilleuls", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(
            (found.position, found.precision),
            ([3.26, 48.20], Precision::HouseNumber)
        );
    }

    /// Grand Ouest 2 is a shopping centre and 30 the door. A slash between two numbers is a range only
    /// when the numbers open the field.
    #[test]
    fn given_a_slash_between_a_centres_name_and_a_door_when_located_then_the_door_answers() {
        let streets = one_street("AVENUE PLATANES", &[(30, [3.25, 48.19])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("C.C. Grand Ouest 2/30 Avenue des Platanes", "89100", "SENS"),
        )
        .expect("an answer");

        assert_eq!(
            (found.position, found.precision),
            ([3.25, 48.19], Precision::HouseNumber)
        );
    }

    #[test]
    fn given_a_site_whose_fields_carry_parentheses_when_stripped_then_each_field_loses_them() {
        let site = Site {
            label: "ACME CITY (FRANCHISE)".into(),
            address: "17 rue des Écoles (LE BOURG)".into(),
            postcode: "89100 (CEDEX)".into(),
            town: "SENS (YONNE)".into(),
            country: "FRANCE (METROPOLITAINE)".into(),
        };

        assert_eq!(
            site.without_parentheses(),
            Site {
                label: "ACME CITY".into(),
                address: "17 rue des Écoles".into(),
                postcode: "89100".into(),
                town: "SENS".into(),
                country: "FRANCE".into()
            }
        );
    }

    /// Read as a word of the street, `(MAISON)` would match any street named after it in the commune.
    #[test]
    fn given_an_annotation_in_parentheses_when_located_then_it_is_as_if_it_were_not_there() {
        let streets = one_street("ROUTE TILLEULS", &[(8, [3.26, 48.20])]);
        let with = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("8 Route des Tilleuls (MAISON)", "89100", "SENS (YONNE)"),
        );
        let without = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("8 Route des Tilleuls", "89100", "SENS"),
        );

        assert_eq!(
            with.map(|found| found.position),
            without.map(|found| found.position)
        );
    }

    fn shops(entries: &[(i64, &str, &str, [f64; 2])]) -> ShopIndex {
        ShopIndex::of(
            entries
                .iter()
                .map(|(commune, name, street, position)| {
                    (
                        *commune,
                        Shop {
                            names: vec![crate::text::comparison_key(name, &FRENCH)],
                            street: crate::text::comparison_key(street, &FRENCH),
                            position: *position,
                        },
                    )
                })
                .collect(),
        )
    }

    fn labelled(label: &str, address: &str) -> Site {
        Site {
            label: label.into(),
            ..site(address, "89100", "SENS")
        }
    }

    #[test]
    fn given_the_door_is_missing_when_a_shop_of_the_label_stands_on_the_asked_street_then_the_shop_answers() {
        let streets = one_street("ROUTE TILLEULS", &[(2, [3.20, 48.10])]);
        let shops = shops(&[(1, "Acme", "Route des Tilleuls", [3.27, 48.21])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &shops,
            &labelled("ACME", "40 Route des Tilleuls"),
        )
        .expect("an answer");

        assert_eq!(
            (found.position, found.precision),
            ([3.27, 48.21], Precision::Shop)
        );
    }

    #[test]
    fn given_the_exact_door_is_recorded_when_a_shop_is_on_the_street_too_then_the_door_answers() {
        let streets = one_street("ROUTE TILLEULS", &[(8, [3.26, 48.20])]);
        let shops = shops(&[(1, "Acme", "Route des Tilleuls", [3.27, 48.21])]);

        let found = locate(
            &gazetteer(),
            &streets,
            &shops,
            &labelled("ACME", "8 Route des Tilleuls"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::HouseNumber);
    }

    #[test]
    fn given_no_street_matched_when_the_commune_has_one_shop_of_the_label_then_the_shop_answers() {
        let shops = shops(&[(1, "Acme Market", "", [3.29, 48.18])]);

        let found = locate(
            &gazetteer(),
            &StreetIndex::default(),
            &shops,
            &labelled("ACME MARKET", "ZA des Sablons"),
        )
        .expect("an answer");

        assert_eq!(
            (found.position, found.precision),
            ([3.29, 48.18], Precision::Shop)
        );
    }

    #[test]
    fn given_no_street_matched_when_the_commune_has_two_shops_of_the_label_then_the_centre_stays() {
        let shops = shops(&[
            (1, "Acme Market", "", [3.29, 48.18]),
            (1, "Acme Market", "", [3.30, 48.22]),
        ]);

        let found = locate(
            &gazetteer(),
            &StreetIndex::default(),
            &shops,
            &labelled("ACME MARKET", "ZA des Sablons"),
        )
        .expect("an answer");

        assert_eq!(found.precision, Precision::Commune);
    }

    #[test]
    fn given_an_annotation_after_a_spaced_dash_when_the_street_before_it_matches_exactly_then_the_annotation_changes_nothing()
     {
        let streets = one_street("ROUTE TILLEULS", &[(8, [3.26, 48.20])]);
        let with = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("8 Route des Tilleuls - ZA des Sablons", "89100", "SENS"),
        );
        let without = locate(
            &gazetteer(),
            &streets,
            &ShopIndex::default(),
            &site("8 Route des Tilleuls", "89100", "SENS"),
        );

        assert_eq!(
            with.map(|found| found.position),
            without.map(|found| found.position)
        );
    }
}
