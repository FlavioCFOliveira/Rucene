//! Top-scoring collection, ported from
//! `org.apache.lucene.search.TopScoreDocCollector` and its package-private
//! `DocScoreEncoder`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::{Collector, LeafCollector};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::max_score_accumulator::MaxScoreAccumulator;
use crate::search::scorable::Scorable;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::top_docs::TopDocs;
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};
use crate::util::{NumericUtils, TernaryLongHeap};

/// The raw IEEE-754 bit pattern of `f32::NEG_INFINITY`.
///
/// Spelled out as a literal because `f32::to_bits` only became `const` in Rust
/// 1.83, after this crate's minimum supported Rust version of 1.80.
const NEG_INFINITY_BITS: i32 = 0xFF80_0000u32 as i32;

/// Encodes a `(doc, score)` pair as an `i64` whose sort order is the same as
/// comparing by score ascending and then by doc ID descending.
///
/// Equivalent to the package-private `org.apache.lucene.search.DocScoreEncoder`.
/// It is what lets [`TopScoreDocCollector`] keep its candidates in a primitive
/// [`TernaryLongHeap`] rather than in a queue of objects.
#[derive(Debug, Clone, Copy)]
pub struct DocScoreEncoder;

impl DocScoreEncoder {
    /// The code of a slot that no real hit can lose against.
    ///
    /// Equivalent to `DocScoreEncoder.LEAST_COMPETITIVE_CODE`.
    pub const LEAST_COMPETITIVE_CODE: i64 = Self::encode_bits(i32::MAX, NEG_INFINITY_BITS);

    /// Encodes a doc ID and the raw bits of a score into a single sortable
    /// `i64`.
    ///
    /// The body of [`encode`](Self::encode) with
    /// [`NumericUtils::float_to_sortable_int`] inlined, so that it can be a
    /// `const fn` on this crate's minimum supported Rust version — where
    /// `f32::to_bits` is not yet `const` — and back the
    /// [`LEAST_COMPETITIVE_CODE`](Self::LEAST_COMPETITIVE_CODE) constant.
    const fn encode_bits(doc_id: i32, score_bits: i32) -> i64 {
        let sortable = score_bits ^ ((score_bits >> 31) & i32::MAX);
        ((sortable as i64) << 32) | ((i32::MAX.wrapping_sub(doc_id)) as i64)
    }

    /// Encodes a doc ID and a score into a single sortable `i64`.
    ///
    /// Equivalent to `DocScoreEncoder.encode(int, float)`.
    pub fn encode(doc_id: i32, score: f32) -> i64 {
        Self::encode_bits(doc_id, score.to_bits() as i32)
    }

    /// Recovers the score from an encoded pair.
    ///
    /// Equivalent to `DocScoreEncoder.toScore(long)`.
    pub fn to_score(value: i64) -> f32 {
        NumericUtils::sortable_int_to_float(((value as u64) >> 32) as u32 as i32)
    }

    /// Recovers the doc ID from an encoded pair.
    ///
    /// Equivalent to `DocScoreEncoder.docId(long)`.
    pub fn doc_id(value: i64) -> i32 {
        i32::MAX.wrapping_sub(value as i32)
    }
}

/// Reproduces `java.lang.Math.nextUp(float)`.
///
/// Rust's `f32::next_up` was stabilised in 1.86, after this crate's minimum
/// supported Rust version of 1.80, so the JDK's bit manipulation is spelled
/// out: `NaN` and positive infinity are returned unchanged, and `-0.0` is
/// normalised to `0.0` before stepping.
fn next_up(f: f32) -> f32 {
    if f.is_nan() || f == f32::INFINITY {
        return f;
    }
    let f = f + 0.0;
    let bits = f.to_bits() as i32;
    let stepped = if f >= 0.0 {
        bits.wrapping_add(1)
    } else {
        bits.wrapping_sub(1)
    };
    f32::from_bits(stepped as u32)
}

/// Collects the top-scoring hits, returning them as a [`TopDocs`].
///
/// Equivalent to `org.apache.lucene.search.TopScoreDocCollector`, used by
/// [`IndexSearcher`](crate::search::IndexSearcher) to implement top-docs based
/// search. Hits are sorted by score descending and then, when the scores are
/// tied, by doc ID ascending.
///
/// The values `NaN` and negative infinity are not valid scores; this collector
/// will not properly collect hits with such scores.
#[derive(Debug)]
pub struct TopScoreDocCollector {
    after: Option<ScoreDoc>,
    heap: TernaryLongHeap,
    total_hits_threshold: i32,
    min_score_acc: Option<Arc<MaxScoreAccumulator>>,
    total_hits: i32,
    total_hits_relation: TotalHitsRelation,
}

