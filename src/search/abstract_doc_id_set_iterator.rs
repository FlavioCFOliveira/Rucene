//! Iterator base classes, ported from
//! `org.apache.lucene.search.AbstractDocIdSetIterator` and
//! `org.apache.lucene.search.FilterDocIdSetIterator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::util::FixedBitSet;

/// Tracks the current doc ID on behalf of a [`DocIdSetIterator`]
/// implementation.
///
/// Equivalent to `org.apache.lucene.search.AbstractDocIdSetIterator`, an
/// abstract class whose only content is a `protected int doc = -1` field and a
/// `final docID()` returning it. Extending it — rather than tracking the doc ID
/// ad hoc — reduces the polymorphism of call sites to `docID()`.
///
/// **Divergence from Lucene 10.5.0.** Rust has no implementation inheritance,
/// so the base class becomes a field an implementation embeds: hold an
/// `AbstractDocIdSetIterator`, keep it up to date from `next_doc`/`advance`,
/// and implement `doc_id()` as [`Self::doc_id`]. The state and its contract are
/// unchanged.
#[derive(Debug, Clone, Copy)]
pub struct AbstractDocIdSetIterator {
    /// The current doc ID, initialized at `-1`.
    ///
    /// Equivalent to the `protected int doc` field.
    pub doc: i32,
}

impl Default for AbstractDocIdSetIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl AbstractDocIdSetIterator {
    /// Creates the tracker in its unpositioned state, with a doc ID of `-1`.
    ///
    /// Equivalent to the sole (protected) constructor.
    pub fn new() -> Self {
        Self { doc: -1 }
    }

    /// Returns the current doc ID.
    ///
    /// Equivalent to `AbstractDocIdSetIterator.docID()`, which is `final` in
    /// Java.
    pub fn doc_id(&self) -> i32 {
        self.doc
    }

    /// Records a new current doc ID and returns it, so that an implementation
    /// can write `Ok(self.base.set(self.inner.next_doc()?))`.
    pub fn set(&mut self, doc: i32) -> i32 {
        self.doc = doc;
        doc
    }
}

/// Wrapper around a [`DocIdSetIterator`].
///
/// Equivalent to `org.apache.lucene.search.FilterDocIdSetIterator`, which
/// delegates every method to the wrapped instance. Implementing
/// [`DocIdSetIterator`] by wrapping this type — rather than from scratch —
/// reduces the polymorphism of call sites to `docID()`.
pub struct FilterDocIdSetIterator {
    /// The wrapped instance.
    ///
    /// Equivalent to the `protected final DocIdSetIterator in` field.
    pub inner: Box<dyn DocIdSetIterator>,
}

impl FilterDocIdSetIterator {
    /// Wraps the given iterator.
    ///
    /// Equivalent to `new FilterDocIdSetIterator(DocIdSetIterator)`.
    pub fn new(inner: Box<dyn DocIdSetIterator>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped iterator, consuming the wrapper.
    pub fn into_inner(self) -> Box<dyn DocIdSetIterator> {
        self.inner
    }
}

impl std::fmt::Debug for FilterDocIdSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterDocIdSetIterator")
    }
}

impl DocIdSetIterator for FilterDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.inner.doc_id_run_end()
    }
}
