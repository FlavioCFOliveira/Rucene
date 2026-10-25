//! Composite per-segment comparison, ported from
//! `org.apache.lucene.search.MultiLeafFieldComparator`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::scorable::Scorable;

/// A [`LeafFieldComparator`] that applies several comparators in priority
/// order, each with its own reverse multiplier.
///
/// Equivalent to `org.apache.lucene.search.MultiLeafFieldComparator`, a
/// package-private `final` class that
/// [`TopFieldCollector`](crate::search::TopFieldCollector) wraps around the
/// leaf comparators of a multi-field sort. It is public here because Rust has
/// no package visibility.
///
/// **Divergence from Lucene 10.5.0.** Java hoists the first comparator and its
/// multiplier into dedicated fields so that the common case — the first
/// comparator already deciding — avoids an array access. That is a
/// micro-optimisation of the JIT-compiled loop, not a difference in behaviour:
/// the sequence of calls and the returned values are identical.
pub struct MultiLeafFieldComparator {
    comparators: Vec<Box<dyn LeafFieldComparator>>,
    reverse_mul: Vec<i32>,
}

impl std::fmt::Debug for MultiLeafFieldComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiLeafFieldComparator")
            .field("reverse_mul", &self.reverse_mul)
            .finish_non_exhaustive()
    }
}

impl MultiLeafFieldComparator {
    /// Combines the given leaf comparators.
    ///
    /// Equivalent to
    /// `new MultiLeafFieldComparator(LeafFieldComparator[], int[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when the two collections have different lengths, and when
    /// they are empty, which Java reports as an `ArrayIndexOutOfBoundsException`
    /// while reading `comparators[0]`.
    pub fn new(
        comparators: Vec<Box<dyn LeafFieldComparator>>,
        reverse_mul: Vec<i32>,
    ) -> Result<Self> {
        if comparators.len() != reverse_mul.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "Must have the same number of comparators and reverseMul, got {} and {}",
                comparators.len(),
                reverse_mul.len()
            )));
        }
        if comparators.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "Must have at least one comparator".to_string(),
            ));
        }
        Ok(Self {
            comparators,
            reverse_mul,
        })
    }
}

impl LeafFieldComparator for MultiLeafFieldComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.set_bottom(slot)?;
        }
        Ok(())
    }

    /// The first non-zero `reverseMul[i] * comparators[i].compareBottom(doc)`,
    /// or `0` when every comparator ties.
    fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        for (comparator, reverse_mul) in self.comparators.iter_mut().zip(&self.reverse_mul) {
            let cmp = reverse_mul * comparator.compare_bottom(doc, scorer)?;
            if cmp != 0 {
                return Ok(cmp);
            }
        }
        Ok(0)
    }

    /// The first non-zero `reverseMul[i] * comparators[i].compareTop(doc)`, or
    /// `0` when every comparator ties.
    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        for (comparator, reverse_mul) in self.comparators.iter_mut().zip(&self.reverse_mul) {
            let cmp = reverse_mul * comparator.compare_top(doc, scorer)?;
            if cmp != 0 {
                return Ok(cmp);
            }
        }
        Ok(0)
    }

    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.copy(slot, doc, scorer)?;
        }
        Ok(())
    }

    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.set_scorer(scorer)?;
        }
        Ok(())
    }

    /// Equivalent to `MultiLeafFieldComparator.setHitsThresholdReached()`,
    /// which only notifies the first comparator: skipping is only relevant for
    /// the primary sort field.
    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.comparators[0].set_hits_threshold_reached()
    }

    /// Equivalent to `MultiLeafFieldComparator.competitiveIterator()`, which
    /// only consults the first comparator: skipping is only relevant for the
    /// primary sort field.
    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.comparators[0].competitive_iterator()
    }
}
