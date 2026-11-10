//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocValuesIndexing.java`.
//!
//! Tests DocValues integration into IndexWriter
//!
//! * A Java `Field` added to a document twice, or whose `BytesRef` array is
//!   changed after `doc.add`, is a second field built from the same value
//!   (guide section 6): Rucene's `Document` owns its fields.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, InvertableType, NumericDocValuesField,
    NumericValue, SharedReader, SortedDocValuesField, SortedSetDocValuesField, Store, StoredValue,
    StringField, TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::DocValues;
use rucene::index::field_infos::FieldInfos;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{DocValuesType, IndexReader, IndexableField, IndexableFieldType};
use rucene::store::Directory;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, rethrow_java};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test};
use crate::support::{npe, CountDownLatch};

/// `TestDocValuesIndexing`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocValuesIndexing",
        vec![
            test("testAddIndexes", test_add_indexes),
            test(
                "testMultiValuedDocValuesField",
                test_multi_valued_doc_values_field,
            ),
            test(
                "testDifferentTypedDocValuesField",
                test_different_typed_doc_values_field,
            ),
            test(
                "testDifferentTypedDocValuesField2",
                test_different_typed_doc_values_field2,
            ),
            test(
                "testLengthPrefixAcrossTwoPages",
                test_length_prefix_across_two_pages,
            ),
            test("testDocValuesUnstored", test_doc_values_unstored),
            test("testMixedTypesSameDocument", test_mixed_types_same_document),
            test(
                "testMixedTypesDifferentDocuments",
                test_mixed_types_different_documents,
            ),
            test("testAddSortedTwice", test_add_sorted_twice),
            test("testAddBinaryTwice", test_add_binary_twice),
            test("testAddNumericTwice", test_add_numeric_twice),
            test("testTooLargeSortedBytes", test_too_large_sorted_bytes),
            test(
                "testTooLargeTermSortedSetBytes",
                test_too_large_term_sorted_set_bytes,
            ),
            test(
                "testMixedTypesDifferentSegments",
                test_mixed_types_different_segments,
            ),
            test(
                "testMixedTypesAfterDeleteAll",
                test_mixed_types_after_delete_all,
            ),
            test(
                "testMixedTypesAfterReopenCreate",
                test_mixed_types_after_reopen_create,
            ),
            test(
                "testMixedTypesAfterReopenAppend1",
                test_mixed_types_after_reopen_append1,
            ),
            test(
                "testMixedTypesAfterReopenAppend2",
                test_mixed_types_after_reopen_append2,
            ),
            test(
                "testMixedTypesAfterReopenAppend3",
                test_mixed_types_after_reopen_append3,
            ),
            test(
                "testMixedTypesDifferentThreads",
                test_mixed_types_different_threads,
            ),
            test(
                "testMixedTypesViaAddIndexes",
                test_mixed_types_via_add_indexes,
            ),
            test("testIllegalTypeChange", test_illegal_type_change),
            test(
                "testIllegalTypeChangeAcrossSegments",
                test_illegal_type_change_across_segments,
            ),
            test(
                "testTypeChangeAfterCloseAndDeleteAll",
                test_type_change_after_close_and_delete_all,
            ),
            test(
                "testTypeChangeAfterDeleteAll",
                test_type_change_after_delete_all,
            ),
            test(
                "testTypeChangeAfterCommitAndDeleteAll",
                test_type_change_after_commit_and_delete_all,
            ),
            test(
                "testTypeChangeAfterOpenCreate",
                test_type_change_after_open_create,
            ),
            test(
                "testTypeChangeViaAddIndexes",
                test_type_change_via_add_indexes,
            ),
            test(
                "testTypeChangeViaAddIndexesIR",
                test_type_change_via_add_indexes_ir,
            ),
            test(
                "testTypeChangeViaAddIndexes2",
                test_type_change_via_add_indexes2,
            ),
            test(
                "testTypeChangeViaAddIndexesIR2",
                test_type_change_via_add_indexes_ir2,
            ),
            test(
                "testSameFieldNameForPostingAndDocValue",
                test_same_field_name_for_posting_and_doc_value,
            ),
            test(
                "testExcIndexingDocBeforeDocValues",
                test_exc_indexing_doc_before_doc_values,
            ),
        ],
    )
}

