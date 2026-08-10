//! Term-level state and scoring-impact types.
//!
//! Equivalent to `org.apache.lucene.codecs.BlockTermState`, `TermStats`,
//! `Impact`, and `CompetitiveImpactAccumulator`.

#![deny(unsafe_code)]

use std::collections::BTreeSet;

use crate::index::TermState;

/// Per-term statistics.
///
/// Equivalent to `org.apache.lucene.codecs.TermStats`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TermStats {
    /// Number of documents containing this term.
    pub doc_freq: i32,
    /// Total number of occurrences of this term across all documents.
    pub total_term_freq: i64,
}

impl TermStats {
    /// Creates a new `TermStats`.
    pub fn new(doc_freq: i32, total_term_freq: i64) -> Self {
        Self {
            doc_freq,
            total_term_freq,
        }
    }
}

/// Per-document scoring factors.
///
/// Equivalent to `org.apache.lucene.codecs.Impact`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Impact {
    /// Term frequency of the term in the document.
    pub freq: i32,
    /// Norm factor of the document.
    pub norm: i64,
}

impl Impact {
    /// Creates a new `Impact`.
    pub fn new(freq: i32, norm: i64) -> Self {
        Self { freq, norm }
    }
}

impl PartialOrd for Impact {
    /// Compares by increasing frequency, then by *decreasing* unsigned norm.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Impact {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let cmp = self.freq.cmp(&other.freq);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
        // Greater unsigned norms compare lower, matching Lucene's tree order.
        (other.norm as u64).cmp(&(self.norm as u64))
    }
}

/// Accumulates the `(freq, norm)` pairs that may produce competitive scores.
///
/// Equivalent to `org.apache.lucene.codecs.CompetitiveImpactAccumulator`.
///
/// This implementation uses a 256-entry array for the common case where norms
/// fit in a single byte, and a `BTreeSet` for outliers.
#[derive(Debug, Clone)]
pub struct CompetitiveImpactAccumulator {
    /// Maximum frequency observed for each unsigned byte norm value.
    max_freqs: [i32; 256],
    /// Competitive pairs for norm values outside the byte range.
    other_pairs: BTreeSet<Impact>,
}

impl Default for CompetitiveImpactAccumulator {
    fn default() -> Self {
        Self {
            max_freqs: [0; 256],
            other_pairs: BTreeSet::new(),
        }
    }
}

impl CompetitiveImpactAccumulator {
    /// Creates a new empty accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the accumulator to its initial empty state.
    pub fn clear(&mut self) {
        self.max_freqs.fill(0);
        self.other_pairs.clear();
    }

    /// Adds a single `(freq, norm)` pair.
    pub fn add(&mut self, freq: i32, norm: i64) {
        if (i8::MIN as i64) <= norm && norm <= (i8::MAX as i64) {
            let index = norm as u8 as usize;
            self.max_freqs[index] = self.max_freqs[index].max(freq);
        } else {
            Self::add_impact(Impact::new(freq, norm), &mut self.other_pairs);
        }
    }

    /// Merges another accumulator into this one.
    pub fn add_all(&mut self, other: &Self) {
        for i in 0..256 {
            self.max_freqs[i] = self.max_freqs[i].max(other.max_freqs[i]);
        }
        for impact in &other.other_pairs {
            Self::add_impact(*impact, &mut self.other_pairs);
        }
    }

    /// Replaces the content of this accumulator with `other`.
    pub fn copy(&mut self, other: &Self) {
        self.max_freqs.copy_from_slice(&other.max_freqs);
        self.other_pairs.clear();
        self.other_pairs.extend(&other.other_pairs);
    }

    /// Returns the competitive `(freq, norm)` pairs, ordered by increasing
    /// frequency and norm.
    pub fn get_competitive_freq_norm_pairs(&self) -> Vec<Impact> {
        let mut impacts = Vec::new();
        let mut max_freq_for_lower_norms = 0;

        for i in 0..256 {
            let max_freq = self.max_freqs[i];
            if max_freq > max_freq_for_lower_norms {
                impacts.push(Impact::new(max_freq, i as i8 as i64));
                max_freq_for_lower_norms = max_freq;
            }
        }

        if self.other_pairs.is_empty() {
            return impacts;
        }

        let mut merged = self.other_pairs.clone();
        for impact in impacts {
            Self::add_impact(impact, &mut merged);
        }
        merged.into_iter().collect()
    }

    /// Adds `new_entry` to `set`, pruning any entries that are less competitive.
    ///
    /// An entry is less competitive when another entry has a greater or equal
    /// frequency and a greater or equal unsigned norm.
    fn add_impact(new_entry: Impact, set: &mut BTreeSet<Impact>) {
        let next = set.range(new_entry..).next().copied();

        match next {
            Some(next) if (next.norm as u64) <= (new_entry.norm as u64) => {
                // Already have this entry or a more competitive one.
                return;
            }
            _ => {
                // Keep `new_entry`: some entries have greater freq but worse norm,
                // so we cannot determine which will score higher.
                set.insert(new_entry);
            }
        }

        // Remove entries that are less competitive than `new_entry`.
        let mut to_remove = Vec::new();
        for entry in set.range(..new_entry).rev() {
            if (entry.norm as u64) >= (new_entry.norm as u64) {
                to_remove.push(*entry);
            } else {
                // Lesser freq but better norm: further entries are not comparable.
                break;
            }
        }
        for entry in to_remove {
            set.remove(&entry);
        }
    }
}