impl TopScoreDocCollector {
    /// Creates a collector keeping `num_hits` candidates.
    ///
    /// Equivalent to the package-private
    /// `TopScoreDocCollector(int, ScoreDoc, int, MaxScoreAccumulator)`
    /// constructor, which passes a `null` priority queue up to
    /// `TopDocsCollector` and keeps its candidates in a ternary heap of encoded
    /// `(doc, score)` pairs instead. It is public here because Rust has no
    /// package visibility;
    /// [`TopScoreDocCollectorManager`](crate::search::TopScoreDocCollectorManager)
    /// is still the intended way to build one.
    ///
    /// # Errors
    ///
    /// Propagates the [`TernaryLongHeap`] construction error for a size the
    /// heap cannot hold.
    pub fn new(
        num_hits: usize,
        after: Option<ScoreDoc>,
        total_hits_threshold: i32,
        min_score_acc: Option<Arc<MaxScoreAccumulator>>,
    ) -> Result<Self> {
        Ok(Self {
            heap: TernaryLongHeap::filled(num_hits, DocScoreEncoder::LEAST_COMPETITIVE_CODE)?,
            after,
            total_hits_threshold,
            min_score_acc,
            total_hits: 0,
            total_hits_relation: TotalHitsRelation::EQUAL_TO,
        })
    }

    /// The number of hits this collector counts accurately.
    ///
    /// Equivalent to reading the package-private `totalHitsThreshold` field.
    pub fn total_hits_threshold(&self) -> i32 {
        self.total_hits_threshold
    }

    /// The accumulator shared with the sibling collectors of the same search,
    /// if any.
    ///
    /// Equivalent to reading the package-private `minScoreAcc` field.
    pub fn min_score_acc(&self) -> Option<&Arc<MaxScoreAccumulator>> {
        self.min_score_acc.as_ref()
    }
}

impl Collector for TopScoreDocCollector {
    fn score_mode(&self) -> ScoreMode {
        if self.total_hits_threshold == i32::MAX {
            ScoreMode::COMPLETE
        } else {
            ScoreMode::TOP_SCORES
        }
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let doc_base = context.doc_base();
        let (after_score, after_doc) = match &self.after {
            None => (f32::INFINITY, NO_MORE_DOCS),
            Some(after) => (after.score, after.doc - doc_base),
        };
        let top_code = self.heap.top();
        let top_score = DocScoreEncoder::to_score(top_code);
        Ok(Box::new(TopScoreDocLeafCollector {
            parent: self,
            doc_base,
            after_score,
            after_doc,
            top_code,
            top_score,
            min_competitive_score: 0.0,
        }))
    }
}

impl TopDocsCollector for TopScoreDocCollector {
    fn total_hits(&self) -> i32 {
        self.total_hits
    }

    fn total_hits_relation(&self) -> TotalHitsRelation {
        self.total_hits_relation
    }

    fn pq_size(&self) -> usize {
        self.heap.size()
    }

    fn pop(&mut self) -> Option<ScoreDoc> {
        let encoded = self.heap.pop().ok()?;
        Some(ScoreDoc::new(
            DocScoreEncoder::doc_id(encoded),
            DocScoreEncoder::to_score(encoded),
        ))
    }

    /// Counts the heap slots still holding a real hit.
    ///
    /// Equivalent to `TopScoreDocCollector.topDocsSize()`, which overrides the
    /// base implementation because the heap is pre-filled with sentinels.
    fn top_docs_size(&self) -> usize {
        let mut cnt = 0;
        for i in 1..=self.heap.size() {
            if self.heap.get(i) != DocScoreEncoder::LEAST_COMPETITIVE_CODE {
                cnt += 1;
            }
        }
        cnt
    }

    /// Equivalent to `TopScoreDocCollector.newTopDocs(ScoreDoc[], int)`, which
    /// returns a result carrying the real hit count rather than the shared
    /// empty result.
    fn new_top_docs(&self, results: Option<Vec<ScoreDoc>>, _start: i32) -> Result<TopDocs> {
        let total_hits = TotalHits::new(i64::from(self.total_hits), self.total_hits_relation)?;
        Ok(TopDocs::new(total_hits, results.unwrap_or_default()))
    }

    /// Equivalent to `TopScoreDocCollector.populateResults(ScoreDoc[], int)`,
    /// decoding each popped heap entry.
    ///
    /// # Panics
    ///
    /// Panics when the heap holds fewer than `how_many` entries, which
    /// [`TopDocsCollector::top_docs_range`] guarantees it does not.
    fn populate_results(&mut self, how_many: usize) -> Vec<ScoreDoc> {
        let mut results = vec![ScoreDoc::new(0, 0.0); how_many];
        for slot in results.iter_mut().rev() {
            let encoded = self
                .heap
                .pop()
                .expect("INVARIANT: prune_least_competitive_hits_to left how_many entries");
            *slot = ScoreDoc::new(
                DocScoreEncoder::doc_id(encoded),
                DocScoreEncoder::to_score(encoded),
            );
        }
        results
    }

