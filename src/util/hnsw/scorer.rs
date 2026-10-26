//! Vector scorer traits used during HNSW graph construction and search.
//!
//! Equivalent to the `RandomVectorScorer*` interfaces in
//! `org.apache.lucene.util.hnsw`.

#![deny(unsafe_code)]

use std::fmt::{self, Debug, Formatter};

use crate::util::Bits;

use super::Result;

/// `Math.max(float, float)` as specified by the JLS.
///
/// `f32::max` cannot be used: it returns the non-NaN operand when exactly one
/// operand is NaN, whereas Java propagates NaN. A cosine similarity against a
/// zero vector is NaN, so `f32::max` would silently hide a score Lucene
/// surfaces. Java also specifies that `max(-0.0, +0.0)` is `+0.0`, which
/// `a >= b` alone does not give.
///
/// The semantics are the same as the `java_math_max` helper in
/// `internal::vectorization`, which exists for exactly this reason.
#[inline]
pub(crate) fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Scores random graph nodes against an abstract query.
///
/// Equivalent to `org.apache.lucene.util.hnsw.RandomVectorScorer`.
///
/// Implementations are not required to be thread-safe; each search/build
/// thread should obtain its own scorer via a supplier.
pub trait RandomVectorScorer {
    /// Returns the score between the query and the given node.
    fn score(&mut self, node: i32) -> Result<f32>;

    /// Scores a batch of nodes, storing the results in `scores`.
    ///
    /// Returns the maximum score, or `f32::NEG_INFINITY` if
    /// `num_nodes == 0`.
    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        let n = num_nodes as usize;
        let mut max = f32::NEG_INFINITY;
        for i in 0..n {
            scores[i] = self.score(nodes[i])?;
            max = java_math_max(max, scores[i]);
        }
        Ok(max)
    }

    /// Returns the maximum ordinal that this scorer can score.
    fn max_ord(&self) -> i32;

    /// Translates a vector ordinal to a document id.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        ord
    }

    /// Returns the [`Bits`] representing the accepted ordinals, given the
    /// accepted documents.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.RandomVectorScorer#getAcceptOrds`
    /// (`RandomVectorScorer.java:77-79`), whose default returns its argument:
    /// the default here is that identity. Every scorer built over a field's
    /// vector values — Java's `RandomVectorScorer.AbstractRandomVectorScorer`
    /// and `UpdateableRandomVectorScorer.AbstractUpdateableRandomVectorScorer`
    /// (`RandomVectorScorer.java:104-107`, `UpdateableRandomVectorScorer.java:63-66`)
    /// — overrides it with `values.getAcceptOrds(acceptDocs)`, which asks
    /// whether the *document* `ordToDoc(ord)` is accepted:
    /// [`KnnVectorValues::get_accept_ords`](crate::index::KnnVectorValues::get_accept_ords)
    /// here. That view borrows only `accept_docs`, never the values, so the
    /// caller keeps scoring through `&mut self` while it is alive.
    ///
    /// Java returns the very `Bits` it was given by default; the result is
    /// boxed here, so the default wraps the argument in [`BorrowedBits`], which
    /// forwards every query to it unchanged.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        accept_docs.map(|bits| Box::new(BorrowedBits(bits)) as Box<dyn Bits + 'a>)
    }
}

/// A [`Bits`] that forwards every query to a borrowed [`Bits`].
///
/// It exists so that [`RandomVectorScorer::get_accept_ords`] can hand back the
/// bits it was given through the same boxed return type an overriding scorer
/// uses for a mapped view; Java simply returns the argument.
#[derive(Debug)]
pub struct BorrowedBits<'a>(&'a dyn Bits);

impl<'a> BorrowedBits<'a> {
    /// Wraps `bits` so it can be returned as an owned [`Bits`].
    pub fn new(bits: &'a dyn Bits) -> Self {
        Self(bits)
    }
}

impl Bits for BorrowedBits<'_> {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn try_get(&self, index: usize) -> Result<bool> {
        self.0.try_get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

/// A scorer whose scoring ordinal can be changed in place.
///
/// Equivalent to `org.apache.lucene.util.hnsw.UpdateableRandomVectorScorer`.
pub trait UpdateableRandomVectorScorer: RandomVectorScorer {
    /// Changes the scoring ordinal to the given node.
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()>;
}

/// Creates `UpdateableRandomVectorScorer` instances.
///
/// Equivalent to `org.apache.lucene.util.hnsw.RandomVectorScorerSupplier`.
pub trait RandomVectorScorerSupplier {
    /// Creates a new scorer.
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>>;

    /// Makes a copy of the supplier that is safe to use on another thread.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>>;
}

/// A scorer supplier that also carries a closeable resource and knows the total
/// number of vectors.
///
/// Equivalent to `org.apache.lucene.util.hnsw.CloseableRandomVectorScorerSupplier`.
pub trait CloseableRandomVectorScorerSupplier: RandomVectorScorerSupplier {
    /// Returns the total number of vectors managed by this supplier.
    fn total_vector_count(&self) -> i32;

    /// Releases the resource this supplier holds.
    ///
    /// Equivalent to the `java.io.Closeable#close` that
    /// `CloseableRandomVectorScorerSupplier` extends.
    ///
    /// # Errors
    ///
    /// Propagates whatever closing the underlying resource raises.
    ///
    /// **Divergence from Lucene 10.5.0.** Java leaves `close()` abstract, since
    /// every implementor is a `Closeable`. Here it defaults to doing nothing,
    /// so that a supplier holding no resource — which Java expresses by
    /// wrapping itself with `create(supplier, count, () -> {})` — does not have
    /// to restate an empty body.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Creates a [`CloseableRandomVectorScorerSupplier`] that delegates its scorers
/// to `supplier`, reports `total_vector_count`, and runs `on_close` when it is
/// closed.
///
/// The supplier returned by [`RandomVectorScorerSupplier::copy`] is not
/// necessarily closeable, exactly as Java documents.
///
/// Equivalent to the static
/// `org.apache.lucene.util.hnsw.CloseableRandomVectorScorerSupplier#create`,
/// which returns an anonymous implementation; Rust has no anonymous classes, so
/// the same four members live on the named [`DelegatingCloseableSupplier`].
pub fn create(
    supplier: Box<dyn RandomVectorScorerSupplier>,
    total_vector_count: i32,
    on_close: Box<dyn FnMut() -> Result<()> + Send + Sync>,
) -> DelegatingCloseableSupplier {
    DelegatingCloseableSupplier {
        supplier,
        total_vector_count,
        on_close,
    }
}

/// The [`CloseableRandomVectorScorerSupplier`] that [`create`] returns.
///
/// Equivalent to the anonymous class declared inside
/// `org.apache.lucene.util.hnsw.CloseableRandomVectorScorerSupplier#create`.
pub struct DelegatingCloseableSupplier {
    supplier: Box<dyn RandomVectorScorerSupplier>,
    total_vector_count: i32,
    on_close: Box<dyn FnMut() -> Result<()> + Send + Sync>,
}

impl Debug for DelegatingCloseableSupplier {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegatingCloseableSupplier")
            .field("total_vector_count", &self.total_vector_count)
            .finish_non_exhaustive()
    }
}

impl RandomVectorScorerSupplier for DelegatingCloseableSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        self.supplier.scorer()
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.supplier.copy()
    }
}

impl CloseableRandomVectorScorerSupplier for DelegatingCloseableSupplier {
    fn total_vector_count(&self) -> i32 {
        self.total_vector_count
    }

    fn close(&mut self) -> Result<()> {
        (self.on_close)()
    }
}
