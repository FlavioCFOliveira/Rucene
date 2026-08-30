//! Top-scoring collector manager, ported from
//! `org.apache.lucene.search.TopScoreDocCollectorManager`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::collector::CollectorManager;
use crate::search::max_score_accumulator::MaxScoreAccumulator;
use crate::search::score_doc::ScoreDoc;
use crate::search::top_docs::TopDocs;
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::top_score_doc_collector::TopScoreDocCollector;

/// Creates [`TopScoreDocCollector`]s that share a
/// [`MaxScoreAccumulator`], so that the minimum competitive score found by one
/// slice prunes the others.
///
/// Equivalent to `org.apache.lucene.search.TopScoreDocCollectorManager`. A new
/// manager should be created for each search, because of its internal state.
///
/// Java also keeps a deprecated four-argument constructor whose
/// `supportsConcurrency` flag is documented as a no-op; it is not ported,
/// because a deprecated alias of an existing constructor carries no behaviour.
#[derive(Debug)]
pub struct TopScoreDocCollectorManager {
    num_hits: usize,
    after: Option<ScoreDoc>,
    total_hits_threshold: i32,
    min_score_acc: Option<Arc<MaxScoreAccumulator>>,
}

impl TopScoreDocCollectorManager {
    /// Creates a manager collecting `num_hits` results after `after`, counting
    /// hits accurately up to `total_hits_threshold`.
    ///
    /// Equivalent to `new TopScoreDocCollectorManager(int, ScoreDoc, int)`.
    ///
    /// If the total hit count of the top docs is less than or exactly
    /// `total_hits_threshold`, the value reported in
    /// [`TopDocs::total_hits`] is accurate; if it is greater, the value is a
    /// lower bound. [`i32::MAX`] makes the hit count accurate but also makes
    /// query processing slower.
    ///
    /// The collectors this manager returns pre-allocate a full heap of length
    /// `num_hits`, filled with sentinel values.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `total_hits_threshold` is negative or `num_hits` is not
    /// positive.
    pub fn new(num_hits: i32, after: Option<ScoreDoc>, total_hits_threshold: i32) -> Result<Self> {
        if total_hits_threshold < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "totalHitsThreshold must be >= 0, got {total_hits_threshold}"
            )));
        }

        if num_hits <= 0 {
            return Err(LuceneError::IllegalArgument(
                "numHits must be > 0; please use TotalHitCountCollectorManager if you just need the total hit count"
                    .to_string(),
            ));
        }

        Ok(Self {
            num_hits: num_hits as usize,
            after,
            total_hits_threshold: total_hits_threshold.max(num_hits),
            min_score_acc: if total_hits_threshold != i32::MAX {
                Some(Arc::new(MaxScoreAccumulator::new()))
            } else {
                None
            },
        })
    }

    /// Creates a manager collecting `num_hits` results from the first page.
    ///
    /// Equivalent to `new TopScoreDocCollectorManager(int, int)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_num_hits(num_hits: i32, total_hits_threshold: i32) -> Result<Self> {
        Self::new(num_hits, None, total_hits_threshold)
    }
}

impl CollectorManager for TopScoreDocCollectorManager {
    type Collector = TopScoreDocCollector;
    type Output = TopDocs;

    fn new_collector(&self) -> Result<TopScoreDocCollector> {
        TopScoreDocCollector::new(
            self.num_hits,
            self.after,
            self.total_hits_threshold,
            self.min_score_acc.clone(),
        )
    }

    fn reduce(&self, collectors: Vec<TopScoreDocCollector>) -> Result<TopDocs> {
        let mut top_docs = Vec::with_capacity(collectors.len());
        for mut collector in collectors {
            top_docs.push(collector.top_docs()?);
        }
        TopDocs::merge_from(0, self.num_hits, &top_docs)
    }
}
