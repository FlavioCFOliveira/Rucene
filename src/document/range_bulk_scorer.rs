//! A bulk scorer restricted to a doc-id interval, ported from
//! `org.apache.lucene.document.RangeBulkScorer`.

use crate::error::Result;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::Scorable;
use crate::util::Bits;

/// The constant [`Scorable`] a [`RangeBulkScorer`] hands to the collector.
///
/// Equivalent to the anonymous `Scorable` `RangeBulkScorer`'s constructor
/// builds, whose `score()` returns the constant it was given.
#[derive(Clone, Copy, Debug)]
struct ConstantScorable {
    score: f32,
}

impl Scorable for ConstantScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }
}

/// A [`BulkScorer`] that restricts collection to the half-open doc-id interval
/// `[min_doc_id, max_doc_id)`.
///
/// Equivalent to `org.apache.lucene.document.RangeBulkScorer`.
///
/// The typical use is a constant-score query backed by
/// [`range`](crate::search::range), where collecting the whole interval in one
/// or a few [`LeafCollector::collect_range`] calls is cheaper than a per-document
/// [`LeafCollector::collect`].
pub struct RangeBulkScorer {
    min_doc_id: i32,
    max_doc_id: i32,
    scorer: ConstantScorable,
    iterator: Box<dyn DocIdSetIterator>,
}

impl std::fmt::Debug for RangeBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeBulkScorer")
            .field("min_doc_id", &self.min_doc_id)
            .field("max_doc_id", &self.max_doc_id)
            .field("score", &self.scorer.score)
            .finish_non_exhaustive()
    }
}

impl RangeBulkScorer {
    /// Creates a bulk scorer that collects only within
    /// `[min_doc_id, max_doc_id)`.
    ///
    /// Equivalent to
    /// `RangeBulkScorer(DocIdSetIterator, float, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `min_doc_id` is not less than `max_doc_id`.
    pub fn new(
        iterator: Box<dyn DocIdSetIterator>,
        score: f32,
        min_doc_id: i32,
        max_doc_id: i32,
    ) -> Result<Self> {
        if min_doc_id >= max_doc_id {
            return Err(crate::error::LuceneError::IllegalArgument(
                "minDocID must be less than maxDocID".to_string(),
            ));
        }
        Ok(Self {
            min_doc_id,
            max_doc_id,
            scorer: ConstantScorable { score },
            iterator,
        })
    }
}

impl BulkScorer for RangeBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        collector.set_scorer(&mut self.scorer)?;
        let mut min = min;
        if let Some(competitive_iterator) = collector.competitive_iterator()? {
            if competitive_iterator.doc_id() > min {
                min = competitive_iterator.doc_id();
                // The competitive iterator may not match any document in the
                // range.
                min = min.min(max);
            }
        }
        if max <= self.min_doc_id {
            self.iterator.advance(self.min_doc_id)?;
        } else if min >= self.max_doc_id {
            self.iterator.advance(self.max_doc_id)?;
        } else {
            let filtered_min = min.max(self.min_doc_id);
            let filtered_max = max.min(self.max_doc_id);
            self.iterator.advance(filtered_min)?;
            match accept_docs {
                None => {
                    collector.collect_range(filtered_min, filtered_max, &mut self.scorer)?;
                }
                Some(accept_docs) => {
                    let mut range_start = -1;
                    for doc in filtered_min..filtered_max {
                        if accept_docs.get(doc as usize) {
                            if range_start < 0 {
                                range_start = doc;
                            }
                        } else if range_start >= 0 {
                            collector.collect_range(range_start, doc, &mut self.scorer)?;
                            range_start = -1;
                        }
                    }
                    if range_start >= 0 {
                        collector.collect_range(range_start, filtered_max, &mut self.scorer)?;
                    }
                }
            }
            self.iterator.advance(filtered_max)?;
        }
        Ok(self.iterator.doc_id())
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc_id) - i64::from(self.min_doc_id)
    }
}
