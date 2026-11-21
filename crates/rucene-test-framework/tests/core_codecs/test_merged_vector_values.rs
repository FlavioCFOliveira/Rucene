//! Port of `lucene/core/src/test/org/apache/lucene/codecs/TestMergedVectorValues.java`.
//!
//! Tests for merged vector values to ensure lastOrd is properly incremented during iteration.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::knn_vectors::{
    ByteVectorValuesSub, FloatVectorValuesSub, MergedByteVectorValues, MergedFloat32VectorValues,
};
use rucene::codecs::{ByteVectorValues, FloatVectorValues};
use rucene::error::Result;
use rucene::index::merge::{DocMap, MergeState};
use rucene::index::{from_bytes, from_floats, FieldInfos, KnnVectorValues};
use rucene::util::NoOutputInfoStream;

use crate::core_support::{assert_array_equals, assert_array_equals_f32, class, test};

/// `TestMergedVectorValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.TestMergedVectorValues",
        vec![
            test(
                "testSkipsInMergedByteVectorValues",
                test_skips_in_merged_byte_vector_values,
            ),
            test(
                "testSkipsInMergedFloat32VectorValues",
                test_skips_in_merged_float32_vector_values,
            ),
        ],
    )
}

/// Java's `new MergeState(null, null, …, null, false, null)`: every array and
/// reference `null`, `needsIndexSort` `false`. Java's constructor stores the
/// arguments without checking them; the null arrays are empty vectors, the
/// null `SegmentInfo` is `None`, the null `mergeFieldInfos` the empty
/// `FieldInfos` and the null `InfoStream` the no-output stream, none of
/// which the merged values read.
fn null_merge_state() -> MergeState {
    MergeState {
        doc_maps: Vec::new(),
        segment_info: None,
        merge_field_infos: FieldInfos::empty(),
        stored_fields_readers: Vec::new(),
        term_vectors_readers: Vec::new(),
        norms_producers: Vec::new(),
        doc_values_producers: Vec::new(),
        field_infos: Vec::new(),
        live_docs: Vec::new(),
        fields_producers: Vec::new(),
        points_readers: Vec::new(),
        knn_vectors_readers: Vec::new(),
        max_docs: Vec::new(),
        needs_index_sort: false,
        info_stream: Arc::new(NoOutputInfoStream),
        intra_merge_task_executor: None,
        one_merge: None,
    }
}

/// Java's identity doc map `x -> x`.
fn identity() -> DocMap {
    Arc::new(|x| x)
}

/// Test that skipping vectors in MergedByteVectorValues via nextDoc() and then loading a
/// subsequent vector via vectorValue() works correctly.
///
/// Port of `TestMergedVectorValues.testSkipsInMergedByteVectorValues`.
fn test_skips_in_merged_byte_vector_values() -> Result<()> {
    // Data
    let vectors: Vec<Vec<u8>> = vec![vec![0], vec![1]];

    // Setup
    let sub = ByteVectorValuesSub::new(identity(), Box::new(from_bytes(vectors.clone(), 1)))?;
    let state = null_merge_state();

    // Run the test
    let values = MergedByteVectorValues::new(vec![sub], &state)?;

    // Skip doc 0 and load doc 1
    values.iterator()?.next_doc()?; // doc 0
    values.iterator()?.next_doc()?; // doc 1

    // Read vector for doc 1
    assert_array_equals(&vectors[1], &values.vector_value(1)?);
    Ok(())
}

/// Test that skipping vectors in MergedFloat32VectorValues via nextDoc() and then loading a
/// subsequent vector via vectorValue() works correctly.
///
/// Port of `TestMergedVectorValues.testSkipsInMergedFloat32VectorValues`.
fn test_skips_in_merged_float32_vector_values() -> Result<()> {
    // Data
    let vectors: Vec<Vec<f32>> = vec![vec![0.0], vec![1.0]];

    // Setup
    let sub = FloatVectorValuesSub::new(identity(), Box::new(from_floats(vectors.clone(), 1)))?;
    let state = null_merge_state();

    // Run the test
    let values = MergedFloat32VectorValues::new(vec![sub], &state)?;

    // Skip doc 0 and load doc 1
    values.iterator()?.next_doc()?; // doc 0
    values.iterator()?.next_doc()?; // doc 1

    // Read vector for doc 1
    assert_array_equals_f32(&vectors[1], &values.vector_value(1)?, 0.0);
    Ok(())
}
