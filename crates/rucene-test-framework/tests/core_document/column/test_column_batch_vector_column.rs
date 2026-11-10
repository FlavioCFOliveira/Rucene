//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchVectorColumn.java`:
//! tests for `VectorColumn` batch indexing.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{FieldType, NumericDocValuesField, StringField};
use rucene::error::Result;
use rucene::index::{
    DocValuesType, IndexOptions, IndexReader, Term, VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::NO_MORE_DOCS;
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::column_batch_test_util::{
    byte_vector_type, float_vector_type, frozen, ft, nbr, nn, only_leaf, simple_batch, AnyColumn,
    ArrayBinaryColumn, ArrayByteVectorColumn, ArrayDenseByteVectorColumn,
    ArrayDenseFloatVectorColumn, ArrayFloatVectorColumn, ArrayLongColumn, Ctx,
};
use crate::core_support::{
    assert_array_equals, assert_array_equals_f32, assert_equals, assert_not_null, class, test, IAE,
};

/// `TestColumnBatchVectorColumn`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchVectorColumn",
        vec![
            test("testDenseFloatVectorColumn", test_dense_float_vector_column),
            test("testDenseByteVectorColumn", test_dense_byte_vector_column),
            test(
                "testSparseFloatVectorColumn",
                test_sparse_float_vector_column,
            ),
            test("testSparseByteVectorColumn", test_sparse_byte_vector_column),
            test(
                "testFloatVectorEncodingMismatchFails",
                test_float_vector_encoding_mismatch_fails,
            ),
            test("testWrongDimensionFails", test_wrong_dimension_fails),
            test(
                "testZeroDimensionFieldTypeFails",
                test_zero_dimension_field_type_fails,
            ),
            test(
                "testVectorWithDocValuesRejected",
                test_vector_with_doc_values_rejected,
            ),
            test(
                "testVectorWithStoredRejected",
                test_vector_with_stored_rejected,
            ),
            test(
                "testVectorWithIndexOptionsRejected",
                test_vector_with_index_options_rejected,
            ),
            test(
                "testVectorWithPointsRejected",
                test_vector_with_points_rejected,
            ),
            test("testDuplicateDocIDFails", test_duplicate_doc_id_fails),
            test("testDecreasingDocIDFails", test_decreasing_doc_id_fails),
            test(
                "testVectorOutOfRangeDocIDFails",
                test_vector_out_of_range_doc_id_fails,
            ),
            test(
                "testDenseVectorColumnTooFewValuesFails",
                test_dense_vector_column_too_few_values_fails,
            ),
            test(
                "testVectorColumnSchemaConsistencyAcrossBatches",
                test_vector_column_schema_consistency_across_batches,
            ),
            test("testVectorWithDeletes", test_vector_with_deletes),
            test("testVectorForceMerge", test_vector_force_merge),
        ],
    )
}

fn euclid_float(dim: i32) -> Result<FieldType> {
    float_vector_type(dim, VectorSimilarityFunction::EUCLIDEAN)
}

/// Port of `testDenseFloatVectorColumn`.
fn test_dense_float_vector_column() -> Result<()> {
    let c = Ctx::open()?;
    let vector_type = euclid_float(3)?;
    let vectors = vec![
        vec![1f32, 2.0, 3.0],
        vec![4f32, 5.0, 6.0],
        vec![7f32, 8.0, 9.0],
    ];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), vectors.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let values = leaf.get_float_vector_values("v")?;
    assert_not_null(&values);
    let values = nn(values);
    let mut it = values.iterator()?;
    for i in 0..vectors.len() {
        assert_equals(&(i as i32), &it.next_doc()?);
        assert_array_equals_f32(&vectors[i], &values.vector_value(it.index())?, 0.0);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    c.close(Some(r))
}

