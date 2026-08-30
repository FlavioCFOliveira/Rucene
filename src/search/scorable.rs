//! Score access, ported from `org.apache.lucene.search.Scorable`.

#![deny(unsafe_code)]

use crate::error::Result;

/// Allows access to the score of a query.
///
/// Equivalent to the abstract class `org.apache.lucene.search.Scorable`.
///
/// **Divergence from Lucene 10.5.0.** Java's `score()` is declared on an
/// immutable-looking receiver, but every non-trivial implementation mutates
/// state while producing the score — [`ScoreCachingWrappingScorer`] caches it,
/// and a [`Scorer`] reads from an iterator it owns. This port therefore takes
/// `&mut self`, which is what Rust requires to express the same object.
///
/// [`ScoreCachingWrappingScorer`]: crate::search::ScoreCachingWrappingScorer
/// [`Scorer`]: crate::search::Scorer
pub trait Scorable {
    /// Returns the score of the current document matching the query.
    ///
    /// Equivalent to `Scorable.score()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while computing the score.
    fn score(&mut self) -> Result<f32>;

    /// Returns the smoothing score of the current document matching the query.
    ///
    /// Equivalent to `Scorable.smoothingScore(int)`, whose default returns
    /// `0f`. This score is used when the query or term does not appear in the
    /// document and behaves like an inverse document frequency. It matters
    /// above all when the scorer returns a product of probabilities, so that a
    /// single zero probability does not drive the document score to zero.
    ///
    /// Smoothing scores are described in Metzler, D. and Croft, W. B.,
    /// "Combining the Language Model and Inference Network Approaches to
    /// Retrieval", Information Processing and Management 40(5), pp. 735-750.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while computing the score.
    fn smoothing_score(&mut self, _doc_id: i32) -> Result<f32> {
        Ok(0.0)
    }

    /// Tells the scorer that its iterator may safely ignore all documents whose
    /// score is less than `min_score`.
    ///
    /// Equivalent to `Scorable.setMinCompetitiveScore(float)`, a no-op by
    /// default. This method may only be called from collectors that use
    /// [`ScoreMode::TOP_SCORES`](crate::search::ScoreMode::TOP_SCORES), and
    /// successive calls may only set increasing values of `min_score`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while applying the new threshold.
    fn set_min_competitive_score(&mut self, _min_score: f32) -> Result<()> {
        Ok(())
    }

    /// Returns the child sub-scorers positioned on the current document.
    ///
    /// Equivalent to `Scorable.getChildren()`, which returns an empty
    /// collection by default.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands out live references to the
    /// children, which stay usable for as long as the parent does. Rust cannot
    /// express that without giving up exclusive access to the parent, so the
    /// children are borrowed from `&mut self` and the borrow ends with the
    /// returned vector.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while collecting the children.
    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(Vec::new())
    }
}

/// A child [`Scorable`] and its relationship to its parent.
///
/// Equivalent to the `Scorable.ChildScorable` record. The meaning of the
/// relationship depends upon the parent query; it can be any string that makes
/// sense to the parent scorer.
pub struct ChildScorable<'a> {
    /// The child scorable. Note that this is typically a direct child, and may
    /// itself also have children.
    pub child: &'a mut dyn Scorable,
    /// An arbitrary string relating this scorable to the parent.
    pub relationship: String,
}

impl<'a> ChildScorable<'a> {
    /// Creates a child/parent relationship record.
    pub fn new(child: &'a mut dyn Scorable, relationship: impl Into<String>) -> Self {
        Self {
            child,
            relationship: relationship.into(),
        }
    }
}

impl std::fmt::Debug for ChildScorable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildScorable")
            .field("relationship", &self.relationship)
            .finish_non_exhaustive()
    }
}

/// Simplest implementation of [`Scorable`], implemented via plain getters and
/// setters.
///
/// Equivalent to `org.apache.lucene.search.SimpleScorable`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and the bulk scorers that need it live in sibling modules.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleScorable {
    score: f32,
    min_competitive_score: f32,
}

impl SimpleScorable {
    /// Creates a scorable whose score and minimum competitive score are `0`.
    ///
    /// Equivalent to `new SimpleScorable()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the score returned by [`Scorable::score`].
    ///
    /// Equivalent to `SimpleScorable.setScore(float)`.
    pub fn set_score(&mut self, score: f32) {
        self.score = score;
    }

    /// Returns the minimum competitive score last set on this scorable.
    ///
    /// Equivalent to `SimpleScorable.minCompetitiveScore()`.
    pub fn min_competitive_score(&self) -> f32 {
        self.min_competitive_score
    }
}

impl Scorable for SimpleScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.min_competitive_score = min_score;
        Ok(())
    }
}

/// Filters a [`Scorable`], intercepting methods and optionally changing their
/// return values.
///
/// Equivalent to `org.apache.lucene.search.FilterScorable`. The default
/// implementation passes all calls to its delegate, with the exception of
/// [`Scorable::set_min_competitive_score`], which is a no-op — exactly as in
/// Java, where `FilterScorable` does not override it and therefore inherits
/// `Scorable`'s no-op.
pub struct FilterScorable<'a> {
    /// The wrapped scorable.
    pub inner: &'a mut dyn Scorable,
}

impl<'a> FilterScorable<'a> {
    /// Wraps the given scorable.
    ///
    /// Equivalent to `new FilterScorable(Scorable)`.
    pub fn new(inner: &'a mut dyn Scorable) -> Self {
        Self { inner }
    }
}

impl std::fmt::Debug for FilterScorable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterScorable")
    }
}

impl Scorable for FilterScorable<'_> {
    fn score(&mut self) -> Result<f32> {
        self.inner.score()
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![ChildScorable::new(&mut *self.inner, "FILTER")])
    }
}

impl<T: Scorable + ?Sized> Scorable for &mut T {
    fn score(&mut self) -> Result<f32> {
        (**self).score()
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        (**self).smoothing_score(doc_id)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        (**self).set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        (**self).children()
    }
}
