//! Value-source rescoring, ported from
//! `org.apache.lucene.search.DoubleValuesSourceRescorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::index_searcher::IndexSearcher;
use crate::search::query_rescorer::select_and_sort;
use crate::search::rescorer::Rescorer;
use crate::search::score_doc::ScoreDoc;
use crate::search::similarities::Explanation;
use crate::search::top_docs::TopDocs;

/// The way a [`DoubleValuesSourceRescorer`] combines the first-pass score with
/// the value read from its source.
///
/// Equivalent to the abstract
/// `DoubleValuesSourceRescorer.combine(float, boolean, double)`. Rust has no
/// implementation inheritance, so the one abstract method of the class becomes
/// this trait.
pub trait DoubleValuesSourceRescorerImpl: Send + Sync {
    /// Combines the first-pass score with the value from the source.
    ///
    /// Equivalent to
    /// `DoubleValuesSourceRescorer.combine(float, boolean, double)`.
    ///
    /// * `first_pass_score` — the score from the first-pass hits;
    /// * `value_present` — whether the source has a value for the hit;
    /// * `source_value` — the value returned by the source.
    fn combine(&self, first_pass_score: f32, value_present: bool, source_value: f64) -> f32;

    /// Names this combination, for the explanation text.
    ///
    /// Equivalent to the `getClass()` Java interpolates into
    /// "combined score from firstPass and DoubleValuesSource=... using ...".
    fn combiner_name(&self) -> String {
        "DoubleValuesSourceRescorer".to_string()
    }
}

/// A [`Rescorer`] that uses a provided [`DoubleValuesSource`] to rescore the
/// first-pass hits.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.DoubleValuesSourceRescorer`; the abstract
/// `combine` lives in [`DoubleValuesSourceRescorerImpl`].
#[derive(Debug)]
pub struct DoubleValuesSourceRescorer<I: DoubleValuesSourceRescorerImpl> {
    values_source: Arc<dyn DoubleValuesSource>,
    inner: I,
}

impl<I: DoubleValuesSourceRescorerImpl> DoubleValuesSourceRescorer<I> {
    /// Creates a rescorer over the given source.
    ///
    /// Equivalent to the sole
    /// `DoubleValuesSourceRescorer(DoubleValuesSource)` constructor.
    pub fn new(values_source: Arc<dyn DoubleValuesSource>, inner: I) -> Self {
        Self {
            values_source,
            inner,
        }
    }

    /// Returns the source this rescorer reads.
    pub fn values_source(&self) -> &Arc<dyn DoubleValuesSource> {
        &self.values_source
    }

    /// Returns the combination this rescorer applies.
    pub fn combiner(&self) -> &I {
        &self.inner
    }
}

impl<I: DoubleValuesSourceRescorerImpl> Rescorer for DoubleValuesSourceRescorer<I> {
    fn rescore(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopDocs> {
        let source = Arc::clone(&self.values_source).rewrite(searcher)?;
        // This still alters the scores, so the hits are cloned to retain the
        // ordering of first_pass_top_docs.
        let mut hits = first_pass_top_docs.score_docs.clone();
        hits.sort_by_key(|hit| hit.doc);

        let leaves = searcher.get_leaf_contexts();
        if leaves.is_empty() {
            return Err(LuceneError::IllegalState(
                "the provided searcher has no leaf".to_string(),
            ));
        }
        let mut curr_leaf = 0usize;
        let mut leaf_end = leaves[curr_leaf].doc_base() + leaves[curr_leaf].leaf_reader().max_doc();

        // Find the leaf holding each hit.
        for hit in hits.iter_mut() {
            while hit.doc >= leaf_end {
                if curr_leaf == leaves.len() - 1 {
                    return Err(LuceneError::IllegalState(format!(
                        "hit docId={}greater than last searcher leaf maxDoc={leaf_end} Ensure \
                         firstPassTopDocs were produced by the searcher provided to rescore.",
                        hit.doc
                    )));
                }
                curr_leaf += 1;
                leaf_end = leaves[curr_leaf].doc_base() + leaves[curr_leaf].leaf_reader().max_doc();
            }

            let ctx = &leaves[curr_leaf];
            let target_doc = hit.doc - ctx.doc_base();
            let mut values = Arc::clone(&source).get_values(ctx, None)?;
            let score_present = values.advance_exact(target_doc)?;
            let second_pass_score = if score_present {
                values.double_value()?
            } else {
                0.0
            };
            hit.score = self
                .inner
                .combine(hit.score, score_present, second_pass_score);
        }

        let hits = select_and_sort(hits, top_n, ScoreDoc::compare);
        Ok(TopDocs::new(first_pass_top_docs.total_hits, hits))
    }

    fn explain(
        &self,
        searcher: &IndexSearcher,
        first_pass_explanation: &Explanation,
        doc_id: i32,
    ) -> Result<Explanation> {
        let first = Explanation::matched(
            first_pass_explanation.value(),
            "first pass score",
            vec![first_pass_explanation.clone()],
        );

        let leaf_with_doc = searcher.get_leaf_contexts().iter().find(|ctx| {
            doc_id >= ctx.doc_base() && doc_id < ctx.doc_base() + ctx.leaf_reader().max_doc()
        });
        let Some(leaf_with_doc) = leaf_with_doc else {
            return Err(LuceneError::IllegalArgument(format!(
                "docId={doc_id} not found in any leaf in provided searcher"
            )));
        };

        let source = Arc::clone(&self.values_source).rewrite(searcher)?;
        let double_values_match = Arc::clone(&source).explain(
            leaf_with_doc,
            doc_id - leaf_with_doc.doc_base(),
            &Explanation::no_match(
                "DoubleValuesSource was not initialized with query scores",
                Vec::new(),
            ),
        )?;
        let second = if double_values_match.is_match() {
            Explanation::matched(
                double_values_match.value(),
                "value from DoubleValuesSource",
                vec![double_values_match.clone()],
            )
        } else {
            Explanation::no_match("no value in DoubleValuesSource", Vec::new())
        };

        let score = self.inner.combine(
            first.value().float_value(),
            double_values_match.is_match(),
            double_values_match.value().double_value(),
        );
        let desc = format!(
            "combined score from firstPass and DoubleValuesSource={} using {}",
            source.to_source_string(),
            self.inner.combiner_name()
        );
        Ok(Explanation::matched(score, desc, vec![first, second]))
    }
}
