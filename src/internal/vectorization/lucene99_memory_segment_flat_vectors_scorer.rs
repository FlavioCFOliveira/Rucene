//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFlatVectorsScorer`.

#![deny(unsafe_code)]

use std::sync::{Arc, LazyLock};

use crate::codecs::hnsw::flat_vectors::check_dimensions;
use crate::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use crate::error::Result;
use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::{
    Lucene99MemorySegmentByteVectorScorer, Lucene99MemorySegmentByteVectorScorerSupplier,
    Lucene99MemorySegmentFloatVectorScorer, Lucene99MemorySegmentFloatVectorScorerSupplier,
};
use crate::store::IndexInput;
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
/// `MemorySegmentAccessInput`; when either fails it calls the delegate. The
/// second test is made by the memory-segment scorers' own `create`, which
/// unwrap and narrow the slice as Java does. The first is not expressible:
/// Rust cannot test a trait object for a *different* trait — there is no
/// downcast from `dyn FloatVectorValues` to `dyn HasIndexSlice` — so the
/// [`FlatVectorsScorer`] implementation below always takes Lucene's delegate
/// branch, and the memory-segment path is reached through the four inherent
/// methods, which take the slice the caller already holds.
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
    /// `None` when the input is not memory-mapped or the data does not fit in
    /// a single mapped segment.
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
        input: &dyn IndexInput,
        vector_values: Arc<dyn FloatVectorValues>,
    ) -> Result<Option<Box<dyn RandomVectorScorerSupplier>>> {
        Ok(Lucene99MemorySegmentFloatVectorScorerSupplier::create(
            similarity_function,
            input,
            vector_values,
        )?
        .map(|supplier| Box::new(supplier) as Box<dyn RandomVectorScorerSupplier>))
    }

    /// Returns a supplier that scores byte vectors read from `input`, or
    /// `None` when the input is not memory-mapped.
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
        input: &dyn IndexInput,
        vector_values: Arc<dyn ByteVectorValues>,
    ) -> Result<Option<Box<dyn RandomVectorScorerSupplier>>> {
        Ok(Lucene99MemorySegmentByteVectorScorerSupplier::create(
            similarity_function,
            input,
            vector_values,
        )?
        .map(|supplier| Box::new(supplier) as Box<dyn RandomVectorScorerSupplier>))
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
    /// the delegate exactly as Java does. When the memory-segment supplier is
    /// not created — the slice is not memory-mapped, or its data does not fit
    /// one mapped segment — the delegate receives the same `vector_values`,
    /// handed back by
    /// `Lucene99MemorySegmentFloatVectorScorerSupplier::create_or_decline`.
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
        slice: Option<&dyn IndexInput>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        let vector_values = match slice {
            Some(slice) => match Lucene99MemorySegmentFloatVectorScorerSupplier::create_or_decline(
                similarity_type,
                slice,
                vector_values,
            )? {
                Ok(scorer) => return Ok(Box::new(scorer)),
                Err(declined) => declined,
            },
            None => vector_values,
        };
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
    /// When the slice is not memory-mapped, the delegate receives the same
    /// `vector_values`, handed back by
    /// `Lucene99MemorySegmentByteVectorScorerSupplier::create_or_decline`,
    /// as Java hands it the same instance.
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
        slice: Option<&dyn IndexInput>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        let vector_values = match slice {
            Some(slice) => match Lucene99MemorySegmentByteVectorScorerSupplier::create_or_decline(
                similarity_type,
                slice,
                vector_values,
            )? {
                Ok(scorer) => return Ok(Box::new(scorer)),
                Err(declined) => declined,
            },
            None => vector_values,
        };
        self.delegate
            .get_random_vector_scorer_supplier_byte(similarity_type, vector_values)
    }

    /// Returns a scorer for a float `target`, reading the stored vectors from
    /// `input`, or `None` when the input is not memory-mapped or the data does
    /// not fit in a single mapped segment.
    ///
    /// Equivalent to the `Lucene99MemorySegmentFloatVectorScorer.create` branch
    /// of `getRandomVectorScorer(..., float[])`, including the dimension check
    /// `FlatVectorsScorer.checkDimensions` performs first.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when `target` does not match
    /// the field dimension, and propagates whatever
    /// [`Lucene99MemorySegmentFloatVectorScorer::create`] returns.
    pub fn random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn IndexInput,
        vector_values: Arc<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Option<Box<dyn RandomVectorScorer>>> {
        check_dimensions(target.len() as i32, vector_values.dimension())?;
        Ok(Lucene99MemorySegmentFloatVectorScorer::create(
            similarity_function,
            input,
            vector_values,
            target,
        )?
        .map(|scorer| Box::new(scorer) as Box<dyn RandomVectorScorer>))
    }

    /// Returns a scorer for a byte `target`, reading the stored vectors from
    /// `input`, or `None` when the input is not memory-mapped.
    ///
    /// Equivalent to the `Lucene99MemorySegmentByteVectorScorer.create` branch
    /// of `getRandomVectorScorer(..., byte[])`, including the dimension check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument) when `target` does not match
    /// the field dimension, and propagates whatever
    /// [`Lucene99MemorySegmentByteVectorScorer::create`] returns.
    pub fn random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        input: &dyn IndexInput,
        vector_values: Arc<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Option<Box<dyn RandomVectorScorer>>> {
        check_dimensions(target.len() as i32, vector_values.dimension())?;
        Ok(Lucene99MemorySegmentByteVectorScorer::create(
            similarity_function,
            input,
            vector_values,
            target,
        )?
        .map(|scorer| Box::new(scorer) as Box<dyn RandomVectorScorer>))
    }
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

