//! Reducing two spellings of one place to the same string.
//!
//! The same address is written a dozen ways: `Rte des Tilleuls` and `Route Des Tilleuls`, `ST ETIENNE` and
//! `Saint-Étienne`, `8 bis` and `8BIS`. None of that is geography, so it is flattened before
//! anything is compared.
//!
//! What counts as an abbreviation, a filler word or a street type is a property of a language, not
//! of geocoding, so it lives in a [`TextProfile`] rather than in the functions. A neutral profile,
//! case and accent folding and nothing else, shows in the tests that nothing French is applied by
//! accident.

/// The abbreviations, filler words and street types of one language.
pub struct TextProfile {
    /// Whole words replaced by their long form, applied after punctuation is stripped.
    pub expansions: &'static [(&'static str, &'static str)],
    /// Words carrying no distinguishing weight, dropped from a comparison key.
    pub fillers: &'static [&'static str],
    /// Words naming the kind of thoroughfare rather than which one, dropped from distinctive words.
    pub street_types: &'static [&'static str],
    /// Words naming the kind of place an address sits in rather than a street, skipped where they open
    /// one. Written as the canonical form reads them, abbreviations expanded and fillers dropped.
    pub designators: &'static [&'static str],
    /// A key shared by spellings that sound alike, when the language has one worth trusting.
    pub sounds: Option<fn(&str) -> String>,
    /// The road number a canonical text names, in one form for every way of writing it, when the
    /// country numbers its roads.
    pub road_number: Option<fn(&str) -> Option<String>>,
}

impl TextProfile {
    fn expand<'a>(&self, word: &'a str) -> &'a str {
        self.expansions
            .iter()
            .find(|(short, _)| *short == word)
            .map(|(_, long)| *long)
            .unwrap_or(word)
    }

    fn is_filler(&self, word: &str) -> bool {
        self.fillers.contains(&word)
    }

    fn is_street_type(&self, word: &str) -> bool {
        self.street_types.contains(&word)
    }

    fn opening_designator(&self, words: &[&str]) -> Option<usize> {
        self.designators
            .iter()
            .map(|designator| designator.split_whitespace().collect::<Vec<_>>())
            .filter(|designator| words.starts_with(designator))
            .map(|designator| designator.len())
            .max()
    }
}

/// Case and accent folding only, for anywhere the local conventions are unknown.
#[cfg(test)]
pub(crate) const NEUTRAL: TextProfile = TextProfile {
    expansions: &[],
    fillers: &[],
    street_types: &[],
    designators: &[],
    sounds: None,
    road_number: None,
};

/// French postal conventions: street type abbreviations, the saint that starts half the communes,
/// and the articles every street name shares.
pub const FRENCH: TextProfile = TextProfile {
    expansions: &[
        ("AV", "AVENUE"),
        ("AVE", "AVENUE"),
        ("BD", "BOULEVARD"),
        ("BLD", "BOULEVARD"),
        ("BLVD", "BOULEVARD"),
        ("RTE", "ROUTE"),
        ("PL", "PLACE"),
        ("CHE", "CHEMIN"),
        ("CHEM", "CHEMIN"),
        ("IMP", "IMPASSE"),
        ("ALL", "ALLEE"),
        ("RES", "RESIDENCE"),
        ("SQ", "SQUARE"),
        ("FG", "FAUBOURG"),
        ("GDE", "GRANDE"),
        ("GD", "GRAND"),
        ("RPT", "ROND POINT"),
        ("ST", "SAINT"),
        ("STE", "SAINTE"),
        ("STS", "SAINTS"),
        ("STES", "SAINTES"),
        ("ZA", "ZONE ARTISANALE"),
        ("ZI", "ZONE INDUSTRIELLE"),
        ("ZAC", "ZONE ARTISANALE"),
        ("CCAL", "CENTRE COMMERCIAL"),
        ("CC", "CENTRE COMMERCIAL"),
        ("UN", "1"),
        ("DEUX", "2"),
        ("TROIS", "3"),
        ("QUATRE", "4"),
        ("CINQ", "5"),
        ("SIX", "6"),
        ("SEPT", "7"),
        ("HUIT", "8"),
        ("NEUF", "9"),
        ("DIX", "10"),
        ("ONZE", "11"),
        ("DOUZE", "12"),
    ],
    fillers: &[
        "DE", "DU", "DES", "LA", "LE", "LES", "LEZ", "L", "D", "A", "AU", "AUX", "ET", "SUR", "SOUS", "EN",
    ],
    street_types: &[
        "RUE",
        "ROUTE",
        "AVENUE",
        "BOULEVARD",
        "PLACE",
        "CHEMIN",
        "IMPASSE",
        "ALLEE",
        "SQUARE",
        "QUAI",
        "COURS",
        "VOIE",
        "FAUBOURG",
        "ROND",
        "POINT",
        "RESIDENCE",
        "GRANDE",
        "GRAND",
        "LOTISSEMENT",
        "MONTEE",
        "PASSAGE",
        "SENTIER",
        "ESPLANADE",
        "TRAVERSE",
        "PROMENADE",
    ],
    designators: &[
        "CENTRE COMMERCIAL",
        "CENTRE CIAL",
        "CTRE CIAL",
        "C CIAL",
        "CCIAL",
        "C C",
        "GALERIE MARCHANDE",
        "GALERIE COMMERCIALE",
        "ESPACE COMMERCIAL",
        "PARC COMMERCIAL",
        "PARC ACTIVITES",
        "PARC ACTIVITE",
        "ZONE ARTISANALE",
        "ZONE INDUSTRIELLE",
        "ZONE COMMERCIALE",
        "ZONE ACTIVITES",
        "ZONE ACTIVITE",
        "ZONE AMENAGEMENT CONCERTE",
        "ZONE",
        "ZAE",
        "ZC",
        "LIEU DIT",
        "LD",
        "QUARTIER",
    ],
    sounds: Some(french_sound_key),
    road_number: Some(french_road_number),
};

