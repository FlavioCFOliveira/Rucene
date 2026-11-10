//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchDictionaryColumn.java`:
//! tests for `DictionaryColumn` end-to-end indexing.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, FieldType, NumericDocValuesField, SortedDocValuesField, SortedSetDocValuesField,
    StoredValueType, TextField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::{DocValuesType, IndexReader};
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use super::column_batch_test_util::{
    br, count_term, field, frozen, ft, nn, only_leaf, searcher, simple_batch, string_like,
    ArrayDenseDictionaryColumn, ArrayDenseLongColumn, ArrayDictionaryColumn, Ctx,
};
use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, class, random, test, JavaHashSet, IAE,
};

/// `TestColumnBatchDictionaryColumn`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchDictionaryColumn",
        vec![
            test("testSortedSparse", test_sorted_sparse),
            test(
                "testSortedSparseLargeBatchReusesDictionary",
                test_sorted_sparse_large_batch_reuses_dictionary,
            ),
            test("testSortedDense", test_sorted_dense),
            test("testSortedSet", test_sorted_set),
            test(
                "testSortedSetDeduplicatesWithinDoc",
                test_sorted_set_deduplicates_within_doc,
            ),
            test("testStoredField", test_stored_field),
            test(
                "testDictionaryWithDuplicateEntries",
                test_dictionary_with_duplicate_entries,
            ),
            test(
                "testMultipleBatchesDifferentDictionaries",
                test_multiple_batches_different_dictionaries,
            ),
            test("testMixedWithPlainDocument", test_mixed_with_plain_document),
            test(
                "testOutOfRangeOrdSparseThrows",
                test_out_of_range_ord_sparse_throws,
            ),
            test(
                "testOutOfRangeOrdDenseThrows",
                test_out_of_range_ord_dense_throws,
            ),
            test("testEmptyDictionaryThrows", test_empty_dictionary_throws),
            test(
                "testNullDictionaryEntryThrows",
                test_null_dictionary_entry_throws,
            ),
            test(
                "testIncompatibleDocValuesTypeThrows",
                test_incompatible_doc_values_type_throws,
            ),
            test("testRejectsBinaryDocValues", test_rejects_binary_doc_values),
            test("testRejectsPoints", test_rejects_points),
            test(
                "testRejectsNumericStoredType",
                test_rejects_numeric_stored_type,
            ),
            test(
                "testInvertedDictionaryColumn",
                test_inverted_dictionary_column,
            ),
            test(
                "testInvertedWithSortedDocValues",
                test_inverted_with_sorted_doc_values,
            ),
            test(
                "testInvertedWithStoredBinary",
                test_inverted_with_stored_binary,
            ),
            test(
                "testInvertedWithStoredString",
                test_inverted_with_stored_string,
            ),
            test(
                "testTokenizedDictionaryColumn",
                test_tokenized_dictionary_column,
            ),
            test(
                "testTokenizedWithStoredString",
                test_tokenized_with_stored_string,
            ),
            test("testRandomSortedCrossCheck", test_random_sorted_cross_check),
            test(
                "testRandomSortedSetCrossCheck",
                test_random_sorted_set_cross_check,
            ),
        ],
    )
}

/// `COLORS`.
fn colors() -> Vec<BytesRef> {
    vec![br("blue"), br("green"), br("red")]
}

fn sorted_type() -> Arc<dyn rucene::index::IndexableFieldType> {
    ft(SortedDocValuesField::type_())
}

fn sorted_set_type() -> Arc<dyn rucene::index::IndexableFieldType> {
    ft(SortedSetDocValuesField::type_())
}

