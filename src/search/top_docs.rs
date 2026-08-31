//! Search results, ported from `org.apache.lucene.search.TopDocs`.
//!
//! # Relationship with [`crate::search::knn::TopDocs`]
//!
//! The kNN module carries a placeholder `TopDocs` — an empty struct — so that
//! [`KnnCollector`](crate::search::knn::KnnCollector) could be ported before
//! the search package existed. This module is the real
//! `org.apache.lucene.search.TopDocs`, and it is the one
//! [`TopDocsCollector`](crate::search::TopDocsCollector) and
//! [`IndexSearcher`](crate::search::IndexSearcher) return. The placeholder
//! should eventually be replaced by this type; doing so touches the kNN
//! readers and the HNSW collectors and is left for the task that owns them.
//!
//! # Overloads
//!
//! Java overloads `merge` on whether a `Sort` is supplied. Rust has no
//! overloading, so the score-based forms keep the `merge*` names and the
//! sort-based ones — which return a
//! [`TopFieldDocs`](crate::search::TopFieldDocs) — are
//! [`merge_sorted`](TopDocs::merge_sorted),
//! [`merge_sorted_from`](TopDocs::merge_sorted_from) and
//! [`merge_sorted_with_tie_breaker`](TopDocs::merge_sorted_with_tie_breaker).

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::error::{LuceneError, Result};
use crate::search::field_doc::FieldDoc;
use crate::search::pruning::Pruning;
use crate::search::score_doc::ScoreDoc;
use crate::search::sort::Sort;
use crate::search::top_field_docs::TopFieldDocs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};
use crate::util::{PriorityQueue, PriorityQueueComparator};

/// Represents the hits returned by
/// [`IndexSearcher::search`](crate::search::IndexSearcher::search).
///
/// Equivalent to `org.apache.lucene.search.TopDocs`, whose two fields are
/// public and mutable in Java and are public here for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub struct TopDocs {
    /// The total number of hits for the query.
    pub total_hits: TotalHits,

    /// The top hits for the query.
    pub score_docs: Vec<ScoreDoc>,
}

/// A comparator that breaks ties between two equally-scoring hits.
///
/// Equivalent to the `Comparator<ScoreDoc>` that Java's merge routines accept.
pub type TieBreaker<'a> = &'a dyn Fn(&ScoreDoc, &ScoreDoc) -> Ordering;

/// Refers to one hit: which shard it comes from, and which hit within that
/// shard.
///
/// Equivalent to the private `TopDocs.ShardRef` class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShardRef {
    /// Which shard, as an index into the shard hits.
    shard_index: usize,
    /// Which hit within the shard.
    hit_index: usize,
}

impl ShardRef {
    fn new(shard_index: usize) -> Self {
        Self {
            shard_index,
            hit_index: 0,
        }
    }
}

/// Internal comparator on the shard index.
///
/// Equivalent to `TopDocs.SHARD_INDEX_TIE_BREAKER`.
fn shard_index_tie_breaker(a: &ScoreDoc, b: &ScoreDoc) -> Ordering {
    a.shard_index.cmp(&b.shard_index)
}

/// Internal comparator on the doc ID.
///
/// Equivalent to `TopDocs.DOC_ID_TIE_BREAKER`.
fn doc_id_tie_breaker(a: &ScoreDoc, b: &ScoreDoc) -> Ordering {
    a.doc.cmp(&b.doc)
}

/// The default tie breaker: shard index, then doc ID.
///
/// Equivalent to `TopDocs.DEFAULT_TIE_BREAKER`.
pub fn default_tie_breaker(a: &ScoreDoc, b: &ScoreDoc) -> Ordering {
    match shard_index_tie_breaker(a, b) {
        Ordering::Equal => doc_id_tie_breaker(a, b),
        ord => ord,
    }
}

/// Uses the tie breaker if it discriminates; otherwise breaks intra-shard ties
/// by hit index.
///
/// Equivalent to the package-private `TopDocs.tieBreakLessThan`.
fn tie_break_less_than(
    first: &ShardRef,
    first_doc: &ScoreDoc,
    second: &ShardRef,
    second_doc: &ScoreDoc,
    tie_breaker: TieBreaker<'_>,
) -> bool {
    let value = tie_breaker(first_doc, second_doc);

    if value == Ordering::Equal {
        // Equal values: tie break in the same shard by resolving however the
        // shard had resolved it.
        debug_assert!(first.hit_index != second.hit_index);
        return first.hit_index < second.hit_index;
    }

    value == Ordering::Less
}

