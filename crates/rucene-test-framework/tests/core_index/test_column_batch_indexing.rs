//! Port of `lucene/core/src/test/org/apache/lucene/index/TestColumnBatchIndexing.java`.
//!
//! Generic / cross-cutting tests for column-oriented batch indexing via
//! `IndexWriter#addBatch`. Type-specific tests live in
//! `org.apache.lucene.document.column.TestColumnBatch{Long,Binary,Vector}Column`.
//!
//! The columns come from the port of `ColumnBatchTestUtil`
//! (`core_document/column/column_batch_test_util.rs`), whose module docs
//! declare how its columns and `simpleBatch` differ from Java's. Java's
//! throwing columns raise the one exception instance the test supplied, and
//! the test checks it with `assertSame`; the port's columns raise a fresh
//! error from a factory, so [`assert_same_exception`] compares the class
//! and message of the error raised with the factory's.

use std::collections::BTreeSet;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::column::NumericKind;
use rucene::document::{
    BinaryDocValuesField, Document, FieldType, KnnFloatVectorField, LongPoint,
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StoredField, StoredValueType, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::doc_values::{
    BinaryDocValues, NumericDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use rucene::index::index_writer::{self, IndexWriter};
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::terms::Term;
use rucene::index::{
    DocValuesType, FloatVectorValues, IndexOptions, IndexReader, IndexableFieldType,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{IndexSearcher, Query, TermQuery, NO_MORE_DOCS};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::column_batch_test_util::{
    field, float_vector_type, ft, nbr, nn, only_leaf, searcher, simple_batch, AnyColumn,
    ArrayBinaryColumn, ArrayDenseFloatVectorColumn, ArrayDenseLongColumn, ArrayFloatVectorColumn,
    ArrayLongColumn, Ctx, ExceptionFactory, ThrowingBinaryColumn, ThrowingDenseLongColumn,
    ThrowingFloatVectorColumn, ThrowingLongColumn,
};
use crate::core_support::java_number::long_value;
use crate::core_support::{
    assert_array_equals_f32, assert_equals, assert_equals_msg, assert_not_null, assert_null, class,
    random, test, IAE,
};
use crate::support::{assert_not_null_msg, assert_null_msg, open_reader};

/// `TestColumnBatchIndexing`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestColumnBatchIndexing",
        vec![
            test("testMultipleColumns", test_multiple_columns),
            test("testParentFieldIndexed", test_parent_field_indexed),
            test(
                "testAddBatchOverMaxDocsRollsBackReservations",
                test_add_batch_over_max_docs_rolls_back_reservations,
            ),
            test(
                "testParentFieldNameRejectedAsColumn",
                test_parent_field_name_rejected_as_column,
            ),
            test(
                "testMixedStoredAndNonStoredColumns",
                test_mixed_stored_and_non_stored_columns,
            ),
            test(
                "testColumnWithNoneDocValuesTypeAndNoPointsThrows",
                test_column_with_none_doc_values_type_and_no_points_throws,
            ),
            test(
                "testDenseColumnCountMismatchThrows",
                test_dense_column_count_mismatch_throws,
            ),
            test(
                "testDenseColumnTooManyValuesThrows",
                test_dense_column_too_many_values_throws,
            ),
            test(
                "testSparseStoredFieldPreservesNumDocs",
                test_sparse_stored_field_preserves_num_docs,
            ),
            test(
                "testSparseIndexedFieldsPreservesNumDocs",
                test_sparse_indexed_fields_preserves_num_docs,
            ),
            test(
                "testSparseRowMixedWithDenseDocValues",
                test_sparse_row_mixed_with_dense_doc_values,
            ),
            test(
                "testSparseBatchMatchesDocByDoc",
                test_sparse_batch_matches_doc_by_doc,
            ),
            test(
                "testRowColumnOutOfOrderDocIdThrows",
                test_row_column_out_of_order_doc_id_throws,
            ),
            test(
                "testRowColumnOutOfRangeDocIdThrows",
                test_row_column_out_of_range_doc_id_throws,
            ),
            test(
                "testVectorMixedWithLongAndBinary",
                test_vector_mixed_with_long_and_binary,
            ),
            test(
                "testVectorAcrossMultipleBatches",
                test_vector_across_multiple_batches,
            ),
            test(
                "testEmptyVectorColumnRejected",
                test_empty_vector_column_rejected,
            ),
            test(
                "testParentFieldWithVectorBatch",
                test_parent_field_with_vector_batch,
            ),
            test(
                "testLongTupleCursorThrowMidBatchRecovers",
                test_long_tuple_cursor_throw_mid_batch_recovers,
            ),
            test(
                "testLongValuesCursorThrowMidBatchRecovers",
                test_long_values_cursor_throw_mid_batch_recovers,
            ),
            test(
                "testBinaryTupleCursorThrowMidBatchRecovers",
                test_binary_tuple_cursor_throw_mid_batch_recovers,
            ),
            test(
                "testVectorTupleCursorThrowMidBatchRecovers",
                test_vector_tuple_cursor_throw_mid_batch_recovers,
            ),
            test("testEmptyBatch", test_empty_batch),
            test(
                "testMultipleBatchesIntoOneSegment",
                test_multiple_batches_into_one_segment,
            ),
            test(
                "testStoredFieldsAppearAfterStoredFreeBatch",
                test_stored_fields_appear_after_stored_free_batch,
            ),
            test(
                "testNoStoredFieldsAcrossSegment",
                test_no_stored_fields_across_segment,
            ),
            test(
                "testAddBatchInterleavedWithAddDocument",
                test_add_batch_interleaved_with_add_document,
            ),
            test("testAddBatchWithDeletes", test_add_batch_with_deletes),
            test("testIndexSortWithAddBatch", test_index_sort_with_add_batch),
            test(
                "testForceMergeMixedOriginSegments",
                test_force_merge_mixed_origin_segments,
            ),
            test("testConcurrentAddBatch", test_concurrent_add_batch),
            test(
                "testRandomizedParityVsDocByDoc",
                test_randomized_parity_vs_doc_by_doc,
            ),
            test(
                "testDuplicateColumnNameRejected",
                test_duplicate_column_name_rejected,
            ),
            test(
                "testSameFieldNameAcrossBatchesAllowed",
                test_same_field_name_across_batches_allowed,
            ),
            test("testUpdateDocumentsByTerm", test_update_documents_by_term),
            test("testUpdateDocumentsByQuery", test_update_documents_by_query),
        ],
    )
}

const RUNTIME_EXCEPTION: &str = "java.lang.RuntimeException";

/// `NumericDocValuesField.TYPE` as a column type.
fn ndv() -> Arc<dyn IndexableFieldType> {
    ft(NumericDocValuesField::type_())
}

/// `new FieldType()` with `setStored(true)`, frozen.
fn stored_only() -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_stored(true)?;
    t.freeze();
    Ok(t)
}

/// `new FieldType()` with `setStored(true)` and `setDocValuesType(NUMERIC)`,
/// frozen.
fn stored_numeric_type() -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_stored(true)?;
    t.set_doc_values_type(DocValuesType::NUMERIC)?;
    t.freeze();
    Ok(t)
}

/// `newIndexWriterConfig().setMergePolicy(new TieredMergePolicy())` with
/// `setRAMBufferSizeMB(16)` and `setMaxBufferedDocs(DISABLE_AUTO_FLUSH)`.
fn stable_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(TieredMergePolicy::new()))?;
    iwc.set_ram_buffer_size_mb(16.0)?;
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    Ok(iwc)
}

/// `newIndexWriterConfig().setMergePolicy(new TieredMergePolicy())`.
fn tiered_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(TieredMergePolicy::new()))?;
    Ok(iwc)
}

/// `newIndexWriterConfig()` with `setParentField("_parent")`.
fn parent_iwc() -> IndexWriterConfig {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_parent_field(Some("_parent".to_string()));
    config
}

/// `searcher.count(new TermQuery(term))`.
fn count_term(s: &IndexSearcher, term: Term) -> Result<i32> {
    s.count(Arc::new(TermQuery::new(term)))
}

/// `assertSame(expected, actual)` over the exception a throwing column was
/// given and the one `addBatch` threw (module docs).
fn assert_same_exception(expected: &ExceptionFactory, actual: &Throwable) {
    let expected = java_throwable(&expected());
    assert_equals(&expected.class_name(), &actual.class_name());
    assert_equals(&expected.message(), &actual.message());
}

/// `new IllegalStateException(message)`, as the factory a throwing column
/// takes.
fn illegal_state(message: &'static str) -> ExceptionFactory {
    Arc::new(move || LuceneError::illegal_state(message))
}

/// `leaf.getNumericDocValues(name)`, then the `nextDoc`/`longValue` checks
/// of `expected`, then `NO_MORE_DOCS`.
fn assert_numeric_values(
    leaf: &Arc<dyn rucene::index::LeafReader>,
    name: &str,
    expected: &[i64],
) -> Result<()> {
    let mut dv = nn(leaf.get_numeric_doc_values(name)?);
    for (i, value) in expected.iter().enumerate() {
        assert_equals(&(i as i32), &dv.next_doc()?);
        assert_equals(value, &dv.long_value()?);
    }
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    Ok(())
}

