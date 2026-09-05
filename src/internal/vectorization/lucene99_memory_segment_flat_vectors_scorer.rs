//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFlatVectorsScorer`.

#![deny(unsafe_code)]

use std::sync::{Arc, LazyLock};

use crate::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use crate::error::{LuceneError, Result};
use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::{
    Lucene99MemorySegmentByteVectorScorer, Lucene99MemorySegmentByteVectorScorerSupplier,
    Lucene99MemorySegmentFloatVectorScorer, Lucene99MemorySegmentFloatVectorScorerSupplier,
};
use crate::store::MemorySegmentAccessInput;
use crate::util::hnsw::{RandomVectorScorer, RandomVectorScorerSupplier};

/// Routes flat-vector scoring to the memory-segment scorers when the vector
/// data is memory-mapped, and to a delegate otherwise.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFlatVectorsScorer`.
///
/// # Divergence from Lucene 10.5.0: the routing is explicit
///
/// Java decides at run time with two `instanceof` tests: the values must be a
/// `HasIndexSlice` whose slice is non-null, and that slice must be a
/// `MemorySegmentAccessInput`; when either fails it calls the delegate. Rust
/// cannot test a trait object for a *different* trait — there is no downcast
/// from `dyn FloatVectorValues` to `dyn HasIndexSlice`, nor from
/// `dyn IndexInput` to `dyn MemorySegmentAccessInput` — so the
/// [`FlatVectorsScorer`] implementation below always takes Lucene's delegate
/// branch, and the memory-segment path is reached through the four inherent
/// methods, which take the narrowed input the caller already holds.
///
/// The `assert !(vectorValues instanceof BaseQuantizedByteVectorValues)` Java
/// carries in its byte methods is not reproduced for the same reason; the typed
/// entry points make the mistake it guards against unrepresentable, since
/// quantized values are scored by
/// [`Lucene99MemorySegmentScalarQuantizedVectorScorer`](super::Lucene99MemorySegmentScalarQuantizedVectorScorer)
/// instead.
pub struct Lucene99MemorySegmentFlatVectorsScorer {
    delegate: Arc<dyn FlatVectorsScorer>,
}

impl std::fmt::Debug for Lucene99MemorySegmentFlatVectorsScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Equivalent to `toString()`, which returns this exact string.
        f.write_str("Lucene99MemorySegmentFlatVectorsScorer()")
    }
}

/// Backs [`Lucene99MemorySegmentFlatVectorsScorer::instance`].
static INSTANCE: LazyLock<Arc<Lucene99MemorySegmentFlatVectorsScorer>> = LazyLock::new(|| {
    Arc::new(Lucene99MemorySegmentFlatVectorsScorer {
        delegate: Arc::new(DefaultFlatVectorScorer::INSTANCE),
    })
});

impl Lucene99MemorySegmentFlatVectorsScorer {
    /// Returns the singleton instance.
    ///
    /// Equivalent to `Lucene99MemorySegmentFlatVectorsScorer.INSTANCE`, which
    /// Java builds over `DefaultFlatVectorScorer.INSTANCE`. It is a function
    /// rather than a constant because the delegate lives behind an [`Arc`],
    /// which cannot be constructed in a `const`.
    pub fn instance() -> Arc<Self> {
        Arc::clone(&INSTANCE)
    }

    /// Returns the scorer used when the memory-segment path does not apply.
    ///
    /// Equivalent to reading the private `delegate` field.
    pub fn delegate(&self) -> &Arc<dyn FlatVectorsScorer> {
        &self.delegate
    }

