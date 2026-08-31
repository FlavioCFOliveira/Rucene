//! Sorted top-N collection, ported from
//! `org.apache.lucene.search.TopFieldCollector`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{sub_index_from_leaves, LeafReaderContext};
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::collector::{Collector, LeafCollector};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_comparator::RelevanceComparator;
use crate::search::field_doc::FieldDoc;
use crate::search::field_value_hit_queue::{Entry, FieldValueHitQueue};
use crate::search::index_searcher::IndexSearcher;
use crate::search::max_score_accumulator::MaxScoreAccumulator;
use crate::search::query::Query;
use crate::search::scorable::Scorable;
use crate::search::score_caching_wrapping_scorer::ScoreCachingWrappingScorer;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::sort::{Sort, SortField};
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::top_field_docs::TopFieldDocs;
use crate::search::top_score_doc_collector::DocScoreEncoder;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// Returns whether a search sorted by `search_sort` may terminate early on a
/// segment sorted by `index_sort`.
///
/// Equivalent to the package-private
/// `TopFieldCollector.canEarlyTerminate(Sort, Sort)`.
pub fn can_early_terminate(search_sort: &Sort, index_sort: Option<&Sort>) -> bool {
    can_early_terminate_on_doc_id(search_sort)
        || can_early_terminate_on_prefix(search_sort, index_sort)
}

/// Equivalent to the private
/// `TopFieldCollector.canEarlyTerminateOnDocId(Sort)`.
fn can_early_terminate_on_doc_id(search_sort: &Sort) -> bool {
    match search_sort.fields().first() {
        Some(first) => *first == SortField::FIELD_DOC,
        None => false,
    }
}

/// Equivalent to the private
/// `TopFieldCollector.canEarlyTerminateOnPrefix(Sort, Sort)`: early
/// termination is possible when the search sort is a prefix of the index sort.
fn can_early_terminate_on_prefix(search_sort: &Sort, index_sort: Option<&Sort>) -> bool {
    match index_sort {
        None => false,
        Some(index_sort) => {
            let fields1 = search_sort.fields();
            let fields2 = index_sort.fields();
            if fields1.len() > fields2.len() {
                return false;
            }
            fields1 == &fields2[..fields1.len()]
        }
    }
}

/// A [`Collector`] that sorts by [`SortField`] using
/// [`FieldComparator`](crate::search::FieldComparator)s.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.TopFieldCollector` and both of its package-private
/// subclasses. See
/// [`TopFieldCollectorManager`](crate::search::TopFieldCollectorManager) for
/// how to instantiate one with support for concurrency in
/// [`IndexSearcher`](crate::search::IndexSearcher).
///
/// **Divergence from Lucene 10.5.0.** Java splits the collector into
/// `SimpleFieldCollector` and `PagingFieldCollector`, which differ only in
/// whether an `after` hit is set and in the extra top-value check that entails.
/// This port carries `after` as an [`Option`] and branches on it, exactly as
/// [`TopScoreDocCollector`](crate::search::TopScoreDocCollector) already does
/// for the same split; the collected hits are identical.
#[derive(Debug)]
pub struct TopFieldCollector {
    sort: Sort,
    queue: FieldValueHitQueue,
    after: Option<FieldDoc>,
    /// The number of hits collected on the current page.
    ///
    /// Equivalent to `PagingFieldCollector.collectedHits`.
    collected_hits: i32,
    num_hits: i32,
    total_hits_threshold: i32,
    can_set_min_score: bool,
    /// Whether the search sort is part of the index sort. As all segments are
    /// sorted the same way, checking the first segment is enough.
    ///
    /// Equivalent to the `Boolean searchSortPartOfIndexSort` field, whose
    /// `null` state is [`None`] here.
    search_sort_part_of_index_sort: Option<bool>,
    /// An accumulator that maintains the maximum of the segments' minimum
    /// competitive scores.
    min_score_acc: Option<Arc<MaxScoreAccumulator>>,
    /// The current local minimum competitive score already propagated to the
    /// underlying scorer.
    min_competitive_score: f32,
    bottom: Option<Entry>,
    queue_full: bool,
    doc_base: i32,
    needs_scores: bool,
    score_mode: ScoreMode,
    total_hits: i32,
    total_hits_relation: TotalHitsRelation,
}

