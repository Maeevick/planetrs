//! How close two spellings of one name are.
//!
//! Three measures, because they fail differently. Word overlap sees a missing or reordered word and
//! is blind to a letter; edit distance sees the letter and is blind to reordering; and neither
//! knows that the first word of a French street name says what kind of street it is rather than
//! which one. The matching cascade in [`crate::street`] uses all three in that order.

/// The Levenshtein distance between two words, or `None` once it is known to exceed `budget`.
///
/// Bounded on purpose: the answer to "are these the same street misspelled" is a small number or
/// nothing at all, and giving up early is what makes it affordable to compare a query against every
/// street in a commune.
pub fn edit_distance_within(left: &str, right: &str, budget: usize) -> Option<usize> {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();

    if left.len().abs_diff(right.len()) > budget {
        return None;
    }

    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];

    for (row, from) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, to) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(from != to);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        if current.iter().min().copied().unwrap_or(0) > budget {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous.last().copied().filter(|distance| *distance <= budget)
}

/// How alike two names are on their letters, from 0 for unrelated to 1 for identical.
pub fn similarity(left: &str, right: &str) -> f64 {
    let longest = left.chars().count().max(right.chars().count());
    if longest == 0 {
        return 0.0;
    }

    match edit_distance_within(left, right, longest) {
        Some(distance) => 1.0 - distance as f64 / longest as f64,
        None => 0.0,
    }
}

/// How alike two names are on the words they share, ignoring their order.
pub fn word_overlap(left: &str, right: &str) -> f64 {
    let left: std::collections::HashSet<&str> = left.split_whitespace().collect();
    let right: std::collections::HashSet<&str> = right.split_whitespace().collect();

    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    left.intersection(&right).count() as f64 / left.union(&right).count() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_two_identical_strings_when_compared_then_the_distance_is_zero() {
        assert_eq!(edit_distance_within("CARNOT", "CARNOT", 4), Some(0));
    }

    #[test]
    fn given_one_inserted_letter_when_compared_then_the_distance_is_one() {
        assert_eq!(edit_distance_within("CLEMANCEAU", "CLEMENCEAU", 4), Some(1));
    }

    #[test]
    fn given_a_substitution_when_compared_then_the_distance_is_one() {
        assert_eq!(edit_distance_within("FLEURY", "FLEURI", 4), Some(1));
    }

    #[test]
    fn given_strings_further_apart_than_the_budget_when_compared_then_it_gives_up() {
        assert_eq!(edit_distance_within("MARAIS", "MARAICHERS", 1), None);
    }

    #[test]
    fn given_a_length_difference_past_the_budget_when_compared_then_it_gives_up_at_once() {
        assert_eq!(edit_distance_within("RUE", "BOULEVARD DES ACACIAS", 2), None);
    }

    #[test]
    fn given_two_identical_strings_when_scored_then_the_similarity_is_one() {
        assert_eq!(similarity("CARNOT", "CARNOT"), 1.0);
    }

    #[test]
    fn given_a_typo_in_a_long_name_when_scored_then_the_similarity_stays_high() {
        assert!(similarity("AVENUE GEORGES CLEMANCEAU", "AVENUE GEORGES CLEMENCEAU") > 0.95);
    }

    #[test]
    fn given_two_unrelated_names_when_scored_then_the_similarity_is_low() {
        assert!(similarity("MARAIS", "MARAICHERS") < 0.8);
    }

    #[test]
    fn given_an_empty_name_when_scored_then_the_similarity_is_zero() {
        assert_eq!(similarity("", "CARNOT"), 0.0);
    }

    #[test]
    fn given_names_sharing_every_word_when_overlapped_then_the_share_is_one() {
        assert_eq!(word_overlap("ROUTE TILLEULS", "TILLEULS ROUTE"), 1.0);
    }

    #[test]
    fn given_names_sharing_half_their_words_when_overlapped_then_the_share_reflects_it() {
        assert_eq!(word_overlap("RUE CARNOT", "RUE GAMBETTA"), 1.0 / 3.0);
    }

    #[test]
    fn given_an_empty_name_when_overlapped_then_the_share_is_zero() {
        assert_eq!(word_overlap("", "RUE CARNOT"), 0.0);
    }
}