/// Per-term state shared between the terms dictionary and the postings
/// implementation.
///
/// Equivalent to `org.apache.lucene.codecs.BlockTermState` (which extends
/// `org.apache.lucene.index.OrdTermState`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlockTermState {
    /// Ordinal of this term in the dictionary.
    pub ord: i64,
    /// Number of documents containing this term.
    pub doc_freq: i32,
    /// Total number of occurrences of this term, or `-1` if frequencies are
    /// omitted.
    pub total_term_freq: i64,
    /// Ordinal of this term inside its block in the terms dictionary.
    pub term_block_ord: i32,
    /// File pointer into the terms dictionary primary file that holds this term.
    pub block_file_pointer: i64,
}

impl BlockTermState {
    /// Copies state from `other` into `self`.
    ///
    /// Equivalent to `BlockTermState.copyFrom` in Lucene.
    pub fn copy_from(&mut self, other: &Self) {
        self.ord = other.ord;
        self.doc_freq = other.doc_freq;
        self.total_term_freq = other.total_term_freq;
        self.term_block_ord = other.term_block_ord;
        self.block_file_pointer = other.block_file_pointer;
    }
}

impl TermState for BlockTermState {
    fn copy_from(&mut self, other: &dyn crate::index::TermState) {
        if let Some(other) = other.as_any().downcast_ref::<Self>() {
            BlockTermState::copy_from(self, other);
        }
    }

    fn clone_box(&self) -> Box<dyn crate::index::TermState> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    #[test]
    fn term_stats_holds_values() {
        let stats = TermStats::new(5, 23);
        assert_eq!(stats.doc_freq, 5);
        assert_eq!(stats.total_term_freq, 23);
    }

    #[test]
    fn impact_ordering_matches_lucene() {
        let a = Impact::new(1, 1);
        let b = Impact::new(2, 1);
        let c = Impact::new(2, 2);
        let d = Impact::new(2, 0);

        assert!(a < b);
        // Greater unsigned norm compares lower in the order.
        assert!(b > c); // (2,1) > (2,2)
        assert!(d > b); // (2,0) > (2,1)
    }

    #[test]
    fn impact_equality_and_hash() {
        let a = Impact::new(3, 7);
        let b = Impact::new(3, 7);
        let c = Impact::new(3, 8);
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
        a.hash(&mut hasher_a);
        let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
        b.hash(&mut hasher_b);
        assert_eq!(hasher_a.finish(), hasher_b.finish());
    }

    #[test]
    fn accumulator_byte_norms_only() {
        let mut acc = CompetitiveImpactAccumulator::new();
        acc.add(1, -1);
        acc.add(2, -1);
        acc.add(1, 1);
        acc.add(3, 1);
        acc.add(2, 2);

        // The byte-norm optimization keeps only pairs whose frequency strictly
        // exceeds every previous frequency; lower-freq pairs are dominated.
        let impacts = acc.get_competitive_freq_norm_pairs();
        assert_eq!(impacts, vec![Impact::new(3, 1)]);
    }

    #[test]
    fn accumulator_prunes_dominated_entries() {
        let mut acc = CompetitiveImpactAccumulator::new();
        // Build a non-byte outlier set manually.
        acc.add(1, 300);
        acc.add(2, 250);
        acc.add(3, 200);
        // (3,200) dominates (2,250) and (1,300) because it has higher freq
        // and a not-worse norm than both.
        let impacts = acc.get_competitive_freq_norm_pairs();
        assert_eq!(impacts, vec![Impact::new(3, 200)]);
    }

    #[test]
    fn accumulator_add_all_merges_byte_norms() {
        let mut a = CompetitiveImpactAccumulator::new();
        a.add(5, 10);
        a.add(3, 20);

        let mut b = CompetitiveImpactAccumulator::new();
        b.add(7, 10);
        b.add(2, 20);

        a.add_all(&b);
        let impacts = a.get_competitive_freq_norm_pairs();
        assert_eq!(impacts, vec![Impact::new(7, 10)]);
    }

    #[test]
    fn accumulator_copy_replaces_content() {
        let mut a = CompetitiveImpactAccumulator::new();
        a.add(5, 10);

        let mut b = CompetitiveImpactAccumulator::new();
        b.add(7, 20);

        a.copy(&b);
        assert_eq!(
            a.get_competitive_freq_norm_pairs(),
            vec![Impact::new(7, 20)]
        );
    }

    #[test]
    fn accumulator_clear_empties() {
        let mut acc = CompetitiveImpactAccumulator::new();
        acc.add(5, 10);
        acc.add(5, 300);
        acc.clear();
        assert!(acc.get_competitive_freq_norm_pairs().is_empty());
    }

    #[test]
    fn block_term_state_copy_from() {
        let mut a = BlockTermState::default();
        let b = BlockTermState {
            ord: 42,
            doc_freq: 7,
            total_term_freq: 100,
            term_block_ord: 3,
            block_file_pointer: 1234,
        };
        a.copy_from(&b);
        assert_eq!(a, b);
    }
}