impl TopFieldCollector {
    /// Creates a collector keeping `num_hits` candidates, sorted by `sort`.
    ///
    /// Equivalent to the private
    /// `TopFieldCollector(FieldValueHitQueue, int, int, boolean, MaxScoreAccumulator)`
    /// constructor plus the `SimpleFieldCollector` and `PagingFieldCollector`
    /// ones. It is public here because Rust has no package visibility;
    /// [`TopFieldCollectorManager`](crate::search::TopFieldCollectorManager) is
    /// still the intended way to build one.
    ///
    /// When `after` is set, every comparator is told the top value it carries,
    /// which is what makes the collector page.
    pub fn new(
        sort: Sort,
        mut queue: FieldValueHitQueue,
        after: Option<FieldDoc>,
        num_hits: i32,
        total_hits_threshold: i32,
        min_score_acc: Option<Arc<MaxScoreAccumulator>>,
    ) -> Self {
        let needs_scores = sort.needs_scores();
        let num_comparators = queue.get_comparators().len();
        let first_is_relevance = queue.get_comparators()[0]
            .as_any()
            .is::<RelevanceComparator>();
        let reverse_mul = queue.get_reverse_mul()[0];

        let (score_mode, can_set_min_score) = if first_is_relevance
            && reverse_mul == 1 // the natural sort is preserved (descending relevance)
            && total_hits_threshold != i32::MAX
        {
            (ScoreMode::TOP_SCORES, true)
        } else if total_hits_threshold != i32::MAX {
            (
                if needs_scores {
                    ScoreMode::TOP_DOCS_WITH_SCORES
                } else {
                    ScoreMode::TOP_DOCS
                },
                false,
            )
        } else {
            (
                if needs_scores {
                    ScoreMode::COMPLETE
                } else {
                    ScoreMode::COMPLETE_NO_SCORES
                },
                false,
            )
        };

        if let Some(after) = after.as_ref() {
            // Tell all comparators their top value.
            if let Some(fields) = after.fields.as_ref() {
                for (i, comparator) in queue.get_comparators_mut().iter_mut().enumerate() {
                    if let Some(value) = fields.get(i) {
                        comparator.set_top_value(value.clone());
                    }
                }
            }
        }
        debug_assert!(num_comparators >= 1);

        Self {
            sort,
            queue,
            after,
            collected_hits: 0,
            num_hits,
            total_hits_threshold: total_hits_threshold.max(num_hits),
            can_set_min_score,
            search_sort_part_of_index_sort: None,
            min_score_acc,
            min_competitive_score: 0.0,
            bottom: None,
            queue_full: false,
            doc_base: 0,
            needs_scores,
            score_mode,
            total_hits: 0,
            total_hits_relation: TotalHitsRelation::EQUAL_TO,
        }
    }

    /// The number of hits this collector counts accurately.
    ///
    /// Equivalent to reading the package-private `totalHitsThreshold` field.
    pub fn total_hits_threshold(&self) -> i32 {
        self.total_hits_threshold
    }

    /// Returns whether collection terminated early.
    ///
    /// Equivalent to `TopFieldCollector.isEarlyTerminated()`.
    pub fn is_early_terminated(&self) -> bool {
        self.total_hits_relation == TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
    }

