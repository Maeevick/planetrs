//! A probabilistic "have I seen this id" test, sized to about sixty-four bits per member.
//!
//! The ways section holds roughly a billion ways averaging ten refs each, so the membership test
//! runs some ten billion times. Against a `HashSet` that is SipHash on every one of them and it
//! dominates the pass. This is a multiply, a shift and one cache line.
//!
//! Sixty-four bits per member puts the false positive rate at about one in sixty-four, so the real
//! table is still consulted for roughly 1.6% of refs. That is the right place to stop: a rate ten
//! times lower needs ten times the memory, and the lookups it saves are already down in the noise
//! beside the inflate.
//!
//! False positives are harmless, because the real set is checked afterwards. False negatives would
//! silently drop geometry, and cannot happen: every inserted id sets its own bit.

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

pub struct IdFilter {
    bits: Vec<u64>,
    shift: u32,
}

impl IdFilter {
    pub fn sized_for(count: usize) -> Self {
        let wanted = (count.max(1) * 64).next_power_of_two();
        Self {
            bits: vec![0; wanted / 64],
            shift: 64 - wanted.trailing_zeros(),
        }
    }

    pub fn of<'a>(ids: impl IntoIterator<Item = &'a i64>, count: usize) -> Self {
        let mut filter = Self::sized_for(count);
        for id in ids {
            filter.insert(*id);
        }
        filter
    }

    fn slot(&self, id: i64) -> (usize, u64) {
        let hashed = (id as u64).wrapping_mul(GOLDEN) >> self.shift;
        ((hashed / 64) as usize, 1u64 << (hashed % 64))
    }

    pub fn insert(&mut self, id: i64) {
        let (word, bit) = self.slot(id);
        self.bits[word] |= bit;
    }

    pub fn may_hold(&self, id: i64) -> bool {
        let (word, bit) = self.slot(id);
        self.bits[word] & bit != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A false negative would drop real geometry with nothing to show for it, so this is the one
    /// property that has to hold without exception.
    #[test]
    fn given_ids_including_the_extremes_when_inserted_then_the_filter_always_admits_them() {
        let ids = [1i64, 24_287_041, -5, i64::MAX, i64::MIN, 11_164_468_656, 0];
        let filter = IdFilter::of(ids.iter(), ids.len());

        assert!(ids.iter().all(|id| filter.may_hold(*id)));
    }

    /// A false positive costs one lookup in the real set and nothing else, so the rate only has to
    /// be low enough that those lookups stay in the noise beside the inflate.
    #[test]
    fn given_a_filter_sized_for_its_set_when_asked_about_a_million_others_then_false_positives_stay_rare() {
        let held: Vec<i64> = (0..100_000).map(|id| id * 7919).collect();
        let filter = IdFilter::of(held.iter(), held.len());

        let wrong = (0..1_000_000i64)
            .map(|id| 1_000_000_000 + id)
            .filter(|id| filter.may_hold(*id))
            .count();

        assert!(
            wrong < 25_000,
            "false positive rate {wrong} per million, expected about 16 000"
        );
    }

    #[test]
    fn given_an_empty_filter_when_asked_about_an_id_then_it_admits_nothing() {
        let filter = IdFilter::of([].iter(), 0);

        assert!(!filter.may_hold(42));
    }
}