/// Port of `testDenseByteVectorColumn`.
fn test_dense_byte_vector_column() -> Result<()> {
    let c = Ctx::open()?;
    let vector_type = byte_vector_type(4, VectorSimilarityFunction::EUCLIDEAN)?;
    let vectors = vec![
        vec![1u8, 2, 3, 4],
        vec![5u8, 6, 7, 8],
        vec![9u8, 10, 11, 12],
    ];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDenseByteVectorColumn::new("v", ft(&vector_type), vectors.clone())?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let values = leaf.get_byte_vector_values("v")?;
    assert_not_null(&values);
    let values = nn(values);
    let mut it = values.iterator()?;
    for i in 0..vectors.len() {
        assert_equals(&(i as i32), &it.next_doc()?);
        assert_array_equals(&vectors[i], &values.vector_value(it.index())?);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSparseFloatVectorColumn`.
fn test_sparse_float_vector_column() -> Result<()> {
    let c = Ctx::open()?;
    let vector_type = euclid_float(2)?;
    let doc_ids = vec![0, 2, 5, 9];
    let vectors = vec![
        vec![1f32, 1.0],
        vec![2f32, 2.0],
        vec![3f32, 3.0],
        vec![4f32, 4.0],
    ];
    // pair with a sparse long column so the batch has a defined doc count > vector count
    c.add_batch(&simple_batch(
        10,
        vec![
            ArrayFloatVectorColumn::new("v", ft(&vector_type), doc_ids.clone(), vectors.clone())?
                .into(),
            ArrayLongColumn::new(
                "anchor",
                ft(NumericDocValuesField::type_()),
                vec![0, 9],
                vec![0, 9],
            )?
            .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let values = leaf.get_float_vector_values("v")?;
    assert_not_null(&values);
    let values = nn(values);
    let mut it = values.iterator()?;
    for i in 0..doc_ids.len() {
        assert_equals(&doc_ids[i], &it.next_doc()?);
        assert_array_equals_f32(&vectors[i], &values.vector_value(it.index())?, 0.0);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSparseByteVectorColumn`.
fn test_sparse_byte_vector_column() -> Result<()> {
    let c = Ctx::open()?;
    let vector_type = byte_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let doc_ids = vec![1, 4];
    let vectors = vec![vec![1u8, 2], vec![3u8, 4]];
    c.add_batch(&simple_batch(
        6,
        vec![
            ArrayByteVectorColumn::new("v", ft(&vector_type), doc_ids.clone(), vectors.clone())?
                .into(),
            ArrayLongColumn::new(
                "anchor",
                ft(NumericDocValuesField::type_()),
                vec![0, 5],
                vec![0, 5],
            )?
            .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let values = leaf.get_byte_vector_values("v")?;
    assert_not_null(&values);
    let values = nn(values);
    let mut it = values.iterator()?;
    for i in 0..doc_ids.len() {
        assert_equals(&doc_ids[i], &it.next_doc()?);
        assert_array_equals(&vectors[i], &values.vector_value(it.index())?);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    c.close(Some(r))
}

/// `expectThrows(class, () -> w.addBatch(simpleBatch(numDocs, column)))`,
/// then `assertTrue(e.getMessage(), e.getMessage().contains(part))` when
/// `part` is given, then `w.rollback(); dir.close();`.
fn expect_add_batch_fails(
    class: &str,
    num_docs: i32,
    column: impl FnOnce() -> Result<AnyColumn>,
    part: Option<&str>,
) -> Result<()> {
    let mut c = Ctx::open()?;
    let e = expect_throws(class, || {
        c.add_batch(&simple_batch(num_docs, vec![column()?]))
    });
    if let Some(part) = part {
        let message = e.message().unwrap_or_default().to_string();
        Assert::assert_true_msg(&message, message.contains(part));
    }
    c.w.rollback()?;
    c.dir.close()
}

/// Port of `testFloatVectorEncodingMismatchFails`.
fn test_float_vector_encoding_mismatch_fails() -> Result<()> {
    // FieldType says FLOAT32 but column carries byte[] vectors.
    let vector_type = euclid_float(2)?;
    let vectors = vec![vec![1u8, 2], vec![3u8, 4]];
    expect_add_batch_fails(
        "java.lang.ClassCastException",
        2,
        || Ok(ArrayDenseByteVectorColumn::new("v", ft(&vector_type), vectors)?.into()),
        None,
    )
}

/// Port of `testWrongDimensionFails`.
fn test_wrong_dimension_fails() -> Result<()> {
    let vector_type = euclid_float(3)?;
    let vectors = vec![vec![1f32, 2.0, 3.0], vec![4f32, 5.0]];
    expect_add_batch_fails(
        IAE,
        2,
        || Ok(ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), vectors)?.into()),
        Some("expected dimension 3"),
    )
}

/// Port of `testZeroDimensionFieldTypeFails`.
fn test_zero_dimension_field_type_fails() -> Result<()> {
    // No vector attributes set -> vectorDimension() == 0
    let bad = frozen(|t| t.set_doc_values_type(DocValuesType::NUMERIC))?;
    let e = expect_throws(IAE, || {
        ArrayDenseFloatVectorColumn::new("v", ft(&bad), vec![vec![1f32, 2.0]])
    });
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(&message, message.contains("vectorDimension() > 0"));
    Ok(())
}

/// `new FieldType()` with `setVectorAttributes(2, FLOAT32, EUCLIDEAN)`, then `extra`, frozen.
fn vector_with(extra: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    frozen(|t| {
        t.set_vector_attributes(
            2,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
        )?;
        extra(t)
    })
}

/// The shared body of the four `testVectorWith*Rejected` methods.
fn expect_not_vector_only(bad: FieldType) -> Result<()> {
    expect_add_batch_fails(
        IAE,
        1,
        || Ok(ArrayDenseFloatVectorColumn::new("v", ft(&bad), vec![vec![1f32, 2.0]])?.into()),
        Some("must be vector-only"),
    )
}

/// Port of `testVectorWithDocValuesRejected`.
fn test_vector_with_doc_values_rejected() -> Result<()> {
    expect_not_vector_only(vector_with(|t| {
        t.set_doc_values_type(DocValuesType::NUMERIC)
    })?)
}

/// Port of `testVectorWithStoredRejected`.
fn test_vector_with_stored_rejected() -> Result<()> {
    expect_not_vector_only(vector_with(|t| t.set_stored(true))?)
}

/// Port of `testVectorWithIndexOptionsRejected`.
fn test_vector_with_index_options_rejected() -> Result<()> {
    expect_not_vector_only(vector_with(|t| {
        t.set_index_options(IndexOptions::DOCS)?;
        t.set_tokenized(false)
    })?)
}

/// Port of `testVectorWithPointsRejected`.
fn test_vector_with_points_rejected() -> Result<()> {
    expect_not_vector_only(vector_with(|t| t.set_dimensions(1, 4))?)
}

/// Port of `testDuplicateDocIDFails`.
fn test_duplicate_doc_id_fails() -> Result<()> {
    let vector_type = euclid_float(2)?;
    expect_add_batch_fails(
        IAE,
        2,
        || {
            Ok(ArrayFloatVectorColumn::new(
                "v",
                ft(&vector_type),
                vec![0, 0],
                vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            )?
            .into())
        },
        Some("strictly increasing"),
    )
}

/// Port of `testDecreasingDocIDFails`.
fn test_decreasing_doc_id_fails() -> Result<()> {
    let vector_type = euclid_float(2)?;
    expect_add_batch_fails(
        IAE,
        4,
        || {
            Ok(ArrayFloatVectorColumn::new(
                "v",
                ft(&vector_type),
                vec![3, 1],
                vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            )?
            .into())
        },
        Some("strictly increasing"),
    )
}

/// Port of `testVectorOutOfRangeDocIDFails`.
fn test_vector_out_of_range_doc_id_fails() -> Result<()> {
    let vector_type = euclid_float(2)?;
    expect_add_batch_fails(
        IAE,
        3,
        || {
            Ok(ArrayFloatVectorColumn::new(
                "v",
                ft(&vector_type),
                vec![0, 5],
                vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            )?
            .into())
        },
        Some("out of range"),
    )
}

/// Port of `testDenseVectorColumnTooFewValuesFails`.
fn test_dense_vector_column_too_few_values_fails() -> Result<()> {
    let vector_type = euclid_float(2)?;
    // 2 values declared DENSE but the batch has 3 docs.
    expect_add_batch_fails(
        IAE,
        3,
        || {
            Ok(ArrayDenseFloatVectorColumn::new(
                "v",
                ft(&vector_type),
                vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            )?
            .into())
        },
        Some("Dense column"),
    )
}

/// Port of `testVectorColumnSchemaConsistencyAcrossBatches`.
fn test_vector_column_schema_consistency_across_batches() -> Result<()> {
    let mut c = Ctx::open()?;
    let float32_type = euclid_float(2)?;
    c.add_batch(&simple_batch(
        1,
        vec![
            ArrayDenseFloatVectorColumn::new("v", ft(&float32_type), vec![vec![1.0, 2.0]])?.into(),
        ],
    ))?;
    let byte_type = byte_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayDenseByteVectorColumn::new("v", ft(&byte_type), vec![vec![1u8, 2]])?.into()],
        ))
    });
    c.w.rollback()?;
    c.dir.close()
}