/// The French road number a canonical text names, `D620` or `N7`, wherever in the text it stands.
///
/// OpenStreetMap tags the road `ref=D 620`; a site file writes `RD 620`, `RD620`, `R.D. 620`, `CD
/// 620`, `D 620`, or `Route Départementale 620`, and a national road `RN 7` or `Route Nationale 7`.
/// Only departmental and national roads are read: an `A` or an `M` alone is too often a word.
pub fn french_road_number(canonical: &str) -> Option<String> {
    let words: Vec<&str> = canonical.split_whitespace().collect();
    let is_number = |word: &str| {
        word.starts_with(|c: char| c.is_ascii_digit())
            && word.chars().all(|c| c.is_ascii_alphanumeric())
            && word.chars().filter(|c| c.is_ascii_alphabetic()).count() <= 1
    };
    let class_of = |word: &str| match word {
        "D" | "RD" | "CD" | "DEPARTEMENTALE" | "DEPARTEMENTAL" => Some('D'),
        "N" | "RN" | "NATIONALE" => Some('N'),
        _ => None,
    };
    for at in 0..words.len() {
        let word = words[at];
        for (prefix, class) in [("RD", 'D'), ("CD", 'D'), ("RN", 'N'), ("D", 'D'), ("N", 'N')] {
            if let Some(number) = word.strip_prefix(prefix)
                && is_number(number)
            {
                return Some(format!("{class}{number}"));
            }
        }
        let next = words.get(at + 1).copied();
        let after_next = words.get(at + 2).copied();
        if let Some(class) = class_of(word)
            && next.is_some_and(is_number)
        {
            return Some(format!("{class}{}", next?));
        }
        if matches!(word, "R" | "C")
            && let Some(class) = next.and_then(class_of)
            && after_next.is_some_and(is_number)
        {
            return Some(format!("{class}{}", after_next?));
        }
    }
    None
}

/// A French sound key: letters that sound alike merged, silent ones dropped, doubles collapsed.
///
/// Much simpler than Phonex or Soundex2, and meant for one thing only: to let `MITERAND`,
/// `MITTERRAND` and `MITERRAN` meet while `MARTIN` and `MARTEL` stay apart.
pub fn french_sound_key(word: &str) -> String {
    let mut key = word.to_string();
    for (from, to) in [
        ("SCH", "X"),
        ("CH", "X"),
        ("SH", "X"),
        ("PH", "F"),
        ("EAU", "O"),
        ("AU", "O"),
        ("OU", "U"),
        ("AI", "E"),
        ("EI", "E"),
        ("QU", "K"),
        ("GU", "G"),
        ("CE", "SE"),
        ("CI", "SI"),
        ("CY", "SI"),
        ("GE", "JE"),
        ("GI", "JI"),
        ("C", "K"),
        ("Q", "K"),
        ("Y", "I"),
        ("Z", "S"),
        ("W", "V"),
        ("H", ""),
        ("EN", "AN"),
        ("EM", "AN"),
        ("AM", "AN"),
        ("AIN", "IN"),
        ("EIN", "IN"),
        ("UN", "IN"),
    ] {
        key = key.replace(from, to);
    }
    while key.len() > 3 && key.ends_with(['E', 'T', 'D', 'S', 'X', 'P']) {
        key.pop();
    }
    let mut collapsed = String::with_capacity(key.len());
    for character in key.chars() {
        if !collapsed.ends_with(character) {
            collapsed.push(character);
        }
    }
    collapsed
}

/// A house number as written on the door, kept apart from the street it sits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseNumber {
    /// The number itself.
    pub number: u32,
    /// `BIS`, `TER` and the like, empty when the number stands alone.
    pub suffix: String,
}

const REPETITION_INDICES: [&str; 4] = ["BIS", "TER", "QUATER", "QUINQUIES"];

fn fold_accent(character: char) -> char {
    match character {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => 'Y',
        'ç' | 'Ç' => 'C',
        'ñ' | 'Ñ' => 'N',
        other => other,
    }
}