    /// Populates the [`scores`](crate::search::ScoreDoc::score) of the given
    /// hits.
    ///
    /// Equivalent to the static
    /// `TopFieldCollector.populateScores(ScoreDoc[], IndexSearcher, Query)`.
    ///
    /// * `top_docs` — the hits to populate, which are rewritten in place;
    /// * `searcher` — the searcher that computed them;
    /// * `query` — the query that computed them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when there is evidence that the
    /// hits were computed against a different searcher or a different query,
    /// and propagates any I/O error raised while scoring.
    pub fn populate_scores(
        top_docs: &mut [ScoreDoc],
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
    ) -> Result<()> {
        // Sort the hits in doc-ID order. Java clones the array first because it
        // sorts a caller-owned array; this port sorts the slice the caller
        // handed over, which is the same slice whose scores it fills in.
        let mut order: Vec<usize> = (0..top_docs.len()).collect();
        order.sort_by_key(|&i| top_docs[i].doc);

        let rewritten = searcher.rewrite(query)?;
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        let contexts = Arc::clone(searcher.get_index_reader()).leaves();
        let max_doc = searcher.get_index_reader().max_doc();

        let mut current_context: Option<Arc<LeafReaderContext>> = None;
        let mut current_scorer: Option<Box<dyn Scorer>> = None;
        for &i in &order {
            let doc = top_docs[i].doc;
            let needs_new_context = match current_context.as_ref() {
                None => true,
                Some(context) => doc >= context.doc_base() + context.leaf_reader().max_doc(),
            };
            if needs_new_context {
                if doc < 0 || doc >= max_doc {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Index {doc} out of bounds for length {max_doc}"
                    )));
                }
                let new_context_index = sub_index_from_leaves(doc, &contexts);
                let context = Arc::clone(&contexts[new_context_index]);
                let scorer_supplier = weight.scorer_supplier(&context)?;
                let Some(mut scorer_supplier) = scorer_supplier else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Doc id {doc} doesn't match the query"
                    )));
                };
                // Random access.
                current_scorer = Some(scorer_supplier.get(1)?);
                current_context = Some(context);
            }
            let context = current_context
                .as_ref()
                .expect("INVARIANT: a context was just installed");
            let scorer = current_scorer
                .as_mut()
                .expect("INVARIANT: a scorer was just installed");
            let leaf_doc = doc - context.doc_base();
            debug_assert!(leaf_doc >= 0);
            let advanced = scorer.iterator().advance(leaf_doc)?;
            if leaf_doc != advanced {
                return Err(LuceneError::IllegalArgument(format!(
                    "Doc id {doc} doesn't match the query"
                )));
            }
            top_docs[i].score = scorer.score()?;
        }
        Ok(())
    }

    /// Equivalent to `TopFieldCollector.updateGlobalMinCompetitiveScore(Scorable)`.
    fn update_global_min_competitive_score(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        let Some(acc) = self.min_score_acc.as_ref() else {
            debug_assert!(false, "minScoreAcc must be present");
            return Ok(());
        };
        if !self.can_set_min_score {
            return Ok(());
        }
        // The global maximum score can be checked even if the local queue is
        // not full or the threshold is not reached on the local competitor: the
        // fact that there is a shared minimum competitive score implies that one
        // of the collectors hit its totalHitsThreshold already.
        let max_min_score = acc.get_raw();
        if max_min_score != i64::MIN {
            let score = DocScoreEncoder::to_score(max_min_score);
            if score > self.min_competitive_score {
                scorer.set_min_competitive_score(score)?;
                self.min_competitive_score = score;
                self.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
            }
        }
        Ok(())
    }

    /// Equivalent to `TopFieldCollector.updateMinCompetitiveScore(Scorable)`.
    fn update_min_competitive_score(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        if self.can_set_min_score && self.queue_full && self.total_hits > self.total_hits_threshold
        {
            let bottom = self
                .bottom
                .expect("INVARIANT: the queue is full, so a bottom entry exists");
            let min_score = self.queue.get_comparators()[0]
                .value(bottom.slot)
                .as_float()
                .expect("INVARIANT: canSetMinScore implies the first comparator is a RelevanceComparator");
            if min_score > self.min_competitive_score {
                scorer.set_min_competitive_score(min_score)?;
                self.min_competitive_score = min_score;
                self.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
                if let Some(acc) = self.min_score_acc.as_ref() {
                    acc.accumulate(DocScoreEncoder::encode(self.doc_base, min_score));
                }
            }
        }
        Ok(())
    }

    /// Equivalent to the `final TopFieldCollector.add(int, int)`.
    fn add(&mut self, slot: i32, doc: i32) {
        let doc_base = self.doc_base;
        self.bottom = self.queue.add(Entry::new(slot, doc_base + doc));
        // The queue is full either when totalHits == numHits, in which case
        // slot = totalHits - 1, or when collectedHits == numHits (hits on the
        // current page) and slot = collectedHits - 1.
        debug_assert!(slot < self.num_hits);
        self.queue_full = slot == self.num_hits - 1;
    }

    /// Equivalent to the `final TopFieldCollector.updateBottom(int)`.
    fn update_bottom(&mut self, doc: i32) {
        // bottom.score is already set to NaN in add().
        let doc_base = self.doc_base;
        self.queue.set_top_doc(doc_base + doc);
        self.bottom = self.queue.update_top();
    }
}