/// Merge queue that merges by relevance score, descending.
///
/// Equivalent to the private `TopDocs.ScoreMergeSortQueue`.
struct ScoreMergeSortQueue<'a> {
    shard_hits: Vec<&'a [ScoreDoc]>,
    tie_breaker: TieBreaker<'a>,
}

impl<'a> PriorityQueueComparator<ShardRef> for ScoreMergeSortQueue<'a> {
    /// Returns `true` if `first` is less than `second`.
    fn less_than(&self, first: &ShardRef, second: &ShardRef) -> bool {
        let first_score_doc = &self.shard_hits[first.shard_index][first.hit_index];
        let second_score_doc = &self.shard_hits[second.shard_index][second.hit_index];
        if first_score_doc.score < second_score_doc.score {
            false
        } else if first_score_doc.score > second_score_doc.score {
            true
        } else {
            tie_break_less_than(
                first,
                first_score_doc,
                second,
                second_score_doc,
                self.tie_breaker,
            )
        }
    }
}

/// Merge queue that merges by the values of a [`Sort`].
///
/// Equivalent to the private `TopDocs.MergeSortQueue`.
struct MergeSortQueue<'a> {
    shard_hits: Vec<&'a [FieldDoc]>,
    comparators: Vec<Box<dyn crate::search::field_comparator::FieldComparator>>,
    reverse_mul: Vec<i32>,
    tie_breaker: TieBreaker<'a>,
}

impl PriorityQueueComparator<ShardRef> for MergeSortQueue<'_> {
    /// Returns `true` if `first` is less than `second`.
    fn less_than(&self, first: &ShardRef, second: &ShardRef) -> bool {
        let first_fd = &self.shard_hits[first.shard_index][first.hit_index];
        let second_fd = &self.shard_hits[second.shard_index][second.hit_index];

        for (comp_idx, comparator) in self.comparators.iter().enumerate() {
            let first_value = first_fd
                .fields
                .as_ref()
                .and_then(|fields| fields.get(comp_idx));
            let second_value = second_fd
                .fields
                .as_ref()
                .and_then(|fields| fields.get(comp_idx));
            let (Some(first_value), Some(second_value)) = (first_value, second_value) else {
                continue;
            };
            let cmp =
                self.reverse_mul[comp_idx] * comparator.compare_values(first_value, second_value);
            if cmp != 0 {
                return cmp < 0;
            }
        }
        tie_break_less_than(
            first,
            &first_fd.score_doc,
            second,
            &second_fd.score_doc,
            self.tie_breaker,
        )
    }
}

/// A `(shard index, doc)` pair, the key of the reciprocal-rank-fusion tally.
///
/// Equivalent to the private `TopDocs.ShardIndexAndDoc` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ShardIndexAndDoc {
    shard_index: i32,
    doc: i32,
}

impl TopDocs {
    /// Constructs a result set.
    ///
    /// Equivalent to `new TopDocs(TotalHits, ScoreDoc[])`.
    pub fn new(total_hits: TotalHits, score_docs: Vec<ScoreDoc>) -> Self {
        Self {
            total_hits,
            score_docs,
        }
    }

    /// Returns a new result set containing the top `top_n` results across the
    /// provided result sets, sorted by score. Each input must already be
    /// sorted.
    ///
    /// Equivalent to `TopDocs.merge(int, TopDocs[])`.
    ///
    /// # Errors
    ///
    /// As [`merge_from`](Self::merge_from).
    pub fn merge(top_n: usize, shard_hits: &[TopDocs]) -> Result<TopDocs> {
        Self::merge_from(0, top_n, shard_hits)
    }

    /// As [`merge`](Self::merge), but also ignores the top `start` results,
    /// which is typically useful for pagination.
    ///
    /// Equivalent to `TopDocs.merge(int, int, TopDocs[])`.
    ///
    /// Doc IDs are expected to follow a consistent pattern: either every hit
    /// has its shard index set, or every hit has it unset (`-1`), signifying
    /// that all hits belong to the same searcher.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the shard indices are
    /// inconsistently set across the merged hits.
    pub fn merge_from(start: usize, top_n: usize, shard_hits: &[TopDocs]) -> Result<TopDocs> {
        Self::merge_with_tie_breaker(start, top_n, shard_hits, &default_tie_breaker)
    }

