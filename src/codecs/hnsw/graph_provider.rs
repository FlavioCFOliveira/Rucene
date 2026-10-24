//! `HnswGraphProvider` and the scorer wrappers ported from
//! `org.apache.lucene.codecs.hnsw`.

use std::sync::Arc;

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::error::Result;
use crate::index::vector_values::DocIndexIterator;
use crate::index::{
    ByteVectorValues, FloatVectorValues, KnnVectorValues, VectorEncoding, VectorSimilarityFunction,
};
use crate::search::VectorScorer;
use crate::util::hnsw::scorer::{
    java_math_max, RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::hnsw::{HasKnnVectorValues, HnswGraph};
use crate::util::quantization::LegacyQuantizedByteVectorValues;
use crate::util::Bits;

/// A reader that can hand out the HNSW graph it stores for a field.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.HnswGraphProvider`, which the
/// HNSW vector readers implement so that a merge can reuse an existing graph
/// instead of rebuilding it.
pub trait HnswGraphProvider {
    /// Returns the graph stored for `field`.
    ///
    /// Equivalent to `HnswGraphProvider.getGraph(String)`.
    fn get_graph(&self, field: &str) -> Result<Box<dyn HnswGraph>>;
}

// -----------------------------------------------------------------------------
// Sharing one field's vector values between the wrapper and the scorer it wraps
// -----------------------------------------------------------------------------

/// One field's float vectors, reachable from more than one owner.
///
/// This is mechanism, not behaviour. Java hands the very same
/// `KnnVectorValues` object to the scorer it builds and to the prefetching
/// wrapper that holds it, because both sides simply hold a reference. Rust's
/// `FlatVectorsScorer` takes the values by value, so the port shares the one
/// object behind an [`Arc`] and gives each owner a handle that forwards every
/// call to it. Both handles therefore read, and prefetch, the same field
/// through the same reader — exactly the aliasing Java has.
struct SharedFloatVectorValues {
    inner: Arc<dyn FloatVectorValues>,
}

impl KnnVectorValues for SharedFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.inner.dimension()
    }

    fn size(&self) -> i32 {
        self.inner.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.inner.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.inner.get_accept_ords(accept_docs)
    }

    fn prefetch(&self, ords_to_prefetch: &[i32], num_ords: i32) -> Result<()> {
        self.inner.prefetch(ords_to_prefetch, num_ords)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        self.inner.copy()
    }

    fn vector_byte_length(&self) -> i32 {
        self.inner.vector_byte_length()
    }

    fn encoding(&self) -> VectorEncoding {
        self.inner.encoding()
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.inner.iterator()
    }

    fn create_dense_iterator(&self) -> Box<dyn DocIndexIterator> {
        self.inner.create_dense_iterator()
    }

    fn create_sparse_iterator(&self) -> Box<dyn DocIndexIterator> {
        self.inner.create_sparse_iterator()
    }
}

impl FloatVectorValues for SharedFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.inner.vector_value(ord)
    }

    fn vector_value_into(&self, ord: i32, dest: &mut [f32]) -> Result<()> {
        self.inner.vector_value_into(ord, dest)
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        self.inner.copy_float()
    }

    fn scorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.scorer(target)
    }

    fn rescorer(&self, target: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.rescorer(target)
    }
}

/// One field's byte vectors, reachable from more than one owner.
///
/// See [`SharedFloatVectorValues`].
struct SharedByteVectorValues {
    inner: Arc<dyn ByteVectorValues>,
}

impl KnnVectorValues for SharedByteVectorValues {
    fn dimension(&self) -> i32 {
        self.inner.dimension()
    }

    fn size(&self) -> i32 {
        self.inner.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.inner.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.inner.get_accept_ords(accept_docs)
    }

    fn prefetch(&self, ords_to_prefetch: &[i32], num_ords: i32) -> Result<()> {
        self.inner.prefetch(ords_to_prefetch, num_ords)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        self.inner.copy()
    }

    fn vector_byte_length(&self) -> i32 {
        self.inner.vector_byte_length()
    }

