//! Query-based rescoring, ported from
//! `org.apache.lucene.search.QueryRescorer`.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::sync::Arc;

use crate::error::Result;
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::rescorer::Rescorer;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::similarities::Explanation;
use crate::search::top_docs::TopDocs;

/// The way a [`QueryRescorer`] combines the first-pass and second-pass scores.
///
/// Equivalent to the abstract
/// `QueryRescorer.combine(float, boolean, float)`. Rust has no implementation
/// inheritance, so the one abstract method of the class becomes this trait and
/// [`QueryRescorer`] holds an implementation of it.
pub trait QueryRescorerImpl: Send + Sync {
    /// Combines the first-pass and second-pass scores. When
    /// `second_pass_matches` is false, the second-pass query did not match the
    /// hit and `second_pass_score` must be ignored.
    ///
    /// Equivalent to `QueryRescorer.combine(float, boolean, float)`.
    fn combine(
        &self,
        first_pass_score: f32,
        second_pass_matches: bool,
        second_pass_score: f32,
    ) -> f32;

    /// Names this combination, for the explanation text.
    ///
    /// Equivalent to the `getClass()` Java interpolates into
    /// "combined first and second pass score using ...".
    fn combiner_name(&self) -> String {
        "QueryRescorer".to_string()
    }
}

/// A [`Rescorer`] that uses a provided [`Query`] to assign scores to the
/// first-pass hits.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.QueryRescorer`; the abstract `combine` lives in
/// [`QueryRescorerImpl`].
pub struct QueryRescorer<I: QueryRescorerImpl> {
    query: Arc<dyn Query>,
    inner: I,
}

impl<I: QueryRescorerImpl> std::fmt::Debug for QueryRescorer<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryRescorer")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl<I: QueryRescorerImpl> QueryRescorer<I> {
    /// Creates a rescorer over the second-pass query that assigns scores to the
    /// first-pass hits.
    ///
    /// Equivalent to the sole `QueryRescorer(Query)` constructor.
    pub fn new(query: Arc<dyn Query>, inner: I) -> Self {
        Self { query, inner }
    }

    /// Returns the second-pass query.
    pub fn query(&self) -> &Arc<dyn Query> {
        &self.query
    }
}

/// Sorts by score descending, then by doc ID ascending.
///
/// Equivalent to the `sortDocComparator` lambda of `QueryRescorer.rescore`.
/// Java compares the scores with `>` and `<` rather than `Float.compare`, so
/// `NaN` falls through to the doc ID comparison; this reproduces that exactly.
fn sort_doc_comparator(a: &ScoreDoc, b: &ScoreDoc) -> Ordering {
    if a.score > b.score {
        Ordering::Less
    } else if a.score < b.score {
        Ordering::Greater
    } else {
        a.doc.cmp(&b.doc)
    }
}

/// Keeps the `top_n` best hits, then sorts them.
///
/// Equivalent to the `ArrayUtil.select` / `copyOfSubArray` / `Arrays.sort`
/// sequence both [`QueryRescorer`] and
/// [`DoubleValuesSourceRescorer`](crate::search::DoubleValuesSourceRescorer)
/// end with.
pub(crate) fn select_and_sort(
    mut hits: Vec<ScoreDoc>,
    top_n: i32,
    comparator: fn(&ScoreDoc, &ScoreDoc) -> Ordering,
) -> Vec<ScoreDoc> {
    if top_n >= 0 && (top_n as usize) < hits.len() {
        let top_n = top_n as usize;
        hits.select_nth_unstable_by(top_n, comparator);
        hits.truncate(top_n);
    }
    hits.sort_by(comparator);
    hits
}