    /// As [`merge_from`](Self::merge_from), but with a caller-supplied tie
    /// breaker.
    ///
    /// Equivalent to `TopDocs.merge(int, int, TopDocs[], Comparator<ScoreDoc>)`.
    ///
    /// # Errors
    ///
    /// As [`merge_from`](Self::merge_from).
    pub fn merge_with_tie_breaker(
        start: usize,
        top_n: usize,
        shard_hits: &[TopDocs],
        tie_breaker: TieBreaker<'_>,
    ) -> Result<TopDocs> {
        Self::merge_aux(start, top_n, shard_hits, tie_breaker)
    }

    /// The auxiliary method used by the merge entry points, sorting by score.
    ///
    /// Equivalent to the score half of the private `TopDocs.mergeAux`.
    fn merge_aux(
        start: usize,
        size: usize,
        shard_hits: &[TopDocs],
        tie_breaker: TieBreaker<'_>,
    ) -> Result<TopDocs> {
        let comparator = ScoreMergeSortQueue {
            shard_hits: shard_hits.iter().map(|d| d.score_docs.as_slice()).collect(),
            tie_breaker,
        };
        let mut queue: PriorityQueue<ShardRef, ScoreMergeSortQueue<'_>> =
            PriorityQueue::new(shard_hits.len().max(1), comparator)?;

        let mut total_hit_count: i64 = 0;
        let mut total_hits_relation = TotalHitsRelation::EQUAL_TO;
        let mut avail_hit_count: usize = 0;
        for (shard_idx, shard) in shard_hits.iter().enumerate() {
            // totalHits can be non-zero even if no hits were collected, when
            // searchAfter was used.
            total_hit_count += shard.total_hits.value();
            // If any hit count is a lower bound then the merged total hit count
            // is a lower bound as well.
            if shard.total_hits.relation() == TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO {
                total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
            }
            if !shard.score_docs.is_empty() {
                avail_hit_count += shard.score_docs.len();
                queue.add(ShardRef::new(shard_idx));
            }
        }

        let mut hits: Vec<ScoreDoc>;
        let mut unset_shard_index = false;
        if avail_hit_count <= start {
            hits = Vec::new();
        } else {
            hits = Vec::with_capacity(size.min(avail_hit_count - start));
            let requested_result_window = start + size;
            let num_iter_on_hits = avail_hit_count.min(requested_result_window);
            let mut hit_upto = 0usize;
            while hit_upto < num_iter_on_hits {
                let mut shard_ref = *queue
                    .top()
                    .ok_or_else(|| LuceneError::IllegalState("merge queue is empty".to_string()))?;
                let hit = shard_hits[shard_ref.shard_index].score_docs[shard_ref.hit_index];
                shard_ref.hit_index += 1;

                // Irrespective of whether shard indices are used for tie
                // breaking, check for a consistent order in shard indices to
                // defend against potential bugs.
                if hit_upto > 0 && unset_shard_index != (hit.shard_index == -1) {
                    return Err(LuceneError::IllegalArgument(
                        "Inconsistent order of shard indices".to_string(),
                    ));
                }

                unset_shard_index |= hit.shard_index == -1;

                if hit_upto >= start {
                    hits.push(hit);
                }

                hit_upto += 1;

                if shard_ref.hit_index < shard_hits[shard_ref.shard_index].score_docs.len() {
                    // Not done with these TopDocs yet.
                    queue.update_top_with(shard_ref);
                } else {
                    queue.pop();
                }
            }
        }

        let total_hits = TotalHits::new(total_hit_count, total_hits_relation)?;
        Ok(TopDocs::new(total_hits, hits))
    }

    /// Returns a new result set containing the top `top_n` results across the
    /// provided result sets, sorted by `sort`. Each input must already be
    /// sorted the same way.
    ///
    /// Equivalent to `TopDocs.merge(Sort, int, TopFieldDocs[])`.
    ///
    /// # Errors
    ///
    /// As [`merge_sorted_with_tie_breaker`](Self::merge_sorted_with_tie_breaker).
    pub fn merge_sorted(
        sort: &Sort,
        top_n: usize,
        shard_hits: &[TopFieldDocs],
    ) -> Result<TopFieldDocs> {
        Self::merge_sorted_from(sort, 0, top_n, shard_hits)
    }