    fn encoding(&self) -> VectorEncoding {
        self.inner.encoding()
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        self.inner.iterator()
    }

    fn create_dense_iterator(&self) -> Box<dyn DocIndexIterator> {
        self.inner.create_dense_iterator()
    }

    fn create_sparse_iterator(&self) -> Box<dyn DocIndexIterator> {
        self.inner.create_sparse_iterator()
    }
}

impl ByteVectorValues for SharedByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        self.inner.vector_value(ord)
    }

    fn vector_value_into(&self, ord: i32, dest: &mut [u8]) -> Result<()> {
        self.inner.vector_value_into(ord, dest)
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        self.inner.copy_byte()
    }

    fn scorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.scorer(target)
    }

    fn rescorer(&self, target: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        self.inner.rescorer(target)
    }
}

/// The shared handle on one field's vectors that the prefetching wrappers keep.
///
/// Java holds a `KnnVectorValues`, which is the common supertype of both
/// encodings. This port keeps the encoding in the variant because `prefetch`
/// has to be reached through the encoding-specific trait: `dyn
/// FloatVectorValues` cannot be coerced to `dyn KnnVectorValues` at the crate's
/// minimum supported Rust version.
enum SharedVectorValues {
    /// Float vectors.
    Float(Arc<SharedFloatVectorValues>),
    /// Byte vectors.
    Byte(Arc<SharedByteVectorValues>),
}

impl SharedVectorValues {
    /// Takes ownership of `values` and returns the shared handle together with
    /// a second handle onto the same values, to be handed to the wrapped
    /// scorer.
    fn from_float(values: Box<dyn FloatVectorValues>) -> (Self, Box<dyn FloatVectorValues>) {
        let inner: Arc<dyn FloatVectorValues> = Arc::from(values);
        let alias = Box::new(SharedFloatVectorValues {
            inner: Arc::clone(&inner),
        });
        (
            Self::Float(Arc::new(SharedFloatVectorValues { inner })),
            alias,
        )
    }

    /// As [`from_float`](Self::from_float), for byte vectors.
    fn from_byte(values: Box<dyn ByteVectorValues>) -> (Self, Box<dyn ByteVectorValues>) {
        let inner: Arc<dyn ByteVectorValues> = Arc::from(values);
        let alias = Box::new(SharedByteVectorValues {
            inner: Arc::clone(&inner),
        });
        (
            Self::Byte(Arc::new(SharedByteVectorValues { inner })),
            alias,
        )
    }

    /// Returns the values as the base type Java's `values()` returns.
    fn as_knn_vector_values(&self) -> &dyn KnnVectorValues {
        match self {
            Self::Float(values) => values.as_ref(),
            Self::Byte(values) => values.as_ref(),
        }
    }

    /// Hands out another handle onto the same values.
    ///
    /// Equivalent to passing the same `KnnVectorValues` reference again, which
    /// is what Java's `PrefetchableRandomVectorScorerSupplier.scorer()` does.
    fn share(&self) -> Self {
        match self {
            Self::Float(values) => Self::Float(Arc::clone(values)),
            Self::Byte(values) => Self::Byte(Arc::clone(values)),
        }
    }

    /// Returns an independent copy of the values.
    ///
    /// Equivalent to `KnnVectorValues.copy()`, which
    /// `PrefetchableRandomVectorScorerSupplier.copy()` calls.
    ///
    /// # Errors
    ///
    /// Propagates whatever copying the underlying values raises.
    fn copy(&self) -> Result<Self> {
        Ok(match self {
            Self::Float(values) => Self::Float(Arc::new(SharedFloatVectorValues {
                inner: Arc::from(values.copy_float()?),
            })),
            Self::Byte(values) => Self::Byte(Arc::new(SharedByteVectorValues {
                inner: Arc::from(values.copy_byte()?),
            })),
        })
    }

    /// Hints that `num_ords` of `ords_to_prefetch` are about to be scored.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying values raise.
    fn prefetch(&self, ords_to_prefetch: &[i32], num_ords: i32) -> Result<()> {
        self.as_knn_vector_values()
            .prefetch(ords_to_prefetch, num_ords)
    }
}