/// Port of `TestColumnBatchIndexing.testMultipleColumns`.
fn test_multiple_columns() -> Result<()> {
    let c = Ctx::open()?;

    let all_docs = vec![0, 1, 2];
    let numeric_values = vec![100i64, 200, 300];
    let sorted_values = vec![nbr("a"), nbr("b"), nbr("c")];

    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayLongColumn::new("numeric", ndv(), all_docs.clone(), numeric_values.clone())?
                .into(),
            ArrayBinaryColumn::new(
                "sorted",
                ft(SortedDocValuesField::type_()),
                all_docs,
                sorted_values.clone(),
            )?
            .into(),
        ],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;

    let mut ndv = nn(leaf.get_numeric_doc_values("numeric")?);
    let mut sdv = nn(leaf.get_sorted_doc_values("sorted")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &ndv.next_doc()?);
        assert_equals(&numeric_values[i], &ndv.long_value()?);
        assert_equals(&(i as i32), &sdv.next_doc()?);
        assert_equals(&sorted_values[i], &sdv.lookup_ord(sdv.ord_value()?)?);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testParentFieldIndexed`.
fn test_parent_field_indexed() -> Result<()> {
    let c = Ctx::open_with(parent_iwc())?;

    let doc_ids = vec![0, 1, 2];
    let values = vec![1i64, 2, 3];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new("numeric", ndv(), doc_ids, values)?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;

    // Every batch doc should have the parent field
    let parent_dv = leaf.get_numeric_doc_values("_parent")?;
    assert_not_null(&parent_dv);
    let mut parent_dv = nn(parent_dv);
    for i in 0..3 {
        assert_equals(&i, &parent_dv.next_doc()?);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testAddBatchOverMaxDocsRollsBackReservations`.
fn test_add_batch_over_max_docs_rolls_back_reservations() -> Result<()> {
    const MAX_DOCS: i32 = 10;
    let c = Ctx::open()?;
    index_writer::set_max_docs(MAX_DOCS)?;
    let body = || -> Result<()> {
        // Batch is larger than the cap — must throw without reserving any docs permanently.
        let oversized = MAX_DOCS + 5;
        let mut doc_ids = vec![0; oversized as usize];
        let mut values = vec![0i64; oversized as usize];
        for i in 0..oversized as usize {
            doc_ids[i] = i as i32;
            values[i] = i as i64;
        }
        let e = expect_throws(IAE, || {
            c.add_batch(&simple_batch(
                oversized,
                vec![ArrayLongColumn::new("v", ndv(), doc_ids, values)?.into()],
            ))
        });
        let message = e.message().unwrap_or_default().to_string();
        Assert::assert_true_msg(&message, message.contains("number of documents"));

        // If reservations leaked, even a single addDocument would now fail. Fill the index up
        // to the cap and then verify the next addDocument is the one that fails.
        for i in 0..MAX_DOCS {
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new("v", i as i64)));
            c.w.add_document(&doc)?;
        }
        let overflow = expect_throws(IAE, || {
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new("v", 999)));
            c.w.add_document(&doc)
        });
        let message = overflow.message().unwrap_or_default().to_string();
        Assert::assert_true_msg(&message, message.contains("number of documents"));
        Ok(())
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    // `finally`
    index_writer::set_max_docs(rucene::index::documents_writer::MAX_DOCS)?;
    match outcome {
        Ok(r) => r?,
        Err(payload) => std::panic::resume_unwind(payload),
    }
    c.close(None)
}

/// Port of `TestColumnBatchIndexing.testParentFieldNameRejectedAsColumn`.
fn test_parent_field_name_rejected_as_column() -> Result<()> {
    let c = Ctx::open_with(parent_iwc())?;
    let doc_ids = vec![0, 1, 2];
    let values = vec![1i64, 2, 3];
    let e = expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            3,
            vec![ArrayLongColumn::new("_parent", ndv(), doc_ids, values)?.into()],
        ))
    });
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(&message, message.contains("_parent"));
    Assert::assert_true_msg(&message, message.contains("reserved"));
    c.close(None)
}