const IAE: &str = "java.lang.IllegalArgumentException";

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())))
}

/// `newIndexWriterConfig(analyzer)` with `setMergePolicy(newLogMergePolicy())`.
fn log_mp_iwc(analyzer: Arc<dyn Analyzer>) -> Result<IndexWriterConfig> {
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    Ok(iwc)
}

/// A document holding the given fields.
fn doc_of(fields: Vec<Box<dyn IndexableField>>) -> Document {
    let mut doc = Document::new();
    for field in fields {
        doc.add(field);
    }
    doc
}

/// `new NumericDocValuesField(name, value)`, boxed.
fn ndv(name: &str, value: i64) -> Box<dyn IndexableField> {
    Box::new(NumericDocValuesField::new(name, value))
}

/// `new SortedDocValuesField(name, newBytesRef(s))`, boxed.
fn sdv(name: &str, s: &str) -> Box<dyn IndexableField> {
    Box::new(SortedDocValuesField::new(
        name,
        LuceneTestCase::new_bytes_ref_str(s),
    ))
}

/// `new BinaryDocValuesField(name, newBytesRef(s))`, boxed.
fn bdv(name: &str, s: &str) -> Box<dyn IndexableField> {
    Box::new(BinaryDocValuesField::new(
        name,
        LuceneTestCase::new_bytes_ref_str(s),
    ))
}

/// `new SortedSetDocValuesField(name, newBytesRef(s))`, boxed.
fn ssdv(name: &str, s: &str) -> Box<dyn IndexableField> {
    Box::new(SortedSetDocValuesField::new(
        name,
        LuceneTestCase::new_bytes_ref_str(s),
    ))
}

/// `IndexReader ir = DirectoryReader.open(w); assertEquals(1, ir.numDocs());
/// ir.close();`.
fn assert_one_doc(w: &IndexWriter) -> Result<()> {
    let ir = directory_reader::open_with_writer(w)?;
    assert_equals(&1, &ir.num_docs());
    ir.close()
}

/// `DocValues.getNumeric(getOnlyLeafReader(r), "field")`: doc 0 holds 17.
fn assert_field_17(r: &Arc<dyn DirectoryReader>) -> Result<()> {
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(r) as Arc<dyn IndexReader>)?;
    let mut values = DocValues::get_numeric(leaf.as_ref(), "field")?;
    assert_equals(&0, &values.next_doc()?);
    assert_equals(&17, &values.long_value()?);
    Ok(())
}

/*
 * - add test for multi segment case with deletes
 * - add multithreaded tests / integrate into stress indexing?
 */

/// Port of `TestDocValuesIndexing.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    let mut d1 = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d1.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    doc.add(ndv("dv", 1));
    w.add_document(&doc)?;
    let r1 = w.get_reader()?;
    w.close()?;

    let mut d2 = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d2.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "2",
        Store::YES,
    )));
    doc.add(ndv("dv", 2));
    w.add_document(&doc)?;
    let r2 = w.get_reader()?;
    w.close()?;

    let mut d3 = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d3.as_directory())?;
    w.add_indexes_codec_readers(&[
        SlowCodecReaderWrapper::wrap(LuceneTestCase::get_only_leaf_reader(
            Arc::clone(&r1) as Arc<dyn IndexReader>
        )?)?,
        SlowCodecReaderWrapper::wrap(LuceneTestCase::get_only_leaf_reader(
            Arc::clone(&r2) as Arc<dyn IndexReader>
        )?)?,
    ])?;
    r1.close()?;
    drop(r1);
    d1.close()?;
    r2.close()?;
    drop(r2);
    d2.close()?;

    w.force_merge(1)?;
    let r3 = w.get_reader()?;
    w.close()?;
    let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r3) as Arc<dyn IndexReader>)?;
    assert_equals(&2, &sr.num_docs());
    let doc_values = sr.get_numeric_doc_values("dv")?;
    assert_not_null(&doc_values);
    r3.close()?;
    drop((sr, r3, w));
    d3.close()
}