impl std::fmt::Debug for SharedVectorValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedVectorValues")
            .field("encoding", &self.as_knn_vector_values().encoding())
            .field("size", &self.as_knn_vector_values().size())
            .finish_non_exhaustive()
    }
}

// -----------------------------------------------------------------------------
// PrefetchableFlatVectorScorer
// -----------------------------------------------------------------------------

/// A [`FlatVectorsScorer`] that asks the vector values to prefetch the bytes it
/// is about to score.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.PrefetchableFlatVectorScorer`,
/// which wraps another scorer and issues the prefetch before each batch so the
/// data is warm by the time the comparison runs.
///
/// **Divergence from Lucene 10.5.0.** Java reads the vector values back off the
/// scorer it wraps: `getRandomVectorScorer` casts the delegate to
/// `RandomVectorScorer.AbstractRandomVectorScorer`
/// (`PrefetchableFlatVectorScorer.java:63-79`) and the wrapper then calls
/// `delegate.values()`. Rust cannot downcast a `Box<dyn RandomVectorScorer>`,
/// so the wrapper keeps the values it handed the delegate instead. That is the
/// same object Java's `delegate.values()` returns, because a `FlatVectorsScorer`
/// scores the values it is given: `DefaultFlatVectorScorer` passes them
/// straight to `AbstractRandomVectorScorer`'s constructor
/// (`DefaultFlatVectorScorer.java:67,82`). Java's cast also rejects a delegate
/// that is not an `AbstractRandomVectorScorer` with a `ClassCastException`;
/// this port has nothing to reject, since it never asks the delegate for its
/// values.
#[derive(Debug)]
pub struct PrefetchableFlatVectorScorer {
    inner: Box<dyn FlatVectorsScorer>,
}

impl PrefetchableFlatVectorScorer {
    /// Wraps `inner`.
    pub fn new(inner: Box<dyn FlatVectorsScorer>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped scorer.
    pub fn get_delegate(&self) -> &dyn FlatVectorsScorer {
        self.inner.as_ref()
    }
}

impl FlatVectorsScorer for PrefetchableFlatVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        let (shared, alias) = SharedVectorValues::from_float(vector_values);
        let delegate = self
            .inner
            .get_random_vector_scorer_supplier_float(similarity_function, alias)?;
        Ok(Box::new(PrefetchableRandomVectorScorerSupplier::new(
            delegate, shared,
        )))
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        let (shared, alias) = SharedVectorValues::from_byte(vector_values);
        let delegate = self
            .inner
            .get_random_vector_scorer_supplier_byte(similarity_function, alias)?;
        Ok(Box::new(PrefetchableRandomVectorScorerSupplier::new(
            delegate, shared,
        )))
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let (shared, alias) = SharedVectorValues::from_float(vector_values);
        let delegate =
            self.inner
                .get_random_vector_scorer_float(similarity_function, alias, target)?;
        Ok(Box::new(PrefetchableRandomVectorScorer::new(
            delegate, shared,
        )))
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let (shared, alias) = SharedVectorValues::from_byte(vector_values);
        let delegate =
            self.inner
                .get_random_vector_scorer_byte(similarity_function, alias, target)?;
        Ok(Box::new(PrefetchableRandomVectorScorer::new(
            delegate, shared,
        )))
    }

    /// Equivalent to `getRandomVectorScorerSupplier` called with a
    /// `LegacyQuantizedByteVectorValues`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java wraps whatever the delegate
    /// returns in a `PrefetchableRandomVectorScorerSupplier` that prefetches
    /// through the values (`PrefetchableFlatVectorScorer.java:53-59`). This
    /// port's [`LegacyQuantizedByteVectorValues`] has no `prefetch`, which is a
    /// no-op hint in `KnnVectorValues` and changes no score, so the delegate's
    /// supplier is returned unwrapped. Lucene 10.5.0 constructs this scorer
    /// only in its tests.
    fn get_random_vector_scorer_supplier_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.inner
            .get_random_vector_scorer_supplier_legacy_quantized(similarity_function, vector_values)
    }

    /// Equivalent to `getRandomVectorScorer(…, float[])` called with a
    /// `LegacyQuantizedByteVectorValues`; the delegate's scorer is returned
    /// unwrapped, for the reason given on
    /// [`get_random_vector_scorer_supplier_legacy_quantized`](Self::get_random_vector_scorer_supplier_legacy_quantized).
    fn get_random_vector_scorer_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.inner.get_random_vector_scorer_legacy_quantized(
            similarity_function,
            vector_values,
            target,
        )
    }
}