/// Port of `TestColumnBatchIndexing.testMixedStoredAndNonStoredColumns`.
fn test_mixed_stored_and_non_stored_columns() -> Result<()> {
    let c = Ctx::open()?;

    let stored_numeric_type = stored_numeric_type()?;

    let all_docs = vec![0, 1, 2];
    let stored_values = vec![100i64, 200, 300];
    let dv_only_values = vec![1i64, 2, 3];
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayLongColumn::new(
                "stored_field",
                ft(&stored_numeric_type),
                all_docs.clone(),
                stored_values.clone(),
            )?
            .into(),
            ArrayLongColumn::new("dv_only", ndv(), all_docs, dv_only_values.clone())?.into(),
        ],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;

    // Verify stored field
    let stored_fields = leaf.stored_fields()?;
    for i in 0..3 {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(
            &stored_values[i],
            &long_value(field(&doc, "stored_field").numeric_value()),
        );
        // non-stored column should not appear
        assert_null(&doc.get_field("dv_only").map(|f| f.to_string()));
    }

    // Verify both doc values columns
    let mut stored_dv = nn(leaf.get_numeric_doc_values("stored_field")?);
    let mut dv_only = nn(leaf.get_numeric_doc_values("dv_only")?);
    for i in 0..3 {
        assert_equals(&(i as i32), &stored_dv.next_doc()?);
        assert_equals(&stored_values[i], &stored_dv.long_value()?);
        assert_equals(&(i as i32), &dv_only.next_doc()?);
        assert_equals(&dv_only_values[i], &dv_only.long_value()?);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testColumnWithNoneDocValuesTypeAndNoPointsThrows`.
fn test_column_with_none_doc_values_type_and_no_points_throws() -> Result<()> {
    let c = Ctx::open()?;

    // FieldType with NONE doc values type and no points
    let mut bad_type = FieldType::new();
    bad_type.freeze();

    let doc_ids = vec![0];
    let values = vec![1i64];
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayLongColumn::new("bad", ft(&bad_type), doc_ids, values)?.into()],
        ))
    });

    // Writer should still be usable after the failure
    c.add_batch(&simple_batch(
        1,
        vec![ArrayLongColumn::new("numeric", ndv(), vec![0], vec![42])?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let dv = leaf.get_numeric_doc_values("numeric")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    // The failed batch's doc was marked deleted; the successful batch's doc is still live
    let doc = dv.next_doc()?;
    Assert::assert_true(doc != NO_MORE_DOCS);
    assert_equals(&42i64, &dv.long_value()?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testDenseColumnCountMismatchThrows`.
fn test_dense_column_count_mismatch_throws() -> Result<()> {
    let c = Ctx::open()?;

    // 2 values but batch expects 3 documents
    let values = vec![10i64, 20];
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            3,
            vec![ArrayDenseLongColumn::new("val", ndv(), values)?.into()],
        ))
    });

    // Writer should still be usable after the failure — use a different field to avoid
    // the partially-written DV entries from the failed batch
    c.add_batch(&simple_batch(
        1,
        vec![ArrayDenseLongColumn::new("val2", ndv(), vec![42])?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let dv = leaf.get_numeric_doc_values("val2")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    let doc = dv.next_doc()?;
    Assert::assert_true(doc != NO_MORE_DOCS);
    assert_equals(&42i64, &dv.long_value()?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testDenseColumnTooManyValuesThrows`.
fn test_dense_column_too_many_values_throws() -> Result<()> {
    let c = Ctx::open()?;

    // 5 values but batch only has 3 documents
    let values = vec![10i64, 20, 30, 40, 50];
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            3,
            vec![ArrayDenseLongColumn::new("val", ndv(), values)?.into()],
        ))
    });

    // Writer should still be usable — no values were written past numDocs
    c.add_batch(&simple_batch(
        1,
        vec![ArrayDenseLongColumn::new("val2", ndv(), vec![42])?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let dv = leaf.get_numeric_doc_values("val2")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    let doc = dv.next_doc()?;
    Assert::assert_true(doc != NO_MORE_DOCS);
    assert_equals(&42i64, &dv.long_value()?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testSparseStoredFieldPreservesNumDocs`.
fn test_sparse_stored_field_preserves_num_docs() -> Result<()> {
    let c = Ctx::open()?;

    let stored_only = stored_only()?;

    // 5 batch docs, but only docs 1 and 3 have a stored value.
    let doc_ids = vec![1, 3];
    let values = vec![nbr("one"), nbr("three")];
    c.add_batch(&simple_batch(
        5,
        vec![ArrayBinaryColumn::new("field", ft(&stored_only), doc_ids, values)?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&5, &leaf.max_doc());

    let stored_fields = leaf.stored_fields()?;
    assert_null(
        &stored_fields
            .document(0)?
            .get_field("field")
            .map(|f| f.to_string()),
    );
    assert_equals(
        &Some(nbr("one")),
        &field(&stored_fields.document(1)?, "field").binary_value(),
    );
    assert_null(
        &stored_fields
            .document(2)?
            .get_field("field")
            .map(|f| f.to_string()),
    );
    assert_equals(
        &Some(nbr("three")),
        &field(&stored_fields.document(3)?, "field").binary_value(),
    );
    assert_null(
        &stored_fields
            .document(4)?
            .get_field("field")
            .map(|f| f.to_string()),
    );

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testSparseIndexedFieldsPreservesNumDocs`.
fn test_sparse_indexed_fields_preserves_num_docs() -> Result<()> {
    let c = Ctx::open()?;

    let mut indexed_type = FieldType::new();
    indexed_type.set_index_options(IndexOptions::DOCS)?;
    indexed_type.set_omit_norms(true)?;
    indexed_type.set_tokenized(false)?;
    indexed_type.freeze();

    // 6 batch docs, only docs 2 and 5 have a term.
    let doc_ids = vec![2, 5];
    let values = vec![nbr("a"), nbr("b")];
    c.add_batch(&simple_batch(
        6,
        vec![ArrayBinaryColumn::new("tag", ft(&indexed_type), doc_ids, values)?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&6, &leaf.max_doc());

    let searcher = searcher(&r)?;
    assert_equals(&1, &count_term(&searcher, Term::from_text("tag", "a"))?);
    assert_equals(&1, &count_term(&searcher, Term::from_text("tag", "b"))?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testSparseRowMixedWithDenseDocValues`.
fn test_sparse_row_mixed_with_dense_doc_values() -> Result<()> {
    let c = Ctx::open()?;

    let stored_only = stored_only()?;

    // Row-sparse stored column: only docs 0 and 3 have a stored value.
    let stored_doc_ids = vec![0, 3];
    let stored_values = vec![nbr("first"), nbr("fourth")];
    // Dense DV column covering every doc.
    let dv_values = vec![100i64, 200, 300, 400];

    c.add_batch(&simple_batch(
        4,
        vec![
            ArrayBinaryColumn::new("stored", ft(&stored_only), stored_doc_ids, stored_values)?
                .into(),
            ArrayDenseLongColumn::new("dv", ndv(), dv_values.clone())?.into(),
        ],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&4, &leaf.max_doc());

    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some(nbr("first")),
        &field(&stored_fields.document(0)?, "stored").binary_value(),
    );
    assert_null(
        &stored_fields
            .document(1)?
            .get_field("stored")
            .map(|f| f.to_string()),
    );
    assert_null(
        &stored_fields
            .document(2)?
            .get_field("stored")
            .map(|f| f.to_string()),
    );
    assert_equals(
        &Some(nbr("fourth")),
        &field(&stored_fields.document(3)?, "stored").binary_value(),
    );

    assert_numeric_values(&leaf, "dv", &dv_values)?;

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testSparseBatchMatchesDocByDoc`.
fn test_sparse_batch_matches_doc_by_doc() -> Result<()> {
    let mut stored_indexed = FieldType::new_from(StringField::type_stored());
    stored_indexed.freeze();

    // 7 docs; only docs 1, 2, and 5 have values for the row column.
    let doc_ids = [1, 2, 5];
    let values = ["alpha", "beta", "gamma"];
    let total_docs = 7;

    // --- Batch path ---
    let mut batch_dir = LuceneTestCase::new_directory();
    let batch_iwc = stable_iwc()?;
    {
        let batch_w = IndexWriter::new(batch_dir.as_directory(), batch_iwc)?;
        let refs: Vec<BytesRef> = values.iter().map(|v| nbr(v)).collect();
        // StringField stores as STRING — use the matching storedType so stored-value round-trip is
        // comparable between the two paths.
        batch_w.add_batch(&simple_batch(
            total_docs,
            vec![ArrayBinaryColumn::with_stored_type(
                "field",
                ft(&stored_indexed),
                doc_ids.to_vec(),
                refs,
                StoredValueType::STRING,
            )?
            .into()],
        ))?;
        batch_w.close()?;
    }

    let mut single_dir = LuceneTestCase::new_directory();
    let single_iwc = stable_iwc()?;
    {
        let single_w = IndexWriter::new(single_dir.as_directory(), single_iwc)?;
        let mut next = 0;
        for d in 0..total_docs {
            let mut doc = Document::new();
            if next < doc_ids.len() && doc_ids[next] == d {
                doc.add(Box::new(StringField::new(
                    "field",
                    values[next].to_string(),
                    Store::YES,
                )?));
                next += 1;
            }
            single_w.add_document(&doc)?;
        }
        single_w.force_merge(1)?;
        single_w.close()?;
    }

    {
        let batch_r: Arc<dyn IndexReader> = open_reader(&batch_dir)?;
        let single_r: Arc<dyn IndexReader> = open_reader(&single_dir)?;
        let batch_leaf = only_leaf(&batch_r)?;
        let single_leaf = only_leaf(&single_r)?;

        assert_equals(&single_leaf.max_doc(), &batch_leaf.max_doc());
        assert_equals(&total_docs, &batch_leaf.max_doc());

        let batch_stored = batch_leaf.stored_fields()?;
        let single_stored = single_leaf.stored_fields()?;
        for d in 0..total_docs {
            let batch_doc = batch_stored.document(d)?;
            let single_doc = single_stored.document(d)?;
            let bf = batch_doc.get_field("field");
            let sf = single_doc.get_field("field");
            match sf {
                None => assert_null_msg(
                    &format!("doc {d} should have no stored field"),
                    &bf.map(|f| f.to_string()),
                ),
                Some(sf) => {
                    assert_not_null_msg(&format!("doc {d} should have a stored field"), &bf);
                    let bf = bf.expect("checked by assertNotNull");
                    assert_equals(&sf.string_value(), &bf.string_value());
                }
            }
        }

        let batch_searcher = searcher(&batch_r)?;
        let single_searcher = searcher(&single_r)?;
        for v in values {
            let t = Term::from_text("field", v);
            assert_equals(
                &count_term(&single_searcher, t.clone())?,
                &count_term(&batch_searcher, t)?,
            );
        }
        batch_r.close()?;
        single_r.close()?;
    }

    batch_dir.close()?;
    single_dir.close()?;
    Ok(())
}

/// Port of `TestColumnBatchIndexing.testRowColumnOutOfOrderDocIdThrows`.
fn test_row_column_out_of_order_doc_id_throws() -> Result<()> {
    let c = Ctx::open()?;
    let stored_only = stored_only()?;

    let doc_ids = vec![2, 1];
    let values = vec![nbr("a"), nbr("b")];
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            3,
            vec![ArrayBinaryColumn::new("f", ft(&stored_only), doc_ids, values)?.into()],
        ))
    });
    c.close(None)
}

/// Port of `TestColumnBatchIndexing.testRowColumnOutOfRangeDocIdThrows`.
fn test_row_column_out_of_range_doc_id_throws() -> Result<()> {
    let c = Ctx::open()?;
    let stored_only = stored_only()?;

    // Batch size 3, but the column advertises a value at doc 5.
    let doc_ids = vec![5];
    let values = vec![nbr("oob")];
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            3,
            vec![ArrayBinaryColumn::new("f", ft(&stored_only), doc_ids, values)?.into()],
        ))
    });
    c.close(None)
}

/// Port of `TestColumnBatchIndexing.testVectorMixedWithLongAndBinary`.
fn test_vector_mixed_with_long_and_binary() -> Result<()> {
    let c = Ctx::open()?;

    let vector_type = float_vector_type(2, VectorSimilarityFunction::DOT_PRODUCT)?;
    let vectors = vec![vec![0.6f32, 0.8], vec![0.8, 0.6], vec![1.0, 0.0]];
    let longs = vec![10i64, 20, 30];
    let bins = vec![nbr("a"), nbr("b"), nbr("c")];
    let ids = vec![0, 1, 2];
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), vectors.clone())?.into(),
            ArrayLongColumn::new("num", ndv(), ids.clone(), longs.clone())?.into(),
            ArrayBinaryColumn::new("bin", ft(BinaryDocValuesField::type_()), ids, bins.clone())?
                .into(),
        ],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut nums = nn(leaf.get_numeric_doc_values("num")?);
    let mut bin_dv = nn(leaf.get_binary_doc_values("bin")?);
    let vec = nn(leaf.get_float_vector_values("v")?);
    let mut it = vec.iterator()?;
    for i in 0..3 {
        assert_equals(&(i as i32), &nums.next_doc()?);
        assert_equals(&longs[i], &nums.long_value()?);
        assert_equals(&(i as i32), &bin_dv.next_doc()?);
        assert_equals(&bins[i], &bin_dv.binary_value()?);
        assert_equals(&(i as i32), &it.next_doc()?);
        assert_array_equals_f32(&vectors[i], &vec.vector_value(it.index())?, 0.0);
    }
    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testVectorAcrossMultipleBatches`.
fn test_vector_across_multiple_batches() -> Result<()> {
    let c = Ctx::open_with(tiered_iwc()?)?;

    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let first_batch = vec![vec![1f32, 1.0], vec![2.0, 2.0]];
    let second_batch = vec![vec![3f32, 3.0], vec![4.0, 4.0], vec![5.0, 5.0]];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), first_batch.clone())?.into()],
    ))?;
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), second_batch.clone())?.into()],
    ))?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let values = nn(leaf.get_float_vector_values("v")?);
    let mut it = values.iterator()?;
    let mut collected: Vec<Vec<f32>> = Vec::new();
    while it.next_doc()? != NO_MORE_DOCS {
        collected.push(values.vector_value(it.index())?.to_vec());
    }
    // Merge order is not guaranteed when batches flush to separate segments,
    // so compare as a sorted set rather than by doc-ID position.
    let by_first = |a: &Vec<f32>, b: &Vec<f32>| (a[0] as f64).total_cmp(&(b[0] as f64));
    collected.sort_by(by_first);
    let mut expected = vec![
        first_batch[0].clone(),
        first_batch[1].clone(),
        second_batch[0].clone(),
        second_batch[1].clone(),
        second_batch[2].clone(),
    ];
    expected.sort_by(by_first);
    assert_equals(&expected.len(), &collected.len());
    for i in 0..expected.len() {
        assert_array_equals_f32(&expected[i], &collected[i], 0.0);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testEmptyVectorColumnRejected`.
fn test_empty_vector_column_rejected() -> Result<()> {
    let c = Ctx::open()?;

    // A field type alone is not enough — every batch must have at least one column with data,
    // and a vector-only column with no values is the equivalent of "no documents have this
    // vector".
    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let anchor_ids = vec![0, 1];
    let anchor_vals = vec![0i64, 1];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayFloatVectorColumn::new("v", ft(&vector_type), Vec::new(), Vec::new())?.into(),
            ArrayLongColumn::new("anchor", ndv(), anchor_ids, anchor_vals)?.into(),
        ],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    if let Some(values) = leaf.get_float_vector_values("v")? {
        assert_equals(&NO_MORE_DOCS, &values.iterator()?.next_doc()?);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testParentFieldWithVectorBatch`.
fn test_parent_field_with_vector_batch() -> Result<()> {
    let c = Ctx::open_with(parent_iwc())?;

    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let vectors = vec![vec![1f32, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), vectors.clone())?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let parent_dv = leaf.get_numeric_doc_values("_parent")?;
    assert_not_null(&parent_dv);
    let mut parent_dv = nn(parent_dv);
    for i in 0..3 {
        assert_equals(&i, &parent_dv.next_doc()?);
    }
    let values = nn(leaf.get_float_vector_values("v")?);
    let mut it = values.iterator()?;
    for i in 0..3 {
        assert_equals(&(i as i32), &it.next_doc()?);
        assert_array_equals_f32(&vectors[i], &values.vector_value(it.index())?, 0.0);
    }

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testLongTupleCursorThrowMidBatchRecovers`.
fn test_long_tuple_cursor_throw_mid_batch_recovers() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    // 5-doc batch; LongTupleCursor#nextDoc throws at index 2 (during column pass).
    let doc_ids = vec![0, 1, 2, 3, 4];
    let values = vec![10i64, 20, 30, 40, 50];
    let expected = illegal_state("boom-long");
    let actual = expect_throws(RUNTIME_EXCEPTION, || {
        c.add_batch(&simple_batch(
            5,
            vec![
                ThrowingLongColumn::new("v", ndv(), doc_ids, values, 2, Arc::clone(&expected))?
                    .into(),
            ],
        ))
    });
    assert_same_exception(&expected, &actual);
    // Force the failed batch's DWPT to flush so the cleanup-induced fully-deleted
    // segment is pruned before the recovery starts a new one.
    c.w.commit()?;

    // Recovery: another batch and a doc-by-doc add must succeed.
    let recovery_ids = vec![0, 1];
    let recovery_vals = vec![100i64, 200];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new("v", ndv(), recovery_ids, recovery_vals)?.into()],
    ))?;
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("v", 999)));
    c.w.add_document(&doc)?;

    // The failed batch's 5 reserved doc-ids are all marked deleted, the resulting
    // fully-deleted segment is pruned at flush time, so only the recovery work
    // remains visible.
    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&(2 + 1), &leaf.max_doc());
    assert_equals(&(2 + 1), &leaf.num_docs());

    assert_numeric_values(&leaf, "v", &[100, 200, 999])?;

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testLongValuesCursorThrowMidBatchRecovers`.
fn test_long_values_cursor_throw_mid_batch_recovers() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    // 6-doc dense batch; LongValuesCursor#fillDocValues throws partway (after 3 fills).
    let values = vec![1i64, 2, 3, 4, 5, 6];
    let expected = illegal_state("boom-dense");
    let actual = expect_throws(RUNTIME_EXCEPTION, || {
        c.add_batch(&simple_batch(
            6,
            vec![
                ThrowingDenseLongColumn::new("v", ndv(), values, 3, Arc::clone(&expected))?.into(),
            ],
        ))
    });
    assert_same_exception(&expected, &actual);
    // Force the failed batch's DWPT to flush so the cleanup-induced fully-deleted
    // segment is pruned before the recovery starts a new one.
    c.w.commit()?;

    // Recovery via a single addDocument followed by another addBatch.
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("v", 777)));
    c.w.add_document(&doc)?;
    let recovery_ids = vec![0, 1];
    let recovery_vals = vec![111i64, 222];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new("v", ndv(), recovery_ids, recovery_vals)?.into()],
    ))?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&(1 + 2), &leaf.max_doc());
    assert_equals(&(1 + 2), &leaf.num_docs());

    assert_numeric_values(&leaf, "v", &[777, 111, 222])?;

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testBinaryTupleCursorThrowMidBatchRecovers`.
fn test_binary_tuple_cursor_throw_mid_batch_recovers() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    let doc_ids = vec![0, 1, 2, 3];
    let values = vec![nbr("a"), nbr("b"), nbr("c"), nbr("d")];
    let expected = illegal_state("boom-binary");
    let actual = expect_throws(RUNTIME_EXCEPTION, || {
        c.add_batch(&simple_batch(
            4,
            vec![ThrowingBinaryColumn::new(
                "b",
                ft(BinaryDocValuesField::type_()),
                doc_ids,
                values,
                1,
                Arc::clone(&expected),
            )?
            .into()],
        ))
    });
    assert_same_exception(&expected, &actual);
    // Force the failed batch's DWPT to flush so the cleanup-induced fully-deleted
    // segment is pruned before the recovery starts a new one.
    c.w.commit()?;

    let recovery_ids = vec![0];
    let recovery_vals = vec![nbr("ok")];
    c.add_batch(&simple_batch(
        1,
        vec![ArrayBinaryColumn::new(
            "b",
            ft(BinaryDocValuesField::type_()),
            recovery_ids,
            recovery_vals,
        )?
        .into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&1, &leaf.max_doc());
    assert_equals(&1, &leaf.num_docs());

    let mut dv = nn(leaf.get_binary_doc_values("b")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&nbr("ok"), &dv.binary_value()?);
    assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testVectorTupleCursorThrowMidBatchRecovers`.