    /// As [`merge_sorted`](Self::merge_sorted), but also ignores the top
    /// `start` results, which is typically useful for pagination.
    ///
    /// Equivalent to `TopDocs.merge(Sort, int, int, TopFieldDocs[])`.
    ///
    /// # Errors
    ///
    /// As [`merge_sorted_with_tie_breaker`](Self::merge_sorted_with_tie_breaker).
    pub fn merge_sorted_from(
        sort: &Sort,
        start: usize,
        top_n: usize,
        shard_hits: &[TopFieldDocs],
    ) -> Result<TopFieldDocs> {
        Self::merge_sorted_with_tie_breaker(sort, start, top_n, shard_hits, &default_tie_breaker)
    }

    /// As [`merge_sorted_from`](Self::merge_sorted_from), but with a
    /// caller-supplied tie breaker.
    ///
    /// Equivalent to
    /// `TopDocs.merge(Sort, int, int, TopFieldDocs[], Comparator<ScoreDoc>)`.
    ///
    /// Doc IDs are expected to follow a consistent pattern: either every hit
    /// has its shard index set, or every hit has it unset (`-1`), signifying
    /// that all hits belong to the same searcher.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a hit did not set its sort
    /// field values — Java's "shard N did not set sort field values" check —
    /// or when the shard indices are inconsistently set across the merged hits;
    /// propagates any error a [`SortField`](crate::search::SortField) raises
    /// while building its comparator.
    pub fn merge_sorted_with_tie_breaker(
        sort: &Sort,
        start: usize,
        top_n: usize,
        shard_hits: &[TopFieldDocs],
        tie_breaker: TieBreaker<'_>,
    ) -> Result<TopFieldDocs> {
        // Fail gracefully if the API is misused.
        for (shard_idx, shard) in shard_hits.iter().enumerate() {
            for hit in &shard.score_docs {
                if hit.fields.is_none() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "shard {shard_idx} did not set sort field values (FieldDoc.fields is null)"
                    )));
                }
            }
        }

        let sort_fields = sort.fields();
        let mut comparators = Vec::with_capacity(sort_fields.len());
        let mut reverse_mul = Vec::with_capacity(sort_fields.len());
        for sort_field in sort_fields {
            comparators.push(sort_field.get_comparator(1, Pruning::NONE)?);
            reverse_mul.push(if sort_field.reverse() { -1 } else { 1 });
        }

        let comparator = MergeSortQueue {
            shard_hits: shard_hits
                .iter()
                .map(|docs| docs.score_docs.as_slice())
                .collect(),
            comparators,
            reverse_mul,
            tie_breaker,
        };
        let mut queue: PriorityQueue<ShardRef, MergeSortQueue<'_>> =
            PriorityQueue::new(shard_hits.len().max(1), comparator)?;

        let mut total_hit_count: i64 = 0;
        let mut total_hits_relation = TotalHitsRelation::EQUAL_TO;
        let mut avail_hit_count: usize = 0;
        for (shard_idx, shard) in shard_hits.iter().enumerate() {
            total_hit_count += shard.total_hits.value();
            if shard.total_hits.relation() == TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO {
                total_hits_relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
            }
            if !shard.score_docs.is_empty() {
                avail_hit_count += shard.score_docs.len();
                queue.add(ShardRef::new(shard_idx));
            }
        }

        let mut hits: Vec<FieldDoc>;
        let mut unset_shard_index = false;
        if avail_hit_count <= start {
            hits = Vec::new();
        } else {
            hits = Vec::with_capacity(top_n.min(avail_hit_count - start));
            let requested_result_window = start + top_n;
            let num_iter_on_hits = avail_hit_count.min(requested_result_window);
            let mut hit_upto = 0usize;
            while hit_upto < num_iter_on_hits {
                let mut shard_ref = *queue
                    .top()
                    .ok_or_else(|| LuceneError::IllegalState("merge queue is empty".to_string()))?;
                let hit = shard_hits[shard_ref.shard_index].score_docs[shard_ref.hit_index].clone();
                shard_ref.hit_index += 1;

                if hit_upto > 0 && unset_shard_index != (hit.score_doc.shard_index == -1) {
                    return Err(LuceneError::IllegalArgument(
                        "Inconsistent order of shard indices".to_string(),
                    ));
                }

                unset_shard_index |= hit.score_doc.shard_index == -1;

                if hit_upto >= start {
                    hits.push(hit);
                }

                hit_upto += 1;

                if shard_ref.hit_index < shard_hits[shard_ref.shard_index].score_docs.len() {
                    // Not done with these TopFieldDocs yet.
                    queue.update_top_with(shard_ref);
                } else {
                    queue.pop();
                }
            }
        }

        let total_hits = TotalHits::new(total_hit_count, total_hits_relation)?;
        Ok(TopFieldDocs::new(total_hits, hits, sort_fields.to_vec()))
    }

    /// Reciprocal Rank Fusion.
    ///
    /// Equivalent to `TopDocs.rrf(int, int, TopDocs[])`. It combines different
    /// search results into a single ranked list by combining their ranks, which
    /// suits hits computed by different methods whose score distributions are
    /// hardly comparable.
    ///
    /// * `top_n` — the number of results to return;
    /// * `k` — a constant determining how much influence documents in the
    ///   individual rankings have on the final result; a higher value gives
    ///   lower-ranked documents more influence. `k` must be at least 1.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `top_n < 1`, when `k < 1`,
    /// or when some hits have their shard index set and others do not.
    pub fn rrf(top_n: usize, k: i32, hits: &[TopDocs]) -> Result<TopDocs> {
        if top_n < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "topN must be >= 1, got {top_n}"
            )));
        }
        if k < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "k must be >= 1, got {k}"
            )));
        }

        let mut shard_index_set: Option<bool> = None;
        for top_docs in hits {
            for score_doc in &top_docs.score_docs {
                let this_shard_index_set = score_doc.shard_index != -1;
                match shard_index_set {
                    None => shard_index_set = Some(this_shard_index_set),
                    Some(set) if set != this_shard_index_set => {
                        return Err(LuceneError::IllegalArgument(
                            "All hits must either have their ScoreDoc#shardIndex set, or unset (-1), not a mix of both."
                                .to_string(),
                        ));
                    }
                    Some(_) => {}
                }
            }
        }

        // Compute the rrf score as a double to reduce accuracy loss due to
        // floating-point arithmetic.
        let mut rrf_score: HashMap<ShardIndexAndDoc, f64> = HashMap::new();
        let mut total_hit_count: i64 = 0;
        for top_doc in hits {
            // A document is a hit globally if it is a hit for any of the top
            // docs, so the total hit count is the max total hit count.
            total_hit_count = total_hit_count.max(top_doc.total_hits.value());
            for (i, score_doc) in top_doc.score_docs.iter().enumerate() {
                let rank = i as i64 + 1;
                let denominator = i64::from(k).checked_add(rank).ok_or_else(|| {
                    LuceneError::IllegalArgument("k + rank overflows".to_string())
                })?;
                let rrf_score_contribution = 1f64 / denominator as f64;
                let key = ShardIndexAndDoc {
                    shard_index: score_doc.shard_index,
                    doc: score_doc.doc,
                };
                *rrf_score.entry(key).or_insert(0f64) += rrf_score_contribution;
            }
        }

        let mut rrf_score_rank: Vec<(ShardIndexAndDoc, f64)> = rrf_score.into_iter().collect();
        rrf_score_rank.sort_by(|(key_a, score_a), (key_b, score_b)| {
            // Sort by descending score, tie-breaking by doc ID then by shard
            // index, like TopDocs#merge.
            score_b
                .partial_cmp(score_a)
                .unwrap_or(Ordering::Equal)
                .then_with(|| key_a.doc.cmp(&key_b.doc))
                .then_with(|| key_a.shard_index.cmp(&key_b.shard_index))
        });

        let len = top_n.min(rrf_score_rank.len());
        let mut rrf_score_docs = Vec::with_capacity(len);
        for (key, score) in rrf_score_rank.into_iter().take(len) {
            rrf_score_docs.push(ScoreDoc::with_shard_index(
                key.doc,
                score as f32,
                key.shard_index,
            ));
        }

        let total_hits =
            TotalHits::new(total_hit_count, TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO)?;
        Ok(TopDocs::new(total_hits, rrf_score_docs))
    }
}