/// Port of `testVectorWithDeletes`: vectors added via `addBatch` can be
/// deleted via `deleteDocuments(Term)` when an indexed column is added
/// alongside, and live-doc filtering hides the deleted vectors from search.
fn test_vector_with_deletes() -> Result<()> {
    let mut c = Ctx::open()?;
    let vector_type = euclid_float(2)?;
    let id_type = FieldType::new_from(StringField::type_not_stored());
    id_type.freeze();
    let vectors = vec![
        vec![1f32, 1.0],
        vec![2f32, 2.0],
        vec![3f32, 3.0],
        vec![4f32, 4.0],
    ];
    let ids = vec![nbr("a"), nbr("b"), nbr("c"), nbr("d")];
    c.add_batch(&simple_batch(
        4,
        vec![
            ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), vectors)?.into(),
            ArrayBinaryColumn::new("id", ft(&id_type), vec![0, 1, 2, 3], ids)?.into(),
        ],
    ))?;
    c.w.delete_documents_by_terms(vec![Term::from_text("id", "b")])?;
    c.w.delete_documents_by_terms(vec![Term::from_text("id", "c")])?;
    // Commit to materialize deletes into the segment before opening the reader.
    c.w.commit()?;
    let r: Arc<dyn IndexReader> = rucene::index::directory_reader::open(c.dir.as_directory())?;
    let leaf = only_leaf(&r)?;
    assert_equals(&2, &leaf.num_docs());

    // Iterate the vector values. Either the deletes have been physically merged out (so the
    // iterator only sees 2 docs) or the segment retains the deleted slots (in which case the
    // live-docs bitset filters them). Either way, live count must be 2 and deleted vectors
    // must not appear among the live ones.
    let live = leaf.get_live_docs();
    let mut live_count = 0;
    let vv = nn(leaf.get_float_vector_values("v")?);
    let mut it = vv.iterator()?;
    loop {
        let doc = it.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        if live.as_ref().is_none_or(|l| l.get(doc as usize)) {
            live_count += 1;
        }
    }
    assert_equals(&2, &live_count);
    r.close()?;
    c.w.close()?;
    c.dir.close()
}

/// Port of `testVectorForceMerge`: `forceMerge(1)` of two vector batches
/// yields a single segment containing all vectors.
fn test_vector_force_merge() -> Result<()> {
    let c = Ctx::open()?;
    let vector_type = euclid_float(2)?;
    let first_batch = vec![vec![1f32, 1.0], vec![2f32, 2.0]];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), first_batch)?.into()],
    ))?;
    c.w.commit()?;
    let second_batch = vec![vec![3f32, 3.0], vec![4f32, 4.0], vec![5f32, 5.0]];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), second_batch)?.into()],
    ))?;
    c.w.commit()?;
    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&5, &leaf.max_doc());
    let vv = nn(leaf.get_float_vector_values("v")?);
    let mut it = vv.iterator()?;
    let mut count = 0;
    while it.next_doc()? != NO_MORE_DOCS {
        count += 1;
    }
    assert_equals(&5, &count);
    c.close(Some(r))
}