fn test_vector_tuple_cursor_throw_mid_batch_recovers() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let doc_ids = vec![0, 1, 2];
    let values = vec![vec![1f32, 1.0], vec![2.0, 2.0], vec![3.0, 3.0]];
    let expected = illegal_state("boom-vector");
    let actual = expect_throws(RUNTIME_EXCEPTION, || {
        c.add_batch(&simple_batch(
            3,
            vec![ThrowingFloatVectorColumn::new(
                "v",
                ft(&vector_type),
                doc_ids,
                values,
                1,
                Arc::clone(&expected),
            )?
            .into()],
        ))
    });
    assert_same_exception(&expected, &actual);
    // Force the failed batch's DWPT to flush so the cleanup-induced fully-deleted
    // segment is pruned before the recovery starts a new one.
    c.w.commit()?;

    // Recover with a clean vector batch.
    let recovery = vec![vec![4f32, 4.0]];
    c.add_batch(&simple_batch(
        1,
        vec![ArrayDenseFloatVectorColumn::new("v", ft(&vector_type), recovery.clone())?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&1, &leaf.max_doc());
    assert_equals(&1, &leaf.num_docs());

    let vv = nn(leaf.get_float_vector_values("v")?);
    let mut it = vv.iterator()?;
    assert_equals(&0, &it.next_doc()?);
    assert_array_equals_f32(&recovery[0], &vv.vector_value(it.index())?, 0.0);
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testEmptyBatch`.
fn test_empty_batch() -> Result<()> {
    let c = Ctx::open()?;

    c.add_batch(&simple_batch(0, Vec::new()))?;

    // Writer still functions for follow-up work.
    c.add_batch(&simple_batch(
        1,
        vec![ArrayLongColumn::new("v", ndv(), vec![0], vec![42])?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&1, &leaf.max_doc());
    assert_numeric_values(&leaf, "v", &[42])?;

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testMultipleBatchesIntoOneSegment`.
fn test_multiple_batches_into_one_segment() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new("v", ndv(), vec![0, 1, 2], vec![10, 20, 30])?.into()],
    ))?;
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new("v", ndv(), vec![0, 1], vec![40, 50])?.into()],
    ))?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&5, &leaf.max_doc());
    assert_numeric_values(&leaf, "v", &[10, 20, 30, 40, 50])?;

    c.close(Some(r))
}

