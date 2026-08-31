//! Precomputed hits, ported from `org.apache.lucene.search.DocAndScoreQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::top_docs::TopDocs;
use crate::search::weight::{DefaultScorerSupplier, Weight};

/// A query that wraps precomputed documents and scores.
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.DocAndScoreQuery`, which
/// [`AbstractKnnVectorQuery`](crate::search::AbstractKnnVectorQuery) rewrites
/// to. It is `pub` here because the port has no package visibility.
#[derive(Debug, Clone)]
pub struct DocAndScoreQuery {
    /// The global doc IDs of the matching documents, in ascending order.
    docs: Arc<Vec<i32>>,
    /// The scores of the matching documents.
    scores: Arc<Vec<f32>>,
    max_score: f32,
    /// The indexes in `docs` and `scores` of the first matching document of
    /// each segment. A segment with no matching document is assigned the index
    /// of the next segment that has one; the final entry is `docs.len()`.
    segment_starts: Arc<Vec<i32>>,
    /// The number of graph nodes that were visited and scored.
    visited: i64,
    /// Identifies the reader context this query was built against.
    ///
    /// Equivalent to the `Object contextIdentity` field, which Java compares by
    /// reference; this port compares the
    /// [`IndexReaderContext::id`](crate::index::IndexReaderContext::id) that
    /// stands for the same identity.
    context_identity: usize,
}

impl DocAndScoreQuery {
    /// Creates a query over precomputed hits.
    ///
    /// Equivalent to the package-private
    /// `DocAndScoreQuery(int[], float[], float, int[], long, Object)`.
    pub fn new(
        docs: Vec<i32>,
        scores: Vec<f32>,
        max_score: f32,
        segment_starts: Vec<i32>,
        visited: i64,
        context_identity: usize,
    ) -> Self {
        Self {
            docs: Arc::new(docs),
            scores: Arc::new(scores),
            max_score,
            segment_starts: Arc::new(segment_starts),
            visited,
            context_identity,
        }
    }

    /// Returns the number of graph nodes that were visited and scored.
    ///
    /// Equivalent to `DocAndScoreQuery.visited()`.
    pub fn visited(&self) -> i64 {
        self.visited
    }

    /// Builds the query that answers a merged kNN result.
    ///
    /// Equivalent to the static
    /// `DocAndScoreQuery.createDocAndScoreQuery(IndexReader, TopDocs)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java takes the `IndexReader` and
    /// reads `reader.getContext()`, which is a field the reader builds once.
    /// [`IndexReader::get_context`](crate::index::IndexReader::get_context)
    /// builds a *fresh* context — and therefore a fresh identity — on every
    /// call in this port, so the identity has to come from the one context the
    /// searcher caches. `searcher.getIndexReader().getContext()` and
    /// `searcher.getTopReaderContext()` are the same object in Java, so the
    /// leaves and the identity are the same values either way.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `top_k` holds no hits,
    /// which Java asserts rather than checks.
    pub fn create_doc_and_score_query(
        searcher: &IndexSearcher,
        mut top_k: TopDocs,
    ) -> Result<Arc<dyn Query>> {
        let len = top_k.score_docs.len();
        if len == 0 {
            return Err(LuceneError::IllegalArgument(
                "createDocAndScoreQuery requires at least one hit".to_string(),
            ));
        }
        let max_score = top_k.score_docs[0].score;

        top_k.score_docs.sort_by_key(|hit| hit.doc);
        let mut docs = Vec::with_capacity(len);
        let mut scores = Vec::with_capacity(len);
        for hit in &top_k.score_docs {
            docs.push(hit.doc);
            scores.push(hit.score);
        }
        let segment_starts = find_segment_starts(searcher.get_leaf_contexts(), &docs);
        Ok(Arc::new(DocAndScoreQuery::new(
            docs,
            scores,
            max_score,
            segment_starts,
            top_k.total_hits.value(),
            searcher.get_top_reader_context().id(),
        )))
    }
}

/// Locates the first matching document of every leaf.
///
/// Equivalent to the static
/// `DocAndScoreQuery.findSegmentStarts(List<LeafReaderContext>, int[])`.
pub fn find_segment_starts(leaves: &[Arc<LeafReaderContext>], docs: &[i32]) -> Vec<i32> {
    let mut starts = vec![0i32; leaves.len() + 1];
    let last = starts.len() - 1;
    starts[last] = docs.len() as i32;
    if starts.len() == 2 {
        return starts;
    }
    let mut result_index = 0usize;
    for (i, start) in starts.iter_mut().enumerate().take(last).skip(1) {
        let upper = leaves[i].doc_base();
        result_index = match docs[result_index..].binary_search(&upper) {
            // Java's `Arrays.binarySearch` may return any matching index; the
            // doc IDs are strictly increasing, so there is at most one.
            Ok(found) => result_index + found,
            Err(insertion) => result_index + insertion,
        };
        *start = result_index as i32;
    }
    starts
}