impl Collector for TopFieldCollector {
    fn score_mode(&self) -> ScoreMode {
        self.score_mode
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // As all segments are sorted in the same way, it is enough to check
        // only the first segment for the index sort.
        if self.search_sort_part_of_index_sort.is_none() {
            let meta = context.leaf_reader().get_meta_data();
            let part_of_index_sort = can_early_terminate(&self.sort, meta.sort());
            self.search_sort_part_of_index_sort = Some(part_of_index_sort);
            if part_of_index_sort {
                self.queue.get_comparators_mut()[0].disable_skipping();
            }
        }
        self.queue.set_next_leaf(context)?;

        // Reset the minimum competitive score.
        self.min_competitive_score = 0.0;
        self.doc_base = context.doc_base();
        let after_doc = self
            .after
            .as_ref()
            .map(|after| after.score_doc.doc - self.doc_base);

        let needs_scores = self.needs_scores;
        let collector = TopFieldLeafCollector {
            parent: self,
            after_doc,
            collected_all_competitive_hits: false,
        };
        if needs_scores {
            // Score-based comparators may need to call score() several times —
            // once for the comparison, and once to copy the score into the
            // priority queue.
            Ok(Box::new(ScoreCachingWrappingScorer::wrap(collector)))
        } else {
            Ok(Box::new(collector))
        }
    }
}

impl TopDocsCollector for TopFieldCollector {
    type Hit = FieldDoc;
    type Docs = TopFieldDocs;

    fn total_hits(&self) -> i32 {
        self.total_hits
    }

    fn total_hits_relation(&self) -> TotalHitsRelation {
        self.total_hits_relation
    }

    fn pq_size(&self) -> usize {
        self.queue.size()
    }

    /// Equivalent to `pq.pop()` followed by `queue.fillFields(...)`, which is
    /// what `TopFieldCollector.populateResults` does with each popped entry.
    fn pop(&mut self) -> Option<FieldDoc> {
        let entry = self.queue.pop()?;
        Some(self.queue.fill_fields(&entry))
    }

    /// Equivalent to `TopFieldCollector.newTopDocs(ScoreDoc[], int)`, which
    /// returns a [`TopFieldDocs`] carrying the sort fields.
    ///
    /// # Errors
    ///
    /// Propagates the [`TotalHits`] validation error, which cannot trigger for
    /// a non-negative hit count.
    fn new_top_docs(&self, results: Option<Vec<FieldDoc>>, _start: i32) -> Result<TopFieldDocs> {
        Ok(TopFieldDocs::new(
            TotalHits::new(i64::from(self.total_hits), self.total_hits_relation)?,
            results.unwrap_or_default(),
            self.queue.get_fields().to_vec(),
        ))
    }
}

/// The per-leaf half of a [`TopFieldCollector`].
///
/// Equivalent to the abstract inner class
/// `TopFieldCollector.TopFieldLeafCollector` together with the two anonymous
/// subclasses that `SimpleFieldCollector.getLeafCollector` and
/// `PagingFieldCollector.getLeafCollector` return.
struct TopFieldLeafCollector<'a> {
    parent: &'a mut TopFieldCollector,
    /// The `after` hit rebased on the current leaf, when paging.
    ///
    /// Equivalent to `final int afterDoc = after.doc - docBase`.
    after_doc: Option<i32>,
    /// Whether every competitive hit of this leaf has already been collected.
    ///
    /// Equivalent to `TopFieldLeafCollector.collectedAllCompetitiveHits`.
    collected_all_competitive_hits: bool,
}

impl std::fmt::Debug for TopFieldLeafCollector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopFieldLeafCollector")
            .field("after_doc", &self.after_doc)
            .finish_non_exhaustive()
    }
}

