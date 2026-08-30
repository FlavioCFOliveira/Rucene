//! Block-level range iteration, ported from
//! `org.apache.lucene.search.SkipBlockRangeIterator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::DocValuesSkipper;
use crate::search::abstract_doc_id_set_iterator::AbstractDocIdSetIterator;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::FixedBitSet;

/// The match state of the block a [`SkipBlockRangeIterator`] is positioned on.
///
/// Equivalent to the `SkipBlockRangeIterator.Match` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Match {
    /// All documents in the block match.
    ///
    /// Equivalent to `Match.YES`.
    Yes,
    /// All documents in the block that have a value match.
    ///
    /// Equivalent to `Match.YES_IF_PRESENT`.
    YesIfPresent,
    /// Some of the documents in the block match.
    ///
    /// Equivalent to `Match.MAYBE`.
    Maybe,
}

/// A [`DocIdSetIterator`] that returns every document inside the
/// [`DocValuesSkipper`] blocks whose minimum and maximum values fall within a
/// range.
///
/// Equivalent to `org.apache.lucene.search.SkipBlockRangeIterator`.
pub struct SkipBlockRangeIterator {
    base: AbstractDocIdSetIterator,
    skipper: Box<dyn DocValuesSkipper>,
    min_value: i64,
    max_value: i64,
    r#match: Match,
}

impl std::fmt::Debug for SkipBlockRangeIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkipBlockRangeIterator")
            .field("doc", &self.base.doc)
            .field("min_value", &self.min_value)
            .field("max_value", &self.max_value)
            .field("match", &self.r#match)
            .finish_non_exhaustive()
    }
}

impl SkipBlockRangeIterator {
    /// Creates a new block-range iterator.
    ///
    /// Equivalent to
    /// `new SkipBlockRangeIterator(DocValuesSkipper, long, long)`.
    ///
    /// * `skipper` — the skipper used to check block bounds;
    /// * `min_value` — only return documents that lie within a block whose
    ///   maximum value is greater than this;
    /// * `max_value` — only return documents that lie within a block whose
    ///   minimum value is less than this.
    pub fn new(skipper: Box<dyn DocValuesSkipper>, min_value: i64, max_value: i64) -> Self {
        Self {
            base: AbstractDocIdSetIterator::new(),
            skipper,
            min_value,
            max_value,
            r#match: Match::Maybe,
        }
    }

    /// Equivalent to the private `SkipBlockRangeIterator.classifyBlock()`.
    fn classify_block(&self) -> Match {
        if self.skipper.min_value(0) >= self.min_value
            && self.skipper.max_value(0) <= self.max_value
        {
            if self.skipper.max_doc_id(0) - self.skipper.min_doc_id(0)
                == self.skipper.doc_count(0) - 1
            {
                return Match::Yes;
            }
            return Match::YesIfPresent;
        }
        Match::Maybe
    }

    /// Returns the match state of the block this iterator is positioned on.
    ///
    /// Equivalent to `SkipBlockRangeIterator.getMatch()`. It should only be
    /// called when the iterator is positioned.
    pub fn get_match(&self) -> Match {
        self.r#match
    }

    /// Returns the exclusive end of the current skip block — the block boundary
    /// the skipper reports — regardless of the match state.
    ///
    /// Equivalent to `SkipBlockRangeIterator.blockEnd()`. Unlike
    /// [`doc_id_run_end`](DocIdSetIterator::doc_id_run_end), which returns
    /// `doc + 1` for a [`Match::Maybe`] block, this always returns the full
    /// block boundary so that callers can bulk-evaluate the whole block at once.
    pub fn block_end(&self) -> i32 {
        self.skipper.max_doc_id(0) + 1
    }
}

impl DocIdSetIterator for SkipBlockRangeIterator {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.base.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target <= self.skipper.max_doc_id(0) {
            // Within the current block.
            if self.base.doc > -1 {
                // Already positioned, so bounds have been checked and this is a
                // matching block.
                return Ok(self.base.set(target));
            }
        } else {
            // Advance to target.
            self.skipper.advance(target)?;
        }

        // Find the next matching block, which may be the current block.
        self.skipper.advance_range(self.min_value, self.max_value)?;
        let next_doc = target.max(self.skipper.min_doc_id(0));
        if next_doc == NO_MORE_DOCS {
            self.r#match = Match::Maybe;
        } else {
            self.r#match = self.classify_block();
        }
        Ok(self.base.set(next_doc))
    }

    fn cost(&self) -> i64 {
        i64::from(NO_MORE_DOCS)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        if self.r#match != Match::Yes {
            return Ok(self.base.doc + 1);
        }
        let mut max_doc = self.skipper.max_doc_id(0);
        let mut next_level = 1;
        while next_level < self.skipper.num_levels()
            && self.skipper.min_value(next_level) >= self.min_value
            && self.skipper.max_value(next_level) <= self.max_value
            && self.skipper.max_doc_id(next_level) - self.skipper.min_doc_id(next_level)
                == self.skipper.doc_count(next_level) - 1
        {
            max_doc = self.skipper.max_doc_id(next_level);
            next_level += 1;
        }
        Ok(max_doc + 1)
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        while self.base.doc < up_to {
            let end = up_to.min(self.doc_id_run_end()?);
            bit_set.set_range((self.base.doc - offset) as usize, (end - offset) as usize);
            self.advance(end)?;
        }
        Ok(())
    }
}
