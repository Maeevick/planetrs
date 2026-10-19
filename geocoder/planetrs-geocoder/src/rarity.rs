//! How rare each word of a street name is, across every street the index holds.
//!
//! A word found in a third of all street names says almost nothing about which street is meant,
//! and a word found in a dozen says nearly all of it. That is what lets a match on `PINSONNIERE` be
//! trusted where a match on `CHAMPS` would not.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone)]
pub struct WordRarity {
    names: f64,
    names_with: HashMap<String, u32>,
}

impl WordRarity {
    /// Counted over street names, each name of each commune counting once.
    pub fn of<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut names_with: HashMap<String, u32> = HashMap::new();
        let mut count = 0usize;
        for name in names {
            count += 1;
            let words: HashSet<&str> = name.split_whitespace().collect();
            for word in words {
                *names_with.entry(word.to_string()).or_default() += 1;
            }
        }
        Self {
            names: count as f64,
            names_with,
        }
    }

    /// A table of counts read back: `names` street names counted, each word held by that many.
    pub(crate) fn counted(names: u64, words: impl IntoIterator<Item = (String, u32)>) -> Self {
        Self {
            names: names as f64,
            names_with: words.into_iter().collect(),
        }
    }

    /// A table as if `names` street names had been counted, with these words in that many of them.
    #[cfg(test)]
    pub(crate) fn from_counts(names: usize, words: &[(&str, u32)]) -> Self {
        Self {
            names: names as f64,
            names_with: words
                .iter()
                .map(|(word, count)| (word.to_string(), *count))
                .collect(),
        }
    }

    /// How many street names were counted.
    pub(crate) fn names(&self) -> u64 {
        self.names as u64
    }

    /// Every word counted, with how many names hold it, in alphabetical order.
    pub(crate) fn words(&self) -> Vec<(&str, u32)> {
        let mut words: Vec<(&str, u32)> = self
            .names_with
            .iter()
            .map(|(word, count)| (word.as_str(), *count))
            .collect();
        words.sort_unstable();
        words
    }

    /// Street names per street name carrying the word, as a logarithm: 0 for a word in every name,
    /// about 7 for one in a thousand, most for a word never seen.
    pub fn of_word(&self, word: &str) -> f64 {
        let with = self.names_with.get(word).copied().unwrap_or(1).max(1) as f64;
        (self.names.max(1.0) / with).ln().max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_word_in_every_name_when_weighed_then_it_says_nothing() {
        let rarity = WordRarity::of(["RUE GARE", "RUE PAIX", "RUE PASTEUR"]);

        assert_eq!(rarity.of_word("RUE"), 0.0);
    }

    #[test]
    fn given_a_word_in_one_name_of_many_when_weighed_then_it_outweighs_a_common_one() {
        let rarity = WordRarity::of(["RUE GARE", "RUE PAIX", "AVENUE GARE", "RUE PINSONNIERE"]);

        assert!(rarity.of_word("PINSONNIERE") > rarity.of_word("GARE"));
        assert!(rarity.of_word("GARE") > rarity.of_word("RUE"));
    }
}