impl Query for DocAndScoreQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "DocAndScoreQuery[{},...][{},...],{}",
            self.docs[0], self.scores[0], self.max_score
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        if searcher.get_top_reader_context().id() != self.context_identity {
            return Err(LuceneError::IllegalState(
                "This DocAndScore query was created by a different reader".to_string(),
            ));
        }
        Ok(Arc::new(DocAndScoreWeight {
            query: Arc::new(self.clone()),
            docs: Arc::clone(&self.docs),
            scores: Arc::clone(&self.scores),
            segment_starts: Arc::clone(&self.segment_starts),
            max_score: self.max_score,
            boost,
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<DocAndScoreQuery>() {
            Some(other) => {
                self.context_identity == other.context_identity
                    && self.docs == other.docs
                    && self.scores == other.scores
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.context_identity.hash(&mut hasher);
        self.docs.hash(&mut hasher);
        for score in self.scores.iter() {
            score.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// The weight [`DocAndScoreQuery`] creates.
///
/// Equivalent to the anonymous `Weight` of `DocAndScoreQuery.createWeight`.
#[derive(Debug)]
struct DocAndScoreWeight {
    query: Arc<dyn Query>,
    docs: Arc<Vec<i32>>,
    scores: Arc<Vec<f32>>,
    segment_starts: Arc<Vec<i32>>,
    max_score: f32,
    boost: f32,
}

impl SegmentCacheable for DocAndScoreWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for DocAndScoreWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        match self.docs.binary_search(&(doc + context.doc_base())) {
            Err(_) => Ok(Explanation::no_match(
                format!("not in top {} docs", self.docs.len()),
                Vec::new(),
            )),
            Ok(found) => Ok(Explanation::matched(
                self.scores[found] * self.boost,
                format!("within top {} docs", self.docs.len()),
                Vec::new(),
            )),
        }
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let ord = context.ord() as usize;
        Ok(self.segment_starts[ord + 1] - self.segment_starts[ord])
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let ord = context.ord() as usize;
        if self.segment_starts[ord] == self.segment_starts[ord + 1] {
            return Ok(None);
        }
        let scorer = DocAndScoreScorer {
            scores: Arc::clone(&self.scores),
            max_score: self.max_score,
            boost: self.boost,
            iterator: DocAndScoreIterator {
                docs: Arc::clone(&self.docs),
                lower: self.segment_starts[ord],
                upper: self.segment_starts[ord + 1],
                up_to: -1,
                doc_base: context.doc_base(),
            },
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}

/// The iteration state of a [`DocAndScoreScorer`].
///
/// Equivalent to the anonymous `DocIdSetIterator` of the scorer Java builds,
/// which reads the enclosing scorer's `upTo`, `lower` and `upper` fields; this
/// port makes those fields the iterator's own, so that
/// [`Scorer::iterator`] can hand out a borrow of them.
#[derive(Debug)]
struct DocAndScoreIterator {
    docs: Arc<Vec<i32>>,
    lower: i32,
    upper: i32,
    up_to: i32,
    doc_base: i32,
}

impl DocAndScoreIterator {
    /// Equivalent to the private `docIdNoShadow()` of the anonymous scorer.
    fn doc_id_no_shadow(&self) -> i32 {
        if self.up_to == -1 {
            return -1;
        }
        if self.up_to >= self.upper {
            return NO_MORE_DOCS;
        }
        self.docs[self.up_to as usize] - self.doc_base
    }
}

impl DocIdSetIterator for DocAndScoreIterator {
    fn doc_id(&self) -> i32 {
        self.doc_id_no_shadow()
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.up_to == -1 {
            self.up_to = self.lower;
        } else {
            self.up_to += 1;
        }
        Ok(self.doc_id_no_shadow())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        (self.upper - self.lower) as i64
    }
}

/// The scorer [`DocAndScoreWeight`] builds.
///
/// Equivalent to the anonymous `Scorer` of
/// `DocAndScoreQuery.createWeight(..).scorerSupplier(..)`.
#[derive(Debug)]
struct DocAndScoreScorer {
    scores: Arc<Vec<f32>>,
    max_score: f32,
    boost: f32,
    iterator: DocAndScoreIterator,
}

impl Scorable for DocAndScoreScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.scores[self.iterator.up_to as usize] * self.boost)
    }
}

impl Scorer for DocAndScoreScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iterator.doc_id_no_shadow()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.max_score * self.boost)
    }
}