/// Port of `TestDocValuesIndexing.testMultiValuedDocValuesField`.
fn test_multi_valued_doc_values_field() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    // `Field f = new NumericDocValuesField("field", 17); doc.add(f);`
    let doc = doc_of(vec![ndv("field", 17)]);

    // add the doc
    w.add_document(&doc)?;

    // Index doc values are single-valued so we should not
    // be able to add same field more than once:
    // `doc.add(f)`: the same field a second time.
    let doc = doc_of(vec![ndv("field", 17), ndv("field", 17)]);
    expect_throws(IAE, || w.add_document(&doc));

    let r = w.get_reader()?;
    w.close()?;
    assert_field_17(&r)?;
    r.close()?;
    d.close()
}

/// Port of `TestDocValuesIndexing.testDifferentTypedDocValuesField`.
fn test_different_typed_doc_values_field() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let doc = doc_of(vec![ndv("field", 17)]);
    w.add_document(&doc)?;

    // Index doc values are single-valued so we should not
    // be able to add same field more than once:
    let doc = doc_of(vec![ndv("field", 17), bdv("field", "blah")]);
    expect_throws(IAE, || w.add_document(&doc));

    let r = w.get_reader()?;
    w.close()?;
    assert_field_17(&r)?;
    r.close()?;
    d.close()
}

/// Port of `TestDocValuesIndexing.testDifferentTypedDocValuesField2`.
fn test_different_typed_doc_values_field2() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let doc = doc_of(vec![ndv("field", 17)]);
    w.add_document(&doc)?;

    // Index doc values are single-valued so we should not
    // be able to add same field more than once:
    let doc = doc_of(vec![ndv("field", 17), sdv("field", "hello")]);
    expect_throws(IAE, || w.add_document(&doc));

    let r = w.get_reader()?;
    assert_field_17(&r)?;
    r.close()?;
    w.close()?;
    d.close()
}

// LUCENE-3870
/// Port of `TestDocValuesIndexing.testLengthPrefixAcrossTwoPages`.
fn test_length_prefix_across_two_pages() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        d.as_directory(),
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random()))),
    )?;
    let mut bytes = vec![0u8; 32764];
    // `BytesRef b = new BytesRef(); b.bytes = bytes; b.length = bytes.length;`:
    // `b` shares `bytes`, so each document holds the array as it is when the
    // document is added.
    let doc = doc_of(vec![Box::new(SortedDocValuesField::new(
        "field",
        BytesRef::new(bytes.clone()),
    ))]);
    w.add_document(&doc)?;
    bytes[0] = 1;
    let doc = doc_of(vec![Box::new(SortedDocValuesField::new(
        "field",
        BytesRef::new(bytes.clone()),
    ))]);
    w.add_document(&doc)?;
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(w.as_ref())?;
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let mut s = DocValues::get_sorted(leaf.as_ref(), "field")?;
    assert_equals(&0, &s.next_doc()?);
    let ord = s.ord_value()?;
    let bytes1 = s.lookup_ord(ord)?;
    assert_equals(&bytes.len(), &(bytes1.length as usize));
    bytes[0] = 0;
    assert_equals(&BytesRef::new(bytes.clone()), &bytes1);

    assert_equals(&1, &s.next_doc()?);
    let ord = s.ord_value()?;
    let bytes1 = s.lookup_ord(ord)?;
    assert_equals(&bytes.len(), &(bytes1.length as usize));
    bytes[0] = 1;
    assert_equals(&BytesRef::new(bytes.clone()), &bytes1);
    r.close()?;
    w.close()?;
    d.close()
}