/// Port of `testSortedSparse`.
fn test_sorted_sparse() -> Result<()> {
    let c = Ctx::open()?;
    // docIds 0,1,2 with ords 2,0,1 → "red","blue","green"
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::new(
            "color",
            sorted_type(),
            colors(),
            vec![0, 1, 2],
            vec![2, 0, 1],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let sdv = leaf.get_sorted_doc_values("color")?;
    assert_not_null(&sdv);
    let mut sdv = nn(sdv);
    assert_equals(&0, &sdv.next_doc()?);
    assert_equals(&br("red"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&1, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&2, &sdv.next_doc()?);
    assert_equals(&br("green"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&NO_MORE_DOCS, &sdv.next_doc()?);
    assert_equals(&3, &sdv.get_value_count()?); // 3 distinct terms used
    c.close(Some(r))
}

/// Port of `testSortedSparseLargeBatchReusesDictionary`: many sparse docs
/// reusing a small dictionary exercise the sparse SORTED bulk path's
/// ord→hash translation table; the unused dictionary entry must not be
/// materialized in the segment's term universe.
fn test_sorted_sparse_large_batch_reuses_dictionary() -> Result<()> {
    let c = Ctx::open()?;
    let n = 1000;
    let doc_ids: Vec<i32> = (0..n).collect();
    // only ords 0 and 1 used; ord 2 ("red") is in the dictionary but unused
    let ords: Vec<i32> = (0..n).map(|i| i % 2).collect();
    c.add_batch(&simple_batch(
        n,
        vec![ArrayDictionaryColumn::new("color", sorted_type(), colors(), doc_ids, ords)?.into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let sdv = leaf.get_sorted_doc_values("color")?;
    assert_not_null(&sdv);
    let mut sdv = nn(sdv);
    // Only two distinct terms ("blue", "green") were used.
    assert_equals(&2, &sdv.get_value_count()?);
    for i in 0..n {
        assert_equals(&i, &sdv.next_doc()?);
        let expected = if i % 2 == 0 { br("blue") } else { br("green") };
        assert_equals(&expected, &sdv.lookup_ord(sdv.ord_value()?)?);
    }
    assert_equals(&NO_MORE_DOCS, &sdv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSortedDense`.
fn test_sorted_dense() -> Result<()> {
    let c = Ctx::open()?;
    // ords: blue, blue, red
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayDenseDictionaryColumn::new("color", sorted_type(), colors(), vec![0, 0, 2])?
                .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let sdv = leaf.get_sorted_doc_values("color")?;
    assert_not_null(&sdv);
    let mut sdv = nn(sdv);
    assert_equals(&0, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&1, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&2, &sdv.next_doc()?);
    assert_equals(&br("red"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&NO_MORE_DOCS, &sdv.next_doc()?);
    // green was in the dictionary but never used; getValueCount must reflect only used terms
    assert_equals(&2, &sdv.get_value_count()?);
    c.close(Some(r))
}

/// `ssdv.lookupOrd(ssdv.nextOrd())`.
fn next_term(ssdv: &mut Box<dyn rucene::index::SortedSetDocValues>) -> Result<BytesRef> {
    let o = ssdv.next_ord()?;
    ssdv.lookup_ord(o)
}

/// Port of `testSortedSet`.
fn test_sorted_set() -> Result<()> {
    let c = Ctx::open()?;
    // doc 0: blue, green  doc 1: red  doc 2: blue, red
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::new(
            "color",
            sorted_set_type(),
            colors(),
            vec![0, 0, 1, 2, 2],
            vec![0, 1, 2, 0, 2],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let ssdv = leaf.get_sorted_set_doc_values("color")?;
    assert_not_null(&ssdv);
    let mut ssdv = nn(ssdv);
    // doc 0: blue=0, green=1 (sorted ords)
    assert_equals(&0, &ssdv.next_doc()?);
    assert_equals(&2, &ssdv.doc_value_count()?);
    assert_equals(&br("blue"), &next_term(&mut ssdv)?);
    assert_equals(&br("green"), &next_term(&mut ssdv)?);
    // doc 1: red
    assert_equals(&1, &ssdv.next_doc()?);
    assert_equals(&1, &ssdv.doc_value_count()?);
    assert_equals(&br("red"), &next_term(&mut ssdv)?);
    // doc 2: blue, red
    assert_equals(&2, &ssdv.next_doc()?);
    assert_equals(&2, &ssdv.doc_value_count()?);
    assert_equals(&br("blue"), &next_term(&mut ssdv)?);
    assert_equals(&br("red"), &next_term(&mut ssdv)?);
    assert_equals(&NO_MORE_DOCS, &ssdv.next_doc()?);
    c.close(Some(r))
}

/// Port of `testSortedSetDeduplicatesWithinDoc`.
fn test_sorted_set_deduplicates_within_doc() -> Result<()> {
    let c = Ctx::open()?;
    // doc 0: blue, blue, red — "blue" should appear only once
    c.add_batch(&simple_batch(
        1,
        vec![ArrayDictionaryColumn::new(
            "color",
            sorted_set_type(),
            colors(),
            vec![0, 0, 0],
            vec![0, 0, 2],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut ssdv = nn(leaf.get_sorted_set_doc_values("color")?);
    assert_equals(&0, &ssdv.next_doc()?);
    assert_equals(&2, &ssdv.doc_value_count()?); // blue + red, not blue + blue + red
    assert_equals(&br("blue"), &next_term(&mut ssdv)?);
    assert_equals(&br("red"), &next_term(&mut ssdv)?);
    c.close(Some(r))
}

/// Port of `testStoredField`.
fn test_stored_field() -> Result<()> {
    let c = Ctx::open()?;
    let stored_sorted = FieldType::new_from(SortedDocValuesField::type_());
    stored_sorted.set_stored(true)?;
    stored_sorted.freeze();
    c.add_batch(&simple_batch(
        2,
        vec![ArrayDictionaryColumn::new(
            "color",
            ft(&stored_sorted),
            colors(),
            vec![0, 1],
            vec![2, 0],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let sf = leaf.stored_fields()?;
    let doc0 = sf.document(0)?;
    let doc1 = sf.document(1)?;
    assert_equals(
        "red",
        nn(doc0.get_binary_value("color"))
            .utf8_to_string()?
            .as_str(),
    );
    assert_equals(
        "blue",
        nn(doc1.get_binary_value("color"))
            .utf8_to_string()?
            .as_str(),
    );
    c.close(Some(r))
}

/// Port of `testDictionaryWithDuplicateEntries`.
fn test_dictionary_with_duplicate_entries() -> Result<()> {
    let c = Ctx::open()?;
    // Dictionary: ["blue", "blue", "red"] — two slots for blue
    let dict = vec![br("blue"), br("blue"), br("red")];
    // ords 0 and 1 both mean "blue"; ord 2 means "red"
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayDictionaryColumn::new("color", sorted_type(), dict, vec![0, 1], vec![0, 1])?
                .into(),
        ],
    ))?; // doc 0 → ord 0 (blue), doc 1 → ord 1 (also blue)
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut sdv = nn(leaf.get_sorted_doc_values("color")?);
    // Both docs have "blue"; getValueCount should be 1 (not 2)
    assert_equals(&0, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&1, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&1, &sdv.get_value_count()?);
    c.close(Some(r))
}

/// Port of `testMultipleBatchesDifferentDictionaries`.
fn test_multiple_batches_different_dictionaries() -> Result<()> {
    let c = Ctx::open()?;
    let dict1 = vec![br("blue"), br("green")];
    let dict2 = vec![br("green"), br("purple"), br("red")];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayDictionaryColumn::new("color", sorted_type(), dict1, vec![0, 1], vec![0, 1])?
                .into(),
        ],
    ))?;
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayDictionaryColumn::new("color", sorted_type(), dict2, vec![0, 1], vec![1, 2])?
                .into(),
        ],
    ))?; // purple, red
    c.w.force_merge(1)?; // merge to one segment; ordinals remapped globally
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let sdv = leaf.get_sorted_doc_values("color")?;
    assert_not_null(&sdv);
    let mut sdv = nn(sdv);
    // After merge, all 4 docs present with correct values
    for i in 0..4 {
        assert_equals(&i, &sdv.next_doc()?);
    }
    // Verify total distinct values = 4
    assert_equals(&4, &sdv.get_value_count()?);
    c.close(Some(r))
}

/// Port of `testMixedWithPlainDocument`.
fn test_mixed_with_plain_document() -> Result<()> {
    // Disable auto-flush so the addBatch and addDocument share one segment; otherwise a
    // randomized tiny maxBufferedDocs can split them and TMP's size-sorted forceMerge
    // may interleave them.
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(TieredMergePolicy::new())))?;
    iwc.set_ram_buffer_size_mb(16.0)?;
    // Java names `IndexWriterConfig.DISABLE_AUTO_FLUSH`; the port declares it on `LiveIndexWriterConfig`.
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    let c = Ctx::open_with(iwc)?;
    // Batch adds doc 0 (blue) and doc 1 (red)
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayDictionaryColumn::new("color", sorted_type(), colors(), vec![0, 1], vec![0, 2])?
                .into(),
        ],
    ))?;
    // Plain document adds "green" for doc 2
    let mut doc = Document::new();
    doc.add(Box::new(SortedDocValuesField::new("color", br("green"))));
    c.w.add_document(&doc)?;
    c.w.force_merge(1)?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let mut sdv = nn(leaf.get_sorted_doc_values("color")?);
    assert_equals(&3, &sdv.get_value_count()?); // blue, green, red
                                                // Verify each doc
    assert_equals(&0, &sdv.next_doc()?);
    assert_equals(&br("blue"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&1, &sdv.next_doc()?);
    assert_equals(&br("red"), &sdv.lookup_ord(sdv.ord_value()?)?);
    assert_equals(&2, &sdv.next_doc()?);
    assert_equals(&br("green"), &sdv.lookup_ord(sdv.ord_value()?)?);
    c.close(Some(r))
}

/// Port of `testOutOfRangeOrdSparseThrows`.
fn test_out_of_range_ord_sparse_throws() -> Result<()> {
    let c = Ctx::open()?;
    let col = ArrayDictionaryColumn::new(
        "color",
        sorted_type(),
        colors(), // length 3
        vec![0],
        vec![5], // ord 5 is out of range
    )?;
    expect_throws(IAE, || c.add_batch(&simple_batch(1, vec![col.into()])));
    c.close(None)
}

/// Port of `testOutOfRangeOrdDenseThrows`.
fn test_out_of_range_ord_dense_throws() -> Result<()> {
    let c = Ctx::open()?;
    let col = ArrayDenseDictionaryColumn::new(
        "color",
        sorted_type(),
        colors(),    // length 3
        vec![0, 99], // ord 99 out of range
    )?;
    expect_throws(IAE, || c.add_batch(&simple_batch(2, vec![col.into()])));
    c.close(None)
}

/// Port of `testEmptyDictionaryThrows`.
fn test_empty_dictionary_throws() -> Result<()> {
    expect_throws(IAE, || {
        ArrayDictionaryColumn::new("f", sorted_type(), vec![], vec![], vec![])
    });
    Ok(())
}

/// Port of `testNullDictionaryEntryThrows`.
fn test_null_dictionary_entry_throws() -> Result<()> {
    let dict = vec![Some(br("a")), None];
    expect_throws(IAE, || {
        ArrayDictionaryColumn::from_nullable("f", sorted_type(), dict, vec![], vec![])
    });
    Ok(())
}

/// `try (IndexWriter w = new IndexWriter(dir, newIndexWriterConfig())) { body(w) }`
/// over a `dir` the Java test never closes (the suite's `CloseableDirectory`
/// does).
fn with_writer(body: impl FnOnce(&Arc<IndexWriter>) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let outcome = body(&w);
    let closed = w.close();
    // `dir` stays open: `newDirectory` registered it for closing after the suite.
    drop(dir);
    outcome?;
    closed
}

/// Port of `testIncompatibleDocValuesTypeThrows`.
fn test_incompatible_doc_values_type_throws() -> Result<()> {
    // NUMERIC is not allowed for DictionaryColumn
    let numeric_type = frozen(|t| t.set_doc_values_type(DocValuesType::NUMERIC))?;
    with_writer(|w| {
        let col = ArrayDictionaryColumn::new("f", ft(&numeric_type), colors(), vec![0], vec![0])?;
        expect_throws(IAE, || w.add_batch(&simple_batch(1, vec![col.into()])));
        Ok(())
    })
}

/// Port of `testRejectsBinaryDocValues`.
fn test_rejects_binary_doc_values() -> Result<()> {
    // BINARY DV writer doesn't dedup terms, so DictionaryColumn buys nothing there;
    // validation should redirect to BinaryColumn.
    let binary_type = frozen(|t| t.set_doc_values_type(DocValuesType::BINARY))?;
    with_writer(|w| {
        let col = ArrayDictionaryColumn::new("f", ft(&binary_type), colors(), vec![0], vec![0])?;
        let e = expect_throws(IAE, || w.add_batch(&simple_batch(1, vec![col.into()])));
        let message = e.message().unwrap_or_default().to_string();
        Assert::assert_true_msg(&message, message.contains("BinaryColumn"));
        Ok(())
    })
}

/// Port of `testRejectsPoints`.
fn test_rejects_points() -> Result<()> {
    let point_type = FieldType::new_from(SortedDocValuesField::type_());
    point_type.set_dimensions(1, 4)?;
    point_type.freeze();
    with_writer(|w| {
        let col = ArrayDictionaryColumn::new("f", ft(&point_type), colors(), vec![0], vec![0])?;
        expect_throws(IAE, || w.add_batch(&simple_batch(1, vec![col.into()])));
        Ok(())
    })
}

/// Port of `testRejectsNumericStoredType`.
fn test_rejects_numeric_stored_type() -> Result<()> {
    // storedType() returning a numeric type should be rejected during validation
    let stored_sorted = FieldType::new_from(SortedDocValuesField::type_());
    stored_sorted.set_stored(true)?;
    stored_sorted.freeze();
    let c = Ctx::open()?;
    // Override storedType() to return INTEGER — rejected by validateDictionaryColumn
    let col = ArrayDictionaryColumn::with_stored_type(
        "f",
        ft(&stored_sorted),
        colors(),
        vec![0],
        vec![0],
        StoredValueType::INTEGER,
    )?;
    expect_throws(IAE, || c.add_batch(&simple_batch(1, vec![col.into()])));
    c.close(None)
}

/// Port of `testInvertedDictionaryColumn`.
fn test_inverted_dictionary_column() -> Result<()> {
    let c = Ctx::open()?;
    let string_type = string_like(|_| Ok(()))?;
    // ords 0,2,0 → "blue","red","blue"
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::new(
            "tag",
            ft(&string_type),
            colors(),
            vec![0, 1, 2],
            vec![0, 2, 0],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "tag", "blue")?);
    assert_equals(&1, &count_term(&s, "tag", "red")?);
    assert_equals(&0, &count_term(&s, "tag", "green")?);
    c.close(Some(r))
}

/// Port of `testInvertedWithSortedDocValues`.
fn test_inverted_with_sorted_doc_values() -> Result<()> {
    let c = Ctx::open()?;
    let inverted_dv_type = string_like(|t| t.set_doc_values_type(DocValuesType::SORTED))?;
    // ords 0,2,0 → "blue","red","blue"
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::new(
            "color",
            ft(&inverted_dv_type),
            colors(),
            vec![0, 1, 2],
            vec![0, 2, 0],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    // Verify inverted index
    assert_equals(&2, &count_term(&s, "color", "blue")?);
    assert_equals(&1, &count_term(&s, "color", "red")?);
    // Verify doc values
    let leaf = only_leaf(&r)?;
    let mut dv = nn(leaf.get_sorted_doc_values("color")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&br("blue"), &dv.lookup_ord(dv.ord_value()?)?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&br("red"), &dv.lookup_ord(dv.ord_value()?)?);
    assert_equals(&2, &dv.next_doc()?);
    assert_equals(&br("blue"), &dv.lookup_ord(dv.ord_value()?)?);
    c.close(Some(r))
}

/// Port of `testInvertedWithStoredBinary`.
fn test_inverted_with_stored_binary() -> Result<()> {
    let c = Ctx::open()?;
    let t = string_like(|t| t.set_stored(true))?;
    // ords 0,1,2 → "blue","green","red"
    c.add_batch(&simple_batch(
        3,
        vec![
            ArrayDictionaryColumn::new("color", ft(&t), colors(), vec![0, 1, 2], vec![0, 1, 2])?
                .into(),
        ],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "color", "blue")?);
    assert_equals(&1, &count_term(&s, "color", "green")?);
    assert_equals(&1, &count_term(&s, "color", "red")?);
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for (i, want) in ["blue", "green", "red"].iter().enumerate() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(&Some(br(want)), &field(&doc, "color").binary_value());
    }
    c.close(Some(r))
}

/// Port of `testInvertedWithStoredString`.
fn test_inverted_with_stored_string() -> Result<()> {
    let c = Ctx::open()?;
    let t = string_like(|t| t.set_stored(true))?;
    // ords 0,1,2 → "blue","green","red"; storedType overridden to STRING
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::with_stored_type(
            "color",
            ft(&t),
            colors(),
            vec![0, 1, 2],
            vec![0, 1, 2],
            StoredValueType::STRING,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "color", "blue")?);
    let leaf = only_leaf(&r)?;
    let stored_fields = leaf.stored_fields()?;
    for (i, want) in ["blue", "green", "red"].iter().enumerate() {
        let doc = stored_fields.document(i as i32)?;
        assert_equals(
            &Some(want.to_string()),
            &field(&doc, "color").string_value(),
        );
    }
    c.close(Some(r))
}

/// Port of `testTokenizedDictionaryColumn`.
fn test_tokenized_dictionary_column() -> Result<()> {
    let c = Ctx::open_mock_analyzer()?;
    let phrases = vec![
        br("quick brown fox"),
        br("lazy brown dog"),
        br("quick fox jumps"),
    ];
    // ords 0,1,2 → each phrase goes to a separate doc
    c.add_batch(&simple_batch(
        3,
        vec![ArrayDictionaryColumn::new(
            "text",
            ft(TextField::type_not_stored()),
            phrases,
            vec![0, 1, 2],
            vec![0, 1, 2],
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "text", "quick")?);
    assert_equals(&2, &count_term(&s, "text", "brown")?);
    assert_equals(&2, &count_term(&s, "text", "fox")?);
    assert_equals(&1, &count_term(&s, "text", "lazy")?);
    assert_equals(&1, &count_term(&s, "text", "dog")?);
    assert_equals(&1, &count_term(&s, "text", "jumps")?);
    assert_equals(&0, &count_term(&s, "text", "missing")?);
    c.close(Some(r))
}

/// Port of `testTokenizedWithStoredString`.
fn test_tokenized_with_stored_string() -> Result<()> {
    let c = Ctx::open_mock_analyzer()?;
    let phrases = vec![br("hello world"), br("goodbye world")];
    c.add_batch(&simple_batch(
        2,
        vec![ArrayDictionaryColumn::with_stored_type(
            "text",
            ft(TextField::type_stored()),
            phrases,
            vec![0, 1],
            vec![0, 1],
            StoredValueType::STRING,
        )?
        .into()],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "text", "world")?);
    assert_equals(&1, &count_term(&s, "text", "hello")?);
    let stored_fields = leaf.stored_fields()?;
    assert_equals(
        &Some("hello world".to_string()),
        &field(&stored_fields.document(0)?, "text").string_value(),
    );
    assert_equals(
        &Some("goodbye world".to_string()),
        &field(&stored_fields.document(1)?, "text").string_value(),
    );
    c.close(Some(r))
}

/// The two writers of a cross-check: `try (Directory dictDir = newDirectory();
/// Directory plainDir = newDirectory(); IndexWriter dictW = …; IndexWriter
/// plainW = …)`, closed in reverse order.
struct CrossCheck {
    dict_dir: rucene_test_framework::util::lucene_test_case::TestDirectory,
    plain_dir: rucene_test_framework::util::lucene_test_case::TestDirectory,
    dict_w: Arc<IndexWriter>,
    plain_w: Arc<IndexWriter>,
}

impl CrossCheck {
    fn open() -> Result<Self> {
        let dict_dir = LuceneTestCase::new_directory();
        let plain_dir = LuceneTestCase::new_directory();
        let dict_w = IndexWriter::new(
            dict_dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let plain_w = IndexWriter::new(
            plain_dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        Ok(Self {
            dict_dir,
            plain_dir,
            dict_w,
            plain_w,
        })
    }

    fn close(mut self) -> Result<()> {
        self.plain_w.close()?;
        self.dict_w.close()?;
        self.plain_dir.close()?;
        self.dict_dir.close()
    }

    /// `if (random().nextBoolean()) { forceMerge(1) both }`, then the two
    /// readers (closed by the caller in reverse order).
    fn readers(&self) -> Result<(Arc<dyn IndexReader>, Arc<dyn IndexReader>)> {
        if random().next_boolean() {
            self.dict_w.force_merge(1)?;
            self.plain_w.force_merge(1)?;
        }
        let dict_r: Arc<dyn IndexReader> =
            rucene::index::directory_reader::open_with_writer(&*self.dict_w)?;
        let plain_r: Arc<dyn IndexReader> =
            rucene::index::directory_reader::open_with_writer(&*self.plain_w)?;
        Ok((dict_r, plain_r))
    }
}

/// Port of `testRandomSortedCrossCheck`.
fn test_random_sorted_cross_check() -> Result<()> {
    let iters = LuceneTestCase::at_least(20);
    for _iter in 0..iters {
        let dict_size = TestUtil::next_int(&*random(), 1, 32);
        let dict = random_dictionary(dict_size);
        let dense = random().next_boolean();
        let num_batches = TestUtil::next_int(&*random(), 1, 4);
        let cc = CrossCheck::open()?;
        // Per-insertion-id expected term, or null if the doc has no value.
        let mut expected: Vec<Option<BytesRef>> = Vec::new();
        let mut next_id = 0i64;
        for _b in 0..num_batches {
            let batch_docs = TestUtil::next_int(&*random(), 1, 60);
            let mut ids = vec![0i64; batch_docs as usize];
            if dense {
                let mut ords = vec![0i32; batch_docs as usize];
                for i in 0..batch_docs as usize {
                    let ord = random().next_int_bound(dict_size);
                    ords[i] = ord;
                    ids[i] = next_id;
                    let term = dict[ord as usize].clone();
                    expected.push(Some(term.clone()));
                    let mut plain_doc = Document::new();
                    plain_doc.add(Box::new(SortedDocValuesField::new("f", term)));
                    plain_doc.add(Box::new(NumericDocValuesField::new("id", next_id)));
                    cc.plain_w.add_document(&plain_doc)?;
                    next_id += 1;
                }
                cc.dict_w.add_batch(&simple_batch(
                    batch_docs,
                    vec![
                        ArrayDenseDictionaryColumn::new("f", sorted_type(), dict.clone(), ords)?
                            .into(),
                        ArrayDenseLongColumn::new("id", ft(NumericDocValuesField::type_()), ids)?
                            .into(),
                    ],
                ))?;
            } else {
                let mut ds = Vec::new();
                let mut os = Vec::new();
                for i in 0..batch_docs as usize {
                    ids[i] = next_id;
                    let mut plain_doc = Document::new();
                    plain_doc.add(Box::new(NumericDocValuesField::new("id", next_id)));
                    if random().next_int_bound(10) < 7 {
                        let ord = random().next_int_bound(dict_size);
                        ds.push(i as i32);
                        os.push(ord);
                        plain_doc.add(Box::new(SortedDocValuesField::new(
                            "f",
                            dict[ord as usize].clone(),
                        )));
                        expected.push(Some(dict[ord as usize].clone()));
                    } else {
                        expected.push(None);
                    }
                    cc.plain_w.add_document(&plain_doc)?;
                    next_id += 1;
                }
                cc.dict_w.add_batch(&simple_batch(
                    batch_docs,
                    vec![
                        ArrayDictionaryColumn::new("f", sorted_type(), dict.clone(), ds, os)?
                            .into(),
                        ArrayDenseLongColumn::new("id", ft(NumericDocValuesField::type_()), ids)?
                            .into(),
                    ],
                ))?;
            }
        }
        let (dict_r, plain_r) = cc.readers()?;
        let outcome = assert_sorted_readers_equal("f", "id", &dict_r, &plain_r, &expected);
        plain_r.close()?;
        dict_r.close()?;
        outcome?;
        cc.close()?;
    }
    Ok(())
}

/// Port of `testRandomSortedSetCrossCheck`.
fn test_random_sorted_set_cross_check() -> Result<()> {
    let iters = LuceneTestCase::at_least(20);
    for _iter in 0..iters {
        let dict_size = TestUtil::next_int(&*random(), 1, 16);
        let dict = random_dictionary(dict_size);
        let num_batches = TestUtil::next_int(&*random(), 1, 4);
        // Per-insertion-id expected term SETS (deduped across the doc).
        let mut expected: Vec<JavaHashSet<BytesRef>> = Vec::new();
        let mut next_id = 0i64;
        let cc = CrossCheck::open()?;
        for _b in 0..num_batches {
            let batch_docs = TestUtil::next_int(&*random(), 1, 40);
            let mut ds = Vec::new();
            let mut os = Vec::new();
            let mut ids = vec![0i64; batch_docs as usize];
            for i in 0..batch_docs as usize {
                let num_values = random().next_int_bound(5); // 0..4 values per doc
                let mut doc_terms = JavaHashSet::new();
                let mut plain_doc = Document::new();
                plain_doc.add(Box::new(NumericDocValuesField::new("id", next_id)));
                ids[i] = next_id;
                for _v in 0..num_values {
                    let ord = random().next_int_bound(dict_size);
                    ds.push(i as i32);
                    os.push(ord);
                    let term = dict[ord as usize].clone();
                    if doc_terms.add(term.clone()) {
                        plain_doc.add(Box::new(SortedSetDocValuesField::new(
                            "f",
                            BytesRef::deep_copy_of(&term),
                        )));
                    }
                }
                cc.plain_w.add_document(&plain_doc)?;
                expected.push(doc_terms);
                next_id += 1;
            }
            cc.dict_w.add_batch(&simple_batch(
                batch_docs,
                vec![
                    ArrayDictionaryColumn::new("f", sorted_set_type(), dict.clone(), ds, os)?
                        .into(),
                    ArrayDenseLongColumn::new("id", ft(NumericDocValuesField::type_()), ids)?
                        .into(),
                ],
            ))?;
        }
        let (dict_r, plain_r) = cc.readers()?;
        let outcome = assert_sorted_set_readers_equal("f", "id", &dict_r, &plain_r, &expected);
        plain_r.close()?;
        dict_r.close()?;
        outcome?;
        cc.close()?;
    }
    Ok(())
}

/// `randomDictionary(size)`: `size` distinct random simple strings, in
/// `HashSet` iteration order.
fn random_dictionary(size: i32) -> Vec<BytesRef> {
    let mut uniq: JavaHashSet<String> = JavaHashSet::new();
    while (uniq.size() as i32) < size {
        uniq.add(TestUtil::random_simple_string_between(&*random(), 1, 12));
    }
    uniq.iter()
        .map(|s| BytesRef::from_char_sequence(s))
        .collect()
}

fn assert_sorted_readers_equal(
    field: &str,
    id_field: &str,
    dict_r: &Arc<dyn IndexReader>,
    plain_r: &Arc<dyn IndexReader>,
    expected: &[Option<BytesRef>],
) -> Result<()> {
    assert_equals(&(expected.len() as i32), &dict_r.max_doc());
    assert_equals(&(expected.len() as i32), &plain_r.max_doc());
    let any_value = expected.iter().any(Option::is_some);
    if !any_value {
        // No doc carries a value for "f"; nothing to compare.
        return Ok(());
    }
    check_sorted_by_insertion_id(field, id_field, dict_r, expected, "dict")?;
    check_sorted_by_insertion_id(field, id_field, plain_r, expected, "plain")
}

fn check_sorted_by_insertion_id(
    field: &str,
    id_field: &str,
    r: &Arc<dyn IndexReader>,
    expected: &[Option<BytesRef>],
    label: &str,
) -> Result<()> {
    let mut seen = 0usize;
    for ctx in Arc::clone(r).leaves()? {
        let leaf = ctx.leaf_reader();
        let ids = leaf.get_numeric_doc_values(id_field)?;
        let mut dv = leaf.get_sorted_doc_values(field)?;
        Assert::assert_true_msg(&format!("{label} reader missing id field"), ids.is_some());
        let mut ids = nn(ids);
        loop {
            let doc_id = ids.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let insertion_id = ids.long_value()? as i32;
            let want = &expected[insertion_id as usize];
            let got = match dv.as_mut() {
                Some(dv) => {
                    if dv.advance_exact(doc_id)? {
                        Some(dv.lookup_ord(dv.ord_value()?)?)
                    } else {
                        None
                    }
                }
                None => None,
            };
            assert_equals_msg(
                Some(&format!("{label} at insertionId {insertion_id}")),
                want,
                &got,
            );
            seen += 1;
        }
    }
    assert_equals_msg(
        Some(&format!("{label} reader saw wrong number of docs")),
        &expected.len(),
        &seen,
    );
    Ok(())
}

fn assert_sorted_set_readers_equal(
    field: &str,
    id_field: &str,
    dict_r: &Arc<dyn IndexReader>,
    plain_r: &Arc<dyn IndexReader>,
    expected: &[JavaHashSet<BytesRef>],
) -> Result<()> {
    assert_equals(&(expected.len() as i32), &dict_r.max_doc());
    assert_equals(&(expected.len() as i32), &plain_r.max_doc());
    let any_value = expected.iter().any(|s| !s.is_empty());
    if !any_value {
        return Ok(());
    }
    check_sorted_set_by_insertion_id(field, id_field, dict_r, expected, "dict")?;
    check_sorted_set_by_insertion_id(field, id_field, plain_r, expected, "plain")
}

fn check_sorted_set_by_insertion_id(
    field: &str,
    id_field: &str,
    r: &Arc<dyn IndexReader>,
    expected: &[JavaHashSet<BytesRef>],
    label: &str,
) -> Result<()> {
    let mut seen = 0usize;
    for ctx in Arc::clone(r).leaves()? {
        let leaf = ctx.leaf_reader();
        let ids = leaf.get_numeric_doc_values(id_field)?;
        let mut dv = leaf.get_sorted_set_doc_values(field)?;
        Assert::assert_true_msg(&format!("{label} reader missing id field"), ids.is_some());
        let mut ids = nn(ids);
        loop {
            let doc_id = ids.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let insertion_id = ids.long_value()? as i32;
            let want = &expected[insertion_id as usize];
            let got = match dv.as_mut() {
                Some(dv) => {
                    if dv.advance_exact(doc_id)? {
                        collect(dv)?
                    } else {
                        JavaHashSet::new()
                    }
                }
                None => JavaHashSet::new(),
            };
            assert_equals_msg(
                Some(&format!("{label} at insertionId {insertion_id}")),
                want,
                &got,
            );
            seen += 1;
        }
    }
    assert_equals_msg(
        Some(&format!("{label} reader saw wrong number of docs")),
        &expected.len(),
        &seen,
    );
    Ok(())
}

fn collect(dv: &mut Box<dyn rucene::index::SortedSetDocValues>) -> Result<JavaHashSet<BytesRef>> {
    let mut out = JavaHashSet::new();
    for _i in 0..dv.doc_value_count()? {
        let o = dv.next_ord()?;
        out.add(BytesRef::deep_copy_of(&dv.lookup_ord(o)?));
    }
    Ok(out)
}
