//! Constant-score weights, ported from
//! `org.apache.lucene.search.ConstantScoreWeight`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::query::Query;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;

/// Renders an `f32` the way `java.lang.Float.toString(float)` does.
///
/// `ConstantScoreWeight.explain` builds its description by string
/// concatenation, so the exact text depends on Java's float rendering: Rust
/// prints `2` where Java prints `2.0`, and `0.0000000001` where Java prints
/// `1.0E-10`. The same rendering exists in
/// `crate::search::similarities::java_fmt`, but that module is private to the
/// similarities package and cannot be reached from here without changing it.
///
/// It is `pub(crate)` because `BoostQuery.toString` and
/// `DisjunctionMaxQuery.toString` concatenate a float the same way, and their
/// modules cannot reach the similarities package either.
pub(crate) fn java_float_to_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let magnitude = f64::from(value.abs());
    let rendered = if magnitude != 0.0 && !(1e-3..1e7).contains(&magnitude) {
        format!("{value:E}")
    } else {
        format!("{value}")
    };
    // Java always keeps a fraction digit: `800` becomes `800.0`, and the
    // mantissa of `1E-10` becomes `1.0E-10`.
    match rendered.find('E') {
        Some(exponent) => {
            let (mantissa, exponent) = rendered.split_at(exponent);
            if mantissa.contains('.') {
                rendered
            } else {
                format!("{mantissa}.0{exponent}")
            }
        }
        None => {
            if rendered.contains('.') {
                rendered
            } else {
                format!("{rendered}.0")
            }
        }
    }
}

/// The part a [`ConstantScoreWeight`] is built from.
///
/// Equivalent to what a Java subclass of
/// `org.apache.lucene.search.ConstantScoreWeight` supplies: the abstract
/// `scorerSupplier` inherited from `Weight`, the `isCacheable` of
/// `SegmentCacheable`, and any override of `count` or `matches`.
pub trait ConstantScoreWeightImpl: Send + Sync + Debug {
    /// Returns the scorer supplier for the given leaf.
    ///
    /// Equivalent to `Weight.scorerSupplier(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while preparing the supplier.
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>>;

    /// Returns whether this weight can be cached against the given leaf.
    ///
    /// Equivalent to `SegmentCacheable.isCacheable(LeafReaderContext)`.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool;

    /// Counts the live documents matching the query in a leaf, or `-1` when the
    /// count cannot be computed in sub-linear time.
    ///
    /// Equivalent to `Weight.count(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while counting.
    fn count(&self, _context: &LeafReaderContext) -> Result<i32> {
        Ok(-1)
    }

    /// Returns the [`Matches`] for a specific document, or `None` when the
    /// document does not match.
    ///
    /// Equivalent to `Weight.matches(LeafReaderContext, int)`, which
    /// `ConstantScoreWeight` does not override but some of its subclasses do —
    /// `ConstantScoreQuery`'s weight delegates it to the wrapped weight. The
    /// default reproduces `Weight`'s own.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while positioning the scorer.
    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let scorer_supplier = self.scorer_supplier(context)?;
        let Some(mut scorer_supplier) = scorer_supplier else {
            return Ok(None);
        };
        let mut scorer = scorer_supplier.get(1)?;
        if scorer.two_phase_iterator().is_some() {
            let two_phase = scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was just observed to be present");
            if two_phase.approximation().advance(doc)? != doc {
                return Ok(None);
            }
            let two_phase = scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was just observed to be present");
            if !two_phase.matches()? {
                return Ok(None);
            }
        } else if scorer.iterator().advance(doc)? != doc {
            return Ok(None);
        }
        Ok(Some(MatchesUtils::match_with_no_terms()))
    }
}

/// A weight with a constant score equal to the boost of the wrapped query.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.ConstantScoreWeight`. It is typically useful when
/// building queries which do not produce meaningful scores and are mostly
/// useful for filtering. Supply the leaf-level behaviour as a
/// [`ConstantScoreWeightImpl`] and wrap it here.
#[derive(Debug)]
pub struct ConstantScoreWeight<I: ConstantScoreWeightImpl> {
    query: Arc<dyn Query>,
    score: f32,
    inner: I,
}

impl<I: ConstantScoreWeightImpl> ConstantScoreWeight<I> {
    /// Creates a constant-score weight for the given query.
    ///
    /// Equivalent to the protected
    /// `ConstantScoreWeight(Query, float)` constructor.
    pub fn new(query: Arc<dyn Query>, score: f32, inner: I) -> Self {
        Self {
            query,
            score,
            inner,
        }
    }

    /// Returns the score produced by this weight.
    ///
    /// Equivalent to the `protected final ConstantScoreWeight.score()`.
    pub fn score(&self) -> f32 {
        self.score
    }

    /// Returns the wrapped leaf-level behaviour.
    pub fn inner(&self) -> &I {
        &self.inner
    }
}

impl<I: ConstantScoreWeightImpl> SegmentCacheable for ConstantScoreWeight<I> {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl<I: ConstantScoreWeightImpl> Weight for ConstantScoreWeight<I> {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.inner.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner.count(context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let scorer = self.scorer(context)?;
        let exists = match scorer {
            None => false,
            Some(mut scorer) => {
                if scorer.two_phase_iterator().is_some() {
                    let two_phase = scorer
                        .two_phase_iterator()
                        .expect("INVARIANT: the two-phase view was just observed to be present");
                    let positioned = two_phase.approximation().advance(doc)? == doc;
                    if positioned {
                        let two_phase = scorer.two_phase_iterator().expect(
                            "INVARIANT: the two-phase view was just observed to be present",
                        );
                        two_phase.matches()?
                    } else {
                        false
                    }
                } else {
                    scorer.iterator().advance(doc)? == doc
                }
            }
        };

        let query_string = self.query.to_query_string("");
        if exists {
            let suffix = if self.score == 1.0 {
                String::new()
            } else {
                format!("^{}", java_float_to_string(self.score))
            };
            Ok(Explanation::matched(
                self.score,
                format!("{query_string}{suffix}"),
                Vec::new(),
            ))
        } else {
            Ok(Explanation::no_match(
                format!("{query_string} doesn't match id {doc}"),
                Vec::new(),
            ))
        }
    }
}