/// Port of `TestDocValuesIndexing.testDocValuesUnstored`.
fn test_doc_values_unstored() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwconfig = mock_iwc();
    iwconfig.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwconfig)?;
    for i in 0..50 {
        let mut doc = Document::new();
        doc.add(ndv("dv", i));
        doc.add(Box::new(TextField::new(
            "docId",
            i.to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
    }
    let r = directory_reader::open_with_writer(writer.as_ref())?;
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let fi = FieldInfos::get_merged_field_infos(Arc::clone(&ir))?;
    let dv_info = fi
        .field_info("dv")
        .unwrap_or_else(|| npe("fieldInfo(\"dv\")"));
    Assert::assert_true(dv_info.get_doc_values_type() != DocValuesType::NONE);
    let mut dv = MultiDocValues::get_numeric_values(&ir, "dv")?
        .unwrap_or_else(|| npe("getNumericValues(r, \"dv\")"));
    let stored_fields = r.stored_fields()?;
    for i in 0..50 {
        assert_equals(&i, &dv.next_doc()?);
        assert_equals(&i64::from(i), &dv.long_value()?);
        let d = stored_fields.document(i)?;
        // cannot use d.get("dv") due to another bug!
        assert_null(&d.get_field("dv").map(|_| ()));
        assert_equals(&Some(i.to_string()), &d.get("docId"));
    }
    r.close()?;
    writer.close()?;
    dir.close()
}

// Same field in one document as different types:
/// Port of `TestDocValuesIndexing.testMixedTypesSameDocument`.
fn test_mixed_types_same_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&Document::new())?;

    let doc = doc_of(vec![ndv("foo", 0), sdv("foo", "hello")]);
    expect_throws(IAE, || w.add_document(&doc));

    assert_one_doc(&w)?;
    w.close()?;
    dir.close()
}

// Two documents with same field as different types:
/// Port of `TestDocValuesIndexing.testMixedTypesDifferentDocuments`.
fn test_mixed_types_different_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let doc = doc_of(vec![ndv("foo", 0)]);
    w.add_document(&doc)?;

    let doc2 = doc_of(vec![sdv("foo", "hello")]);
    expect_throws(IAE, || w.add_document(&doc2));

    assert_one_doc(&w)?;
    w.close()?;
    dir.close()
}

/// The body `testAddSortedTwice`, `testAddBinaryTwice` and
/// `testAddNumericTwice` share: `first` alone is accepted, `first` and
/// `second` together are not.
fn add_twice<V: Clone>(
    first_value: impl FnOnce() -> V,
    first: impl Fn(V) -> Box<dyn IndexableField>,
    second: impl FnOnce() -> Box<dyn IndexableField>,
) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));

    let mut directory = LuceneTestCase::new_directory();
    // we don't use RandomIndexWriter because it might add more docvalues than we expect !!!!1
    let iwc = log_mp_iwc(analyzer)?;
    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
    let value = first_value();
    let doc = doc_of(vec![first(value.clone())]);
    iwriter.add_document(&doc)?;

    // `doc.add(second)`: the document now holds both fields. The first is
    // rebuilt from the same value; building it draws nothing.
    let doc = doc_of(vec![first(value), second()]);
    expect_throws(IAE, || iwriter.add_document(&doc));

    assert_one_doc(&iwriter)?;
    iwriter.close()?;
    directory.close()
}

/// Port of `TestDocValuesIndexing.testAddSortedTwice`.
fn test_add_sorted_twice() -> Result<()> {
    add_twice(
        || LuceneTestCase::new_bytes_ref_str("foo!"),
        |foo| Box::new(SortedDocValuesField::new("dv", foo)),
        || sdv("dv", "bar!"),
    )
}

/// Port of `TestDocValuesIndexing.testAddBinaryTwice`.
fn test_add_binary_twice() -> Result<()> {
    add_twice(
        || LuceneTestCase::new_bytes_ref_str("foo!"),
        |foo| Box::new(BinaryDocValuesField::new("dv", foo)),
        || bdv("dv", "bar!"),
    )
}

/// Port of `TestDocValuesIndexing.testAddNumericTwice`.
fn test_add_numeric_twice() -> Result<()> {
    add_twice(|| 1, |v| ndv("dv", v), || ndv("dv", 2))
}

/// The body `testTooLargeSortedBytes` and `testTooLargeTermSortedSetBytes`
/// share.
fn too_large(field: fn(&str, BytesRef) -> Box<dyn IndexableField>) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));

    let mut directory = LuceneTestCase::new_directory();
    // we don't use RandomIndexWriter because it might add more docvalues than we expect !!!!1
    let iwc = log_mp_iwc(analyzer)?;
    let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
    let doc = doc_of(vec![field(
        "dv",
        LuceneTestCase::new_bytes_ref_str("just fine"),
    )]);
    iwriter.add_document(&doc)?;

    let mut bytes = vec![0u8; 100000];
    // `newBytesRef(bytes)` copies the array, so the bytes drawn next do not
    // reach `b`.
    let b = LuceneTestCase::new_bytes_ref_bytes(&bytes);
    random().next_bytes(&mut bytes);
    let huge_doc = doc_of(vec![field("dv", b)]);
    expect_throws(IAE, || iwriter.add_document(&huge_doc));

    assert_one_doc(&iwriter)?;
    iwriter.close()?;
    directory.close()
}