/// Upper case, accent folded, punctuation removed, abbreviations expanded, single spaced.
pub fn canonical_form(text: &str, profile: &TextProfile) -> String {
    let unligatured = text
        .replace(['œ', 'Œ'], "OE")
        .replace(['æ', 'Æ'], "AE")
        .replace('ß', "SS");

    let flattened: String = unligatured
        .chars()
        .map(fold_accent)
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                ' '
            }
        })
        .collect();

    flattened
        .split_whitespace()
        .map(|word| profile.expand(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The canonical form stripped of the words every name in the language shares.
///
/// `ROUTE DES TILLEULS` and `RTE TILLEULS` both reduce to `ROUTE TILLEULS`, which is the point: the two
/// records being compared were typed by different people years apart.
pub fn comparison_key(text: &str, profile: &TextProfile) -> String {
    canonical_form(text, profile)
        .split_whitespace()
        .filter(|word| !profile.is_filler(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A comparison key stripped of the words naming the kind of thoroughfare.
///
/// `ROUTE DE GENEVE` and `RUE DE GENEVE` are the same street written by two people who disagreed
/// about its type, while `ROUTE MARAIS` and `RUE MARAICHERS` are two streets that merely look
/// alike. Comparing what is left after the type is removed tells those cases apart, which overall
/// string distance cannot.
///
/// A name made entirely of street types keeps all of them, because `GRAND RUE` has nothing else.
pub fn distinctive_words(key: &str, profile: &TextProfile) -> String {
    let distinctive: Vec<&str> = key
        .split_whitespace()
        .filter(|word| !profile.is_street_type(word))
        .collect();

    if distinctive.is_empty() {
        key.to_string()
    } else {
        distinctive.join(" ")
    }
}

/// Splits a leading house number off an address line.
///
/// A line with no leading digit keeps all of itself as the street, which is common in this data:
/// `La Charmeraie` and `Place de La Gare` name a hamlet and a square, not a numbered address. A
/// range such as `104-106` or `1 a 5` yields its first number; [`parse_leading_doors`] reads them all.
pub fn split_leading_house_number(line: &str, profile: &TextProfile) -> (Option<HouseNumber>, String) {
    let (doors, street) = parse_leading_doors(line, profile);
    (doors.into_iter().next(), street)
}

/// Words that join two door numbers: `8 et 10`, `22 à 28`. A dash, a slash or an ampersand joins
/// them too, but punctuation is gone by the time words are read, so those arrive as two numbers side
/// by side.
const DOOR_CONNECTORS: [&str; 3] = ["ET", "A", "AU"];

/// Reads every door number at the head of an address line, and the street after them.
///
/// A shop spanning two buildings is written `5-7`, `5/7`, `8 et 10`, `22 à 28` or `30 Bis à 30
/// Ter`, and any of its doors may be the one recorded. Each keeps its own suffix, so `46 et 46 Bis`
/// is two doors. A letter after a number is its suffix, `4 C`, unless another number follows it, in
/// which case it is the French `à` of a range.
pub fn parse_leading_doors(line: &str, profile: &TextProfile) -> (Vec<HouseNumber>, String) {
    let canonical = canonical_form(line, profile);
    let words: Vec<&str> = canonical.split_whitespace().collect();
    let followed_by_a_number = |at: usize| words.get(at).is_some_and(|word| starts_with_digit(word));

    let mut doors = Vec::new();
    let mut at = 0;
    let mut glued_street: Option<&str> = None;

    while let Some(word) = words.get(at) {
        let digits: String = word.chars().take_while(char::is_ascii_digit).collect();
        let Ok(number) = digits.parse::<u32>() else { break };
        let glued = &word[digits.len()..];
        at += 1;

        let mut suffix = String::new();
        if REPETITION_INDICES.contains(&glued) || is_single_letter(glued) {
            suffix = glued.to_string();
        } else if !glued.is_empty() {
            glued_street = Some(glued);
        } else if let Some(next) = words.get(at)
            && (REPETITION_INDICES.contains(next)
                || (is_single_letter(next) && !followed_by_a_number(at + 1)))
        {
            suffix = (*next).to_string();
            at += 1;
        }
        doors.push(HouseNumber { number, suffix });

        if glued_street.is_some() {
            break;
        }
        match words.get(at) {
            Some(next) if starts_with_digit(next) => {}
            Some(next) if DOOR_CONNECTORS.contains(next) && followed_by_a_number(at + 1) => at += 1,
            _ => break,
        }
    }

    let rest: Vec<&str> = glued_street
        .into_iter()
        .chain(words[at.min(words.len())..].iter().copied())
        .collect();
    (doors, drop_fillers(&rest.join(" "), profile))
}

/// A street as [`parse_leading_doors`] reads it, without the designators that open it, or nothing
/// when none does.
///
/// `C.C. La Pinsonnière` names a shopping centre after its neighbourhood and `CC du Chemin Creux`
/// after its street, so `PINSONNIERE` and `CHEMIN CREUX` are what the index can match. Designators
/// are skipped one after another, `C.Cial Galerie Marchande Sommet` leaving `SOMMET`, and only where
/// they open the street: inside a name they are part of it.
pub fn without_opening_designators(street: &str, profile: &TextProfile) -> Option<String> {
    let mut words: Vec<&str> = street.split_whitespace().collect();
    let mut skipped = false;
    while let Some(length) = profile.opening_designator(&words) {
        words.drain(..length);
        skipped = true;
    }
    skipped.then(|| words.join(" "))
}

/// Reads an `addr:housenumber` value as the number a door carries.
///
/// Mappers write the same door as `8bis`, `8 bis` and `8 BIS`, and a building spanning several as
/// `8-10` or `12;14`. The first number is the one that answers a search for it, and its suffix is
/// kept so `8 BIS` is not taken for `8`.
pub fn parse_house_number(value: &str, profile: &TextProfile) -> Option<HouseNumber> {
    let first = value.split([';', ',']).next().unwrap_or("");
    let canonical = canonical_form(first, profile);
    let head = canonical.split_whitespace().next()?;
    let digits: String = head.chars().take_while(char::is_ascii_digit).collect();
    let number = digits.parse().ok()?;

    let glued = &head[digits.len()..];
    let suffix = if !glued.is_empty() && glued.chars().all(|character| character.is_ascii_alphabetic()) {
        glued.to_string()
    } else {
        canonical
            .split_whitespace()
            .nth(1)
            .filter(|word| REPETITION_INDICES.contains(word) || is_single_letter(word))
            .unwrap_or("")
            .to_string()
    };
    Some(HouseNumber { number, suffix })
}

fn is_single_letter(word: &str) -> bool {
    word.len() == 1 && word.chars().all(|character| character.is_ascii_alphabetic())
}

fn drop_fillers(canonical: &str, profile: &TextProfile) -> String {
    canonical
        .split_whitespace()
        .filter(|word| !profile.is_filler(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Removes every parenthesised group from a field, nested ones included.
///
/// A parenthesis in a site file is an annotation, not part of the address: `17 rue des Écoles (LE
/// BOURG)` names a part of the town, and read as words of the street it could match another street
/// altogether. An unclosed parenthesis is a typo rather than an annotation, so it is left
/// alone instead of swallowing the rest of the field.
pub fn without_parentheses(field: &str) -> String {
    let characters: Vec<char> = field.chars().collect();
    let mut kept = vec![true; characters.len()];
    let mut open: Vec<usize> = Vec::new();
    for (at, character) in characters.iter().enumerate() {
        match character {
            '(' => open.push(at),
            ')' => {
                if let Some(start) = open.pop() {
                    kept[start..=at].iter_mut().for_each(|keep| *keep = false);
                }
            }
            _ => {}
        }
    }
    if kept.iter().all(|keep| *keep) {
        return field.to_string();
    }
    let remaining: String = characters
        .iter()
        .zip(&kept)
        .filter(|(_, keep)| **keep)
        .map(|(character, _)| *character)
        .collect();
    remaining.split_whitespace().collect::<Vec<_>>().join(" ")
}

const DASHES: [char; 3] = ['-', '–', '—'];

/// Splits an address at its first spaced dash, returning what is kept and what is ignored.
///
/// A dash with a space on either side introduces a note, `3 Grande Rue - Le Hameau Fleuri` or
/// `Rue Louis Pasteur - ZAC des Landes`, and read as part of the street it spoils the match. A
/// dash inside a name has no space, `Saint-Exupéry`, and a spaced dash between two numbers is a
/// door range, `86 - 88 Rue Corot`; neither is cut. A field opening with a dash is left whole,
/// since ignoring everything after it would leave nothing to read.
pub fn cut_at_spaced_dash(address: &str) -> (String, Option<String>) {
    let characters: Vec<(usize, char)> = address.char_indices().collect();
    for (at, (offset, character)) in characters.iter().enumerate() {
        if !DASHES.contains(character) {
            continue;
        }
        let spaced_before = at > 0 && characters[at - 1].1.is_whitespace();
        let spaced_after = characters
            .get(at + 1)
            .is_some_and(|(_, next)| next.is_whitespace());
        if !(spaced_before || spaced_after) {
            continue;
        }
        let before = address[..*offset].trim_end();
        let after = address[offset + character.len_utf8()..].trim();
        let joins_two_doors =
            before.ends_with(|c: char| c.is_ascii_digit()) && after.starts_with(|c: char| c.is_ascii_digit());
        if joins_two_doors {
            continue;
        }
        if before.trim().is_empty() {
            return (address.to_string(), None);
        }
        return (before.to_string(), (!after.is_empty()).then(|| after.to_string()));
    }
    (address.to_string(), None)
}

/// Removes a French arrondissement suffix, leaving the town the postcode already identifies.
///
/// `PARIS 11EME ARRONDISSEMENT` is how a postal file writes what OpenStreetMap calls `Paris`. The
/// arrondissement is not lost: the postcode carries it.
pub fn strip_arrondissement(town: &str) -> String {
    let words: Vec<&str> = town.split_whitespace().collect();
    let without_suffix = match words.as_slice() {
        [head @ .., ordinal, "ARRONDISSEMENT"] if starts_with_digit(ordinal) => head,
        _ => words.as_slice(),
    };
    without_suffix.join(" ")
}

fn starts_with_digit(word: &str) -> bool {
    word.starts_with(|character: char| character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_every_way_of_writing_a_departmental_road_when_read_then_they_share_one_number() {
        for written in [
            "RD 620",
            "RD620",
            "R.D. 620",
            "D 620",
            "D620",
            "Route Départementale 620",
            "Départementale 620",
        ] {
            assert_eq!(
                french_road_number(&canonical_form(written, &FRENCH)).as_deref(),
                Some("D620"),
                "{written}"
            );
        }
        assert_eq!(
            french_road_number(&canonical_form("CD 18", &FRENCH)).as_deref(),
            Some("D18")
        );
        assert_eq!(
            french_road_number(&canonical_form("Chemin Départemental 18", &FRENCH)).as_deref(),
            Some("D18")
        );
    }

    #[test]
    fn given_every_way_of_writing_a_national_road_when_read_then_they_share_one_number() {
        for written in ["RN 7", "RN7", "N 7", "Route Nationale 7", "Nationale 7"] {
            assert_eq!(
                french_road_number(&canonical_form(written, &FRENCH)).as_deref(),
                Some("N7"),
                "{written}"
            );
        }
    }

    #[test]
    fn given_a_road_number_inside_a_longer_address_when_read_then_it_is_found() {
        assert_eq!(
            french_road_number("118 RD 712 PONTS NEUFS").as_deref(),
            Some("D712")
        );
        assert_eq!(
            french_road_number("ROUTE CAGNES SUR MER CD 18").as_deref(),
            Some("D18")
        );
    }

    #[test]
    fn given_a_street_with_a_number_but_no_road_class_when_read_then_it_is_no_road_number() {
        assert_eq!(french_road_number("RUE 8 MAI 1945"), None);
        assert_eq!(french_road_number("AVENUE 11 NOVEMBRE"), None);
        assert_eq!(french_road_number("ROUTE CAEN"), None);
    }

    #[test]
    fn given_spellings_that_sound_alike_when_keyed_then_they_share_a_key() {
        assert_eq!(french_sound_key("MITTERRAND"), french_sound_key("MITERAND"));
        assert_eq!(french_sound_key("PHARE"), french_sound_key("FARE"));
        assert_eq!(french_sound_key("BEAUMONT"), french_sound_key("BOMONT"));
    }

    #[test]
    fn given_names_that_sound_different_when_keyed_then_their_keys_differ() {
        assert_ne!(french_sound_key("MARTIN"), french_sound_key("MARTEL"));
    }

    fn searched(line: &str, profile: &TextProfile) -> String {
        let street = parse_leading_doors(line, profile).1;
        without_opening_designators(&street, profile).unwrap_or(street)
    }

    #[test]
    fn given_a_shopping_centre_abbreviation_when_doors_are_parsed_then_only_its_name_is_the_street() {
        assert_eq!(searched("C.C. La Pinsonnière", &FRENCH), "PINSONNIERE");
        assert_eq!(searched("CC du Chemin Creux", &FRENCH), "CHEMIN CREUX");
        assert_eq!(searched("C.Cial Camargue 2000", &FRENCH), "CAMARGUE 2000");
        assert_eq!(searched("Ctre Cial du Clos des Lys", &FRENCH), "CLOS LYS");
    }

    #[test]
    fn given_a_spelled_out_designator_when_doors_are_parsed_then_it_is_skipped() {
        assert_eq!(searched("Centre Commercial Aubépine", &FRENCH), "AUBEPINE");
        assert_eq!(searched("Parc d'Activités Peupliers", &FRENCH), "PEUPLIERS");
        assert_eq!(searched("Zone d'activité des Prés Hauts", &FRENCH), "PRES HAUTS");
        assert_eq!(searched("Lieu-dit Quatre Vents", &FRENCH), "4 VENTS");
    }

    #[test]
    fn given_a_zone_abbreviation_when_doors_are_parsed_then_it_is_skipped() {
        assert_eq!(searched("ZA Les Essarts", &FRENCH), "ESSARTS");
        assert_eq!(searched("ZAC de la Croix Blanche", &FRENCH), "CROIX BLANCHE");
        assert_eq!(searched("ZI Route de Bernay", &FRENCH), "ROUTE BERNAY");
        assert_eq!(searched("ZAE Bellevue", &FRENCH), "BELLEVUE");
    }

    #[test]
    fn given_doors_before_a_designator_when_parsed_then_the_doors_are_kept_and_the_designator_skipped() {
        let (doors, street) = parse_leading_doors("2 Zone Commerciale des Vergers", &FRENCH);

        assert_eq!(
            doors,
            vec![HouseNumber {
                number: 2,
                suffix: String::new()
            }]
        );
        assert_eq!(
            without_opening_designators(&street, &FRENCH).as_deref(),
            Some("VERGERS")
        );
    }

    #[test]
    fn given_designators_one_after_another_when_parsed_then_each_is_skipped() {
        assert_eq!(searched("C.Cial Galerie Marchande Sommet", &FRENCH), "SOMMET");
    }

    #[test]
    fn given_a_designator_alone_when_parsed_then_no_street_is_left() {
        assert_eq!(searched("C.C", &FRENCH), "");
        assert_eq!(searched("Zone Industrielle", &FRENCH), "");
    }

    /// Only a designator opening the street is skipped. Inside a name it is part of the name, and a
    /// word that merely starts like one, `Zoneville`, is not touched.
    #[test]
    fn given_a_designator_word_inside_a_street_name_when_parsed_then_it_is_kept() {
        let inside = parse_leading_doors("12 Rue du Centre Commercial", &FRENCH).1;
        assert_eq!(without_opening_designators(&inside, &FRENCH), None);
        assert_eq!(
            searched("12 Rue du Centre Commercial", &FRENCH),
            "RUE CENTRE COMMERCIAL"
        );
        assert_eq!(searched("Zoneville", &FRENCH), "ZONEVILLE");
        assert_eq!(searched("Galerie des Arcades", &FRENCH), "GALERIE ARCADES");
    }

    #[test]
    fn given_the_neutral_profile_when_parsed_then_no_designator_is_skipped() {
        assert_eq!(searched("CC Le Belvédère", &NEUTRAL), "CC LE BELVEDERE");
    }

    #[test]
    fn given_an_abbreviated_street_type_when_canonicalised_then_it_reads_as_the_long_form() {
        assert_eq!(canonical_form("Rte des Tilleuls", &FRENCH), "ROUTE DES TILLEULS");
    }

    #[test]
    fn given_accents_when_canonicalised_then_they_are_folded_rather_than_compared() {
        assert_eq!(canonical_form("Saint-Étienne", &FRENCH), "SAINT ETIENNE");
        assert_eq!(canonical_form("ST ETIENNE", &FRENCH), "SAINT ETIENNE");
    }

    #[test]
    fn given_a_ligature_when_canonicalised_then_it_expands_to_two_letters() {
        assert_eq!(canonical_form("Nœux-les-Mines", &FRENCH), "NOEUX LES MINES");
    }

    #[test]
    fn given_a_spelled_out_number_when_canonicalised_then_it_reads_as_the_digit() {
        assert_eq!(
            canonical_form("Rue du Onze Novembre", &FRENCH),
            "RUE DU 11 NOVEMBRE"
        );
    }

    #[test]
    fn given_filler_words_differing_when_keyed_then_the_two_spellings_match() {
        assert_eq!(
            comparison_key("Route des Tilleuls", &FRENCH),
            comparison_key("Rte Tilleuls", &FRENCH)
        );
    }

    #[test]
    fn given_a_street_type_and_a_name_when_reduced_to_distinctive_words_then_the_type_is_dropped() {
        assert_eq!(distinctive_words("ROUTE GENEVE", &FRENCH), "GENEVE");
        assert_eq!(distinctive_words("RUE GENEVE", &FRENCH), "GENEVE");
    }

    #[test]
    fn given_a_street_that_is_only_a_type_when_reduced_then_the_type_is_kept_rather_than_emptied() {
        assert_eq!(distinctive_words("GRAND RUE", &FRENCH), "GRAND RUE");
    }

    #[test]
    fn given_a_line_starting_with_a_number_when_split_then_the_number_comes_off() {
        let (number, street) = split_leading_house_number("8 Route des Tilleuls", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 8,
                suffix: String::new()
            })
        );
        assert_eq!(street, "ROUTE TILLEULS");
    }

    #[test]
    fn given_a_number_with_a_repetition_index_when_split_then_the_index_travels_with_it() {
        let (number, street) = split_leading_house_number("12 bis rue des Lilas", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 12,
                suffix: "BIS".into()
            })
        );
        assert_eq!(street, "RUE LILAS");
    }

    #[test]
    fn given_a_range_of_numbers_when_split_then_the_first_is_taken() {
        let (number, street) = split_leading_house_number("104-106 rue Sisley", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 104,
                suffix: String::new()
            })
        );
        assert_eq!(street, "RUE SISLEY");
    }

    #[test]
    fn given_a_place_rather_than_a_numbered_address_when_split_then_all_of_it_is_the_street() {
        let (number, street) = split_leading_house_number("La Charmeraie", &FRENCH);

        assert_eq!(number, None);
        assert_eq!(street, "CHARMERAIE");
    }

    #[test]
    fn given_an_arrondissement_suffix_when_stripped_then_the_town_name_remains() {
        assert_eq!(strip_arrondissement("PARIS 11EME ARRONDISSEMENT"), "PARIS");
        assert_eq!(strip_arrondissement("LYON 3EME ARRONDISSEMENT"), "LYON");
    }

    #[test]
    fn given_a_town_with_no_arrondissement_when_stripped_then_it_is_unchanged() {
        assert_eq!(strip_arrondissement("BOURG EN BRESSE"), "BOURG EN BRESSE");
    }

    #[test]
    fn given_the_neutral_profile_when_canonicalised_then_no_abbreviation_is_invented() {
        assert_eq!(canonical_form("St Peter Street", &NEUTRAL), "ST PETER STREET");
    }

    #[test]
    fn given_the_neutral_profile_when_keyed_then_nothing_is_dropped() {
        assert_eq!(comparison_key("Calle de la Paz", &NEUTRAL), "CALLE DE LA PAZ");
    }

    #[test]
    fn given_a_range_written_with_a_connector_when_split_then_only_the_first_number_is_taken() {
        let (number, street) = split_leading_house_number("1 a 5 Rue des Lilas", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 1,
                suffix: String::new()
            })
        );
        assert_eq!(street, "RUE LILAS");
    }

    #[test]
    fn given_a_single_letter_after_the_number_when_split_then_it_is_the_repetition_index() {
        let (number, street) = split_leading_house_number("4 C Rue des Acacias", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 4,
                suffix: "C".into()
            })
        );
        assert_eq!(street, "RUE ACACIAS");
    }

    #[test]
    fn given_a_range_followed_by_a_repetition_index_when_split_then_the_index_belongs_to_the_last_door() {
        let (number, street) = split_leading_house_number("17-19 Bis Av des Tilleuls", &FRENCH);

        assert_eq!(
            number,
            Some(HouseNumber {
                number: 17,
                suffix: String::new()
            })
        );
        assert_eq!(street, "AVENUE TILLEULS");
    }

    #[test]
    fn given_a_bare_number_when_parsed_then_it_has_no_suffix() {
        assert_eq!(
            parse_house_number("8", &FRENCH),
            Some(HouseNumber {
                number: 8,
                suffix: String::new()
            })
        );
    }

    #[test]
    fn given_a_suffix_glued_to_the_number_when_parsed_then_it_is_separated() {
        assert_eq!(
            parse_house_number("8bis", &FRENCH),
            Some(HouseNumber {
                number: 8,
                suffix: "BIS".into()
            })
        );
        assert_eq!(
            parse_house_number("12B", &FRENCH),
            Some(HouseNumber {
                number: 12,
                suffix: "B".into()
            })
        );
    }

    #[test]
    fn given_a_spaced_suffix_when_parsed_then_it_is_kept() {
        assert_eq!(
            parse_house_number("8 ter", &FRENCH),
            Some(HouseNumber {
                number: 8,
                suffix: "TER".into()
            })
        );
    }

    #[test]
    fn given_a_range_or_a_list_when_parsed_then_the_first_number_is_the_door() {
        assert_eq!(
            parse_house_number("8-10", &FRENCH),
            Some(HouseNumber {
                number: 8,
                suffix: String::new()
            })
        );
        assert_eq!(
            parse_house_number("12;14", &FRENCH),
            Some(HouseNumber {
                number: 12,
                suffix: String::new()
            })
        );
    }

    #[test]
    fn given_a_value_with_no_digit_when_parsed_then_it_is_no_house_number() {
        assert_eq!(parse_house_number("SN", &FRENCH), None);
    }

    fn doors(line: &str) -> (Vec<(u32, String)>, String) {
        let (found, street) = parse_leading_doors(line, &FRENCH);
        (
            found.into_iter().map(|door| (door.number, door.suffix)).collect(),
            street,
        )
    }

    fn plain(numbers: &[u32]) -> Vec<(u32, String)> {
        numbers.iter().map(|number| (*number, String::new())).collect()
    }

    #[test]
    fn given_doors_joined_by_a_dash_when_parsed_then_both_are_read() {
        assert_eq!(
            doors("5-7 Boulevard des Cèdres"),
            (plain(&[5, 7]), "BOULEVARD CEDRES".into())
        );
        assert_eq!(
            doors("130-134 Rue des Charmes"),
            (plain(&[130, 134]), "RUE CHARMES".into())
        );
    }

    #[test]
    fn given_doors_joined_by_a_slash_when_parsed_then_both_are_read() {
        assert_eq!(
            doors("5/7 Rue Saint-Exupéry"),
            (plain(&[5, 7]), "RUE SAINT EXUPERY".into())
        );
    }

    #[test]
    fn given_doors_joined_by_a_word_when_parsed_then_both_are_read() {
        assert_eq!(doors("8 et 10 Rue des Lilas").0, plain(&[8, 10]));
        assert_eq!(doors("22 à 28 Quai des Brumes").0, plain(&[22, 28]));
        assert_eq!(doors("158 A 164 Grande Rue").0, plain(&[158, 164]));
    }

    #[test]
    fn given_doors_carrying_suffixes_when_parsed_then_each_keeps_its_own() {
        assert_eq!(
            doors("46 et 46 Bis Quai des Saules").0,
            vec![(46, String::new()), (46, "BIS".into())]
        );
        assert_eq!(
            doors("30 Bis à 30 Ter Boulevard des Alliés").0,
            vec![(30, "BIS".into()), (30, "TER".into())]
        );
        assert_eq!(
            doors("7-7Bis Rue Courbet").0,
            vec![(7, String::new()), (7, "BIS".into())]
        );
    }

    #[test]
    fn given_three_doors_across_two_separators_when_parsed_then_all_three_are_read() {
        assert_eq!(
            doors("3 Bis et 5-7 Place des Halles").0,
            vec![(3, "BIS".into()), (5, String::new()), (7, String::new())]
        );
    }

    #[test]
    fn given_a_door_glued_to_the_street_when_parsed_then_the_street_starts_after_the_digits() {
        assert_eq!(
            doors("21-23rue des Platanes"),
            (plain(&[21, 23]), "RUE PLATANES".into())
        );
    }

    #[test]
    fn given_a_letter_suffix_before_the_street_when_parsed_then_it_is_a_suffix_not_a_connector() {
        assert_eq!(
            doors("4 C Rue des Acacias"),
            (vec![(4, "C".into())], "RUE ACACIAS".into())
        );
    }

    #[test]
    fn given_no_leading_door_when_parsed_then_there_are_none_and_all_of_it_is_the_street() {
        assert_eq!(doors("La Charmeraie"), (vec![], "CHARMERAIE".into()));
    }

    #[test]
    fn given_a_parenthesis_when_removed_then_the_rest_of_the_field_is_kept() {
        assert_eq!(
            without_parentheses("17 rue des Écoles (LE BOURG)"),
            "17 rue des Écoles"
        );
    }

    #[test]
    fn given_several_or_nested_parentheses_when_removed_then_all_of_them_go() {
        assert_eq!(
            without_parentheses("(CC) 8 Route (ex (RN) 6) des Tilleuls"),
            "8 Route des Tilleuls"
        );
    }

    /// An unclosed parenthesis is a typo, not an annotation, and dropping everything after it would
    /// throw the street away.
    #[test]
    fn given_an_unclosed_parenthesis_when_removed_then_nothing_after_it_is_lost() {
        assert_eq!(
            without_parentheses("8 Route des Tilleuls (Sens"),
            "8 Route des Tilleuls (Sens"
        );
    }

    #[test]
    fn given_no_parenthesis_when_removed_then_the_field_is_unchanged() {
        assert_eq!(
            without_parentheses("8 Route des Tilleuls"),
            "8 Route des Tilleuls"
        );
    }

    #[test]
    fn given_a_spaced_dash_when_cut_then_what_follows_is_ignored_and_returned() {
        assert_eq!(
            cut_at_spaced_dash("3 Grande Rue - Le Hameau Fleuri"),
            ("3 Grande Rue".to_string(), Some("Le Hameau Fleuri".to_string()))
        );
    }

    #[test]
    fn given_a_dash_with_a_space_on_one_side_only_when_cut_then_it_still_counts() {
        assert_eq!(
            cut_at_spaced_dash("53 Avenue des Amandiers- Val Fleuri").0,
            "53 Avenue des Amandiers"
        );
        assert_eq!(
            cut_at_spaced_dash("Rue Lavandière -Cap Horizon").0,
            "Rue Lavandière"
        );
    }

    #[test]
    fn given_a_dash_inside_a_name_when_cut_then_nothing_is_ignored() {
        assert_eq!(
            cut_at_spaced_dash("5/7 Rue Saint-Exupéry"),
            ("5/7 Rue Saint-Exupéry".to_string(), None)
        );
    }

    /// `86 - 88 Rue Corot` is one shop across two doors, not a street followed by a note.
    #[test]
    fn given_a_spaced_dash_between_two_door_numbers_when_cut_then_it_is_a_range_and_kept() {
        assert_eq!(
            cut_at_spaced_dash("86 - 88 Rue Corot"),
            ("86 - 88 Rue Corot".to_string(), None)
        );
    }

    #[test]
    fn given_an_en_dash_or_an_em_dash_spaced_when_cut_then_it_counts_like_a_hyphen() {
        assert_eq!(cut_at_spaced_dash("Rue X – Zone Y").0, "Rue X");
        assert_eq!(cut_at_spaced_dash("Rue X — Zone Y").0, "Rue X");
    }

    #[test]
    fn given_a_field_opening_with_a_spaced_dash_when_cut_then_nothing_is_left_to_ignore_it_for() {
        assert_eq!(cut_at_spaced_dash(" - Rue X"), (" - Rue X".to_string(), None));
    }
}
