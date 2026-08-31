//! Similarity-threshold kNN collection, ported from
//! `org.apache.lucene.search.VectorSimilarityCollector`.

#![deny(unsafe_code)]

use crate::search::abstract_knn_collector::{AbstractKnnCollector, AbstractKnnCollectorState};
use crate::search::abstract_vector_similarity_query::{
    default_similarity_strategy, DECAY_MAX_APPROXIMATION, DECAY_MAX_QUALITY,
};
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::score_doc::ScoreDoc;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// Performs a similarity-based graph search, finding all (approximate) vectors
/// above a similarity threshold.
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.VectorSimilarityCollector`; it is `pub` here
/// because the port has no package visibility.
///
/// The buffer for graph traversal is adaptive: it starts with a high value and
/// decays towards the scores of the nodes traversed but not collected, with a
/// provided factor. The decay factor lies in `[0, 1]`; higher values produce
/// better recall with more graph exploration.
///
/// Some behaviours deviate from [`KnnCollector`] and should only be used with
/// queries aware of the differences — [`ByteVectorSimilarityQuery`] and
/// [`FloatVectorSimilarityQuery`]:
///
/// * [`k`](KnnCollector::k) is *not* a good estimate of the number of collected
///   results;
/// * [`top_docs`](KnnCollector::top_docs) does *not* return docs sorted by
///   descending score;
/// * [`collect`](KnnCollector::collect) does *not* report whether the document
///   was collected.
///
/// [`ByteVectorSimilarityQuery`]: crate::search::ByteVectorSimilarityQuery
/// [`FloatVectorSimilarityQuery`]: crate::search::FloatVectorSimilarityQuery
#[derive(Debug, Clone)]
pub struct VectorSimilarityCollector {
    base: AbstractKnnCollectorState,
    result_similarity: f32,
    decay: f32,
    score_doc_list: Vec<ScoreDoc>,
    min_competitive_similarity: f32,
}

impl VectorSimilarityCollector {
    /// Performs a similarity-based graph search with the default strategy of
    /// the similarity-threshold queries.
    ///
    /// Equivalent to
    /// `new VectorSimilarityCollector(float, float, long)`.
    pub fn new(result_similarity: f32, decay: f32, visit_limit: i64) -> Self {
        Self::with_strategy(result_similarity, decay, visit_limit, None)
    }

    /// Performs a similarity-based graph search with a caller-supplied search
    /// strategy; `None` uses
    /// [`default_similarity_strategy`].
    ///
    /// Equivalent to
    /// `new VectorSimilarityCollector(float, float, long, KnnSearchStrategy)`.
    pub fn with_strategy(
        result_similarity: f32,
        decay: f32,
        visit_limit: i64,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Self {
        debug_assert!(
            !result_similarity.is_nan(),
            "resultSimilarity must have a valid value"
        );
        debug_assert!(!decay.is_nan(), "decay must have a valid value");
        debug_assert!(
            (DECAY_MAX_APPROXIMATION..=DECAY_MAX_QUALITY).contains(&decay),
            "decay must lie in range [DECAY_MAX_APPROXIMATION = 0, DECAY_MAX_QUALITY = 1]"
        );
        Self {
            base: AbstractKnnCollectorState::new(
                1,
                visit_limit,
                Some(search_strategy.unwrap_or_else(default_similarity_strategy)),
            ),
            result_similarity,
            decay,
            score_doc_list: Vec::new(),
            // Equivalent to `Math.nextUp(Float.NEGATIVE_INFINITY)`, which is
            // the most negative finite float.
            min_competitive_similarity: f32::MIN,
        }
    }
}

impl KnnCollector for VectorSimilarityCollector {
    fn early_terminated(&self) -> bool {
        self.base.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.base.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.base.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.base.visit_limit()
    }

    fn k(&self) -> i32 {
        self.base.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        // Returns whether min_competitive_similarity has been updated, not
        // whether the document was collected.
        if similarity >= self.result_similarity {
            self.score_doc_list.push(ScoreDoc::new(doc_id, similarity));
        } else if self.decay < DECAY_MAX_QUALITY {
            // Decay the buffer towards the score of the current node.
            self.min_competitive_similarity = (similarity as f64
                + (self.min_competitive_similarity as f64 - similarity as f64) * self.decay as f64)
                as f32;
            return true;
        }
        false
    }

    fn min_competitive_similarity(&self) -> f32 {
        self.min_competitive_similarity
    }

    fn top_docs(&mut self) -> TopDocs {
        // The results are not returned in sorted order, to avoid unnecessary
        // calculations: there is no top-k to maintain.
        let relation = if self.early_terminated() {
            TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
        } else {
            TotalHitsRelation::EQUAL_TO
        };
        TopDocs {
            total_hits: TotalHits::new(self.visited_count(), relation)
                .expect("INVARIANT: a visited count is never negative"),
            score_docs: self.score_doc_list.clone(),
        }
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.base.get_search_strategy()
    }
}

impl AbstractKnnCollector for VectorSimilarityCollector {
    fn num_collected(&self) -> i32 {
        self.score_doc_list.len() as i32
    }
}