/// Port of `TestDocValuesIndexing.testTooLargeSortedBytes`.
fn test_too_large_sorted_bytes() -> Result<()> {
    too_large(|name, b| Box::new(SortedDocValuesField::new(name, b)))
}

/// Port of `TestDocValuesIndexing.testTooLargeTermSortedSetBytes`.
fn test_too_large_term_sorted_set_bytes() -> Result<()> {
    too_large(|name, b| Box::new(SortedSetDocValuesField::new(name, b)))
}

// Two documents across segments
/// Port of `TestDocValuesIndexing.testMixedTypesDifferentSegments`.
fn test_mixed_types_different_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ndv("foo", 0)]))?;
    w.commit()?;

    let doc2 = doc_of(vec![sdv("foo", "hello")]);
    expect_throws(IAE, || w.add_document(&doc2));

    w.close()?;
    dir.close()
}

// Add inconsistent document after deleteAll
/// Port of `TestDocValuesIndexing.testMixedTypesAfterDeleteAll`.
fn test_mixed_types_after_delete_all() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ndv("foo", 0)]))?;
    w.delete_all()?;

    w.add_document(&doc_of(vec![sdv("foo", "hello")]))?;
    w.close()?;
    dir.close()
}

// Add inconsistent document after reopening IW w/ create
/// Port of `TestDocValuesIndexing.testMixedTypesAfterReopenCreate`.
fn test_mixed_types_after_reopen_create() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ndv("foo", 0)]))?;
    w.close()?;

    let mut iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&Document::new())?;
    w.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testMixedTypesAfterReopenAppend1`.
fn test_mixed_types_after_reopen_append1() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ndv("foo", 0)]))?;
    w.close()?;

    let w2 = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let doc2 = doc_of(vec![sdv("foo", "hello")]);
    expect_throws(IAE, || w2.add_document(&doc2));

    w2.close()?;
    dir.close()
}

/// The body `testMixedTypesAfterReopenAppend2` and `…3` share; the second
/// adds an empty document before the merge.
fn mixed_types_after_reopen_append(add_empty: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ssdv("foo", "foo")]))?;
    w.close()?;

    let w2 = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let doc2 = doc_of(vec![
        Box::new(StringField::new("foo", "bar".to_string(), Store::NO)?),
        bdv("foo", "foo"),
    ]);
    // NOTE: this case follows a different code path inside
    // DefaultIndexingChain/FieldInfos, because the field (foo)
    // is first added without DocValues:
    expect_throws(IAE, || w2.add_document(&doc2));

    if add_empty {
        // Also add another document so there is a segment to write here:
        w2.add_document(&Document::new())?;
    }
    w2.force_merge(1)?;
    w2.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testMixedTypesAfterReopenAppend2`.
///
/// Java creates `doc2` before the second writer; creating a document draws
/// nothing, so building it after the writer keeps the draws.
fn test_mixed_types_after_reopen_append2() -> Result<()> {
    mixed_types_after_reopen_append(false)
}

/// Port of `TestDocValuesIndexing.testMixedTypesAfterReopenAppend3`.
fn test_mixed_types_after_reopen_append3() -> Result<()> {
    mixed_types_after_reopen_append(true)
}