/// Stored fields appearing in a later batch are correctly aligned to their
/// docIDs even when earlier batches in the same segment had no stored
/// columns.
///
/// Port of `TestColumnBatchIndexing.testStoredFieldsAppearAfterStoredFreeBatch`.
fn test_stored_fields_appear_after_stored_free_batch() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    // Batch A: 3 docs, indexed-only string column (no stored).
    let ids = vec![nbr("a"), nbr("b"), nbr("c")];
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayBinaryColumn::new("id", ft(StringField::type_not_stored()), vec![0, 1, 2], ids)?
                .into(),
        ],
    ))?;

    // Batch B: 3 docs, one stored numeric column.
    let stored_numeric_type = stored_numeric_type()?;
    let stored_values = vec![100i64, 200, 300];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new(
            "v",
            ft(&stored_numeric_type),
            vec![0, 1, 2],
            stored_values.clone(),
        )?
        .into()],
    ))?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&6, &leaf.max_doc());

    let stored_fields = leaf.stored_fields()?;
    // Docs 0..2 (batch A) had no stored column — empty frames retroactively filled in.
    for i in 0..3 {
        let doc = stored_fields.document(i)?;
        assert_null_msg(
            &format!("doc {i} should have no stored 'v' field"),
            &doc.get_field("v").map(|f| f.to_string()),
        );
    }
    // Docs 3..5 (batch B) have the stored numeric value.
    for i in 0..3 {
        let doc = stored_fields.document(3 + i as i32)?;
        assert_equals(
            &stored_values[i],
            &long_value(field(&doc, "v").numeric_value()),
        );
    }

    c.close(Some(r))
}

/// When no batch in the segment has a stored column, the segment opens
/// cleanly and stored-field reads return empty docs.
///
/// Port of `TestColumnBatchIndexing.testNoStoredFieldsAcrossSegment`.
fn test_no_stored_fields_across_segment() -> Result<()> {
    let c = Ctx::open_with(tiered_iwc()?)?;

    // Two indexed-only batches (no stored column anywhere).
    let ids_a = vec![nbr("a"), nbr("b")];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayBinaryColumn::new("id", ft(StringField::type_not_stored()), vec![0, 1], ids_a)?
                .into(),
        ],
    ))?;
    let ids_b = vec![nbr("c"), nbr("d"), nbr("e")];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayBinaryColumn::new(
            "id",
            ft(StringField::type_not_stored()),
            vec![0, 1, 2],
            ids_b,
        )?
        .into()],
    ))?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&5, &leaf.max_doc());

    // Indexed values are still searchable.
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, Term::from_text("id", "a"))?);
    assert_equals(&1, &count_term(&s, Term::from_text("id", "e"))?);

    // Stored reads return empty docs for every batch-local doc-id.
    let stored_fields = leaf.stored_fields()?;
    for i in 0..leaf.max_doc() {
        let doc = stored_fields.document(i)?;
        Assert::assert_true_msg(
            &format!("doc {i} should have no stored fields"),
            doc.get_fields().is_empty(),
        );
    }

    c.close(Some(r))
}

/// Interleaving addBatch and addDocument keeps doc-ids contiguous and values
/// consistent.
///
/// Port of `TestColumnBatchIndexing.testAddBatchInterleavedWithAddDocument`.
fn test_add_batch_interleaved_with_add_document() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;

    // batch (3) → doc → batch (2) → doc, expecting docs 0..6 in order.
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new("v", ndv(), vec![0, 1, 2], vec![10, 20, 30])?.into()],
    ))?;
    let mut d3 = Document::new();
    d3.add(Box::new(NumericDocValuesField::new("v", 40)));
    c.w.add_document(&d3)?;
    c.add_batch(&simple_batch(
        2,
        vec![ArrayLongColumn::new("v", ndv(), vec![0, 1], vec![50, 60])?.into()],
    ))?;
    let mut d6 = Document::new();
    d6.add(Box::new(NumericDocValuesField::new("v", 70)));
    c.w.add_document(&d6)?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&7, &leaf.max_doc());
    assert_numeric_values(&leaf, "v", &[10, 20, 30, 40, 50, 60, 70])?;

    c.close(Some(r))
}

/// `new FieldType(StringField.TYPE_NOT_STORED)`, frozen.
fn id_type() -> FieldType {
    let mut id_type = FieldType::new_from(StringField::type_not_stored());
    id_type.freeze();
    id_type
}

/// Port of `TestColumnBatchIndexing.testAddBatchWithDeletes`.
fn test_add_batch_with_deletes() -> Result<()> {
    let c = Ctx::open()?;

    let id_type = id_type();

    let ids = vec![nbr("a"), nbr("b"), nbr("c"), nbr("d")];
    let doc_ids = vec![0, 1, 2, 3];
    c.add_batch(&simple_batch(
        4,
        vec![ArrayBinaryColumn::new("id", ft(&id_type), doc_ids, ids)?.into()],
    ))?;

    c.w.delete_documents_by_terms(vec![Term::from_text("id", "b")])?;
    c.w.delete_documents_by_queries(vec![
        Arc::new(TermQuery::new(Term::from_text("id", "d"))) as Arc<dyn Query>
    ])?;

    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&0, &count_term(&s, Term::from_text("id", "b"))?);
    assert_equals(&0, &count_term(&s, Term::from_text("id", "d"))?);
    assert_equals(&1, &count_term(&s, Term::from_text("id", "a"))?);
    assert_equals(&1, &count_term(&s, Term::from_text("id", "c"))?);

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testIndexSortWithAddBatch`.
fn test_index_sort_with_add_batch() -> Result<()> {
    let mut config = LuceneTestCase::new_index_writer_config();
    config.set_index_sort(Sort::new_fields(vec![SortField::new(
        Some("sort".to_string()),
        SortFieldType::Long,
    )?])?)?;
    let c = Ctx::open_with(config)?;

    // Out-of-order: input is 30, 10, 20 — segment should reorder to 10, 20, 30.
    let values = vec![30i64, 10, 20];
    let doc_ids = vec![0, 1, 2];
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new("sort", ndv(), doc_ids, values)?.into()],
    ))?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&3, &leaf.max_doc());

    assert_numeric_values(&leaf, "sort", &[10, 20, 30])?;

    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testForceMergeMixedOriginSegments`.
fn test_force_merge_mixed_origin_segments() -> Result<()> {
    // Pin a non-mock merge policy: MockRandomMergePolicy.reorder may reverse doc-ids during
    // merge, which is legitimate behavior but defeats the strict per-doc-id assertions below.
    // Also disable auto-flush so the 3 addDocument calls land in a single segment; otherwise a
    // randomized tiny maxBufferedDocs splits them into multiple small segments and TMP's
    // size-sorted forceMerge can interleave them with the addBatch segment.
    let c = Ctx::open_with(stable_iwc()?)?;

    // Segment A via addDocument: values 10, 20, 30.
    for i in 0..3i64 {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("v", 10 * (i + 1))));
        c.w.add_document(&doc)?;
    }
    c.w.commit()?;

    // Segment B via addBatch: values 40, 50, 60.
    c.add_batch(&simple_batch(
        3,
        vec![ArrayLongColumn::new("v", ndv(), vec![0, 1, 2], vec![40, 50, 60])?.into()],
    ))?;
    c.w.commit()?;

    c.w.force_merge(1)?;

    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    assert_equals(&6, &leaf.max_doc());

    assert_numeric_values(&leaf, "v", &[10, 20, 30, 40, 50, 60])?;

    c.close(Some(r))
}

