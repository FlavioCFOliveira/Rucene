//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene95/TestOffHeapVectorValues.java`.
//!
//! Java keeps a reference to each `CountingIndexInput` it hands to the
//! vector values; Rucene's values own their input, so the input's counter is
//! an `Arc<AtomicI32>` the test keeps a handle of. Java builds the values
//! with a `null` `FlatVectorsScorer`, which their constructor stores without
//! a check and the test never uses; Rucene's constructor takes a scorer, so
//! the test passes `DefaultFlatVectorScorer.INSTANCE`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::hnsw::flat_vectors::DefaultFlatVectorScorer;
use rucene::codecs::lucene99::flat_vectors_format::{
    new_dense_off_heap_byte_vector_values, new_dense_off_heap_float_vector_values,
};
use rucene::error::{LuceneError, Result};
use rucene::index::vector_values::{ByteVectorValues, FloatVectorValues};
use rucene::index::VectorSimilarityFunction;
use rucene::store::{DataInput, IndexInput};

use crate::core_support::{assert_equals, class, test};

/// `TestOffHeapVectorValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene95.TestOffHeapVectorValues",
        vec![
            test(
                "testPrefetchVectorValuesWithMoreThanOneOrds",
                test_prefetch_vector_values_with_more_than_one_ords,
            ),
            test(
                "testPrefetchVectorValuesWithLessThanOneOrds",
                test_prefetch_vector_values_with_less_than_one_ords,
            ),
            test(
                "testPrefetchBoundedByArrayLength",
                test_prefetch_bounded_by_array_length,
            ),
        ],
    )
}

/// Port of `TestOffHeapVectorValues.testPrefetchVectorValuesWithMoreThanOneOrds`.
fn test_prefetch_vector_values_with_more_than_one_ords() -> Result<()> {
    let byte_index_input = CountingIndexInput::new();
    let values = create_test_byte_vector_values(byte_index_input.clone());
    values.prefetch(Some(&[1, 2, 5]), 2)?;
    assert_equals(&2, &byte_index_input.get_prefetch_count());

    let float_index_input = CountingIndexInput::new();
    let float_values = create_test_float_vector_values(float_index_input.clone());
    float_values.prefetch(Some(&[1, 2, 5]), 2)?;
    assert_equals(&2, &float_index_input.get_prefetch_count());
    Ok(())
}

/// Port of `TestOffHeapVectorValues.testPrefetchVectorValuesWithLessThanOneOrds`.
fn test_prefetch_vector_values_with_less_than_one_ords() -> Result<()> {
    let byte_index_input = CountingIndexInput::new();
    let values = create_test_byte_vector_values(byte_index_input.clone());
    values.prefetch(None, 0)?;
    assert_equals(&0, &byte_index_input.get_prefetch_count());
    values.prefetch(Some(&[1]), 1)?;
    assert_equals(&0, &byte_index_input.get_prefetch_count());

    let float_index_input = CountingIndexInput::new();
    let float_values = create_test_float_vector_values(float_index_input.clone());
    float_values.prefetch(None, 0)?;
    assert_equals(&0, &float_index_input.get_prefetch_count());
    float_values.prefetch(Some(&[1]), 1)?;
    assert_equals(&0, &float_index_input.get_prefetch_count());
    Ok(())
}

/// Port of `TestOffHeapVectorValues.testPrefetchBoundedByArrayLength`.
fn test_prefetch_bounded_by_array_length() -> Result<()> {
    // numOrds may exceed the length of the (reused) scratch array. Prefetch must be bounded by the
    // array length so it only prefetches the available ords instead of running past the array end.
    let byte_index_input = CountingIndexInput::new();
    let values = create_test_byte_vector_values(byte_index_input.clone());
    values.prefetch(Some(&[1, 2]), 5)?;
    assert_equals(&2, &byte_index_input.get_prefetch_count());

    let float_index_input = CountingIndexInput::new();
    let float_values = create_test_float_vector_values(float_index_input.clone());
    float_values.prefetch(Some(&[1, 2]), 5)?;
    assert_equals(&2, &float_index_input.get_prefetch_count());
    Ok(())
}

/// Port of `TestOffHeapVectorValues.createTestByteVectorValues`.
fn create_test_byte_vector_values(index_input: CountingIndexInput) -> Box<dyn ByteVectorValues> {
    new_dense_off_heap_byte_vector_values(
        100,
        100,
        Box::new(index_input),
        1,
        Arc::new(DefaultFlatVectorScorer::INSTANCE),
        VectorSimilarityFunction::EUCLIDEAN,
    )
}

/// Port of `TestOffHeapVectorValues.createTestFloatVectorValues`.
fn create_test_float_vector_values(index_input: CountingIndexInput) -> Box<dyn FloatVectorValues> {
    new_dense_off_heap_float_vector_values(
        100,
        100,
        Box::new(index_input),
        4,
        Arc::new(DefaultFlatVectorScorer::INSTANCE),
        VectorSimilarityFunction::EUCLIDEAN,
    )
}

/// `TestOffHeapVectorValues.CountingIndexInput`: an empty input that counts
/// its `prefetch` calls. A clone shares the counter, as Java's
/// `IndexInput.clone()` shares the `counter` field.
#[derive(Clone, Debug)]
struct CountingIndexInput {
    counter: Arc<AtomicI32>,
}

impl CountingIndexInput {
    /// `CountingIndexInput()`: `super("closing index input")`.
    fn new() -> Self {
        Self {
            counter: Arc::new(AtomicI32::new(0)),
        }
    }

    /// `getPrefetchCount()`.
    fn get_prefetch_count(&self) -> i32 {
        self.counter.load(Ordering::SeqCst)
    }
}

impl DataInput for CountingIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        Ok(0)
    }

    fn to_java_string(&self) -> String {
        self.resource_description().to_string()
    }

    fn read_bytes(&mut self, _b: Option<&mut [u8]>, _offset: usize, _len: usize) -> Result<()> {
        let _b = rucene::store::require_array(_b)?;
        Ok(())
    }

    /// Java inherits `DataInput.skipBytes`, which reads the bytes through
    /// `readBytes`, a no-op here.
    fn skip_bytes(&mut self, _num_bytes: i64) -> Result<()> {
        Ok(())
    }
}

impl IndexInput for CountingIndexInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        0
    }

    fn length(&self) -> i64 {
        0
    }

    fn seek(&mut self, _pos: i64) -> Result<()> {
        Ok(())
    }

    /// **Divergence.** Java returns `null` (`TestOffHeapVectorValues.java:115`);
    /// Rucene's `slice` returns an input, so it reports the `null` as an
    /// `IllegalStateException`. The test never slices the input.
    fn slice(
        &self,
        _slice_description: &str,
        _offset: i64,
        _length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Err(LuceneError::illegal_state(
            "CountingIndexInput.slice returns null".to_string(),
        ))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.clone()))
    }

    fn resource_description(&self) -> &str {
        "closing index input"
    }

    fn prefetch(&self, _offset: i64, _length: i64) -> Result<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