impl TopFieldLeafCollector<'_> {
    /// Equivalent to `TopFieldLeafCollector.countHit()`.
    fn count_hit(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.parent.total_hits += 1;
        let hit_count_so_far = self.parent.total_hits;

        if let Some(acc) = self.parent.min_score_acc.as_ref() {
            let mod_interval = acc.mod_interval();
            if (i64::from(hit_count_so_far) & mod_interval) == 0 {
                self.parent.update_global_min_competitive_score(scorer)?;
            }
        }
        if !self.parent.score_mode.is_exhaustive()
            && self.parent.total_hits_relation == TotalHitsRelation::EQUAL_TO
            && self.parent.total_hits > self.parent.total_hits_threshold
        {
            // The hits threshold was reached for the first time; notify the
            // comparator about it.
            self.parent.queue.set_hits_threshold_reached()?;
            self.parent.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
        }
        Ok(())
    }

    /// Equivalent to `TopFieldLeafCollector.thresholdCheck(int)`.
    fn threshold_check(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<bool> {
        if self.collected_all_competitive_hits
            || self.parent.queue.compare_bottom(doc, scorer)? <= 0
        {
            // Since docs are visited in doc ID order, a comparison of 0 means
            // this document is larger than anything else in the queue and
            // therefore not competitive.
            if self.parent.search_sort_part_of_index_sort == Some(true) {
                if self.parent.total_hits > self.parent.total_hits_threshold {
                    self.parent.total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
                    return Err(CollectionError::CollectionTerminated);
                }
                self.collected_all_competitive_hits = true;
            } else if self.parent.total_hits_relation == TotalHitsRelation::EQUAL_TO {
                // The minimum competitive score can start being set here, the
                // first time the threshold is reached.
                self.parent.update_min_competitive_score(scorer)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Equivalent to `TopFieldLeafCollector.collectCompetitiveHit(int)`.
    fn collect_competitive_hit(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        // This hit is competitive: replace the bottom element of the queue and
        // adjust the top.
        let bottom_slot = self
            .parent
            .bottom
            .expect("INVARIANT: the queue is full, so a bottom entry exists")
            .slot;
        self.parent.queue.copy(bottom_slot, doc, scorer)?;
        self.parent.update_bottom(doc);
        let bottom_slot = self
            .parent
            .bottom
            .expect("INVARIANT: updateBottom leaves a bottom entry")
            .slot;
        self.parent.queue.set_bottom(bottom_slot)?;
        self.parent.update_min_competitive_score(scorer)
    }

    /// Equivalent to `TopFieldLeafCollector.collectAnyHit(int, int)`.
    fn collect_any_hit(
        &mut self,
        doc: i32,
        hits_collected: i32,
        scorer: &mut dyn Scorable,
    ) -> Result<()> {
        // Startup transient: the queue has not gathered numHits yet.
        let slot = hits_collected - 1;
        // Copy the hit into the queue.
        self.parent.queue.copy(slot, doc, scorer)?;
        self.parent.add(slot, doc);
        if self.parent.queue_full {
            let bottom_slot = self
                .parent
                .bottom
                .expect("INVARIANT: a full queue has a bottom entry")
                .slot;
            self.parent.queue.set_bottom(bottom_slot)?;
            self.parent.update_min_competitive_score(scorer)?;
        }
        Ok(())
    }
}

impl LeafCollector for TopFieldLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.parent.queue.set_scorer(scorer)?;
        if self.parent.min_score_acc.is_none() {
            self.parent.update_min_competitive_score(scorer)
        } else {
            self.parent.update_global_min_competitive_score(scorer)
        }
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.count_hit(scorer)?;
        match self.after_doc {
            None => {
                if self.parent.queue_full {
                    if self.threshold_check(doc, scorer)? {
                        return Ok(());
                    }
                    self.collect_competitive_hit(doc, scorer)?;
                } else {
                    let total_hits = self.parent.total_hits;
                    self.collect_any_hit(doc, total_hits, scorer)?;
                }
            }
            Some(after_doc) => {
                if self.parent.queue_full && self.threshold_check(doc, scorer)? {
                    return Ok(());
                }
                let top_cmp = self.parent.queue.compare_top(doc, scorer)?;
                if top_cmp > 0 || (top_cmp == 0 && doc <= after_doc) {
                    // Already collected on a previous page. Check whether the
                    // hits threshold is reached and the competitive score can be
                    // updated, which is necessary to account for a possible
                    // update to the global minimum competitive score.
                    if self.parent.total_hits_relation == TotalHitsRelation::EQUAL_TO {
                        self.parent.update_min_competitive_score(scorer)?;
                    }
                    return Ok(());
                }
                if self.parent.queue_full {
                    self.collect_competitive_hit(doc, scorer)?;
                } else {
                    self.parent.collected_hits += 1;
                    let collected_hits = self.parent.collected_hits;
                    self.collect_any_hit(doc, collected_hits, scorer)?;
                }
            }
        }
        Ok(())
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.parent.queue.competitive_iterator()
    }
}