#[cfg(test)]
mod tests {
    //! The four memory-segment scorers take Java's `IndexInput`, unwrap the
    //! test-only filters with `FilterIndexInput.unwrapOnlyTest`, then narrow
    //! to a `MemorySegmentAccessInput`
    //! (`Lucene99MemorySegment{Byte,Float}VectorScorer{,Supplier}.java:46-48`).
    use super::*;
    use crate::error::Result;
    use crate::index::vector_values::{from_bytes, from_floats, KnnVectorValues};
    use crate::index::VectorEncoding;
    use crate::store::mmap::MMapDirectory;
    use crate::store::{
        ByteBuffersDirectory, DataInput, Directory, FilterIndexInput, NativeFSLockFactory,
        TestFilterInputRegistration, DEFAULT_IO_CONTEXT,
    };
    use crate::util::vector_util;
    use std::any::TypeId;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A test-only `FilterIndexInput` subclass, like the wrappers Lucene's
    /// test framework registers in `TEST_FILTER_INPUTS`.
    struct TestOnlyInput(FilterIndexInput);

    impl DataInput for TestOnlyInput {
        fn to_java_string(&self) -> String {
            self.0.to_java_string()
        }

        fn read_byte(&mut self) -> Result<u8> {
            self.0.read_byte()
        }

        fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
            self.0.read_bytes(b, offset, len)
        }

        fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
            self.0.skip_bytes(num_bytes)
        }
    }

    impl IndexInput for TestOnlyInput {
        fn close(&mut self) -> Result<()> {
            self.0.close()
        }

        fn file_pointer(&self) -> i64 {
            self.0.file_pointer()
        }

        fn length(&self) -> i64 {
            self.0.length()
        }

        fn seek(&mut self, pos: i64) -> Result<()> {
            self.0.seek(pos)
        }

        fn slice(
            &self,
            description: &str,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn IndexInput>> {
            self.0.slice(description, offset, length)
        }

        fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
            self.0.clone_input()
        }

        fn resource_description(&self) -> &str {
            self.0.resource_description()
        }

        /// A `FilterIndexInput` of its own class, over the input it wraps.
        fn as_filter_index_input(&self) -> Option<(TypeId, &dyn IndexInput)> {
            Some((TypeId::of::<TestOnlyInput>(), self.0.get_delegate()))
        }
    }

    const FLOATS: [[f32; 4]; 4] = [
        [1.0, 2.0, 3.0, 4.0],
        [-1.0, 0.5, 0.25, 8.0],
        [0.0, 0.0, 1.0, 0.0],
        [3.0, -2.0, 1.5, -0.5],
    ];
    const BYTES: [[u8; 4]; 4] = [[1, 2, 3, 4], [255, 7, 9, 0], [0, 0, 1, 0], [3, 254, 1, 128]];
    const FLOAT_QUERY: [f32; 4] = [0.5, -1.0, 2.0, 1.0];
    const BYTE_QUERY: [u8; 4] = [2, 1, 0, 250];

    fn float_values() -> Arc<dyn FloatVectorValues> {
        Arc::new(from_floats(FLOATS.iter().map(|v| v.to_vec()).collect(), 4))
    }

    fn byte_values() -> Arc<dyn ByteVectorValues> {
        Arc::new(from_bytes(BYTES.iter().map(|v| v.to_vec()).collect(), 4))
    }

    /// Scores of nodes 0..4 against the query, from `create` and `create`
    /// alone, or `None` when the factory declines the input.
    fn float_scores(input: &dyn IndexInput) -> Option<Vec<f32>> {
        let mut scorer = Lucene99MemorySegmentFloatVectorScorer::create(
            VectorSimilarityFunction::EUCLIDEAN,
            input,
            float_values(),
            &FLOAT_QUERY,
        )
        .unwrap()?;
        let mut scores = [0f32; 4];
        scorer.bulk_score(&[0, 1, 2, 3], &mut scores, 4).unwrap();
        Some(scores.to_vec())
    }

    fn float_supplier_scores(input: &dyn IndexInput) -> Option<Vec<f32>> {
        let supplier = Lucene99MemorySegmentFloatVectorScorerSupplier::create(
            VectorSimilarityFunction::EUCLIDEAN,
            input,
            float_values(),
        )
        .unwrap()?;
        let mut scorer = supplier.scorer().unwrap();
        scorer.set_scoring_ordinal(0).unwrap();
        Some((0..4).map(|node| scorer.score(node).unwrap()).collect())
    }

    fn byte_scores(input: &dyn IndexInput) -> Option<Vec<f32>> {
        let mut scorer = Lucene99MemorySegmentByteVectorScorer::create(
            VectorSimilarityFunction::EUCLIDEAN,
            input,
            byte_values(),
            &BYTE_QUERY,
        )
        .unwrap()?;
        Some((0..4).map(|node| scorer.score(node).unwrap()).collect())
    }

    fn byte_supplier_scores(input: &dyn IndexInput) -> Option<Vec<f32>> {
        let supplier = Lucene99MemorySegmentByteVectorScorerSupplier::create(
            VectorSimilarityFunction::EUCLIDEAN,
            input,
            byte_values(),
        )
        .unwrap()?;
        let mut scorer = supplier.scorer().unwrap();
        scorer.set_scoring_ordinal(0).unwrap();
        Some((0..4).map(|node| scorer.score(node).unwrap()).collect())
    }

    /// All four factories, through each kind of input. The whole sequence is
    /// one test because the registration it makes is process-wide.
    #[test]
    fn the_scorers_unwrap_test_filters_then_narrow_as_lucene() {
        let tmp = tempfile::tempdir().unwrap();
        let dir =
            MMapDirectory::with_lock_factory(tmp.path(), Box::new(NativeFSLockFactory)).unwrap();
        let mut out = dir
            .create_output("floats.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        for vector in FLOATS {
            for value in vector {
                out.write_bytes(&value.to_le_bytes(), 0, 4).unwrap();
            }
        }
        out.close().unwrap();
        let mut out = dir
            .create_output("bytes.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        for vector in BYTES {
            out.write_bytes(&vector, 0, 4).unwrap();
        }
        out.close().unwrap();
        let open = |name: &str| dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
        let test_only = |name: &str| TestOnlyInput(FilterIndexInput::new("test-only", open(name)));
        let plain_filter = |name: &str| FilterIndexInput::new("plain filter", open(name));
        let memory = ByteBuffersDirectory::new();
        let mut out = memory
            .create_output("floats.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(&[0u8; 64], 0, 64).unwrap();
        out.close().unwrap();
        let heap = memory
            .open_input("floats.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();

        // The mapped input itself is scored straight off the segment.
        let floats = float_scores(open("floats.vec").as_ref()).expect("a mapped input");
        let expected: Vec<f32> = FLOATS
            .iter()
            .map(|v| {
                vector_util::normalize_distance_to_unit_interval(
                    vector_util::square_distance_f32(&FLOAT_QUERY, v).unwrap(),
                )
            })
            .collect();
        assert_eq!(floats, expected);
        let float_supplier = float_supplier_scores(open("floats.vec").as_ref()).unwrap();
        let bytes = byte_scores(open("bytes.vec").as_ref()).expect("a mapped input");
        let byte_supplier = byte_supplier_scores(open("bytes.vec").as_ref()).unwrap();

        // An unregistered wrapper is not unwrapped, and is not a
        // MemorySegmentAccessInput: every factory declines it, as it declines
        // an input that is not mapped.
        assert_eq!(float_scores(&test_only("floats.vec")), None);
        assert_eq!(float_supplier_scores(&test_only("floats.vec")), None);
        assert_eq!(byte_scores(&test_only("bytes.vec")), None);
        assert_eq!(byte_supplier_scores(&test_only("bytes.vec")), None);
        assert_eq!(float_scores(heap.as_ref()), None);
        assert_eq!(byte_scores(heap.as_ref()), None);

        // Registered, the wrapper is peeled off and the mapped input under it
        // scored exactly as when it is passed directly.
        let registration = TestFilterInputRegistration::register(TypeId::of::<TestOnlyInput>());
        assert_eq!(float_scores(&test_only("floats.vec")), Some(floats));
        assert_eq!(
            float_supplier_scores(&test_only("floats.vec")),
            Some(float_supplier)
        );
        assert_eq!(byte_scores(&test_only("bytes.vec")), Some(bytes));
        assert_eq!(
            byte_supplier_scores(&test_only("bytes.vec")),
            Some(byte_supplier)
        );
        // `FilterIndexInput` itself is not registered: it stays wrapped.
        assert_eq!(float_scores(&plain_filter("floats.vec")), None);
        assert_eq!(byte_scores(&plain_filter("bytes.vec")), None);
        drop(registration);
    }

    /// Float values that count how often they, or any copy of them, are
    /// copied.
    struct CountingFloats {
        inner: Box<dyn FloatVectorValues>,
        copies: Arc<AtomicUsize>,
    }

    impl CountingFloats {
        fn copied(&self) -> Result<Self> {
            self.copies.fetch_add(1, Ordering::SeqCst);
            Ok(Self {
                inner: self.inner.copy_float()?,
                copies: Arc::clone(&self.copies),
            })
        }
    }

    impl KnnVectorValues for CountingFloats {
        fn dimension(&self) -> i32 {
            self.inner.dimension()
        }

        fn size(&self) -> i32 {
            self.inner.size()
        }

        fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
            Ok(Box::new(self.copied()?))
        }

        fn encoding(&self) -> VectorEncoding {
            self.inner.encoding()
        }
    }

    impl FloatVectorValues for CountingFloats {
        fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
            self.inner.vector_value(ord)
        }

        fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
            Ok(Box::new(self.copied()?))
        }
    }

    /// Byte values that count how often they, or any copy of them, are
    /// copied.
    struct CountingBytes {
        inner: Box<dyn ByteVectorValues>,
        copies: Arc<AtomicUsize>,
    }

    impl CountingBytes {
        fn copied(&self) -> Result<Self> {
            self.copies.fetch_add(1, Ordering::SeqCst);
            Ok(Self {
                inner: self.inner.copy_byte()?,
                copies: Arc::clone(&self.copies),
            })
        }
    }

    impl KnnVectorValues for CountingBytes {
        fn dimension(&self) -> i32 {
            self.inner.dimension()
        }

        fn size(&self) -> i32 {
            self.inner.size()
        }

        fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
            Ok(Box::new(self.copied()?))
        }

        fn encoding(&self) -> VectorEncoding {
            self.inner.encoding()
        }
    }

    impl ByteVectorValues for CountingBytes {
        fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
            self.inner.vector_value(ord)
        }

        fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
            Ok(Box::new(self.copied()?))
        }
    }

    /// When the memory-segment supplier declines the slice, Java hands the
    /// delegate the same `vectorValues` it was given
    /// (`Lucene99MemorySegmentFlatVectorsScorer.java:51-63, 65-81`), exactly as
    /// for values without a slice: the delegate copies them as often on
    /// either path, and the values themselves are never copied on the way.
    #[test]
    fn a_declined_slice_hands_the_delegate_the_same_values() {
        let memory = ByteBuffersDirectory::new();
        let mut out = memory
            .create_output("vectors.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(&[0u8; 64], 0, 64).unwrap();
        out.close().unwrap();
        let heap = memory
            .open_input("vectors.vec", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        let scorer = Lucene99MemorySegmentFlatVectorsScorer::instance();

        let float_copies = |slice: Option<&dyn IndexInput>| {
            let copies = Arc::new(AtomicUsize::new(0));
            let values = CountingFloats {
                inner: Box::new(from_floats(FLOATS.iter().map(|v| v.to_vec()).collect(), 4)),
                copies: Arc::clone(&copies),
            };
            scorer
                .get_float_scoring_supplier(
                    Box::new(values),
                    VectorSimilarityFunction::EUCLIDEAN,
                    slice,
                )
                .unwrap();
            copies.load(Ordering::SeqCst)
        };
        assert_eq!(float_copies(Some(heap.as_ref())), float_copies(None));

        let byte_copies = |slice: Option<&dyn IndexInput>| {
            let copies = Arc::new(AtomicUsize::new(0));
            let values = CountingBytes {
                inner: Box::new(from_bytes(BYTES.iter().map(|v| v.to_vec()).collect(), 4)),
                copies: Arc::clone(&copies),
            };
            scorer
                .get_byte_scorer_supplier(
                    Box::new(values),
                    VectorSimilarityFunction::EUCLIDEAN,
                    slice,
                )
                .unwrap();
            copies.load(Ordering::SeqCst)
        };
        assert_eq!(byte_copies(Some(heap.as_ref())), byte_copies(None));
    }
}
