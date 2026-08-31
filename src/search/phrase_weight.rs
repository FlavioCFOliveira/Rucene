//! Expert: the weight of a phrase match, ported from
//! `org.apache.lucene.search.PhraseWeight`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::index_searcher::IndexSearcher;
use crate::search::matches::{owned_leaf_context, Matches, MatchesIterator, MatchesUtils};
use crate::search::phrase_matcher::PhraseMatcher;
use crate::search::phrase_scorer::PhraseScorer;
use crate::search::query::Query;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sim_scorer_source::{similarity_simple_name, OneSimScorer, SharedSimScorer};
use crate::search::similarities::{Explanation, Similarity};
use crate::search::weight::{DefaultScorerSupplier, Weight};

/// The two abstract members of a [`PhraseWeight`].
///
/// Equivalent to `PhraseWeight.getStats(IndexSearcher)` and
/// `PhraseWeight.getPhraseMatcher(LeafReaderContext, SimScorer, boolean)`,
/// which Java leaves for a subclass to define. A Rust "subclass" supplies them
/// through this trait; the implementation captures whatever Java's anonymous
/// subclass reads from its enclosing scope — the boost and the query's terms.
pub trait PhraseWeightImpl: Send + Sync + Debug {
    /// Returns the similarity scorer for the phrase, or `None` when the phrase
    /// has no term at all and the similarity will not be used.
    ///
    /// Equivalent to the `protected abstract
    /// PhraseWeight.getStats(IndexSearcher)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics, and the
    /// [`LuceneError::IllegalState`](crate::error::LuceneError::IllegalState)
    /// a phrase of fewer than two terms reports.
    fn get_stats(&self, searcher: &IndexSearcher) -> Result<Option<SharedSimScorer>>;

    /// Returns the matcher for a leaf, or `None` when the leaf cannot match.
    ///
    /// Equivalent to the `protected abstract
    /// PhraseWeight.getPhraseMatcher(LeafReaderContext, SimScorer, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the postings.
    fn get_phrase_matcher(
        &self,
        context: &LeafReaderContext,
        scorer: &SharedSimScorer,
        expose_offsets: bool,
    ) -> Result<Option<Box<dyn PhraseMatcher>>>;
}

/// Expert: the [`Weight`] of a phrase match.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.PhraseWeight`. Supply the leaf-level behaviour as
/// a [`PhraseWeightImpl`] and wrap it here.
pub struct PhraseWeight {
    query: Arc<dyn Query>,
    score_mode: ScoreMode,
    stats: SharedSimScorer,
    similarity: Arc<dyn Similarity>,
    field: String,
    inner: Arc<dyn PhraseWeightImpl>,
}

impl Debug for PhraseWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhraseWeight")
            .field("query", &self.query)
            .field("field", &self.field)
            .field("score_mode", &self.score_mode)
            .finish_non_exhaustive()
    }
}

impl PhraseWeight {
    /// Expert: creates a phrase weight.
    ///
    /// Equivalent to
    /// `PhraseWeight(Query, String, IndexSearcher, ScoreMode)`.
    ///
    /// # Errors
    ///
    /// Propagates the error [`PhraseWeightImpl::get_stats`] raises.
    pub fn new(
        query: Arc<dyn Query>,
        field: impl Into<String>,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        inner: Arc<dyn PhraseWeightImpl>,
    ) -> Result<Self> {
        let similarity = Arc::clone(searcher.get_similarity());
        // `None` means no terms, or that scores are not needed.
        let stats: SharedSimScorer = match inner.get_stats(searcher)? {
            Some(stats) => stats,
            None => Arc::new(OneSimScorer),
        };
        Ok(Self {
            query,
            score_mode,
            stats,
            similarity,
            field: field.into(),
            inner,
        })
    }

    /// Returns the score mode this weight was built for.
    ///
    /// Equivalent to reading the `final ScoreMode scoreMode` field.
    pub fn score_mode(&self) -> ScoreMode {
        self.score_mode
    }