impl std::fmt::Display for PrefetchableFlatVectorScorer {
    /// Port of `PrefetchableFlatVectorScorer.toString()`, which names only
    /// itself and does not render the scorer it wraps.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrefetchableFlatVectorScorer()")
    }
}

// -----------------------------------------------------------------------------
// The three wrappers PrefetchableFlatVectorScorer hands out
// -----------------------------------------------------------------------------

/// A [`RandomVectorScorerSupplier`] whose scorers prefetch.
///
/// Equivalent to
/// `PrefetchableFlatVectorScorer.PrefetchableRandomVectorScorerSupplier`, which
/// holds the supplier it wraps and the vector values the prefetch is issued
/// against.
pub struct PrefetchableRandomVectorScorerSupplier {
    random_vector_scorer_supplier: Box<dyn RandomVectorScorerSupplier>,
    vector_values: SharedVectorValues,
}

impl PrefetchableRandomVectorScorerSupplier {
    /// Wraps `random_vector_scorer_supplier`, prefetching against
    /// `vector_values`.
    ///
    /// Equivalent to the
    /// `PrefetchableRandomVectorScorerSupplier(RandomVectorScorerSupplier, KnnVectorValues)`
    /// constructor.
    fn new(
        random_vector_scorer_supplier: Box<dyn RandomVectorScorerSupplier>,
        vector_values: SharedVectorValues,
    ) -> Self {
        Self {
            random_vector_scorer_supplier,
            vector_values,
        }
    }
}

impl std::fmt::Debug for PrefetchableRandomVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefetchableRandomVectorScorerSupplier")
            .field("vector_values", &self.vector_values)
            .finish_non_exhaustive()
    }
}

impl RandomVectorScorerSupplier for PrefetchableRandomVectorScorerSupplier {
    /// Equivalent to `PrefetchableRandomVectorScorerSupplier.scorer()`, which
    /// hands the new scorer the very values this supplier holds.
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(PrefetchableUpdatableRandomVectorScorer::new(
            self.random_vector_scorer_supplier.scorer()?,
            self.vector_values.share(),
        )))
    }

    /// Equivalent to `PrefetchableRandomVectorScorerSupplier.copy()`, which
    /// copies both the supplier it wraps and the vector values.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(
            self.random_vector_scorer_supplier.copy()?,
            self.vector_values.copy()?,
        )))
    }
}

impl HasKnnVectorValues for PrefetchableRandomVectorScorerSupplier {
    fn values(&self) -> Option<&dyn KnnVectorValues> {
        Some(self.vector_values.as_knn_vector_values())
    }
}

/// A [`RandomVectorScorer`] that prefetches the vectors of a batch before
/// scoring it.
///
/// Equivalent to
/// `PrefetchableFlatVectorScorer.PrefetchableRandomVectorScorer`, which
/// delegates every operation and intercepts only `bulkScore`.
pub struct PrefetchableRandomVectorScorer {
    delegate: Box<dyn RandomVectorScorer>,
    vector_values: SharedVectorValues,
}

impl PrefetchableRandomVectorScorer {
    /// Wraps `delegate`, prefetching against the values it scores.
    ///
    /// Equivalent to the
    /// `PrefetchableRandomVectorScorer(RandomVectorScorer.AbstractRandomVectorScorer)`
    /// constructor, which reaches the values through `delegate.values()`; see
    /// the divergence declared on [`PrefetchableFlatVectorScorer`].
    fn new(delegate: Box<dyn RandomVectorScorer>, vector_values: SharedVectorValues) -> Self {
        Self {
            delegate,
            vector_values,
        }
    }
}