impl<I: QueryRescorerImpl> Rescorer for QueryRescorer<I> {
    fn rescore(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopDocs> {
        let mut hits = first_pass_top_docs.score_docs.clone();
        hits.sort_by_key(|hit| hit.doc);

        let leaves = searcher.get_leaf_contexts();

        let rewritten = searcher.rewrite(Arc::clone(&self.query))?;
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;

        // Now merge-sort the doc IDs from the hits with the reader's leaves.
        let mut hit_upto = 0usize;
        let mut reader_upto: i32 = -1;
        let mut end_doc = 0;
        let mut doc_base = 0;
        let mut scorer = None;

        while hit_upto < hits.len() {
            let doc_id = hits[hit_upto].doc;
            let mut reader_context = None;
            while doc_id >= end_doc {
                reader_upto += 1;
                let context = &leaves[reader_upto as usize];
                end_doc = context.doc_base() + context.leaf_reader().max_doc();
                reader_context = Some(context);
            }

            if let Some(context) = reader_context {
                // We advanced to another segment.
                doc_base = context.doc_base();
                scorer = weight.scorer(context)?;
            }

            let combined = match scorer.as_mut() {
                Some(scorer) => {
                    let target_doc = doc_id - doc_base;
                    let mut actual_doc = scorer.doc_id();
                    if actual_doc < target_doc {
                        actual_doc = scorer.iterator().advance(target_doc)?;
                    }
                    if actual_doc == target_doc {
                        // The query did match this doc.
                        let second_pass = scorer.score()?;
                        self.inner.combine(hits[hit_upto].score, true, second_pass)
                    } else {
                        // The query did not match this doc.
                        debug_assert!(actual_doc > target_doc);
                        self.inner.combine(hits[hit_upto].score, false, 0.0)
                    }
                }
                // The query did not match this doc.
                None => self.inner.combine(hits[hit_upto].score, false, 0.0),
            };
            hits[hit_upto].score = combined;

            hit_upto += 1;
        }

        let hits = select_and_sort(hits, top_n, sort_doc_comparator);
        Ok(TopDocs::new(first_pass_top_docs.total_hits, hits))
    }

    fn explain(
        &self,
        searcher: &IndexSearcher,
        first_pass_explanation: &Explanation,
        doc_id: i32,
    ) -> Result<Explanation> {
        let second_pass_explanation = searcher.explain(Arc::clone(&self.query), doc_id)?;

        let second_pass_score = if second_pass_explanation.is_match() {
            Some(second_pass_explanation.value())
        } else {
            None
        };

        let first_pass_value = first_pass_explanation.value().float_value();
        let score = match second_pass_score {
            None => self.inner.combine(first_pass_value, false, 0.0),
            Some(value) => self
                .inner
                .combine(first_pass_value, true, value.float_value()),
        };

        let first = Explanation::matched(
            first_pass_explanation.value(),
            "first pass score",
            vec![first_pass_explanation.clone()],
        );

        let second = match second_pass_score {
            None => Explanation::no_match("no second pass score", Vec::new()),
            Some(value) => Explanation::matched(
                value,
                "second pass score",
                vec![second_pass_explanation.clone()],
            ),
        };

        Ok(Explanation::matched(
            score,
            format!(
                "combined first and second pass score using {}",
                self.inner.combiner_name()
            ),
            vec![first, second],
        ))
    }
}

/// The linear combination `firstPassScore + weight * secondPassScore`.
///
/// Equivalent to the anonymous `QueryRescorer` of the static
/// `QueryRescorer.rescore(IndexSearcher, TopDocs, Query, double, int)`.
#[derive(Debug, Clone, Copy)]
pub struct LinearCombination {
    weight: f64,
}

impl LinearCombination {
    /// Creates the combination with the given second-pass weight.
    pub fn new(weight: f64) -> Self {
        Self { weight }
    }
}

impl QueryRescorerImpl for LinearCombination {
    fn combine(
        &self,
        first_pass_score: f32,
        second_pass_matches: bool,
        second_pass_score: f32,
    ) -> f32 {
        let mut score = first_pass_score;
        if second_pass_matches {
            // Java computes `score += weight * secondPassScore` with `weight` a
            // double, so the sum is formed in double precision and narrowed
            // once, at the assignment.
            score = (score as f64 + self.weight * second_pass_score as f64) as f32;
        }
        score
    }

    fn combiner_name(&self) -> String {
        "QueryRescorer$LinearCombination".to_string()
    }
}

/// Rescores using a simple linear combination of
/// `firstPassScore + weight * secondPassScore`.
///
/// Equivalent to the static
/// `QueryRescorer.rescore(IndexSearcher, TopDocs, Query, double, int)`.
///
/// # Errors
///
/// Propagates any I/O error raised while rescoring.
pub fn rescore(
    searcher: &IndexSearcher,
    top_docs: &TopDocs,
    query: Arc<dyn Query>,
    weight: f64,
    top_n: i32,
) -> Result<TopDocs> {
    QueryRescorer::new(query, LinearCombination::new(weight)).rescore(searcher, top_docs, top_n)
}