/// Multiple threads calling addBatch concurrently on the same writer produce
/// a consistent index with every doc reachable by its unique id and the
/// expected total doc count.
///
/// Port of `TestColumnBatchIndexing.testConcurrentAddBatch`.
fn test_concurrent_add_batch() -> Result<()> {
    let c = Ctx::open()?;

    const NUM_THREADS: i32 = 4;
    const BATCHES_PER_THREAD: i32 = 5;
    const DOCS_PER_BATCH: i32 = 10;
    let start = Arc::new(crate::support::CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(NUM_THREADS as usize);
    for t in 0..NUM_THREADS {
        let thread_idx = t;
        let start = Arc::clone(&start);
        let w = Arc::clone(&c.w);
        let thread = Thread::unnamed(move || {
            let body = || -> Result<()> {
                start.await_();
                let mut doc_ids = vec![0; DOCS_PER_BATCH as usize];
                for i in 0..DOCS_PER_BATCH as usize {
                    doc_ids[i] = i as i32;
                }
                for b in 0..BATCHES_PER_THREAD {
                    let mut ids = Vec::with_capacity(DOCS_PER_BATCH as usize);
                    for i in 0..DOCS_PER_BATCH {
                        ids.push(nbr(&format!("t{thread_idx}_b{b}_d{i}")));
                    }
                    w.add_batch(&simple_batch(
                        DOCS_PER_BATCH,
                        vec![ArrayBinaryColumn::new(
                            "id",
                            ft(StringField::type_not_stored()),
                            doc_ids.clone(),
                            ids,
                        )?
                        .into()],
                    ))?;
                }
                Ok(())
            };
            if let Err(e) = body() {
                Throwable::from_cause("java.lang.AssertionError", java_throwable(&e)).throw()
            }
        });
        thread.start();
        threads.push(thread);
    }
    start.count_down();
    for th in &threads {
        if let Err(payload) = th.join() {
            std::panic::resume_unwind(payload);
        }
    }

    let total_docs = NUM_THREADS * BATCHES_PER_THREAD * DOCS_PER_BATCH;
    let r = c.reader()?;
    assert_equals(&total_docs, &r.num_docs());
    let s = searcher(&r)?;
    for t in 0..NUM_THREADS {
        for b in 0..BATCHES_PER_THREAD {
            for i in 0..DOCS_PER_BATCH {
                let term = Term::from_text("id", &format!("t{t}_b{b}_d{i}"));
                assert_equals_msg(Some(&format!("missing {term}")), &1, &count_term(&s, term)?);
            }
        }
    }

    c.close(Some(r))
}

/// Per-doc payload covering the union of column types we stress in the
/// parity test.
#[derive(Default)]
struct ParityDoc {
    /// numeric DV
    num: Option<i64>,
    /// multi-valued sorted-numeric DV
    snum: Option<Vec<i64>>,
    /// binary DV + stored
    bin_stored: Option<BytesRef>,
    /// multi-valued sorted-set DV
    set: Option<Vec<BytesRef>>,
    /// single 1-D long point
    lpoint: Option<i64>,
    /// indexed StringField for term queries
    tag: Option<BytesRef>,
    /// float vector
    vec: Option<Vec<f32>>,
}

fn random_parity_doc(r: &dyn Random) -> ParityDoc {
    let mut d = ParityDoc::default();
    if r.next_float() < 0.7 {
        d.num = Some(r.next_int_bound(1000) as i64);
    }
    if r.next_float() < 0.5 {
        let n = TestUtil::next_int(r, 1, 3);
        let mut snum = vec![0i64; n as usize];
        for v in snum.iter_mut() {
            *v = r.next_int_bound(1000) as i64;
        }
        snum.sort();
        d.snum = Some(snum);
    }
    if r.next_float() < 0.6 {
        d.bin_stored = Some(nbr(&format!("b{}", r.next_int_bound(50))));
    }
    if r.next_float() < 0.5 {
        let n = TestUtil::next_int(r, 1, 3) as usize;
        let mut uniq = BTreeSet::new();
        while uniq.len() < n {
            uniq.insert(format!("s{}", r.next_int_bound(20)));
        }
        d.set = Some(uniq.iter().map(|s| nbr(s)).collect());
    }
    if r.next_float() < 0.6 {
        d.lpoint = Some(r.next_int_bound(10000) as i64);
    }
    if r.next_float() < 0.7 {
        d.tag = Some(nbr(&format!("tag{}", r.next_int_bound(15))));
    }
    if r.next_float() < 0.5 {
        let a = r.next_float();
        let b = r.next_float();
        d.vec = Some(vec![a, b]);
    }
    d
}

/// `new IndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(new
/// TieredMergePolicy())` with `setRAMBufferSizeMB(16)` and
/// `setMaxBufferedDocs(DISABLE_AUTO_FLUSH)`.
fn parity_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_merge_policy(Arc::new(TieredMergePolicy::new()))?;
    iwc.set_ram_buffer_size_mb(16.0)?;
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    Ok(iwc)
}

