//! Sort-based rescoring, ported from
//! `org.apache.lucene.search.SortRescorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::collection_terminated_exception::CollectionError;
use crate::search::collector::{Collector, CollectorManager};
use crate::search::index_searcher::IndexSearcher;
use crate::search::rescorer::Rescorer;
use crate::search::scorable::SimpleScorable;
use crate::search::score_doc::ScoreDoc;
use crate::search::similarities::Explanation;
use crate::search::sort::Sort;
use crate::search::top_docs::TopDocs;
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::top_field_collector_manager::TopFieldCollectorManager;
use crate::search::top_field_docs::TopFieldDocs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// A [`Rescorer`] that re-sorts according to a provided [`Sort`].
///
/// Equivalent to `org.apache.lucene.search.SortRescorer`.
#[derive(Debug, Clone)]
pub struct SortRescorer {
    sort: Sort,
}

impl SortRescorer {
    /// Creates a rescorer that re-sorts by `sort`.
    ///
    /// Equivalent to the sole `SortRescorer(Sort)` constructor.
    pub fn new(sort: Sort) -> Self {
        Self { sort }
    }

    /// Rescores the first-pass hits, keeping the per-hit sort values.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `rescore` returns a `TopDocs`
    /// whose `scoreDocs` are in fact `FieldDoc` instances, so a caller can cast
    /// them back and read the sort values — which `explain` does. This port
    /// types the hits, so the sort values cannot survive the
    /// [`Rescorer::rescore`] signature; this method returns them, and
    /// [`Rescorer::rescore`] is [`TopFieldDocs::to_top_docs`] of its result.
    /// The hits, their order and their scores are identical either way.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while collecting, and
    /// [`LuceneError::IllegalState`] when a first-pass hit falls outside every
    /// leaf of the searcher — which is Java's `IndexOutOfBoundsException`.
    pub fn rescore_sorted(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopFieldDocs> {
        // Copy the hits and sort by ascending doc ID.
        let mut hits = first_pass_top_docs.score_docs.clone();
        hits.sort_by_key(|hit| hit.doc);

        let leaves = searcher.get_leaf_contexts();
        let mut collector =
            TopFieldCollectorManager::new(self.sort.clone(), top_n, None, i32::MAX)?
                .new_collector()?;

        // Now merge-sort the doc IDs from the hits with the reader's leaves.
        //
        // **Divergence from Lucene 10.5.0.** Java walks the hits one at a time
        // and keeps one `LeafCollector` alive across the hits of a segment. A
        // leaf collector borrows its parent collector in this port, so the
        // borrow cannot be re-taken inside the loop; the hits are sorted by
        // ascending doc ID, so the hits of a segment are contiguous and are
        // instead collected in one pass per segment. The leaf collector is
        // still created once per segment, `set_scorer` is still called once
        // before its first hit, and the hits reach it in the same order.
        let mut hit_upto = 0usize;
        let mut reader_upto: i32 = -1;
        let mut end_doc = 0;
        let mut score = SimpleScorable::new();

        while hit_upto < hits.len() {
            let doc_id = hits[hit_upto].doc;
            while doc_id >= end_doc {
                reader_upto += 1;
                let context = leaves.get(reader_upto as usize).ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "hit docId={doc_id} is beyond the last leaf of the provided searcher"
                    ))
                })?;
                end_doc = context.doc_base() + context.leaf_reader().max_doc();
            }
            let context = &leaves[reader_upto as usize];
            let doc_base = context.doc_base();

            // Collect every hit that falls in this segment.
            {
                let mut leaf_collector = collector
                    .get_leaf_collector(context)
                    .map_err(collection_error_to_lucene)?;
                score.set_score(hits[hit_upto].score);
                leaf_collector
                    .set_scorer(&mut score)
                    .map_err(LuceneError::from)?;
                while hit_upto < hits.len() && hits[hit_upto].doc < end_doc {
                    score.set_score(hits[hit_upto].score);
                    leaf_collector
                        .collect(hits[hit_upto].doc - doc_base, &mut score)
                        .map_err(collection_error_to_lucene)?;
                    hit_upto += 1;
                }
            }
        }

        let mut rescored_docs = collector.top_docs()?;
        // Set the scores from the original score docs.
        debug_assert_eq!(hits.len(), rescored_docs.score_docs.len());
        let mut order: Vec<usize> = (0..rescored_docs.score_docs.len()).collect();
        order.sort_by_key(|&i| rescored_docs.score_docs[i].score_doc.doc);
        for (rank, &slot) in order.iter().enumerate() {
            rescored_docs.score_docs[slot].score_doc.score = hits[rank].score;
        }
        Ok(rescored_docs)
    }
}

impl Rescorer for SortRescorer {
    fn rescore(
        &self,
        searcher: &IndexSearcher,
        first_pass_top_docs: &TopDocs,
        top_n: i32,
    ) -> Result<TopDocs> {
        Ok(self
            .rescore_sorted(searcher, first_pass_top_docs, top_n)?
            .to_top_docs())
    }

    fn explain(
        &self,
        searcher: &IndexSearcher,
        first_pass_explanation: &Explanation,
        doc_id: i32,
    ) -> Result<Explanation> {
        let one_hit = TopDocs::new(
            TotalHits::new(1, TotalHitsRelation::EQUAL_TO)?,
            vec![ScoreDoc::new(
                doc_id,
                first_pass_explanation.value().float_value(),
            )],
        );
        let hits = self.rescore_sorted(searcher, &one_hit, 1)?;
        debug_assert_eq!(hits.total_hits.value(), 1);

        let mut subs = Vec::new();

        // Add the first pass.
        subs.push(Explanation::matched(
            first_pass_explanation.value(),
            "first pass score",
            vec![first_pass_explanation.clone()],
        ));

        let field_doc = &hits.score_docs[0];

        // Add the sort values.
        for (i, sort_field) in self.sort.fields().iter().enumerate() {
            subs.push(Explanation::matched(
                0.0f32,
                format!(
                    "sort field {} value={:?}",
                    sort_field,
                    field_doc.fields.as_ref().and_then(|fields| fields.get(i))
                ),
                Vec::new(),
            ));
        }

        Ok(Explanation::matched(
            0.0f32,
            format!("sort field values for sort={}", self.sort),
            subs,
        ))
    }
}

/// Maps a collection outcome onto the error type
/// [`Rescorer::rescore`](crate::search::Rescorer::rescore) reports.
///
/// Java's `SortRescorer` lets `CollectionTerminatedException` and
/// `TimeExceededException` propagate out of `rescore` as unchecked exceptions;
/// this port turns them into errors with the same message.
fn collection_error_to_lucene(error: CollectionError) -> LuceneError {
    match error {
        CollectionError::Lucene(error) => error,
        other => LuceneError::IllegalState(other.to_string()),
    }
}
