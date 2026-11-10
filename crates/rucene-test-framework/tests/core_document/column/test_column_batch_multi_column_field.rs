//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchMultiColumnField.java`:
//! tests for composing multiple columns under a single field name in
//! `ColumnBatch.columns()`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::column::NumericKind;
use rucene::document::{Document, Field, FieldType, IntPoint, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::{IndexOptions, IndexReader, VectorSimilarityFunction};
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use super::column_batch_test_util::{
    count_term, field, float_vector_type, frozen, ft, nbr, nn, only_leaf, searcher, simple_batch,
    AnyColumn, ArrayBinaryColumn, ArrayFloatVectorColumn, ArrayLongColumn, Ctx,
};
use crate::core_support::{
    assert_array_equals_f32, assert_equals, assert_not_null, assert_null, class, random, test, IAE,
};

/// `TestColumnBatchMultiColumnField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchMultiColumnField",
        vec![
            test(
                "testComposeStoredAndInverted",
                test_compose_stored_and_inverted,
            ),
            test(
                "testComposeDocValuesAndInverted",
                test_compose_doc_values_and_inverted,
            ),
            test(
                "testComposeOrderIndependent",
                test_compose_order_independent,
            ),
            test(
                "testComposeDocValuesAndPoints",
                test_compose_doc_values_and_points,
            ),
            test(
                "testComposeDocValuesAndVector",
                test_compose_doc_values_and_vector,
            ),
            test(
                "testComposeSparseDisjointDocIds",
                test_compose_sparse_disjoint_doc_ids,
            ),
            test("testComposeAcrossBatches", test_compose_across_batches),
            test(
                "testComposeMixedWithAddDocument",
                test_compose_mixed_with_add_document,
            ),
            test(
                "testRejectsDuplicateInversionFeature",
                test_rejects_duplicate_inversion_feature,
            ),
            test(
                "testRejectsDuplicateStoredFeature",
                test_rejects_duplicate_stored_feature,
            ),
            test(
                "testRejectsDuplicateDocValuesFeature",
                test_rejects_duplicate_doc_values_feature,
            ),
            test(
                "testRejectsDuplicatePointsFeature",
                test_rejects_duplicate_points_feature,
            ),
            test(
                "testRejectsDuplicateVectorFeature",
                test_rejects_duplicate_vector_feature,
            ),
        ],
    )
}

/// `invertedType()`: a frozen `FieldType` for an untokenized inverted-only field.
fn inverted_type() -> Result<FieldType> {
    frozen(|t| {
        t.set_index_options(IndexOptions::DOCS)?;
        t.set_tokenized(false)?;
        t.set_omit_norms(true)
    })
}

/// `storedBinaryType()`: a frozen `FieldType` for a stored-only (non-indexed) field.
fn stored_binary_type() -> Result<FieldType> {
    frozen(|t| t.set_stored(true))
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())))
}

/// `stableIwc()`.
fn stable_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(TieredMergePolicy::new())))?;
    iwc.set_ram_buffer_size_mb(16.0)?;
    // Java names `IndexWriterConfig.DISABLE_AUTO_FLUSH`; the port declares it on `LiveIndexWriterConfig`.
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    Ok(iwc)
}

fn bin_col(
    name: &str,
    t: &FieldType,
    doc_ids: Vec<i32>,
    values: Vec<BytesRef>,
) -> Result<AnyColumn> {
    Ok(ArrayBinaryColumn::new(name, ft(t), doc_ids, values)?.into())
}

/// Port of `testComposeStoredAndInverted`.
fn test_compose_stored_and_inverted() -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    let stored = vec![nbr("Hello World"), nbr("Goodbye World")];
    // Index the lower-cased tokens so the stored and indexed representations differ.
    let indexed = vec![nbr("hello"), nbr("goodbye")];
    c.add_batch(&simple_batch(
        2,
        vec![
            bin_col("body", &stored_binary_type()?, vec![0, 1], stored.clone())?,
            bin_col("body", &inverted_type()?, vec![0, 1], indexed)?,
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // Inverted postings come from the inverted column.
    assert_equals(&1, &count_term(&s, "body", "hello")?);
    assert_equals(&1, &count_term(&s, "body", "goodbye")?);
    // The upper-cased original is not indexed.
    assert_equals(&0, &count_term(&s, "body", "Hello")?);
    // Stored values come from the stored column.
    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some(stored[0].clone()),
        &field(&stored_fields.document(0)?, "body").binary_value(),
    );
    assert_equals(
        &Some(stored[1].clone()),
        &field(&stored_fields.document(1)?, "body").binary_value(),
    );
    c.close(Some(r))
}

/// Port of `testComposeDocValuesAndInverted`.
fn test_compose_doc_values_and_inverted() -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    let tokens = vec![nbr("alpha"), nbr("beta")];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayLongColumn::new(
                "x",
                ft(NumericDocValuesField::type_()),
                vec![0, 1],
                vec![42, 99],
            )?
            .into(),
            bin_col("x", &inverted_type()?, vec![0, 1], tokens)?,
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // Inverted postings come from the binary column.
    assert_equals(&1, &count_term(&s, "x", "alpha")?);
    assert_equals(&1, &count_term(&s, "x", "beta")?);
    // Numeric doc values come from the long column.
    let dv = leaf.get_numeric_doc_values("x")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&42i64, &dv.long_value()?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&99i64, &dv.long_value()?);
    c.close(Some(r))
}