    /// Returns the similarity scorer for the phrase.
    ///
    /// Equivalent to reading the `final Similarity.SimScorer stats` field.
    pub fn stats(&self) -> &SharedSimScorer {
        &self.stats
    }

    /// Returns the field the phrase is on.
    ///
    /// Equivalent to reading the `final String field` field.
    pub fn field(&self) -> &str {
        &self.field
    }
}

impl SegmentCacheable for PhraseWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for PhraseWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(matcher) = self.inner.get_phrase_matcher(context, &self.stats, false)? else {
            return Ok(None);
        };
        let norms = if self.score_mode.needs_scores() {
            context.leaf_reader().get_norm_values(&self.field)?
        } else {
            None
        };
        let scorer = PhraseScorer::new(matcher, self.score_mode, Arc::clone(&self.stats), norms);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let matcher = self.inner.get_phrase_matcher(context, &self.stats, false)?;
        let Some(mut matcher) = matcher else {
            return Ok(Explanation::no_match("no matching terms", Vec::new()));
        };
        if matcher.approximation().advance(doc)? != doc {
            return Ok(Explanation::no_match("no matching terms", Vec::new()));
        }
        matcher.reset_positions()?;
        if !matcher.next_match()? {
            return Ok(Explanation::no_match("no matching phrase", Vec::new()));
        }
        let mut freq = matcher.sloppy_weight();
        while matcher.next_match()? {
            freq += matcher.sloppy_weight();
        }
        let freq_explanation = Explanation::matched(freq, format!("phraseFreq={freq}"), Vec::new());
        let mut norms = if self.score_mode.needs_scores() {
            context.leaf_reader().get_norm_values(&self.field)?
        } else {
            None
        };
        let mut norm = 1i64;
        if let Some(norms) = norms.as_mut() {
            if norms.advance_exact(doc)? {
                norm = norms.long_value()?;
            }
        }
        let score_explanation = self.stats.explain(&freq_explanation, norm);
        Ok(Explanation::matched(
            score_explanation.value().float_value(),
            format!(
                "weight({} in {doc}) [{}], result of:",
                self.query.to_query_string(""),
                similarity_simple_name(&*self.similarity)
            ),
            vec![score_explanation],
        ))
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let inner = Arc::clone(&self.inner);
        let stats = Arc::clone(&self.stats);
        let query = Arc::clone(&self.query);
        let context = owned_leaf_context(context);
        MatchesUtils::for_field(
            self.field.clone(),
            Arc::new(move || {
                let matcher = inner.get_phrase_matcher(&context, &stats, true)?;
                let Some(mut matcher) = matcher else {
                    return Ok(None);
                };
                if matcher.approximation().advance(doc)? != doc {
                    return Ok(None);
                }
                matcher.reset_positions()?;
                if !matcher.next_match()? {
                    return Ok(None);
                }
                Ok(Some(Box::new(PhraseMatchesIterator {
                    matcher,
                    query: Arc::clone(&query),
                    started: false,
                }) as Box<dyn MatchesIterator>))
            }),
        )
    }
}

/// The [`MatchesIterator`] a [`PhraseWeight`] hands out.
///
/// Equivalent to the anonymous `MatchesIterator` of
/// `PhraseWeight.matches(LeafReaderContext, int)`.
struct PhraseMatchesIterator {
    matcher: Box<dyn PhraseMatcher>,
    query: Arc<dyn Query>,
    started: bool,
}

impl MatchesIterator for PhraseMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        if !self.started {
            self.started = true;
            return Ok(true);
        }
        self.matcher.next_match()
    }

    fn start_position(&self) -> i32 {
        self.matcher.start_position()
    }

    fn end_position(&self) -> i32 {
        self.matcher.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.matcher.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.matcher.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        // Phrases are treated as leaves.
        Ok(None)
    }

    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }
}