// Two documents with same field as different types, added
// from separate threads:
/// Port of `TestDocValuesIndexing.testMixedTypesDifferentThreads`.
fn test_mixed_types_different_threads() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;

    let starting_gun = Arc::new(CountDownLatch::new(1));
    let hit_exc = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::with_capacity(3);
    for i in 0..3 {
        // The field's value is drawn here, as Java builds the field here;
        // the document is built on its thread, as Rucene's `Document` is not
        // `Send`.
        let field: Box<dyn FnOnce() -> Box<dyn IndexableField> + Send> = if i == 0 {
            let value = LuceneTestCase::new_bytes_ref_str("hello");
            Box::new(move || Box::new(SortedDocValuesField::new("foo", value)))
        } else if i == 1 {
            Box::new(|| ndv("foo", 0))
        } else {
            let value = LuceneTestCase::new_bytes_ref_str("bazz");
            Box::new(move || Box::new(BinaryDocValuesField::new("foo", value)))
        };

        let thread = {
            let starting_gun = Arc::clone(&starting_gun);
            let hit_exc = Arc::clone(&hit_exc);
            let w = Arc::clone(&w);
            Thread::unnamed(move || {
                let doc = doc_of(vec![field()]);
                starting_gun.await_();
                match w.add_document(&doc) {
                    Ok(_) => {}
                    Err(e) if is_instance(&e, IAE) => {
                        // expected
                        hit_exc.store(true, Ordering::SeqCst);
                    }
                    Err(e) => rethrow_java(LuceneError::runtime_exception(e)),
                }
            })
        };
        thread.start();
        threads.push(thread);
    }

    starting_gun.count_down();

    for t in &threads {
        if let Err(payload) = t.join() {
            std::panic::resume_unwind(payload);
        }
    }
    Assert::assert_true(hit_exc.load(Ordering::SeqCst));
    w.close()?;
    dir.close()
}

// Adding documents via addIndexes
/// Port of `TestDocValuesIndexing.testMixedTypesViaAddIndexes`.
fn test_mixed_types_via_add_indexes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    w.add_document(&doc_of(vec![ndv("foo", 0)]))?;

    // Make 2nd index w/ inconsistent field
    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = IndexWriter::new(dir2.as_directory(), mock_iwc())?;
    w2.add_document(&doc_of(vec![sdv("foo", "hello")]))?;
    w2.close()?;

    expect_throws(IAE, || w.add_indexes_dirs(&[dir2.as_directory()]));

    let r = directory_reader::open(dir2.as_directory())?;
    expect_throws(IAE, || TestUtil::add_indexes_slowly(&w, &[Arc::clone(&r)]));

    r.close()?;
    drop((r, w2));
    dir2.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testIllegalTypeChange`.
fn test_illegal_type_change() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc_of(vec![ndv("dv", 0)]))?;
    let doc2 = doc_of(vec![sdv("dv", "foo")]);
    expect_throws(IAE, || writer.add_document(&doc2));

    assert_one_doc(&writer)?;
    writer.close()?;
    dir.close()
}

/// `IndexWriter writer = new IndexWriter(dir, conf); doc.add(new
/// NumericDocValuesField("dv", 0L)); writer.addDocument(doc);
/// writer.close();`.
fn numeric_dv_index(dir: &Arc<dyn Directory>) -> Result<()> {
    let conf = mock_iwc();
    let writer = IndexWriter::new(Arc::clone(dir), conf)?;
    writer.add_document(&doc_of(vec![ndv("dv", 0)]))?;
    writer.close()
}

/// Port of `TestDocValuesIndexing.testIllegalTypeChangeAcrossSegments`.
fn test_illegal_type_change_across_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let conf = mock_iwc();
    let writer2 = IndexWriter::new(dir.as_directory(), conf)?;
    let doc2 = doc_of(vec![sdv("dv", "foo")]);
    expect_throws(IAE, || writer2.add_document(&doc2));

    writer2.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeAfterCloseAndDeleteAll`.
fn test_type_change_after_close_and_delete_all() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.delete_all()?;
    writer.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeAfterDeleteAll`.
fn test_type_change_after_delete_all() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc_of(vec![ndv("dv", 0)]))?;
    writer.delete_all()?;
    writer.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeAfterCommitAndDeleteAll`.
fn test_type_change_after_commit_and_delete_all() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc_of(vec![ndv("dv", 0)]))?;
    writer.commit()?;
    writer.delete_all()?;
    writer.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeAfterOpenCreate`.
fn test_type_change_after_open_create() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;
    let mut conf = mock_iwc();
    conf.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    writer.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeViaAddIndexes`.
fn test_type_change_via_add_indexes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let mut dir2 = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer2 = IndexWriter::new(dir2.as_directory(), conf)?;
    writer2.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    expect_throws(IAE, || writer2.add_indexes_dirs(&[dir.as_directory()]));
    writer2.close()?;
    drop(writer2);

    dir.close()?;
    dir2.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeViaAddIndexesIR`.