impl std::fmt::Debug for PrefetchableRandomVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefetchableRandomVectorScorer")
            .field("vector_values", &self.vector_values)
            .finish_non_exhaustive()
    }
}

impl RandomVectorScorer for PrefetchableRandomVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.delegate.score(node)
    }

    /// Prefetches the batch, then lets the wrapped scorer score it.
    ///
    /// Equivalent to `PrefetchableRandomVectorScorer.bulkScore`, which calls
    /// `values().prefetch(nodes, numNodes)` and then `delegate.bulkScore(...)`.
    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        self.vector_values.prefetch(nodes, num_nodes)?;
        self.delegate.bulk_score(nodes, scores, num_nodes)
    }

    fn max_ord(&self) -> i32 {
        self.delegate.max_ord()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.delegate.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.delegate.get_accept_ords(accept_docs)
    }
}

impl HasKnnVectorValues for PrefetchableRandomVectorScorer {
    /// Equivalent to `PrefetchableRandomVectorScorer.values()`.
    fn values(&self) -> Option<&dyn KnnVectorValues> {
        Some(self.vector_values.as_knn_vector_values())
    }
}

/// An [`UpdateableRandomVectorScorer`] that prefetches the vectors of a batch
/// before scoring it.
///
/// Equivalent to
/// `PrefetchableFlatVectorScorer.PrefetchableUpdatableRandomVectorScorer`.
pub struct PrefetchableUpdatableRandomVectorScorer {
    delegate: Box<dyn UpdateableRandomVectorScorer>,
    vector_values: SharedVectorValues,
}

impl PrefetchableUpdatableRandomVectorScorer {
    /// Wraps `delegate`, prefetching against `vector_values`.
    ///
    /// Equivalent to the
    /// `PrefetchableUpdatableRandomVectorScorer(UpdateableRandomVectorScorer, KnnVectorValues)`
    /// constructor.
    fn new(
        delegate: Box<dyn UpdateableRandomVectorScorer>,
        vector_values: SharedVectorValues,
    ) -> Self {
        Self {
            delegate,
            vector_values,
        }
    }
}

impl std::fmt::Debug for PrefetchableUpdatableRandomVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefetchableUpdatableRandomVectorScorer")
            .field("vector_values", &self.vector_values)
            .finish_non_exhaustive()
    }
}

impl RandomVectorScorer for PrefetchableUpdatableRandomVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.delegate.score(node)
    }

    /// Prefetches the batch, then scores it one node at a time.
    ///
    /// Equivalent to `PrefetchableUpdatableRandomVectorScorer.bulkScore`, which
    /// prefetches and then calls `super.bulkScore(...)` — the default of
    /// `RandomVectorScorer`, which loops over `this.score(node)` — rather than
    /// the wrapped scorer's own `bulkScore`. The loop is written out here
    /// because Rust cannot invoke a trait method's default body from an
    /// overriding implementation.
    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        self.vector_values.prefetch(nodes, num_nodes)?;
        let mut max = f32::NEG_INFINITY;
        for i in 0..num_nodes as usize {
            scores[i] = self.score(nodes[i])?;
            max = java_math_max(max, scores[i]);
        }
        Ok(max)
    }

    fn max_ord(&self) -> i32 {
        self.delegate.max_ord()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.delegate.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.delegate.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for PrefetchableUpdatableRandomVectorScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        self.delegate.set_scoring_ordinal(node)
    }
}

impl HasKnnVectorValues for PrefetchableUpdatableRandomVectorScorer {
    /// Equivalent to `PrefetchableUpdatableRandomVectorScorer.values()`, which
    /// returns `super.values()` — the values the supplier passed in, not the
    /// wrapped scorer's.
    fn values(&self) -> Option<&dyn KnnVectorValues> {
        Some(self.vector_values.as_knn_vector_values())
    }
}
