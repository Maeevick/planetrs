//! Step fifteen: the street a messy name means, when none of the tiers of [`crate::street`] found it.
//!
//! Three rules, strictest first, each forgiving one kind of difference only:
//!
//! - **both ways**: every distinctive word pairs with one on the other side, forgiven a typo scaled to
//!   its length, its sound, an initial, or a space; spelling is forgiven only when the street types
//!   agree;
//! - **asked within the street**: every asked word is in the street letter for letter, and the street
//!   has more, `RUE MITTERRAND` for `RUE PRESIDENT MITTERRAND`;
//! - **street within the asked text**: every word of the street is in what was asked letter for
//!   letter, and the text has more as one block before or after it, a centre's name or a locality.
//!
//! Each answers only when a single street of the commune fits it, and two fitting streets end the
//! search: a tie says the name is ambiguous, and a looser rule that broke it would be breaking it by
//! luck. Measured on 3 000 streets drawn from the index with fourteen kinds of noise, these rules
//! raise the streets found from 64% to 82% (`geocoder/discovery`).

use std::collections::HashSet;

use crate::rarity::WordRarity;
use crate::similarity::edit_distance_within;
use crate::street::{Quality, StreetMatch};
use crate::text::TextProfile;

/// The one street of `candidates` the rules allow `asked` to mean, or nothing.
///
/// Both are comparison keys. Names repeated across the communes searched count once.
pub fn unique_street_match(
    asked: &str,
    candidates: &[&str],
    profile: &TextProfile,
    rarity: &WordRarity,
) -> Option<StreetMatch> {
    let asked = Name::of(asked, profile);
    if asked.words.is_empty() {
        return None;
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let candidates: Vec<Name> = candidates
        .iter()
        .filter(|name| !name.is_empty() && seen.insert(name))
        .map(|name| Name::of(name, profile))
        .collect();
    let pairings: Vec<Pairing> = candidates
        .iter()
        .map(|candidate| pair_words(&asked, candidate))
        .collect();
    let designator_words: HashSet<&str> = profile
        .designators
        .iter()
        .flat_map(|designator| designator.split_whitespace())
        .collect();

    let both_ways = |candidate: &Name, pairing: &Pairing| {
        every_distinctive_paired(&asked, &pairing.asked)
            && every_distinctive_paired(candidate, &pairing.recorded)
            && (asked.same_types_as(candidate) || letter_for_letter(&asked, &pairing.asked))
    };
    let asked_within = |candidate: &Name, pairing: &Pairing| {
        every_distinctive_paired(&asked, &pairing.asked)
            && letter_for_letter(&asked, &pairing.asked)
            && rare_enough(&asked, &pairing.asked, rarity)
            && outweighs(candidate, &pairing.recorded, rarity, &designator_words)
    };
    let street_within = |candidate: &Name, pairing: &Pairing| {
        every_distinctive_paired(candidate, &pairing.recorded)
            && letter_for_letter(candidate, &pairing.recorded)
            && rare_enough(candidate, &pairing.recorded, rarity)
            && unpaired_in_one_block(&pairing.asked)
            && outweighs(&asked, &pairing.asked, rarity, &designator_words)
    };
    let rules: [(Quality, &Rule); 3] = [
        (Quality::BothWays, &both_ways),
        (Quality::AskedWithin, &asked_within),
        (Quality::StreetWithin, &street_within),
    ];
    for (quality, rule) in rules {
        let fitting: Vec<&Name> = candidates
            .iter()
            .zip(&pairings)
            .filter(|(candidate, pairing)| rule(candidate, pairing))
            .map(|(candidate, _)| candidate)
            .collect();
        match fitting[..] {
            [only] => {
                return Some(StreetMatch {
                    name: only.text.clone(),
                    quality,
                });
            }
            [] => continue,
            _ => return None,
        }
    }
    None
}

/// Whether a candidate street fits one of the rules, given how its words paired with the asked ones.
type Rule<'a> = dyn Fn(&Name, &Pairing) -> bool + 'a;

/// A name split into words, each marked as a street type or not, with its sound key.
struct Name {
    text: String,
    words: Vec<String>,
    is_type: Vec<bool>,
    sounds: Vec<Option<String>>,
}

impl Name {
    fn of(text: &str, profile: &TextProfile) -> Self {
        let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
        Self {
            text: text.to_string(),
            is_type: words
                .iter()
                .map(|word| profile.street_types.contains(&word.as_str()))
                .collect(),
            sounds: words
                .iter()
                .map(|word| profile.sounds.map(|key| key(word)))
                .collect(),
            words,
        }
    }

    fn same_types_as(&self, other: &Name) -> bool {
        let types = |name: &Name| -> Vec<String> {
            let mut types: Vec<String> = name
                .words
                .iter()
                .zip(&name.is_type)
                .filter(|(_, is_type)| **is_type)
                .map(|(word, _)| word.clone())
                .collect();
            types.sort();
            types
        };
        types(self) == types(other)
    }
}

/// How well each word of each side found a partner: 1 for itself or for two words glued, less for a
/// typo, a sound or an initial, 0 for none.
struct Pairing {
    asked: Vec<f64>,
    recorded: Vec<f64>,
}

/// How far two words can be apart and still be one word misspelt: nothing up to three letters, one
/// edit up to seven, two beyond. Elasticsearch's `fuzziness: AUTO`, one letter later, since French
/// street words are long.
fn edit_budget(letters: usize) -> usize {
    match letters {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

const ONE_EDIT: f64 = 0.85;
const SAME_SOUND: f64 = 0.85;
const INITIAL: f64 = 0.5;
/// A sound key is trusted only on words this long: `ETRE` and `HETRES` sound alike and are not the
/// same place.
const SOUND_FROM_LETTERS: usize = 6;

fn word_likeness(asked: &Name, at: usize, recorded: &Name, to: usize) -> f64 {
    let (word, other) = (&asked.words[at], &recorded.words[to]);
    if word == other {
        return 1.0;
    }
    let letters = word.chars().count().min(other.chars().count());
    let budget = edit_budget(letters);
    if budget > 0
        && let Some(distance) = edit_distance_within(word, other, budget)
    {
        return 1.0 - (1.0 - ONE_EDIT) * distance as f64;
    }
    if letters >= SOUND_FROM_LETTERS && asked.sounds[at].is_some() && asked.sounds[at] == recorded.sounds[to]
    {
        return SAME_SOUND;
    }
    if word.chars().count() == 1 && other.chars().count() >= 3 && other.starts_with(word.as_str()) {
        return INITIAL;
    }
    0.0
}

/// Pairs asked and recorded words one to one, glued words first, then the likeliest pairs, a street
/// type only ever with a street type.
fn pair_words(asked: &Name, recorded: &Name) -> Pairing {
    let mut pairing = Pairing {
        asked: vec![0.0; asked.words.len()],
        recorded: vec![0.0; recorded.words.len()],
    };
    for at in 0..asked.words.len() {
        for to in 0..recorded.words.len().saturating_sub(1) {
            if pairing.recorded[to] == 0.0
                && asked.words[at] == format!("{}{}", recorded.words[to], recorded.words[to + 1])
            {
                pairing.asked[at] = 1.0;
                pairing.recorded[to] = 1.0;
                pairing.recorded[to + 1] = 1.0;
            }
        }
    }
    for to in 0..recorded.words.len() {
        for at in 0..asked.words.len().saturating_sub(1) {
            if pairing.recorded[to] == 0.0
                && pairing.asked[at] == 0.0
                && pairing.asked[at + 1] == 0.0
                && recorded.words[to] == format!("{}{}", asked.words[at], asked.words[at + 1])
            {
                pairing.recorded[to] = 1.0;
                pairing.asked[at] = 1.0;
                pairing.asked[at + 1] = 1.0;
            }
        }
    }
    let mut likely: Vec<(f64, usize, usize)> = Vec::new();
    for at in 0..asked.words.len() {
        for to in 0..recorded.words.len() {
            if pairing.asked[at] > 0.0
                || pairing.recorded[to] > 0.0
                || asked.is_type[at] != recorded.is_type[to]
            {
                continue;
            }
            let likeness = word_likeness(asked, at, recorded, to);
            if likeness > 0.0 {
                likely.push((likeness, at, to));
            }
        }
    }
    likely.sort_by(|left, right| right.0.total_cmp(&left.0));
    for (likeness, at, to) in likely {
        if pairing.asked[at] == 0.0 && pairing.recorded[to] == 0.0 {
            pairing.asked[at] = likeness;
            pairing.recorded[to] = likeness;
        }
    }
    pairing
}

fn every_distinctive_paired(name: &Name, paired: &[f64]) -> bool {
    let distinctive: Vec<f64> = paired
        .iter()
        .zip(&name.is_type)
        .filter(|(_, is_type)| !**is_type)
        .map(|(paired, _)| *paired)
        .collect();
    !distinctive.is_empty() && distinctive.iter().all(|paired| *paired > 0.0)
}

/// Whether every distinctive word that found a partner found it letter for letter, or glued to its
/// neighbour. The rules that forgive a missing or an extra word forgive nothing else.
fn letter_for_letter(name: &Name, paired: &[f64]) -> bool {
    paired
        .iter()
        .zip(&name.is_type)
        .all(|(paired, is_type)| *is_type || *paired == 0.0 || *paired == 1.0)
}

/// Enough rarity among the paired words that the match is about a name and not about `CHAMPS` or
/// `MAIRIE`: roughly one word found in fewer than one street name in a thousand.
const RARE_ENOUGH: f64 = 7.0;

fn rare_enough(name: &Name, paired: &[f64], rarity: &WordRarity) -> bool {
    name.words
        .iter()
        .zip(paired)
        .zip(&name.is_type)
        .filter(|((_, paired), is_type)| !**is_type && **paired >= ONE_EDIT)
        .map(|((word, _), _)| rarity.of_word(word))
        .sum::<f64>()
        >= RARE_ENOUGH
}

/// Whether the paired words weigh at least as much as those left over, so that `RUE MADELEINE` does
/// not answer `COLLONGE MADELEINE` for want of a `COLLONGE`. The profile's designator words weigh
/// nothing: it already knows `CENTRE COMMERCIAL` names a place and not a street.
fn outweighs(name: &Name, paired: &[f64], rarity: &WordRarity, designator_words: &HashSet<&str>) -> bool {
    let (mut paired_weight, mut left_over) = (0.0, 0.0);
    for ((word, paired), is_type) in name.words.iter().zip(paired).zip(&name.is_type) {
        if *is_type || designator_words.contains(word.as_str()) {
            continue;
        }
        if *paired > 0.0 {
            paired_weight += rarity.of_word(word);
        } else {
            left_over += rarity.of_word(word);
        }
    }
    paired_weight >= left_over
}

/// Whether the asked words left unpaired sit together before or after the paired ones, as a
/// centre's name or a locality does, rather than scattered through them.
fn unpaired_in_one_block(paired: &[f64]) -> bool {
    let first = paired.iter().position(|paired| *paired > 0.0);
    let last = paired.iter().rposition(|paired| *paired > 0.0);
    match (first, last) {
        (Some(first), Some(last)) => paired[first..=last].iter().all(|paired| *paired > 0.0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FRENCH;

    fn rarity() -> WordRarity {
        WordRarity::from_counts(
            3_000_000,
            &[
                ("RUE", 900_000),
                ("CHEMIN", 300_000),
                ("PRESIDENT", 5_000),
                ("MITTERRAND", 800),
                ("GARE", 20_000),
                ("JAURES", 3_000),
                ("JEAN", 40_000),
                ("JACQUES", 20_000),
                ("MADELEINE", 2_000),
                ("COLLONGE", 50),
                ("CAP", 3_000),
                ("HORIZON", 3_000),
                ("RIEZ", 400),
                ("LONGS", 3_000),
                ("CHANIN", 40),
            ],
        )
    }

    fn matched(asked: &str, candidates: &[&str]) -> Option<(String, Quality)> {
        unique_street_match(asked, candidates, &FRENCH, &rarity()).map(|found| (found.name, found.quality))
    }

    #[test]
    fn given_misspelt_words_on_both_sides_when_matched_then_the_one_street_they_pair_with_answers() {
        assert_eq!(
            matched(
                "RUE PRESIDANT MITERAND",
                &["RUE PRESIDENT MITTERRAND", "RUE GARE"]
            ),
            Some(("RUE PRESIDENT MITTERRAND".into(), Quality::BothWays))
        );
    }

    #[test]
    fn given_an_initial_for_a_first_name_when_matched_then_the_street_it_starts_answers() {
        assert_eq!(
            matched("RUE J JAURES", &["RUE JEAN JAURES", "RUE GARE"]),
            Some(("RUE JEAN JAURES".into(), Quality::BothWays))
        );
    }

    #[test]
    fn given_two_streets_fitting_the_same_rule_when_matched_then_neither_answers() {
        assert_eq!(
            matched("RUE J JAURES", &["RUE JEAN JAURES", "RUE JACQUES JAURES"]),
            None
        );
    }

    #[test]
    fn given_a_word_dropped_from_the_name_when_matched_then_the_street_holding_the_rest_answers() {
        assert_eq!(
            matched("RUE MITTERRAND", &["RUE PRESIDENT MITTERRAND", "RUE GARE"]),
            Some(("RUE PRESIDENT MITTERRAND".into(), Quality::AskedWithin))
        );
    }

    /// `COLLONGE` is rarer than `MADELEINE`: a street carrying it is a different street, not the
    /// asked one with a word the asker left out.
    #[test]
    fn given_the_street_has_a_rarer_word_than_the_asked_ones_when_matched_then_it_does_not_answer() {
        assert_eq!(
            matched("RUE MADELEINE", &["RUE COLLONGE MADELEINE", "RUE GARE"]),
            None
        );
    }

    #[test]
    fn given_a_centres_name_before_the_street_when_matched_then_the_street_answers() {
        assert_eq!(
            matched(
                "CENTRE COMMERCIAL CAP HORIZON RUE LAVANDIERE",
                &["RUE LAVANDIERE", "RUE GARE"]
            ),
            Some(("RUE LAVANDIERE".into(), Quality::StreetWithin))
        );
    }

    #[test]
    fn given_extra_words_scattered_through_the_street_when_matched_then_it_does_not_answer() {
        assert_eq!(
            matched("RUE CAP LAVANDIERE HORIZON", &["RUE LAVANDIERE", "RUE GARE"]),
            None
        );
    }

    /// `CHEMIN` is a kind of road, `CHANIN` a place: a street type pairs only with a street type.
    #[test]
    fn given_a_street_type_resembling_a_place_name_when_matched_then_they_are_not_paired() {
        assert_eq!(
            matched("CHEMIN DEPARTEMENTAL SUCRERIE", &["CHANIN", "RUE GARE"]),
            None
        );
    }

    /// Forgiving a typo and a missing word at once is how `RIEU` came to answer `LONGS RIEZ`.
    #[test]
    fn given_a_misspelt_word_and_a_missing_one_when_matched_then_it_does_not_answer() {
        assert_eq!(matched("CHEMIN RIEU", &["CHEMIN LONGS RIEZ", "RUE GARE"]), None);
    }

    #[test]
    fn given_a_name_repeated_across_the_communes_searched_when_matched_then_it_counts_once() {
        assert_eq!(
            matched(
                "RUE MITTERRAND",
                &["RUE PRESIDENT MITTERRAND", "RUE PRESIDENT MITTERRAND"]
            ),
            Some(("RUE PRESIDENT MITTERRAND".into(), Quality::AskedWithin))
        );
    }
}
