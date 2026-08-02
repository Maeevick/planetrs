//! Deciding whether a street as asked for and a street as recorded are the same street.
//!
//! Four tiers, tried in order and stopping at the first that holds, so that an answer carries how
//! much had to be forgiven to reach it. The order is not arbitrary: each tier sees a kind of
//! difference the ones above it cannot, and each is looser than the one above, so consulting a
//! looser tier before a stricter one would answer with an approximation while an exact match sat
//! unread in the same commune.
//!
//! Every tier assumes its candidates are the streets of one commune, already settled. Against a
//! national list these thresholds would join streets in different towns; against a few hundred
//! streets they are affordable and safe.

use crate::similarity::{similarity, word_overlap};
use crate::text::{TextProfile, distinctive_words};

/// How much had to be forgiven for a street to match, strictest first, so that comparing two
/// qualities says which match to believe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Quality {
    /// The street as recorded, letter for letter once both are reduced to comparison keys.
    Exact,
    /// The same letters, spaced differently: `VAN GOGH` for `VANGOGH`.
    Spacing,
    /// Most words shared, in any order.
    Words,
    /// Most letters shared: a typo, `CLEMANCEAU` for `CLEMENCEAU`.
    Characters,
    /// The same name with another street type: `ROUTE GENEVE` for `RUE GENEVE`.
    StreetType,
    /// Only when every tier above found nothing in any part: the one street whose distinctive
    /// words all pair with the asked ones, each forgiven a typo, its sound or an initial.
    BothWays,
    /// The one street holding every asked word letter for letter, and more.
    AskedWithin,
    /// The one street whose every word is in what was asked, which says more before or after it.
    StreetWithin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreetMatch {
    pub name: String,
    pub quality: Quality,
}

/// Deliberately high. At 0.6 `RUE JEAN RENOIR` does not answer a search for `RUE AUGUSTE
/// RENOIR`, which is a different street in the same city. Anything looser starts inventing
/// matches, and a wrong street is worse than no street because it looks precise.
const SHARED_WORDS: f64 = 0.6;

/// Calibrated so that a typo passes and a different name does not. `CLEMANCEAU` against
/// `CLEMENCEAU` in a whole name scores 0.96; `MARAIS` against `MARAICHERS` scores 0.6.
const SHARED_LETTERS: f64 = 0.88;

/// Applied to what is left once the street type is removed, where a single wrong word is most of
/// the string, so the bar has to be higher than for a whole name.
const SHARED_DISTINCTIVE_LETTERS: f64 = 0.9;

/// The recorded street that best answers `asked`, or nothing when none of them does.
pub fn best_street_match<'a>(
    asked: &str,
    candidates: impl IntoIterator<Item = &'a str> + Clone,
    profile: &TextProfile,
) -> Option<StreetMatch> {
    if asked.is_empty() {
        return None;
    }

    exact_match(asked, candidates.clone())
        .or_else(|| spacing_match(asked, candidates.clone()))
        .or_else(|| shared_words_match(asked, candidates.clone()))
        .or_else(|| shared_letters_match(asked, candidates.clone()))
        .or_else(|| street_type_match(asked, candidates, profile))
}

/// Whether a recorded street is the asked one, spelt, spaced or typed differently at most.
///
/// Stricter than [`best_street_match`], which also accepts a street merely sharing most of its words:
/// the right call when choosing among a commune's streets, the wrong one when asking whether a shop
/// stands on the asked street, since Faubourg Saint-Jacques shares three of five words with Faubourg
/// Saint-Vincent.
pub fn is_same_street(asked: &str, recorded: &str, profile: &TextProfile) -> bool {
    !asked.is_empty()
        && !recorded.is_empty()
        && (exact_match(asked, [recorded]).is_some()
            || spacing_match(asked, [recorded]).is_some()
            || shared_letters_match(asked, [recorded]).is_some()
            || street_type_match(asked, [recorded], profile).is_some())
}

fn exact_match<'a>(asked: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<StreetMatch> {
    candidates
        .into_iter()
        .find(|candidate| *candidate == asked)
        .map(|name| StreetMatch {
            name: name.to_string(),
            quality: Quality::Exact,
        })
}

fn spacing_match<'a>(asked: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<StreetMatch> {
    let without_spaces = asked.replace(' ', "");
    candidates
        .into_iter()
        .find(|candidate| candidate.replace(' ', "") == without_spaces)
        .map(|name| StreetMatch {
            name: name.to_string(),
            quality: Quality::Spacing,
        })
}

fn shared_words_match<'a>(asked: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<StreetMatch> {
    best_scoring(candidates, SHARED_WORDS, |candidate| {
        word_overlap(asked, candidate)
    })
    .map(|name| StreetMatch {
        name,
        quality: Quality::Words,
    })
}

fn shared_letters_match<'a>(
    asked: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<StreetMatch> {
    best_scoring(candidates, SHARED_LETTERS, |candidate| {
        similarity(asked, candidate)
    })
    .map(|name| StreetMatch {
        name,
        quality: Quality::Characters,
    })
}