fn build_doc_by_doc_index(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    docs: &[ParityDoc],
) -> Result<()> {
    let iwc = parity_iwc()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for d in docs {
        let mut doc = Document::new();
        if let Some(num) = d.num {
            doc.add(Box::new(NumericDocValuesField::new("num", num)));
        }
        if let Some(snum) = &d.snum {
            for &v in snum {
                doc.add(Box::new(SortedNumericDocValuesField::new("snum", v)));
            }
        }
        if let Some(bin_stored) = &d.bin_stored {
            doc.add(Box::new(BinaryDocValuesField::new(
                "bin_stored",
                bin_stored.clone(),
            )));
            doc.add(Box::new(StoredField::new_bytes(
                "bin_stored",
                bin_stored.clone(),
            )?));
        }
        if let Some(set) = &d.set {
            for v in set {
                doc.add(Box::new(SortedSetDocValuesField::new("set", v.clone())));
            }
        }
        if let Some(lpoint) = d.lpoint {
            doc.add(Box::new(LongPoint::new("lpoint", &[lpoint])?));
        }
        if let Some(tag) = &d.tag {
            doc.add(Box::new(StringField::new_with_bytes(
                "tag",
                tag.clone(),
                Store::NO,
            )?));
        }
        if let Some(vec) = &d.vec {
            doc.add(Box::new(KnnFloatVectorField::with_euclidean("vec", vec)?));
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()
}

/// Builds an index by partitioning the docs into random-sized batches and
/// feeding each batch through addBatch. Every per-batch column carries only
/// the batch-local docs that actually have a value for that field; columns
/// absent from a batch (no doc has a value) are simply not included.
///
/// Java hands `addBatch` an anonymous `ColumnBatch` over `batchSize` and
/// `cols`; `simple_batch(batchSize, cols)` is that batch.
fn build_batch_index(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    docs: &[ParityDoc],
    r: &dyn Random,
) -> Result<()> {
    // Partition into 1..min(4, numDocs) contiguous batches.
    let num_batches = TestUtil::next_int(r, 1, 4.min(1.max(docs.len() as i32)));
    let mut starts = vec![0i32; num_batches as usize + 1];
    starts[0] = 0;
    starts[num_batches as usize] = docs.len() as i32;
    for i in 1..num_batches as usize {
        starts[i] = TestUtil::next_int(
            r,
            starts[i - 1] + 1,
            docs.len() as i32 - (num_batches - i as i32),
        );
    }

    // Field types used on the batch path.
    let mut bin_stored_type = FieldType::new();
    bin_stored_type.set_doc_values_type(DocValuesType::BINARY)?;
    bin_stored_type.set_stored(true)?;
    bin_stored_type.freeze();
    let mut lpoint_type = FieldType::new();
    lpoint_type.set_dimensions(1, 8)?;
    lpoint_type.freeze();
    let tag_type = StringField::type_not_stored();
    let mut vec_type = FieldType::new();
    vec_type.set_vector_attributes(
        2,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
    )?;
    vec_type.freeze();

    let iwc = parity_iwc()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for bi in 0..num_batches as usize {
        let from = starts[bi] as usize;
        let to_excl = starts[bi + 1] as usize;
        let batch_size = (to_excl - from) as i32;
        let mut cols: Vec<AnyColumn> = Vec::new();

        // num: single-valued NUMERIC
        add_long_tuple_column(
            &mut cols,
            "num",
            ft(NumericDocValuesField::type_()),
            docs,
            from,
            to_excl,
            |d| d.num,
        )?;

        // snum: multi-valued SORTED_NUMERIC
        add_long_multi_column(
            &mut cols,
            "snum",
            ft(SortedNumericDocValuesField::type_()),
            docs,
            from,
            to_excl,
            |d| d.snum.as_deref(),
        )?;

        // bin_stored: BINARY DV + stored
        add_binary_tuple_column(
            &mut cols,
            "bin_stored",
            ft(&bin_stored_type),
            docs,
            from,
            to_excl,
            |d| d.bin_stored.as_ref(),
        )?;

        // set: multi-valued SORTED_SET
        add_binary_multi_column(
            &mut cols,
            "set",
            ft(SortedSetDocValuesField::type_()),
            docs,
            from,
            to_excl,
            |d| d.set.as_deref(),
        )?;

        // lpoint: 1-D long point
        add_long_tuple_column_kind(
            &mut cols,
            "lpoint",
            ft(&lpoint_type),
            NumericKind::Long,
            docs,
            from,
            to_excl,
            |d| d.lpoint,
        )?;

        // tag: indexed StringField
        add_binary_tuple_column(&mut cols, "tag", ft(tag_type), docs, from, to_excl, |d| {
            d.tag.as_ref()
        })?;

        // vec: float vectors (sparse)
        add_float_vector_column(&mut cols, "vec", ft(&vec_type), docs, from, to_excl)?;

        w.add_batch(&simple_batch(batch_size, cols))?;
    }
    w.force_merge(1)?;
    w.close()
}

fn add_long_tuple_column(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
    getter: impl Fn(&ParityDoc) -> Option<i64>,
) -> Result<()> {
    add_long_tuple_column_kind(
        cols,
        name,
        field_type,
        NumericKind::Long,
        docs,
        from,
        to_excl,
        getter,
    )
}

#[allow(clippy::too_many_arguments)]
fn add_long_tuple_column_kind(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    kind: NumericKind,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
    getter: impl Fn(&ParityDoc) -> Option<i64>,
) -> Result<()> {
    let mut n = 0;
    for d in &docs[from..to_excl] {
        if getter(d).is_some() {
            n += 1;
        }
    }
    if n == 0 {
        return Ok(());
    }
    let mut doc_ids = vec![0i32; n];
    let mut values = vec![0i64; n];
    let mut p = 0;
    for i in from..to_excl {
        if let Some(v) = getter(&docs[i]) {
            doc_ids[p] = (i - from) as i32;
            values[p] = v;
            p += 1;
        }
    }
    cols.push(ArrayLongColumn::with_kind(name, field_type, kind, doc_ids, values)?.into());
    Ok(())
}

fn add_long_multi_column(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
    getter: impl Fn(&ParityDoc) -> Option<&[i64]>,
) -> Result<()> {
    let mut n = 0;
    for d in &docs[from..to_excl] {
        if let Some(v) = getter(d) {
            n += v.len();
        }
    }
    if n == 0 {
        return Ok(());
    }
    let mut doc_ids = vec![0i32; n];
    let mut values = vec![0i64; n];
    let mut p = 0;
    for i in from..to_excl {
        if let Some(v) = getter(&docs[i]) {
            for &x in v {
                doc_ids[p] = (i - from) as i32;
                values[p] = x;
                p += 1;
            }
        }
    }
    cols.push(ArrayLongColumn::new(name, field_type, doc_ids, values)?.into());
    Ok(())
}

fn add_binary_tuple_column(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
    getter: impl Fn(&ParityDoc) -> Option<&BytesRef>,
) -> Result<()> {
    let mut n = 0;
    for d in &docs[from..to_excl] {
        if getter(d).is_some() {
            n += 1;
        }
    }
    if n == 0 {
        return Ok(());
    }
    let mut doc_ids = vec![0i32; n];
    let mut values = Vec::with_capacity(n);
    let mut p = 0;
    for i in from..to_excl {
        if let Some(v) = getter(&docs[i]) {
            doc_ids[p] = (i - from) as i32;
            values.push(v.clone());
            p += 1;
        }
    }
    cols.push(ArrayBinaryColumn::new(name, field_type, doc_ids, values)?.into());
    Ok(())
}

fn add_binary_multi_column(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
    getter: impl Fn(&ParityDoc) -> Option<&[BytesRef]>,
) -> Result<()> {
    let mut n = 0;
    for d in &docs[from..to_excl] {
        if let Some(v) = getter(d) {
            n += v.len();
        }
    }
    if n == 0 {
        return Ok(());
    }
    let mut doc_ids = vec![0i32; n];
    let mut values = Vec::with_capacity(n);
    let mut p = 0;
    for i in from..to_excl {
        if let Some(v) = getter(&docs[i]) {
            for x in v {
                doc_ids[p] = (i - from) as i32;
                values.push(x.clone());
                p += 1;
            }
        }
    }
    cols.push(ArrayBinaryColumn::new(name, field_type, doc_ids, values)?.into());
    Ok(())
}

fn add_float_vector_column(
    cols: &mut Vec<AnyColumn>,
    name: &str,
    field_type: Arc<dyn IndexableFieldType>,
    docs: &[ParityDoc],
    from: usize,
    to_excl: usize,
) -> Result<()> {
    let mut n = 0;
    for d in &docs[from..to_excl] {
        if d.vec.is_some() {
            n += 1;
        }
    }
    if n == 0 {
        return Ok(());
    }
    let mut doc_ids = vec![0i32; n];
    let mut values = Vec::with_capacity(n);
    let mut p = 0;
    for i in from..to_excl {
        if let Some(vec) = &docs[i].vec {
            doc_ids[p] = (i - from) as i32;
            values.push(vec.clone());
            p += 1;
        }
    }
    cols.push(ArrayFloatVectorColumn::new(name, field_type, doc_ids, values)?.into());
    Ok(())
}

/// Port of `TestColumnBatchIndexing.testRandomizedParityVsDocByDoc`.
fn test_randomized_parity_vs_doc_by_doc() -> Result<()> {
    let r = random();
    let num_docs = TestUtil::next_int(&*r, 30, 80);
    let mut docs = Vec::with_capacity(num_docs as usize);
    for _ in 0..num_docs {
        docs.push(random_parity_doc(&*r));
    }

    let mut batch_dir = LuceneTestCase::new_directory();
    let mut doc_dir = LuceneTestCase::new_directory();
    build_batch_index(&batch_dir, &docs, &*r)?;
    build_doc_by_doc_index(&doc_dir, &docs)?;

    {
        let b_r: Arc<dyn IndexReader> = open_reader(&batch_dir)?;
        let d_r: Arc<dyn IndexReader> = open_reader(&doc_dir)?;
        let b_l = only_leaf(&b_r)?;
        let d_l = only_leaf(&d_r)?;

        assert_equals_msg(Some("maxDoc"), &d_l.max_doc(), &b_l.max_doc());
        assert_equals_msg(Some("numDocs"), &d_l.num_docs(), &b_l.num_docs());
        assert_equals(&num_docs, &b_l.max_doc());

        // Numeric DV
        assert_numeric_doc_values_equal(
            d_l.get_numeric_doc_values("num")?,
            b_l.get_numeric_doc_values("num")?,
        )?;
        // Sorted Numeric DV
        assert_sorted_numeric_doc_values_equal(
            d_l.get_sorted_numeric_doc_values("snum")?,
            b_l.get_sorted_numeric_doc_values("snum")?,
        )?;
        // Binary DV (paired with stored)
        assert_binary_doc_values_equal(
            d_l.get_binary_doc_values("bin_stored")?,
            b_l.get_binary_doc_values("bin_stored")?,
        )?;
        // Sorted Set DV
        assert_sorted_set_doc_values_equal(
            d_l.get_sorted_set_doc_values("set")?,
            b_l.get_sorted_set_doc_values("set")?,
        )?;

        // Stored fields per doc
        let b_sf = b_l.stored_fields()?;
        let d_sf = d_l.stored_fields()?;
        for i in 0..num_docs as usize {
            let expected = &docs[i].bin_stored;
            let d_doc = d_sf.document(i as i32)?;
            let b_doc = b_sf.document(i as i32)?;
            let d_f = d_doc.get_field("bin_stored");
            let b_f = b_doc.get_field("bin_stored");
            match expected {
                None => {
                    assert_null_msg(
                        &format!("doc {i} bin_stored doc-path"),
                        &d_f.map(|f| f.to_string()),
                    );
                    assert_null_msg(
                        &format!("doc {i} bin_stored batch-path"),
                        &b_f.map(|f| f.to_string()),
                    );
                }
                Some(expected) => {
                    let d_f = d_f.unwrap_or_else(|| crate::support::npe("dF"));
                    let b_f = b_f.unwrap_or_else(|| crate::support::npe("bF"));
                    assert_equals(&Some(expected.clone()), &d_f.binary_value());
                    assert_equals(&Some(expected.clone()), &b_f.binary_value());
                }
            }
        }

        // Point query parity
        let d_s = searcher(&d_r)?;
        let b_s = searcher(&b_r)?;
        for pq in [0i64, 100, 5000, 9999] {
            assert_equals_msg(
                Some(&format!("lpoint exact {pq}")),
                &d_s.count(LongPoint::new_exact_query("lpoint", pq)?)?,
                &b_s.count(LongPoint::new_exact_query("lpoint", pq)?)?,
            );
        }
        assert_equals_msg(
            Some("lpoint range"),
            &d_s.count(LongPoint::new_range_query("lpoint", &[0], &[9999])?)?,
            &b_s.count(LongPoint::new_range_query("lpoint", &[0], &[9999])?)?,
        );

        // Term query parity for the inverted "tag" field.
        for i in 0..15 {
            let t = Term::from_text("tag", &format!("tag{i}"));
            assert_equals_msg(
                Some(&format!("tag {t}")),
                &count_term(&d_s, t.clone())?,
                &count_term(&b_s, t)?,
            );
        }

        // Vector parity per doc (codec may reorder storage; iterate by doc-id).
        let d_v = d_l.get_float_vector_values("vec")?;
        let b_v = b_l.get_float_vector_values("vec")?;
        assert_vectors_equal(d_v, b_v, &docs)?;

        b_r.close()?;
        d_r.close()?;
    }
    batch_dir.close()?;
    doc_dir.close()?;
    Ok(())
}

fn assert_numeric_doc_values_equal(
    a: Option<Box<dyn NumericDocValues>>,
    b: Option<Box<dyn NumericDocValues>>,
) -> Result<()> {
    if a.is_none() && b.is_none() {
        return Ok(());
    }
    assert_not_null(&a);
    assert_not_null(&b);
    let (mut a, mut b) = (nn(a), nn(b));
    loop {
        let da = a.next_doc()?;
        if da == NO_MORE_DOCS {
            break;
        }
        let db = b.next_doc()?;
        assert_equals(&da, &db);
        assert_equals_msg(
            Some(&format!("num docID {da}")),
            &a.long_value()?,
            &b.long_value()?,
        );
    }
    assert_equals(&NO_MORE_DOCS, &b.next_doc()?);
    Ok(())
}

fn assert_sorted_numeric_doc_values_equal(
    a: Option<Box<dyn SortedNumericDocValues>>,
    b: Option<Box<dyn SortedNumericDocValues>>,
) -> Result<()> {
    if a.is_none() && b.is_none() {
        return Ok(());
    }
    assert_not_null(&a);
    assert_not_null(&b);
    let (mut a, mut b) = (nn(a), nn(b));
    loop {
        let da = a.next_doc()?;
        if da == NO_MORE_DOCS {
            break;
        }
        let db = b.next_doc()?;
        assert_equals(&da, &db);
        let ca = a.doc_value_count()?;
        assert_equals_msg(
            Some(&format!("snum count docID {da}")),
            &ca,
            &b.doc_value_count()?,
        );
        for _ in 0..ca {
            assert_equals_msg(
                Some(&format!("snum value docID {da}")),
                &a.next_value()?,
                &b.next_value()?,
            );
        }
    }
    assert_equals(&NO_MORE_DOCS, &b.next_doc()?);
    Ok(())
}

fn assert_binary_doc_values_equal(
    a: Option<Box<dyn BinaryDocValues>>,
    b: Option<Box<dyn BinaryDocValues>>,
) -> Result<()> {
    if a.is_none() && b.is_none() {
        return Ok(());
    }
    assert_not_null(&a);
    assert_not_null(&b);
    let (mut a, mut b) = (nn(a), nn(b));
    loop {
        let da = a.next_doc()?;
        if da == NO_MORE_DOCS {
            break;
        }
        let db = b.next_doc()?;
        assert_equals(&da, &db);
        assert_equals_msg(
            Some(&format!("bin docID {da}")),
            &a.binary_value()?,
            &b.binary_value()?,
        );
    }
    assert_equals(&NO_MORE_DOCS, &b.next_doc()?);
    Ok(())
}

fn assert_sorted_set_doc_values_equal(
    a: Option<Box<dyn SortedSetDocValues>>,
    b: Option<Box<dyn SortedSetDocValues>>,
) -> Result<()> {
    if a.is_none() && b.is_none() {
        return Ok(());
    }
    assert_not_null(&a);
    assert_not_null(&b);
    let (mut a, mut b) = (nn(a), nn(b));
    loop {
        let da = a.next_doc()?;
        if da == NO_MORE_DOCS {
            break;
        }
        let db = b.next_doc()?;
        assert_equals(&da, &db);
        let ca = a.doc_value_count()?;
        assert_equals_msg(
            Some(&format!("set count docID {da}")),
            &ca,
            &b.doc_value_count()?,
        );
        for _ in 0..ca {
            let ord_a = a.next_ord()?;
            let va = a.lookup_ord(ord_a)?;
            let ord_b = b.next_ord()?;
            let vb = b.lookup_ord(ord_b)?;
            assert_equals_msg(Some(&format!("set value docID {da}")), &va, &vb);
        }
    }
    assert_equals(&NO_MORE_DOCS, &b.next_doc()?);
    Ok(())
}

fn assert_vectors_equal(
    d_v: Option<Box<dyn FloatVectorValues>>,
    b_v: Option<Box<dyn FloatVectorValues>>,
    docs: &[ParityDoc],
) -> Result<()> {
    if d_v.is_none() && b_v.is_none() {
        return Ok(());
    }
    assert_not_null(&d_v);
    assert_not_null(&b_v);
    let (d_v, b_v) = (nn(d_v), nn(b_v));
    let mut d_it = d_v.iterator()?;
    let mut b_it = b_v.iterator()?;
    loop {
        let n_doc = d_it.next_doc()?;
        if n_doc == NO_MORE_DOCS {
            break;
        }
        assert_equals(&n_doc, &b_it.next_doc()?);
        let expected = &docs[n_doc as usize].vec;
        assert_not_null_msg(&format!("expected vector at doc {n_doc}"), expected);
        let expected = expected.as_ref().expect("checked by assertNotNull");
        assert_array_equals_f32(expected, &d_v.vector_value(d_it.index())?, 0.0);
        assert_array_equals_f32(expected, &b_v.vector_value(b_it.index())?, 0.0);
    }
    assert_equals(&NO_MORE_DOCS, &b_it.next_doc()?);
    Ok(())
}

/// Port of `TestColumnBatchIndexing.testDuplicateColumnNameRejected`.
fn test_duplicate_column_name_rejected() -> Result<()> {
    let c = Ctx::open()?;
    let ids = vec![0, 1];
    let vals = vec![1i64, 2];
    let e = expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            2,
            vec![
                ArrayLongColumn::new("field", ndv(), ids.clone(), vals.clone())?.into(),
                ArrayLongColumn::new("field", ndv(), ids.clone(), vals.clone())?.into(),
            ],
        ))
    });
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(&message, message.contains("field"));

    // Writer still usable after validation failure.
    c.add_batch(&simple_batch(
        1,
        vec![ArrayLongColumn::new("v", ndv(), vec![0], vec![7])?.into()],
    ))?;
    c.w.force_merge(1)?;
    let r = c.reader()?;
    // After forceMerge, fully-deleted segments are pruned; only the recovery doc remains.
    assert_equals(&1, &r.num_docs());
    let leaf = only_leaf(&r)?;
    let dv = leaf.get_numeric_doc_values("v")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    Assert::assert_true(dv.next_doc()? != NO_MORE_DOCS);
    assert_equals(&7i64, &dv.long_value()?);
    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testSameFieldNameAcrossBatchesAllowed`.
fn test_same_field_name_across_batches_allowed() -> Result<()> {
    // Pin a stable IWC so randomized small buffers / merge policies cannot reorder
    // this test asserts doc-id ordering.
    let c = Ctx::open_with(stable_iwc()?)?;
    c.add_batch(&simple_batch(
        1,
        vec![ArrayLongColumn::new("f", ndv(), vec![0], vec![1])?.into()],
    ))?;
    c.add_batch(&simple_batch(
        1,
        vec![ArrayLongColumn::new("f", ndv(), vec![0], vec![2])?.into()],
    ))?;
    c.w.force_merge(1)?;
    let r = c.reader()?;
    {
        let leaf = only_leaf(&r)?;
        let dv = leaf.get_numeric_doc_values("f")?;
        assert_not_null(&dv);
        let mut dv = nn(dv);
        assert_equals(&0, &dv.next_doc()?);
        assert_equals(&1i64, &dv.long_value()?);
        assert_equals(&1, &dv.next_doc()?);
        assert_equals(&2i64, &dv.long_value()?);
        assert_equals(&NO_MORE_DOCS, &dv.next_doc()?);
    }
    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testUpdateDocumentsByTerm`.