    /// Returns a supplier that scores float vectors read from `input`, or
    /// `None` when the data does not fit in a single mapped segment.
    ///
    /// Equivalent to the `Lucene99MemorySegmentFloatVectorScorerSupplier.create`
    /// branch of `getRandomVectorScorerSupplier`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`Lucene99MemorySegmentFloatVectorScorerSupplier::create`] returns.
    pub fn random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        vector_values: Arc<dyn FloatVectorValues>,
    ) -> Result<Option<Box<dyn RandomVectorScorerSupplier>>> {
        Ok(Lucene99MemorySegmentFloatVectorScorerSupplier::create(
            similarity_function,
            input,
            vector_values,
        )?
        .map(|supplier| Box::new(supplier) as Box<dyn RandomVectorScorerSupplier>))
    }

    /// Returns a supplier that scores byte vectors read from `input`.
    ///
    /// Equivalent to the `Lucene99MemorySegmentByteVectorScorerSupplier.create`
    /// branch of `getRandomVectorScorerSupplier`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`Lucene99MemorySegmentByteVectorScorerSupplier::create`] returns.
    pub fn random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        vector_values: Arc<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(
            Lucene99MemorySegmentByteVectorScorerSupplier::create(
                similarity_function,
                input,
                vector_values,
            )?,
        ))
    }

    /// Returns a supplier for float vectors, falling back to the delegate when
    /// the memory-segment path does not apply.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFlatVectorsScorer#getFloatScoringSupplier`,
    /// the `FLOAT32` arm of `getRandomVectorScorerSupplier`. Where Java tests
    /// `vectorValues instanceof HasIndexSlice sliceableValues && getSlice() != null`,
    /// this port takes that slice as `slice`: `None` stands for values that are
    /// not sliceable, or whose slice is null, and sends the call straight to
    /// the delegate exactly as Java does.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// When the values *are* sliceable but the data does not fit one mapped
    /// segment, Java hands the delegate the same `vectorValues` instance. The
    /// delegate takes owned values here, and the memory-segment supplier needs
    /// them behind an [`Arc`] which cannot be unwrapped back into a `Box`, so
    /// that one fallback path hands the delegate `copy_float()` — the fresh
    /// view over the same data that `FloatVectorValues.copy()` exists to
    /// provide. The common `slice == None` path still passes the original.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`Lucene99MemorySegmentFloatVectorScorerSupplier::create`] or the
    /// delegate returns.
    pub fn get_float_scoring_supplier(
        &self,
        vector_values: Box<dyn FloatVectorValues>,
        similarity_type: VectorSimilarityFunction,
        slice: Option<&dyn MemorySegmentAccessInput>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        if let Some(slice) = slice {
            let values: Arc<dyn FloatVectorValues> = Arc::from(vector_values);
            if let Some(scorer) = Lucene99MemorySegmentFloatVectorScorerSupplier::create(
                similarity_type,
                slice,
                Arc::clone(&values),
            )? {
                return Ok(Box::new(scorer));
            }
            return self
                .delegate
                .get_random_vector_scorer_supplier_float(similarity_type, values.copy_float()?);
        }
        self.delegate
            .get_random_vector_scorer_supplier_float(similarity_type, vector_values)
    }

    /// Returns a supplier for byte vectors, falling back to the delegate when
    /// the memory-segment path does not apply.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFlatVectorsScorer#getByteScorerSupplier`,
    /// the `BYTE` arm of `getRandomVectorScorerSupplier`. As for
    /// [`get_float_scoring_supplier`](Self::get_float_scoring_supplier),
    /// `slice` carries what Java obtains from `HasIndexSlice.getSlice()`, and
    /// `None` takes Lucene's delegate branch.
    ///
    /// Java also asserts that the values are not
    /// `BaseQuantizedByteVectorValues`, since a quantized value reaching here
    /// would be a wiring mistake; the typed entry points of this port make that
    /// mistake unrepresentable, as the type's own documentation explains.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`Lucene99MemorySegmentByteVectorScorerSupplier::create`] or the
    /// delegate returns.
    pub fn get_byte_scorer_supplier(
        &self,
        vector_values: Box<dyn ByteVectorValues>,
        similarity_type: VectorSimilarityFunction,
        slice: Option<&dyn MemorySegmentAccessInput>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        if let Some(slice) = slice {
            return Ok(Box::new(
                Lucene99MemorySegmentByteVectorScorerSupplier::create(
                    similarity_type,
                    slice,
                    Arc::from(vector_values),
                )?,
            ));
        }
        self.delegate
            .get_random_vector_scorer_supplier_byte(similarity_type, vector_values)
    }

    /// Returns a scorer for a float `target`, reading the stored vectors from
    /// `input`, or `None` when the data does not fit in a single mapped
    /// segment.
    ///
    /// Equivalent to the `Lucene99MemorySegmentFloatVectorScorer.create` branch
    /// of `getRandomVectorScorer(..., float[])`, including the dimension check
    /// `FlatVectorsScorer.checkDimensions` performs first.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `target` does not match
    /// the field dimension, and propagates whatever
    /// [`Lucene99MemorySegmentFloatVectorScorer::create`] returns.
    pub fn random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        vector_values: Arc<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Option<Box<dyn RandomVectorScorer>>> {
        check_dimensions(target.len(), vector_values.dimension())?;
        Ok(Lucene99MemorySegmentFloatVectorScorer::create(
            similarity_function,
            input,
            vector_values,
            target,
        )?
        .map(|scorer| Box::new(scorer) as Box<dyn RandomVectorScorer>))
    }

    /// Returns a scorer for a byte `target`, reading the stored vectors from
    /// `input`.
    ///
    /// Equivalent to the `Lucene99MemorySegmentByteVectorScorer.create` branch
    /// of `getRandomVectorScorer(..., byte[])`, including the dimension check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `target` does not match
    /// the field dimension, and propagates whatever
    /// [`Lucene99MemorySegmentByteVectorScorer::create`] returns.
    pub fn random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        vector_values: Arc<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        check_dimensions(target.len(), vector_values.dimension())?;
        Ok(Box::new(Lucene99MemorySegmentByteVectorScorer::create(
            similarity_function,
            input,
            vector_values,
            target,
        )?))
    }
}

/// Fails when the query and the field disagree on dimension.
///
/// Equivalent to `FlatVectorsScorer.checkDimensions(int, int)`.
fn check_dimensions(query_len: usize, field_dimension: i32) -> Result<()> {
    if query_len as i32 != field_dimension {
        return Err(LuceneError::IllegalArgument(format!(
            "vector query dimension: {query_len} differs from field dimension: {field_dimension}"
        )));
    }
    Ok(())
}

impl FlatVectorsScorer for Lucene99MemorySegmentFlatVectorsScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.delegate
            .get_random_vector_scorer_supplier_float(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.delegate
            .get_random_vector_scorer_supplier_byte(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.delegate
            .get_random_vector_scorer_float(similarity_function, vector_values, target)
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.delegate
            .get_random_vector_scorer_byte(similarity_function, vector_values, target)
    }
}

impl std::fmt::Display for Lucene99MemorySegmentFlatVectorsScorer {
    /// Port of `Lucene99MemorySegmentFlatVectorsScorer.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lucene99MemorySegmentFlatVectorsScorer()")
    }
}