/// The last tier, for when two people disagreed about whether a road is a `rue` or a `route`.
///
/// What is compared is the name with the type removed, because that is the part that says which
/// road is meant. `ROUTE GENEVE` and `RUE GENEVE` agree on it; `AVENUE CHARLES GAULLE` and
/// `AVENUE CHARLES GARNIER` do not, although overall they look about as alike.
fn street_type_match<'a>(
    asked: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    profile: &TextProfile,
) -> Option<StreetMatch> {
    let asked_distinctive = distinctive_words(asked, profile);
    if asked_distinctive == asked {
        return None;
    }

    best_scoring(candidates, SHARED_DISTINCTIVE_LETTERS, |candidate| {
        let candidate_distinctive = distinctive_words(candidate, profile);
        if candidate_distinctive == candidate {
            return 0.0;
        }
        similarity(&asked_distinctive, &candidate_distinctive)
    })
    .map(|name| StreetMatch {
        name,
        quality: Quality::StreetType,
    })
}

fn best_scoring<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    floor: f64,
    score: impl Fn(&str) -> f64,
) -> Option<String> {
    candidates
        .into_iter()
        .map(|candidate| (score(candidate), candidate))
        .filter(|(scored, _)| *scored >= floor)
        .max_by(|(left, left_name), (right, right_name)| {
            left.total_cmp(right).then_with(|| right_name.cmp(left_name))
        })
        .map(|(_, name)| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::FRENCH;

    fn matched(asked: &str, candidates: &[&str]) -> Option<StreetMatch> {
        best_street_match(asked, candidates.iter().copied(), &FRENCH)
    }

    #[test]
    fn given_the_street_as_recorded_when_matched_then_it_is_exact() {
        let found = matched("ROUTE TILLEULS", &["ROUTE TILLEULS", "RUE GAMBETTA"]);

        assert_eq!(
            found,
            Some(StreetMatch {
                name: "ROUTE TILLEULS".into(),
                quality: Quality::Exact
            })
        );
    }

    #[test]
    fn given_a_name_spaced_differently_when_matched_then_the_spacing_is_forgiven() {
        let found = matched("RUE VAN GOGH", &["RUE VANGOGH"]);

        assert_eq!(found.map(|m| m.quality), Some(Quality::Spacing));
    }

    #[test]
    fn given_a_missing_word_when_matched_then_the_shared_words_carry_it() {
        let found = matched("AVENUE MARECHAL MAUNOURY", &["AVENUE MARECHAL JOSEPH MAUNOURY"]);

        assert_eq!(found.map(|m| m.quality), Some(Quality::Words));
    }

    #[test]
    fn given_a_misspelling_when_matched_then_the_letters_carry_it() {
        let found = matched("AVENUE GEORGES CLEMANCEAU", &["AVENUE GEORGES CLEMENCEAU"]);

        assert_eq!(
            found,
            Some(StreetMatch {
                name: "AVENUE GEORGES CLEMENCEAU".into(),
                quality: Quality::Characters
            })
        );
    }

    #[test]
    fn given_only_the_street_type_differing_when_matched_then_the_distinctive_part_carries_it() {
        let found = matched("ROUTE GENEVE", &["RUE GENEVE"]);

        assert_eq!(
            found,
            Some(StreetMatch {
                name: "RUE GENEVE".into(),
                quality: Quality::StreetType
            })
        );
    }

    /// The failure this guards against invents a street rather than declining one. `ROUTE MARAIS`
    /// and `RUE MARAICHERS` can sit in one commune and look alike letter by letter; they are not
    /// the same road, and answering with one would look precise while being wrong.
    #[test]
    fn given_a_different_street_that_merely_looks_alike_when_matched_then_nothing_is_returned() {
        assert_eq!(matched("ROUTE MARAIS", &["RUE MARAICHERS"]), None);
    }

    /// Two streets named after the same person are two streets. Loosening far enough to join them
    /// is how a geocoder starts inventing addresses.
    #[test]
    fn given_two_streets_sharing_a_surname_when_matched_then_they_stay_apart() {
        assert_eq!(matched("RUE AUGUSTE RENOIR", &["RUE JEAN RENOIR"]), None);
    }

    #[test]
    fn given_no_candidates_at_all_when_matched_then_nothing_is_returned() {
        assert_eq!(matched("ROUTE TILLEULS", &[]), None);
    }

    #[test]
    fn given_an_exact_and_an_approximate_candidate_when_matched_then_the_exact_one_wins() {
        let found = matched("RUE CARNOT", &["RUE CARNOTT", "RUE CARNOT"]);

        assert_eq!(found.map(|m| m.quality), Some(Quality::Exact));
    }

    #[test]
    fn given_two_candidates_at_the_same_tier_when_matched_then_the_closer_one_wins() {
        let found = matched("RUE BOISIERE", &["RUE BOISSIERES", "RUE BOISSIERE"]);

        assert_eq!(found.map(|m| m.name), Some("RUE BOISSIERE".into()));
    }

    #[test]
    fn given_an_empty_query_when_matched_then_nothing_is_returned() {
        assert_eq!(matched("", &["RUE CARNOT"]), None);
    }
}