fn test_type_change_via_add_indexes_ir() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let mut dir2 = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer2 = IndexWriter::new(dir2.as_directory(), conf)?;
    writer2.add_document(&doc_of(vec![sdv("dv", "foo")]))?;
    let reader = directory_reader::open(dir.as_directory())?;
    expect_throws(IAE, || {
        TestUtil::add_indexes_slowly(&writer2, &[Arc::clone(&reader)])
    });

    reader.close()?;
    writer2.close()?;
    drop((reader, writer2));

    dir.close()?;
    dir2.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeViaAddIndexes2`.
fn test_type_change_via_add_indexes2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let mut dir2 = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer2 = IndexWriter::new(dir2.as_directory(), conf)?;
    writer2.add_indexes_dirs(&[dir.as_directory()])?;
    let doc2 = doc_of(vec![sdv("dv", "foo")]);
    expect_throws(IAE, || writer2.add_document(&doc2));

    writer2.close()?;
    drop(writer2);
    dir2.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testTypeChangeViaAddIndexesIR2`.
fn test_type_change_via_add_indexes_ir2() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    numeric_dv_index(&dir.as_directory())?;

    let mut dir2 = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer2 = IndexWriter::new(dir2.as_directory(), conf)?;
    let reader = directory_reader::open(dir.as_directory())?;
    TestUtil::add_indexes_slowly(&writer2, &[Arc::clone(&reader)])?;
    reader.close()?;
    drop(reader);
    let doc2 = doc_of(vec![sdv("dv", "foo")]);
    expect_throws(IAE, || writer2.add_document(&doc2));

    writer2.close()?;
    drop(writer2);
    dir2.close()?;
    dir.close()
}

/// Port of `TestDocValuesIndexing.testSameFieldNameForPostingAndDocValue`.
fn test_same_field_name_for_posting_and_doc_value() -> Result<()> {
    // LUCENE-5192: FieldInfos.Builder neglected to update
    // globalFieldNumbers.docValuesType map if the field existed, resulting in
    // potentially adding the same field with different DV types.
    let mut dir = LuceneTestCase::new_directory();
    let conf = mock_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let doc = doc_of(vec![
        Box::new(StringField::new("f", "mock-value".to_string(), Store::NO)?),
        ndv("f", 5),
    ]);
    writer.add_document(&doc)?;
    writer.commit()?;

    let doc2 = doc_of(vec![bdv("f", "mock")]);
    expect_throws(IAE, || writer.add_document(&doc2));
    writer.rollback()?;
    drop(writer);

    dir.close()
}

/// The anonymous `Field` subclass of `testExcIndexingDocBeforeDocValues`:
/// `new Field("test", new BytesRef("value"), ft)` whose `tokenStream`
/// returns a stream that throws `RuntimeException` from `incrementToken`.
struct ThrowingTokenStreamField(Field);

/// `new TokenStream() { incrementToken() { throw new RuntimeException(); } }`.
#[derive(Debug)]
struct ThrowingTokenStream {
    source: AttributeSource,
}

impl TokenStream for ThrowingTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        Err(LuceneError::Unchecked {
            class: "java.lang.RuntimeException",
            message: None,
            cause: None,
        })
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

impl std::fmt::Display for ThrowingTokenStreamField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl IndexableField for ThrowingTokenStreamField {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.0.field_type()
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(ThrowingTokenStream {
            source: AttributeSource::new(),
        })
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.0.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.0.string_value()
    }

    fn reader_value(&self) -> Option<SharedReader> {
        self.0.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.0.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        self.0.stored_value()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        self.0.invertable_type()
    }
}

// LUCENE-6049
/// Port of `TestDocValuesIndexing.testExcIndexingDocBeforeDocValues`.
fn test_exc_indexing_doc_before_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut ft = FieldType::new_from(StringField::type_not_stored());
    ft.set_doc_values_type(DocValuesType::SORTED)?;
    ft.freeze();
    let field = ThrowingTokenStreamField(Field::new_with_bytes(
        "test",
        BytesRef::from_char_sequence("value"),
        ft,
    )?);
    let doc = doc_of(vec![Box::new(field)]);
    expect_throws("java.lang.RuntimeException", || w.add_document(&doc));

    w.add_document(&Document::new())?;
    w.close()?;
    dir.close()
}