/// Port of `testComposeOrderIndependent`.
fn test_compose_order_independent() -> Result<()> {
    let stored = nbr("Hello World");
    let indexed = nbr("hello");
    let stored_first = index_composed_binary_columns(true, &stored, &indexed)?;
    let inverted_first = index_composed_binary_columns(false, &stored, &indexed)?;
    for mut d in [stored_first, inverted_first] {
        let r: Arc<dyn IndexReader> = rucene::index::directory_reader::open(d.as_directory())?;
        let leaf = only_leaf(&r)?;
        let s = searcher(&r)?;
        assert_equals(&1, &count_term(&s, "body", "hello")?);
        assert_equals(&0, &count_term(&s, "body", "Hello")?);
        assert_equals(
            &Some(stored.clone()),
            &field(&leaf.stored_fields()?.document(0)?, "body").binary_value(),
        );
        r.close()?;
        d.close()?;
    }
    Ok(())
}

/// `indexComposedBinaryColumns(storedFirst, stored, indexed)`.
///
/// Java returns `dir` and leaves `w` unclosed and unreachable; the port
/// leaks the writer the same way (`std::mem::forget`) so dropping it runs
/// nothing Java does not.
fn index_composed_binary_columns(
    stored_first: bool,
    stored: &BytesRef,
    indexed: &BytesRef,
) -> Result<TestDirectory> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let stored_col = bin_col(
        "body",
        &stored_binary_type()?,
        vec![0],
        vec![stored.clone()],
    )?;
    let inverted_col = bin_col("body", &inverted_type()?, vec![0], vec![indexed.clone()])?;
    w.add_batch(&if stored_first {
        simple_batch(1, vec![stored_col, inverted_col])
    } else {
        simple_batch(1, vec![inverted_col, stored_col])
    })?;
    std::mem::forget(w);
    Ok(dir)
}

/// Port of `testComposeDocValuesAndPoints`.
fn test_compose_doc_values_and_points() -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    let point_type = frozen(|t| t.set_dimensions(1, 4))?;
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayLongColumn::new(
                "n",
                ft(NumericDocValuesField::type_()),
                vec![0, 1],
                vec![7, 8],
            )?
            .into(),
            ArrayLongColumn::with_kind(
                "n",
                ft(&point_type),
                NumericKind::Int,
                vec![0, 1],
                vec![7, 8],
            )?
            .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // Points come from the point column.
    assert_equals(&1, &s.count(IntPoint::new_exact_query("n", 7)?)?);
    assert_equals(&2, &s.count(IntPoint::new_range_query("n", &[7], &[8])?)?);
    // NUMERIC doc values come from the doc-values column.
    let dv = leaf.get_numeric_doc_values("n")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&7i64, &dv.long_value()?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&8i64, &dv.long_value()?);
    c.close(Some(r))
}

/// Port of `testComposeDocValuesAndVector`.
fn test_compose_doc_values_and_vector() -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    let vectors = vec![vec![1f32, 2.0], vec![3f32, 4.0]];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayLongColumn::new(
                "v",
                ft(NumericDocValuesField::type_()),
                vec![0, 1],
                vec![11, 22],
            )?
            .into(),
            ArrayFloatVectorColumn::new("v", ft(&vector_type), vec![0, 1], vectors.clone())?.into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let dv = leaf.get_numeric_doc_values("v")?;
    assert_not_null(&dv);
    let mut dv = nn(dv);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&11i64, &dv.long_value()?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&22i64, &dv.long_value()?);
    let fvv = leaf.get_float_vector_values("v")?;
    assert_not_null(&fvv);
    let fvv = nn(fvv);
    let mut it = fvv.iterator()?;
    for i in 0..2 {
        assert_equals(&i, &it.next_doc()?);
        assert_array_equals_f32(&vectors[i as usize], &fvv.vector_value(it.index())?, 0.0);
    }
    assert_equals(&NO_MORE_DOCS, &it.next_doc()?);
    c.close(Some(r))
}

/// Port of `testComposeSparseDisjointDocIds`.
fn test_compose_sparse_disjoint_doc_ids() -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    c.add_batch(&simple_batch(
        2,
        vec![
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("kept")])?,
            bin_col("body", &inverted_type()?, vec![1], vec![nbr("token")])?,
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    // The token is indexed on doc 1 only.
    assert_equals(&1, &count_term(&s, "body", "token")?);
    let stored_fields = leaf.stored_fields()?;
    // doc 0 carries the stored value; doc 1 has no stored "body".
    assert_equals(
        &Some(nbr("kept")),
        &field(&stored_fields.document(0)?, "body").binary_value(),
    );
    assert_null(
        &stored_fields
            .document(1)?
            .get_field("body")
            .map(|f| f.name().to_string()),
    );
    c.close(Some(r))
}

