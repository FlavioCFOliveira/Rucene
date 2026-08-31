//! Port of `org.apache.lucene.util.hnsw.OrdinalTranslatedKnnCollector`.

use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::{TotalHits, TotalHitsRelation};

use super::int_to_int_function::IntToIntFunction;

/// Wraps a provided [`KnnCollector`], translating the provided vector ordinal to a
/// document id.
///
/// Equivalent to `org.apache.lucene.util.hnsw.OrdinalTranslatedKnnCollector`, which
/// extends `KnnCollector.Decorator`; Rust has no implementation inheritance, so the
/// decorator's delegation is written out.
pub struct OrdinalTranslatedKnnCollector {
    collector: Box<dyn KnnCollector>,
    vector_ordinal_to_doc_id: Box<dyn IntToIntFunction + Send>,
}

impl OrdinalTranslatedKnnCollector {
    /// Wraps `collector`, mapping every collected ordinal through
    /// `vector_ordinal_to_doc_id`.
    pub fn new(
        collector: Box<dyn KnnCollector>,
        vector_ordinal_to_doc_id: Box<dyn IntToIntFunction + Send>,
    ) -> Self {
        Self {
            collector,
            vector_ordinal_to_doc_id,
        }
    }

    /// Returns the wrapped collector.
    pub fn inner(&self) -> &dyn KnnCollector {
        self.collector.as_ref()
    }
}

impl KnnCollector for OrdinalTranslatedKnnCollector {
    fn early_terminated(&self) -> bool {
        self.collector.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.collector.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.collector.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.collector.visit_limit()
    }

    fn k(&self) -> i32 {
        self.collector.k()
    }

    fn collect(&mut self, vector_id: i32, similarity: f32) -> bool {
        self.collector
            .collect(self.vector_ordinal_to_doc_id.apply(vector_id), similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        self.collector.min_competitive_similarity()
    }

    fn top_docs(&mut self) -> TopDocs {
        let td = self.collector.top_docs();
        let relation = if self.early_terminated() {
            TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
        } else {
            TotalHitsRelation::EQUAL_TO
        };
        TopDocs {
            total_hits: TotalHits::new(self.visited_count(), relation)
                .expect("INVARIANT: a visited count is never negative"),
            score_docs: td.score_docs,
        }
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.collector.get_search_strategy()
    }
}