    /// Equivalent to
    /// `TopScoreDocCollector.pruneLeastCompetitiveHitsTo(int)`, popping from
    /// the ternary heap.
    fn prune_least_competitive_hits_to(&mut self, keep: usize) {
        let mut i = self.heap.size().saturating_sub(keep);
        while i > 0 {
            let _ = self.heap.pop();
            i -= 1;
        }
    }
}

/// The per-leaf half of [`TopScoreDocCollector`].
///
/// Equivalent to the anonymous `LeafCollector` that
/// `TopScoreDocCollector.getLeafCollector(LeafReaderContext)` returns.
#[derive(Debug)]
struct TopScoreDocLeafCollector<'a> {
    parent: &'a mut TopScoreDocCollector,
    doc_base: i32,
    after_score: f32,
    after_doc: i32,
    top_code: i64,
    top_score: f32,
    min_competitive_score: f32,
}

impl TopScoreDocLeafCollector<'_> {
    /// Equivalent to the anonymous collector's `collectCompetitiveHit`.
    fn collect_competitive_hit(
        &mut self,
        doc: i32,
        score: f32,
        scorer: &mut dyn Scorable,
    ) -> Result<()> {
        let code = DocScoreEncoder::encode(doc + self.doc_base, score);
        self.top_code = self.parent.heap.update_top(code);
        self.top_score = DocScoreEncoder::to_score(self.top_code);
        self.update_min_competitive_score(scorer)
    }

    /// Equivalent to the anonymous collector's
    /// `updateGlobalMinCompetitiveScore`.
    fn update_global_min_competitive_score(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        let Some(acc) = self.parent.min_score_acc.as_ref() else {
            debug_assert!(false, "minScoreAcc must be present");
            return Ok(());
        };
        let max_min_score = acc.get_raw();
        if max_min_score != i64::MIN {
            // Since we tie-break on doc ID and collect in doc ID order, we can
            // require the next float if the global minimum score is set on a
            // document ID smaller than the IDs in the current leaf.
            let mut score = DocScoreEncoder::to_score(max_min_score);
            if self.doc_base >= DocScoreEncoder::doc_id(max_min_score) {
                score = next_up(score);
            }
            if score > self.min_competitive_score {
                scorer.set_min_competitive_score(score)?;
                self.min_competitive_score = score;
                self.parent.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
            }
        }
        Ok(())
    }

    /// Equivalent to the anonymous collector's `updateMinCompetitiveScore`.
    fn update_min_competitive_score(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        if self.parent.total_hits > self.parent.total_hits_threshold {
            // Since we tie-break on doc ID and collect in doc ID order, we can
            // require the next float. The top is never absent, because the heap
            // is filled with sentinel values; if the top element is a sentinel
            // its score is negative infinity and the logic below still holds.
            let local_min_score = next_up(self.top_score);
            if local_min_score > self.min_competitive_score {
                scorer.set_min_competitive_score(local_min_score)?;
                self.parent.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
                self.min_competitive_score = local_min_score;
                if let Some(acc) = self.parent.min_score_acc.as_ref() {
                    // We do not use the next float, but we register the document
                    // ID so that other leaves or leaf partitions can require it
                    // if they are after the current maximum.
                    acc.accumulate(self.top_code);
                }
            }
        }
        Ok(())
    }
}

impl LeafCollector for TopScoreDocLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        if self.parent.min_score_acc.is_none() {
            self.update_min_competitive_score(scorer)
        } else {
            self.update_global_min_competitive_score(scorer)
        }
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;

        self.parent.total_hits += 1;
        let hit_count_so_far = self.parent.total_hits;

        if let Some(acc) = self.parent.min_score_acc.as_ref() {
            let mod_interval = acc.mod_interval();
            if (i64::from(hit_count_so_far) & mod_interval) == 0 {
                self.update_global_min_competitive_score(scorer)?;
            }
        }

        if self.parent.after.is_some()
            && (score > self.after_score || (score == self.after_score && doc <= self.after_doc))
        {
            // The hit was collected on a previous page.
            if self.parent.total_hits_relation == TotalHitsRelation::EQUAL_TO {
                // We just reached totalHitsThreshold, we can start setting the
                // min competitive score now.
                self.update_min_competitive_score(scorer)?;
            }
            return Ok(());
        }

        if score <= self.top_score {
            // Note: for queries that match lots of hits, this is the common
            // case: most hits are not competitive.
            if hit_count_so_far == self.parent.total_hits_threshold.wrapping_add(1) {
                // We just exceeded totalHitsThreshold, we can start setting the
                // min competitive score now.
                self.update_min_competitive_score(scorer)?;
            }

            // Since docs are returned in increasing doc ID order, a document
            // with a score equal to the top's cannot compete, because the
            // ordering favours documents with lower doc IDs. Reject those too.
        } else {
            self.collect_competitive_hit(doc, score, scorer)?;
        }
        Ok(())
    }
}