fn test_update_documents_by_term() -> Result<()> {
    let c = Ctx::open()?;
    let id_type = id_type();

    let ids12 = vec![nbr("1"), nbr("2")];
    let ids3 = vec![nbr("3")];

    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayBinaryColumn::new("id", ft(&id_type), vec![0, 1], ids12)?.into(),
            ArrayLongColumn::new("value", ndv(), vec![0, 1], vec![100, 200])?.into(),
        ],
    ))?;

    // Update documents where id="1": delete and add new batch
    c.w.update_documents_batch(
        Some(Term::from_text("id", "1")),
        &simple_batch(
            1,
            vec![
                ArrayBinaryColumn::new("id", ft(&id_type), vec![0], ids3)?.into(),
                ArrayLongColumn::new("value", ndv(), vec![0], vec![300])?.into(),
            ],
        ),
    )?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    {
        let s = searcher(&r)?;
        // Old doc with id="1" should be deleted
        assert_equals(&0, &count_term(&s, Term::from_text("id", "1"))?);
        // New doc with id="3" should exist
        assert_equals(&1, &count_term(&s, Term::from_text("id", "3"))?);
        // Doc with id="2" should still exist
        assert_equals(&1, &count_term(&s, Term::from_text("id", "2"))?);

        // Should have 2 live docs: value=200 and value=300
        assert_equals(&2, &r.num_docs());

        let leaf = only_leaf(&r)?;
        let value_dv = leaf.get_numeric_doc_values("value")?;
        assert_not_null(&value_dv);
        let mut value_dv = nn(value_dv);

        let mut values = Vec::new();
        while value_dv.next_doc()? != NO_MORE_DOCS {
            values.push(value_dv.long_value()?);
        }
        values.sort();
        assert_equals(&2, &values.len());
        assert_equals(&200i64, &values[0]);
        assert_equals(&300i64, &values[1]);
    }
    c.close(Some(r))
}

/// Port of `TestColumnBatchIndexing.testUpdateDocumentsByQuery`.
fn test_update_documents_by_query() -> Result<()> {
    let c = Ctx::open()?;
    let id_type = id_type();

    let ids12 = vec![nbr("1"), nbr("2")];
    let ids3 = vec![nbr("3")];

    // Add initial batch with string id field (indexed for TermQuery)
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayBinaryColumn::new("id", ft(&id_type), vec![0, 1], ids12)?.into(),
            ArrayLongColumn::new("value", ndv(), vec![0, 1], vec![100, 200])?.into(),
        ],
    ))?;

    // Update documents where id="2" using a Query: delete and add new batch
    c.w.update_documents_batch_by_query(
        Some(Arc::new(TermQuery::new(Term::from_text("id", "2")))),
        &simple_batch(
            1,
            vec![
                ArrayBinaryColumn::new("id", ft(&id_type), vec![0], ids3)?.into(),
                ArrayLongColumn::new("value", ndv(), vec![0], vec![300])?.into(),
            ],
        ),
    )?;

    c.w.force_merge(1)?;
    let r = c.reader()?;
    {
        let s = searcher(&r)?;
        // Old doc with id="2" should be deleted
        assert_equals(&0, &count_term(&s, Term::from_text("id", "2"))?);
        // New doc with id="3" should exist
        assert_equals(&1, &count_term(&s, Term::from_text("id", "3"))?);
        // Doc with id="1" should still exist
        assert_equals(&1, &count_term(&s, Term::from_text("id", "1"))?);

        assert_equals(&2, &r.num_docs());
    }
    c.close(Some(r))
}
