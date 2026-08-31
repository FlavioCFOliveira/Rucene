//! Sorted top-N collector manager, ported from
//! `org.apache.lucene.search.TopFieldCollectorManager`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::collector::CollectorManager;
use crate::search::field_doc::FieldDoc;
use crate::search::field_value_hit_queue::FieldValueHitQueue;
use crate::search::max_score_accumulator::MaxScoreAccumulator;
use crate::search::sort::Sort;
use crate::search::top_docs::TopDocs;
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::top_field_collector::TopFieldCollector;
use crate::search::top_field_docs::TopFieldDocs;

/// Creates [`TopFieldCollector`]s that share a [`MaxScoreAccumulator`], so that
/// the minimum competitive score found by one slice prunes the others.
///
/// Equivalent to `org.apache.lucene.search.TopFieldCollectorManager`. A new
/// manager should be created for each search, because of its internal state.
///
/// Java also keeps a deprecated five-argument constructor whose
/// `supportsConcurrency` flag it ignores, and a deprecated `getCollectors()`
/// accessor over the collectors it has handed out; neither is ported, because
/// the first carries no behaviour and the second would require the manager to
/// retain collectors it has already given away.
#[derive(Debug)]
pub struct TopFieldCollectorManager {
    sort: Sort,
    num_hits: i32,
    after: Option<FieldDoc>,
    total_hits_threshold: i32,
    min_score_acc: Option<Arc<MaxScoreAccumulator>>,
}

impl TopFieldCollectorManager {
    /// Creates a manager collecting `num_hits` results after `after`, counting
    /// hits accurately up to `total_hits_threshold`.
    ///
    /// Equivalent to
    /// `new TopFieldCollectorManager(Sort, int, FieldDoc, int)`.
    ///
    /// If the total hit count of the top docs is less than or exactly
    /// `total_hits_threshold`, the value reported in
    /// [`TopFieldDocs::total_hits`] is accurate; if it is greater, the value is
    /// a lower bound. [`i32::MAX`] makes the hit count accurate but also makes
    /// query processing slower.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `total_hits_threshold` is negative, when `num_hits` is
    /// not positive, when `sort` has no field, or when `after` does not carry
    /// exactly one sort value per sort field.
    pub fn new(
        sort: Sort,
        num_hits: i32,
        after: Option<FieldDoc>,
        total_hits_threshold: i32,
    ) -> Result<Self> {
        if total_hits_threshold < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "totalHitsThreshold must be >= 0, got {total_hits_threshold}"
            )));
        }
        if num_hits <= 0 {
            return Err(LuceneError::IllegalArgument(
                "numHits must be > 0; please use TotalHitCountCollector if you just need the total hit count"
                    .to_string(),
            ));
        }
        if sort.fields().is_empty() {
            return Err(LuceneError::IllegalArgument(
                "Sort must contain at least one field".to_string(),
            ));
        }
        if let Some(after) = after.as_ref() {
            Self::check_after(after, &sort)?;
        }

        Ok(Self {
            min_score_acc: if total_hits_threshold != i32::MAX {
                Some(Arc::new(MaxScoreAccumulator::new()))
            } else {
                None
            },
            sort,
            num_hits,
            after,
            total_hits_threshold,
        })
    }

    /// Creates a manager collecting `num_hits` results from the first page.
    ///
    /// Equivalent to `new TopFieldCollectorManager(Sort, int, int)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_num_hits(sort: Sort, num_hits: i32, total_hits_threshold: i32) -> Result<Self> {
        Self::new(sort, num_hits, None, total_hits_threshold)
    }

    /// Equivalent to the `after` validation Java repeats in the constructor and
    /// in `newCollector()`.
    fn check_after(after: &FieldDoc, sort: &Sort) -> Result<()> {
        let Some(fields) = after.fields.as_ref() else {
            return Err(LuceneError::IllegalArgument(
                "after.fields wasn't set; you must pass fillFields=true for the previous search"
                    .to_string(),
            ));
        };
        if fields.len() != sort.fields().len() {
            return Err(LuceneError::IllegalArgument(format!(
                "after.fields has {} values but sort has {}",
                fields.len(),
                sort.fields().len()
            )));
        }
        Ok(())
    }
}

impl CollectorManager for TopFieldCollectorManager {
    type Collector = TopFieldCollector;
    type Output = TopFieldDocs;

    fn new_collector(&self) -> Result<TopFieldCollector> {
        let mut queue =
            FieldValueHitQueue::create(self.sort.fields().to_vec(), self.num_hits as usize)?;
        if self.after.is_none() {
            // Inform the comparator that the sort is based on this single
            // field, to enable some optimizations for skipping over
            // non-competitive documents. Single sort cannot be set when the
            // `after` parameter is non-null, because that is an implicit sort
            // over the doc ID.
            if queue.get_comparators().len() == 1 {
                queue.get_comparators_mut()[0].set_single_sort();
            }
        } else if let Some(after) = self.after.as_ref() {
            Self::check_after(after, &self.sort)?;
        }
        Ok(TopFieldCollector::new(
            self.sort.clone(),
            queue,
            self.after.clone(),
            self.num_hits,
            self.total_hits_threshold,
            self.min_score_acc.clone(),
        ))
    }

    fn reduce(&self, collectors: Vec<TopFieldCollector>) -> Result<TopFieldDocs> {
        let mut top_docs = Vec::with_capacity(collectors.len());
        for mut collector in collectors {
            top_docs.push(collector.top_docs()?);
        }
        TopDocs::merge_sorted_from(&self.sort, 0, self.num_hits as usize, &top_docs)
    }
}
