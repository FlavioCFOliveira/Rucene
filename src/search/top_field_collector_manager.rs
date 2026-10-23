//! Sorted top-N collector manager, ported from
//! `org.apache.lucene.search.TopFieldCollectorManager`.

#![deny(unsafe_code)]

use std::sync::{Arc, Mutex, PoisonError};

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

/// How the manager holds its collectors: `Send` and `Sync` whenever the
/// collector is `Send`.
pub(crate) type CollectorHandles<C> = Mutex<Vec<C>>;

/// Creates [`TopFieldCollector`]s that share a [`MaxScoreAccumulator`], so that
/// the minimum competitive score found by one slice prunes the others.
///
/// Equivalent to `org.apache.lucene.search.TopFieldCollectorManager`. A new
/// manager should be created for each search, because of its internal state.
///
/// Java also keeps a deprecated five-argument constructor whose
/// `supportsConcurrency` flag it ignores — it simply delegates to the
/// four-argument one — which is not ported: it carries no behaviour at all.
/// The deprecated `getCollectors()` is
/// [`get_collectors`](Self::get_collectors).
///
/// **Divergence from Lucene 10.5.0, and from this port at `6cb74bf`.** At
/// `6cb74bf` this manager was `Send` and `Sync`. Keeping a handle to every
/// collector it creates — which `getCollectors()` needs — makes it `Send` and
/// `Sync` only when [`TopFieldCollector`] is, and today that collector is
/// neither, because [`FieldComparator`](crate::search::FieldComparator)
/// carries no `Send` bound. In Lucene the manager stays on the calling
/// thread: `IndexSearcher` creates every collector and reduces them there,
/// and each executor task captures only its own collector
/// (`IndexSearcher.java:764-796`). The collectors cross threads while the
/// manager keeps a handle to each, so the handles and their list are
/// `Arc<Mutex<..>>` and `Mutex<..>`, and the manager regains both traits
/// automatically once the comparators are `Send`; the unit test
/// `manager_is_send_and_sync_but_for_its_collectors` pins every other part.
#[derive(Debug)]
pub struct TopFieldCollectorManager {
    sort: Sort,
    num_hits: i32,
    after: Option<FieldDoc>,
    total_hits_threshold: i32,
    min_score_acc: Option<Arc<MaxScoreAccumulator>>,
    /// A handle to every collector [`new_collector`](CollectorManager::new_collector)
    /// created, in creation order.
    ///
    /// Equivalent to the `private final List<TopFieldCollector> collectors`
    /// field.
    collectors: CollectorHandles<TopFieldCollector>,
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
            collectors: Mutex::new(Vec::new()),
        })
    }

    /// Returns the collectors created by this manager, in creation order.
    ///
    /// Equivalent to `TopFieldCollectorManager#getCollectors`
    /// (`TopFieldCollectorManager.java:189-198`), deprecated in Lucene 10.5.0
    /// for removal in Lucene 11: the reduced [`TopFieldDocs`] carries what it
    /// exposes. Each element is a handle to the collector a search was given,
    /// so it shows what that search collected — including that
    /// [`reduce`](CollectorManager::reduce) has drained its queue, as Java's
    /// does.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns its live list; this
    /// returns the handles held when it is called, so a collector created
    /// afterwards is not in a list obtained before.
    #[deprecated(note = "Lucene 10.5.0 deprecates getCollectors() for removal in Lucene 11")]
    pub fn get_collectors(&self) -> Vec<TopFieldCollector> {
        self.collectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(TopFieldCollector::share)
            .collect()
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
        let collector = TopFieldCollector::new(
            self.sort.clone(),
            queue,
            self.after.clone(),
            self.num_hits,
            self.total_hits_threshold,
            self.min_score_acc.clone(),
        );
        self.collectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(collector.share());
        Ok(collector)
    }

    fn reduce(&self, collectors: Vec<TopFieldCollector>) -> Result<TopFieldDocs> {
        let mut top_docs = Vec::with_capacity(collectors.len());
        for mut collector in collectors {
            top_docs.push(collector.top_docs()?);
        }
        TopDocs::merge_sorted_from(&self.sort, 0, self.num_hits as usize, &top_docs)
    }
}

#[cfg(test)]
mod send_sync_tests {
    use super::*;
    use crate::search::field_value_hit_queue;
    use crate::search::top_field_collector::{self, SharedCollectorState};

    fn assert_send_sync<T: Send + Sync>(_: &T) {}

    /// Every field of the manager but its collectors is `Send + Sync`. The
    /// destructuring names every field, so a field added later fails to
    /// compile until it is listed, and a listed field that is not both fails
    /// its assertion.
    fn manager_fields(manager: &TopFieldCollectorManager) {
        let TopFieldCollectorManager {
            sort,
            num_hits,
            after,
            total_hits_threshold,
            min_score_acc,
            collectors: _,
        } = manager;
        assert_send_sync(sort);
        assert_send_sync(num_hits);
        assert_send_sync(after);
        assert_send_sync(total_hits_threshold);
        assert_send_sync(min_score_acc);
    }

    /// A collector handle and the manager's list of handles are both
    /// `Send + Sync` for any `Send` state — checked on the very type aliases
    /// the two structs use, for every `S` at once.
    fn handles_are_send_and_sync<S: Send>(
        handle: &SharedCollectorState<S>,
        handles: &CollectorHandles<SharedCollectorState<S>>,
    ) {
        assert_send_sync(handle);
        assert_send_sync(handles);
    }

    /// The manager is `Send + Sync` exactly when the comparators are `Send`.
    /// Checked at compile time on the real structs, field by field: the
    /// manager's fields but its collectors, the generic handle and list,
    /// the collector and its state but the hit queue, and the queue but its
    /// comparators.
    ///
    /// The assertion cannot be written on the concrete manager today, because
    /// it does not hold: the hit queue holds `Box<dyn FieldComparator>`, and
    /// `FieldComparator` has no `Send` bound. Once it has,
    /// `assert_send_sync::<TopFieldCollectorManager>()` compiles and belongs
    /// here.
    #[test]
    fn manager_is_send_and_sync_but_for_its_collectors() {
        let _: fn(&TopFieldCollectorManager) = manager_fields;
        let _: fn(&SharedCollectorState<()>, &CollectorHandles<SharedCollectorState<()>>) =
            handles_are_send_and_sync::<()>;
        top_field_collector::send_checks::collector_is_send_but_for_its_queue();
        field_value_hit_queue::send_checks::queue_is_send_but_for_its_comparators();
    }
}