/// Port of `testComposeAcrossBatches`.
fn test_compose_across_batches() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;
    c.add_batch(&simple_batch(
        1,
        vec![
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("Hello")])?,
            bin_col("body", &inverted_type()?, vec![0], vec![nbr("hello")])?,
        ],
    ))?;
    c.add_batch(&simple_batch(
        1,
        vec![
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("World")])?,
            bin_col("body", &inverted_type()?, vec![0], vec![nbr("world")])?,
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "body", "hello")?);
    assert_equals(&1, &count_term(&s, "body", "world")?);
    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some(nbr("Hello")),
        &field(&stored_fields.document(0)?, "body").binary_value(),
    );
    assert_equals(
        &Some(nbr("World")),
        &field(&stored_fields.document(1)?, "body").binary_value(),
    );
    c.close(Some(r))
}

/// Port of `testComposeMixedWithAddDocument`.
fn test_compose_mixed_with_add_document() -> Result<()> {
    let c = Ctx::open_with(stable_iwc()?)?;
    // A field type that is both stored and inverted, matching the merged schema produced by
    // composing storedBinaryType() + invertedType() in a batch.
    let combined = frozen(|t| {
        t.set_stored(true)?;
        t.set_index_options(IndexOptions::DOCS)?;
        t.set_tokenized(false)?;
        t.set_omit_norms(true)
    })?;
    // doc 0 via addDocument establishes the FieldInfo (and the frozen-field fast path).
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("body", "alpha".to_string(), combined)?));
    c.w.add_document(&doc)?;
    // doc 1 via a composed batch for the same field name, in the same segment.
    c.add_batch(&simple_batch(
        1,
        vec![
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("Beta")])?,
            bin_col("body", &inverted_type()?, vec![0], vec![nbr("beta")])?,
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "body", "alpha")?);
    assert_equals(&1, &count_term(&s, "body", "beta")?);
    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some("alpha".to_string()),
        &field(&stored_fields.document(0)?, "body").string_value(),
    );
    assert_equals(
        &Some(nbr("Beta")),
        &field(&stored_fields.document(1)?, "body").binary_value(),
    );
    c.close(Some(r))
}

/// `expectThrows(IllegalArgumentException.class, () -> w.addBatch(simpleBatch(1, columns())))`
/// on a `MockAnalyzer` writer, then `w.close(); dir.close();`.
fn expect_rejected(columns: impl FnOnce() -> Result<Vec<AnyColumn>>) -> Result<()> {
    let c = Ctx::open_with(mock_iwc())?;
    expect_throws(IAE, || c.add_batch(&simple_batch(1, columns()?)));
    c.close(None)
}

/// Port of `testRejectsDuplicateInversionFeature`.
fn test_rejects_duplicate_inversion_feature() -> Result<()> {
    expect_rejected(|| {
        Ok(vec![
            bin_col("body", &inverted_type()?, vec![0], vec![nbr("a")])?,
            bin_col("body", &inverted_type()?, vec![0], vec![nbr("b")])?,
        ])
    })
}

/// Port of `testRejectsDuplicateStoredFeature`.
fn test_rejects_duplicate_stored_feature() -> Result<()> {
    expect_rejected(|| {
        Ok(vec![
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("a")])?,
            bin_col("body", &stored_binary_type()?, vec![0], vec![nbr("b")])?,
        ])
    })
}

/// Port of `testRejectsDuplicateDocValuesFeature`.
fn test_rejects_duplicate_doc_values_feature() -> Result<()> {
    expect_rejected(|| {
        Ok(vec![
            ArrayLongColumn::new("x", ft(NumericDocValuesField::type_()), vec![0], vec![1])?.into(),
            ArrayLongColumn::new("x", ft(NumericDocValuesField::type_()), vec![0], vec![2])?.into(),
        ])
    })
}

/// Port of `testRejectsDuplicatePointsFeature`.
fn test_rejects_duplicate_points_feature() -> Result<()> {
    let point_type = frozen(|t| t.set_dimensions(1, 4))?;
    expect_rejected(|| {
        Ok(vec![
            ArrayLongColumn::with_kind("p", ft(&point_type), NumericKind::Int, vec![0], vec![1])?
                .into(),
            ArrayLongColumn::with_kind("p", ft(&point_type), NumericKind::Int, vec![0], vec![2])?
                .into(),
        ])
    })
}

/// Port of `testRejectsDuplicateVectorFeature`.
fn test_rejects_duplicate_vector_feature() -> Result<()> {
    let vector_type = float_vector_type(2, VectorSimilarityFunction::EUCLIDEAN)?;
    expect_rejected(|| {
        Ok(vec![
            ArrayFloatVectorColumn::new("v", ft(&vector_type), vec![0], vec![vec![1.0, 2.0]])?
                .into(),
            ArrayFloatVectorColumn::new("v", ft(&vector_type), vec![0], vec![vec![3.0, 4.0]])?
                .into(),
        ])
    })
}
