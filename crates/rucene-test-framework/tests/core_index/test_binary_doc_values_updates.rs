//! Port of `lucene/core/src/test/org/apache/lucene/index/TestBinaryDocValuesUpdates.java`.
//!
//! * `doc.removeField("id"); doc.add(...)` on a document already added
//!   (`testSegmentMerges`) is the same document changed in place: Rucene's
//!   `Document::remove_field` and `add` do what Java's do.
//! * The anonymous `AssertingCodec` subclasses are
//!   `AssertingCodec::with_formats_for_field`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::codecs::DocValuesFormat;
use rucene::document::{
    BinaryDocValuesField, Document, Field, NumericDocValuesField, SortedDocValuesField,
    SortedSetDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::BinaryDocValues;
use rucene::index::documents_writer::SharedDirectory;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::store::{Directory, NRTCachingDirectory};
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::codecs::asserting::asserting_doc_values_format::AssertingDocValuesFormat;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, class, random, test, verbose, JavaHashSet,
    IAE,
};
use crate::support::{npe, CountDownLatch};

/// `TestBinaryDocValuesUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestBinaryDocValuesUpdates",
        vec![
            test("testUpdatesAreFlushed", test_updates_are_flushed),
            test("testSimple", test_simple),
            test("testUpdateFewSegments", test_update_few_segments),
            test("testReopen", test_reopen),
            test("testUpdatesAndDeletes", test_updates_and_deletes),
            test("testUpdatesWithDeletes", test_updates_with_deletes),
            test("testMultipleDocValuesTypes", test_multiple_doc_values_types),
            test(
                "testMultipleBinaryDocValues",
                test_multiple_binary_doc_values,
            ),
            test("testDocumentWithNoValue", test_document_with_no_value),
            test(
                "testUpdateNonBinaryDocValuesField",
                test_update_non_binary_doc_values_field,
            ),
            test(
                "testDifferentDVFormatPerField",
                test_different_dv_format_per_field,
            ),
            test(
                "testUpdateSameDocMultipleTimes",
                test_update_same_doc_multiple_times,
            ),
            test("testSegmentMerges", test_segment_merges),
            test(
                "testUpdateDocumentByMultipleTerms",
                test_update_document_by_multiple_terms,
            ),
            test("testSortedIndex", test_sorted_index),
            test("testManyReopensAndFields", test_many_reopens_and_fields),
            test(
                "testUpdateSegmentWithNoDocValues",
                test_update_segment_with_no_doc_values,
            ),
            test(
                "testUpdateSegmentWithPostingButNoDocValues",
                test_update_segment_with_posting_but_no_doc_values,
            ),
            test(
                "testUpdateBinaryDVFieldWithSameNameAsPostingField",
                test_update_binary_dv_field_with_same_name_as_posting_field,
            ),
            test("testStressMultiThreading", test_stress_multi_threading),
            test(
                "testUpdateDifferentDocsInDifferentGens",
                test_update_different_docs_in_different_gens,
            ),
            test("testChangeCodec", test_change_codec),
            test("testAddIndexes", test_add_indexes),
            test(
                "testDeleteUnusedUpdatesFiles",
                test_delete_unused_updates_files,
            ),
            test("testTonsOfUpdates", test_tons_of_updates).annotate(Nightly),
            test("testUpdatesOrder", test_updates_order),
            test(
                "testUpdateAllDeletedSegment",
                test_update_all_deleted_segment,
            ),
            test(
                "testUpdateTwoNonexistingTerms",
                test_update_two_nonexisting_terms,
            ),
            test("testIOContext", test_io_context),
        ],
    )
}

// ---------------------------------------------------------------------------
// Plumbing.
// ---------------------------------------------------------------------------

/// `static long getValue(BinaryDocValues bdv)`.
fn get_value(bdv: &dyn BinaryDocValues) -> Result<i64> {
    let term = bdv.binary_value()?;
    let mut idx = term.offset;
    debug_assert!(term.length > 0);
    let mut b = term.bytes[idx] as i8;
    idx += 1;
    let mut value = (b as i64) & 0x7F;
    let mut shift = 7;
    while ((b as i64) & 0x80) != 0 {
        b = term.bytes[idx] as i8;
        idx += 1;
        value |= ((b as i64) & 0x7F) << shift;
        shift += 7;
    }
    Ok(value)
}

/// `static BytesRef toBytes(long value)`: encodes a long into a BytesRef as
/// VLong so that we get varying number of bytes when we update.
fn to_bytes(value: i64) -> BytesRef {
    let mut value = value as u64;
    let mut bytes = LuceneTestCase::new_bytes_ref_len(10); // negative longs may take 10 bytes
    let mut upto = 0;
    while (value & !0x7F) != 0 {
        let offset = bytes.offset;
        bytes.bytes[offset + upto] = ((value & 0x7F) | 0x80) as u8;
        upto += 1;
        value >>= 7;
    }
    let offset = bytes.offset;
    bytes.bytes[offset + upto] = value as u8;
    upto += 1;
    bytes.length = upto;
    bytes
}

/// `new BinaryDocValuesField(name, value)`.
fn bdv_field(name: &str, value: BytesRef) -> Box<BinaryDocValuesField> {
    Box::new(BinaryDocValuesField::new(name, value))
}

/// `new BinaryDocValuesField(name, value)` as the `Field` `updateDocValues`
/// takes.
fn bdv_update(name: &str, value: BytesRef) -> Result<Field> {
    Field::new_with_bytes(name, value, BinaryDocValuesField::type_().clone())
}

/// `private Document doc(int id)`.
fn doc(id: i32) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "id",
        format!("doc-{id}"),
        Store::NO,
    )?));
    doc.add(bdv_field("val", to_bytes((id + 1) as i64)));
    Ok(doc)
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())))
}

/// `newIndexWriterConfig(new MockAnalyzer(random))`.
fn new_iwc_with(random: &dyn Random) -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(random)))
}

/// `r.getBinaryDocValues(field)`, dereferenced as Java does.
fn binary(r: &Arc<dyn LeafReader>, field: &str) -> Result<Box<dyn BinaryDocValues>> {
    Ok(r.get_binary_doc_values(field)?
        .unwrap_or_else(|| npe("getBinaryDocValues")))
}

/// `MultiDocValues.getBinaryValues(reader, field)`, dereferenced.
fn multi_binary(
    reader: &Arc<dyn DirectoryReader>,
    field: &str,
) -> Result<Box<dyn BinaryDocValues>> {
    let ir: Arc<dyn IndexReader> = Arc::clone(reader) as Arc<dyn IndexReader>;
    Ok(MultiDocValues::get_binary_values(&ir, field)?
        .unwrap_or_else(|| npe("MultiDocValues.getBinaryValues")))
}

/// `reader.leaves().get(i).reader()`.
fn leaf(reader: &Arc<dyn DirectoryReader>, i: usize) -> Result<Arc<dyn LeafReader>> {
    Ok(Arc::clone(reader).leaves()?[i].leaf_reader())
}

/// The `if (random().nextBoolean()) { // not NRT ... } else { // NRT ... }`
/// block of several tests: the writer is closed either way.
fn close_and_open_reader(
    writer: &Arc<IndexWriter>,
    dir: &TestDirectory,
) -> Result<Arc<dyn DirectoryReader>> {
    if random().next_boolean() {
        // not NRT
        writer.close()?;
        directory_reader::open(dir.as_directory())
    } else {
        // NRT
        let reader = directory_reader::open_with_writer(&**writer)?;
        writer.close()?;
        Ok(reader)
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

/// Port of `TestBinaryDocValuesUpdates.testUpdatesAreFlushed`.
fn test_updates_are_flushed() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut c = LuceneTestCase::new_index_writer_config_analyzer(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&WHITESPACE), false),
    ));
    c.set_ram_buffer_size_mb(0.00000001)?;
    let writer = IndexWriter::new(dir.as_directory(), c)?;
    writer.add_document(&doc(0)?)?; // val=1
    writer.add_document(&doc(1)?)?; // val=2
    writer.add_document(&doc(3)?)?; // val=2
    writer.commit()?;
    assert_equals(&1, &writer.get_flush_deletes_count());
    writer.update_binary_doc_value(Term::from_text("id", "doc-0"), "val", to_bytes(5))?;
    assert_equals(&2, &writer.get_flush_deletes_count());
    writer.update_binary_doc_value(Term::from_text("id", "doc-1"), "val", to_bytes(6))?;
    assert_equals(&3, &writer.get_flush_deletes_count());
    writer.update_binary_doc_value(Term::from_text("id", "doc-2"), "val", to_bytes(7))?;
    assert_equals(&4, &writer.get_flush_deletes_count());
    writer.get_config().set_ram_buffer_size_mb(1000.0)?;
    writer.update_binary_doc_value(Term::from_text("id", "doc-2"), "val", to_bytes(7))?;
    assert_equals(&4, &writer.get_flush_deletes_count());
    writer.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testSimple`.
fn test_simple() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    // make sure random config doesn't flush on us
    conf.set_max_buffered_docs(10)?;
    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc(0)?)?; // val=1
    writer.add_document(&doc(1)?)?; // val=2
    if random().next_boolean() {
        // randomly commit before the update is sent
        writer.commit()?;
    }
    writer.update_binary_doc_value(Term::from_text("id", "doc-0"), "val", to_bytes(2))?; // doc=0, exp=2

    let reader = close_and_open_reader(&writer, &dir)?;

    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    let r = leaf(&reader, 0)?;
    let mut bdv = binary(&r, "val")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&2, &get_value(&*bdv)?);
    assert_equals(&1, &bdv.next_doc()?);
    assert_equals(&2, &get_value(&*bdv)?);
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateFewSegments`.
fn test_update_few_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_max_buffered_docs(2)?; // generate few segments
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?; // prevent merges for this test
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_docs = 10;
    let mut expected_values = vec![0i64; num_docs];
    for i in 0..num_docs {
        writer.add_document(&doc(i as i32)?)?;
        expected_values[i] = (i + 1) as i64;
    }
    writer.commit()?;

    // update few docs
    for i in 0..num_docs {
        if random().next_double() < 0.4 {
            let value = ((i + 1) * 2) as i64;
            writer.update_binary_doc_value(
                Term::from_text("id", &format!("doc-{i}")),
                "val",
                to_bytes(value),
            )?;
            expected_values[i] = value;
        }
    }

    let reader = close_and_open_reader(&writer, &dir)?;

    for context in Arc::clone(&reader).leaves()? {
        let r = context.leaf_reader();
        let bdv = r.get_binary_doc_values("val")?;
        assert_not_null(&bdv);
        let mut bdv = bdv.unwrap_or_else(|| npe("bdv"));
        for i in 0..r.max_doc() {
            assert_equals(&i, &bdv.next_doc()?);
            let expected = expected_values[(i + context.doc_base()) as usize];
            let actual = get_value(&*bdv)?;
            assert_equals(&expected, &actual);
        }
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testReopen`.
fn test_reopen() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.add_document(&doc(0)?)?;
    writer.add_document(&doc(1)?)?;

    let is_nrt = random().next_boolean();
    let reader1 = if is_nrt {
        directory_reader::open_with_writer(&*writer)?
    } else {
        writer.commit()?;
        directory_reader::open(dir.as_directory())?
    };
    println!("TEST: isNRT={is_nrt} reader1={reader1}");

    // update doc
    writer.update_binary_doc_value(Term::from_text("id", "doc-0"), "val", to_bytes(10))?; // update doc-0's value to 10
    if !is_nrt {
        writer.commit()?;
    }

    println!("TEST: now reopen");
    // reopen reader and assert only it sees the update
    let reader2 = directory_reader::open_if_changed(Arc::clone(&reader1))?;
    assert_not_null(&reader2);
    let reader2 = reader2.unwrap_or_else(|| npe("reader2"));
    Assert::assert_true(!Arc::ptr_eq(&reader1, &reader2));

    let mut bdv1 = binary(&leaf(&reader1, 0)?, "val")?;
    let mut bdv2 = binary(&leaf(&reader2, 0)?, "val")?;
    assert_equals(&0, &bdv1.next_doc()?);
    assert_equals(&1, &get_value(&*bdv1)?);
    assert_equals(&0, &bdv2.next_doc()?);
    assert_equals(&10, &get_value(&*bdv2)?);

    writer.close()?;
    IOUtils::close([
        IOUtils::closeable(|| reader1.close()),
        IOUtils::closeable(|| reader2.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestBinaryDocValuesUpdates.testUpdatesAndDeletes`.
fn test_updates_and_deletes() -> Result<()> {
    // create an index with a segment with only deletes, a segment with both
    // deletes and updates and a segment with only updates
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_max_buffered_docs(10)?; // control segment flushing
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?; // prevent merges for this test
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for i in 0..6 {
        writer.add_document(&doc(i)?)?;
        if i % 2 == 1 {
            writer.commit()?; // create 2-docs segments
        }
    }

    // delete doc-1 and doc-2
    writer.delete_documents_by_terms(vec![
        Term::from_text("id", "doc-1"),
        Term::from_text("id", "doc-2"),
    ])?; // 1st and 2nd segments

    // update docs 3 and 5
    writer.update_binary_doc_value(Term::from_text("id", "doc-3"), "val", to_bytes(17))?;
    writer.update_binary_doc_value(Term::from_text("id", "doc-5"), "val", to_bytes(17))?;

    let reader = close_and_open_reader(&writer, &dir)?;

    let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let live_docs = MultiBits::get_live_docs(&ir)?.unwrap_or_else(|| npe("liveDocs"));
    let expected_live_docs = [true, false, false, true, true, true];
    for i in 0..expected_live_docs.len() {
        assert_equals(&expected_live_docs[i], &live_docs.get(i));
    }

    let expected_values = [1i64, 2, 3, 17, 5, 17];
    let mut bdv = multi_binary(&reader, "val")?;
    for i in 0..expected_values.len() {
        assert_equals(&(i as i32), &bdv.next_doc()?);
        assert_equals(&expected_values[i], &get_value(&*bdv)?);
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdatesWithDeletes`.
fn test_updates_with_deletes() -> Result<()> {
    // update and delete different documents in the same commit session
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?; // otherwise the delete might force a merge
    conf.set_max_buffered_docs(10)?; // control segment flushing
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    writer.add_document(&doc(0)?)?;
    writer.add_document(&doc(1)?)?;

    if random().next_boolean() {
        writer.commit()?;
    }

    writer.delete_documents_by_terms(vec![Term::from_text("id", "doc-0")])?;
    writer.update_binary_doc_value(Term::from_text("id", "doc-1"), "val", to_bytes(17))?;

    let reader = close_and_open_reader(&writer, &dir)?;

    let r = leaf(&reader, 0)?;
    Assert::assert_false(
        r.get_live_docs()
            .unwrap_or_else(|| npe("getLiveDocs"))
            .get(0),
    );
    let mut bdv = binary(&r, "val")?;
    assert_equals(&1, &bdv.advance(1)?);
    assert_equals(&17, &get_value(&*bdv)?);

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testMultipleDocValuesTypes`.
fn test_multiple_doc_values_types() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_max_buffered_docs(10)?; // prevent merges
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for i in 0..4 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "dvUpdateKey",
            "dv".into(),
            Store::NO,
        )?));
        doc.add(Box::new(NumericDocValuesField::new("ndv", i as i64)));
        doc.add(bdv_field(
            "bdv",
            LuceneTestCase::new_bytes_ref_str(&i.to_string()),
        ));
        doc.add(Box::new(SortedDocValuesField::new(
            "sdv",
            LuceneTestCase::new_bytes_ref_str(&i.to_string()),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "ssdv",
            LuceneTestCase::new_bytes_ref_str(&i.to_string()),
        )));
        doc.add(Box::new(SortedSetDocValuesField::new(
            "ssdv",
            LuceneTestCase::new_bytes_ref_str(&(i * 2).to_string()),
        )));
        writer.add_document(&doc)?;
    }
    writer.commit()?;

    // update all docs' bdv field
    writer.update_binary_doc_value(Term::from_text("dvUpdateKey", "dv"), "bdv", to_bytes(17))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let r = leaf(&reader, 0)?;
    let mut ndv = r
        .get_numeric_doc_values("ndv")?
        .unwrap_or_else(|| npe("ndv"));
    let mut bdv = binary(&r, "bdv")?;
    let mut sdv = r
        .get_sorted_doc_values("sdv")?
        .unwrap_or_else(|| npe("sdv"));
    let mut ssdv = r
        .get_sorted_set_doc_values("ssdv")?
        .unwrap_or_else(|| npe("ssdv"));
    for i in 0..r.max_doc() {
        assert_equals(&i, &ndv.next_doc()?);
        assert_equals(&(i as i64), &ndv.long_value()?);
        assert_equals(&i, &bdv.next_doc()?);
        assert_equals(&17, &get_value(&*bdv)?);
        assert_equals(&i, &sdv.next_doc()?);
        let term = sdv.lookup_ord(sdv.ord_value()?)?;
        assert_equals(&LuceneTestCase::new_bytes_ref_str(&i.to_string()), &term);
        assert_equals(&i, &ssdv.next_doc()?);

        let mut ord = ssdv.next_ord()?;
        let term = ssdv.lookup_ord(ord)?;
        assert_equals(&i, &parse_int(&term.utf8_to_string()?));
        // For the i=0 case, we added the same value twice, which was dedup'd by IndexWriter so it has
        // only one value:
        if i == 0 {
            assert_equals(&1, &ssdv.doc_value_count()?);
        } else {
            assert_equals(&2, &ssdv.doc_value_count()?);
            ord = ssdv.next_ord()?;
            let term = ssdv.lookup_ord(ord)?;
            assert_equals(&(i * 2), &parse_int(&term.utf8_to_string()?));
        }
    }

    reader.close()?;
    dir.close()
}

/// `Integer.parseInt(s)`.
fn parse_int(s: &str) -> i32 {
    s.parse().unwrap_or_else(|_| {
        Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// Port of `TestBinaryDocValuesUpdates.testMultipleBinaryDocValues`.
fn test_multiple_binary_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_max_buffered_docs(10)?; // prevent merges
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for i in 0..2 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "dvUpdateKey",
            "dv".into(),
            Store::NO,
        )?));
        doc.add(bdv_field("bdv1", to_bytes(i)));
        doc.add(bdv_field("bdv2", to_bytes(i)));
        writer.add_document(&doc)?;
    }
    writer.commit()?;

    // update all docs' bdv1 field
    writer.update_binary_doc_value(Term::from_text("dvUpdateKey", "dv"), "bdv1", to_bytes(17))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let r = leaf(&reader, 0)?;

    let mut bdv1 = binary(&r, "bdv1")?;
    let mut bdv2 = binary(&r, "bdv2")?;
    for i in 0..r.max_doc() {
        assert_equals(&i, &bdv1.next_doc()?);
        assert_equals(&17, &get_value(&*bdv1)?);
        assert_equals(&i, &bdv2.next_doc()?);
        assert_equals(&(i as i64), &get_value(&*bdv2)?);
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testDocumentWithNoValue`.
fn test_document_with_no_value() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for i in 0..2 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "dvUpdateKey",
            "dv".into(),
            Store::NO,
        )?));
        if i == 0 {
            // index only one document with value
            doc.add(bdv_field("bdv", to_bytes(5)));
        }
        writer.add_document(&doc)?;
    }
    writer.commit()?;

    // update all docs' bdv field
    writer.update_binary_doc_value(Term::from_text("dvUpdateKey", "dv"), "bdv", to_bytes(17))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let r = leaf(&reader, 0)?;
    let mut bdv = binary(&r, "bdv")?;
    for i in 0..r.max_doc() {
        assert_equals(&i, &bdv.next_doc()?);
        assert_equals(&17, &get_value(&*bdv)?);
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateNonBinaryDocValuesField`.
fn test_update_non_binary_doc_values_field() -> Result<()> {
    // we don't support adding new fields or updating existing non-binary-dv
    // fields through binary updates
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("key", "doc".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    writer.add_document(&doc)?; // flushed document
    writer.commit()?;
    writer.add_document(&doc)?; // in-memory document

    expect_throws(IAE, || {
        writer.update_binary_doc_value(Term::from_text("key", "doc"), "bdv", to_bytes(17))
    });

    expect_throws(IAE, || {
        writer.update_binary_doc_value(Term::from_text("key", "doc"), "foo", to_bytes(17))
    });

    writer.close()?;
    dir.close()
}

/// `new AssertingCodec() { getDocValuesFormatForField(field) { return
/// format(); } }`.
fn asserting_codec_with_dv_format(
    format: impl Fn() -> Arc<dyn DocValuesFormat> + Send + Sync + 'static,
) -> AssertingCodec {
    AssertingCodec::with_formats_for_field(None, Some(Arc::new(move |_field: &str| format())), None)
}

/// Port of `TestBinaryDocValuesUpdates.testDifferentDVFormatPerField`.
fn test_different_dv_format_per_field() -> Result<()> {
    // test relies on separate instances of "same thing"
    debug_assert!(!Arc::ptr_eq(
        &TestUtil::get_default_doc_values_format(),
        &TestUtil::get_default_doc_values_format()
    ));
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_codec(Arc::new(asserting_codec_with_dv_format(
        TestUtil::get_default_doc_values_format,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("key", "doc".into(), Store::NO)?));
    doc.add(bdv_field("bdv", to_bytes(5)));
    doc.add(Box::new(SortedDocValuesField::new(
        "sorted",
        LuceneTestCase::new_bytes_ref_str("value"),
    )));
    writer.add_document(&doc)?; // flushed document
    writer.commit()?;
    writer.add_document(&doc)?; // in-memory document

    writer.update_binary_doc_value(Term::from_text("key", "doc"), "bdv", to_bytes(17))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;

    let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut bdv = multi_binary(&reader, "bdv")?;
    let mut sdv = MultiDocValues::get_sorted_values(&ir, "sorted")?
        .unwrap_or_else(|| npe("MultiDocValues.getSortedValues"));
    for i in 0..reader.max_doc() {
        assert_equals(&i, &bdv.next_doc()?);
        assert_equals(&17, &get_value(&*bdv)?);
        assert_equals(&i, &sdv.next_doc()?);
        let term = sdv.lookup_ord(sdv.ord_value()?)?;
        assert_equals(&LuceneTestCase::new_bytes_ref_str("value"), &term);
    }

    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateSameDocMultipleTimes`.
fn test_update_same_doc_multiple_times() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("key", "doc".into(), Store::NO)?));
    doc.add(bdv_field("bdv", to_bytes(5)));
    writer.add_document(&doc)?; // flushed document
    writer.commit()?;
    writer.add_document(&doc)?; // in-memory document

    writer.update_binary_doc_value(Term::from_text("key", "doc"), "bdv", to_bytes(17))?; // update existing field
    writer.update_binary_doc_value(Term::from_text("key", "doc"), "bdv", to_bytes(3))?; // update existing field 2nd time in this commit
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let mut bdv = multi_binary(&reader, "bdv")?;
    for i in 0..reader.max_doc() {
        assert_equals(&i, &bdv.next_doc()?);
        assert_equals(&3, &get_value(&*bdv)?);
    }
    reader.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testSegmentMerges`.
fn test_segment_merges() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let random = random();
    let conf = new_iwc_with(&*random);
    let mut writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut docid = 0;
    let num_rounds = LuceneTestCase::at_least(10);
    for rnd in 0..num_rounds {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("key", "doc".into(), Store::NO)?));
        doc.add(bdv_field("bdv", to_bytes(-1)));
        let num_docs = LuceneTestCase::at_least(30);
        for _ in 0..num_docs {
            doc.remove_field("id");
            doc.add(Box::new(StringField::new(
                "id",
                docid.to_string(),
                Store::NO,
            )?));
            docid += 1;
            writer.add_document(&doc)?;
        }

        let value = (rnd + 1) as i64;
        writer.update_binary_doc_value(Term::from_text("key", "doc"), "bdv", to_bytes(value))?;

        if random.next_double() < 0.2 {
            // randomly delete one doc
            writer.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &random.next_int_bound(docid).to_string(),
            )])?;
        }

        // randomly commit or reopen-IW (or nothing), before forceMerge
        if random.next_double() < 0.4 {
            writer.commit()?;
        } else if random.next_double() < 0.1 {
            writer.close()?;
            let conf = new_iwc_with(&*random);
            writer = IndexWriter::new(dir.as_directory(), conf)?;
        }

        // add another document with the current value, to be sure forceMerge has
        // something to merge (for instance, it could be that CMS finished merging
        // all segments down to 1 before the delete was applied, so when
        // forceMerge is called, the index will be with one segment and deletes
        // and some MPs might now merge it, thereby invalidating test's
        // assumption that the reader has no deletes).
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            docid.to_string(),
            Store::NO,
        )?));
        docid += 1;
        doc.add(Box::new(StringField::new("key", "doc".into(), Store::NO)?));
        doc.add(bdv_field("bdv", to_bytes(value)));
        writer.add_document(&doc)?;

        writer.force_merge_with(1, true)?;
        let reader = if random.next_boolean() {
            writer.commit()?;
            directory_reader::open(dir.as_directory())?
        } else {
            directory_reader::open_with_writer(&*writer)?
        };

        assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
        let r = leaf(&reader, 0)?;
        Assert::assert_true_msg(
            "index should have no deletes after forceMerge",
            r.get_live_docs().is_none(),
        );
        let bdv = r.get_binary_doc_values("bdv")?;
        assert_not_null(&bdv);
        let mut bdv = bdv.unwrap_or_else(|| npe("bdv"));
        for i in 0..r.max_doc() {
            assert_equals(&i, &bdv.next_doc()?);
            assert_equals(&value, &get_value(&*bdv)?);
        }
        reader.close()?;
    }

    writer.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateDocumentByMultipleTerms`.
fn test_update_document_by_multiple_terms() -> Result<()> {
    // make sure the order of updates is respected, even when multiple terms affect same document
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("k1", "v1".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("k2", "v2".into(), Store::NO)?));
    doc.add(bdv_field("bdv", to_bytes(5)));
    writer.add_document(&doc)?; // flushed document
    writer.commit()?;
    writer.add_document(&doc)?; // in-memory document

    writer.update_binary_doc_value(Term::from_text("k1", "v1"), "bdv", to_bytes(17))?;
    writer.update_binary_doc_value(Term::from_text("k2", "v2"), "bdv", to_bytes(3))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let mut bdv = multi_binary(&reader, "bdv")?;
    for i in 0..reader.max_doc() {
        assert_equals(&i, &bdv.next_doc()?);
        assert_equals(&3, &get_value(&*bdv)?);
    }
    reader.close()?;
    dir.close()
}

/// `static class OneSortDoc implements Comparable<OneSortDoc>`.
#[derive(Debug)]
struct OneSortDoc {
    value: BytesRef,
    sort_value: i64,
    #[allow(dead_code)]
    id: i32,
    deleted: bool,
}

impl OneSortDoc {
    fn new(id: i32, value: BytesRef, sort_value: i64) -> Self {
        OneSortDoc {
            value,
            sort_value,
            id,
            deleted: false,
        }
    }

    /// `compareTo(OneSortDoc other)`.
    #[allow(dead_code)]
    fn compare_to(&self, other: &OneSortDoc) -> std::cmp::Ordering {
        let mut cmp = self.sort_value.cmp(&other.sort_value);
        if cmp == std::cmp::Ordering::Equal {
            cmp = self.id.cmp(&other.id);
            debug_assert!(cmp != std::cmp::Ordering::Equal);
        }
        cmp
    }
}

/// Port of `TestBinaryDocValuesUpdates.testSortedIndex`.
fn test_sorted_index() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_sort(Sort::new_fields(vec![SortField::new(
        Some("sort".to_string()),
        SortFieldType::Long,
    )?])?)?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let value_range = TestUtil::next_int(&*random(), 1, 1000);
    let sort_value_range = TestUtil::next_int(&*random(), 1, 1000);

    let refresh_chance = TestUtil::next_int(&*random(), 5, 200);
    let delete_chance = TestUtil::next_int(&*random(), 2, 100);

    let mut deleted_count = 0;

    let mut docs: Vec<OneSortDoc> = Vec::new();
    let mut r = w.get_reader()?;

    let num_iters = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        LuceneTestCase::at_least(100)
    };
    for iter in 0..num_iters {
        let value = to_bytes(random().next_int_bound(value_range) as i64);
        if docs.is_empty() || random().next_int_bound(3) == 1 {
            let id = docs.len() as i32;
            // add new doc
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &id.to_string(),
                Store::YES,
            )));
            doc.add(bdv_field("number", value.clone()));
            let sort_value = random().next_int_bound(sort_value_range);
            doc.add(Box::new(NumericDocValuesField::new(
                "sort",
                sort_value as i64,
            )));
            if verbose() {
                println!("TEST: iter={iter} add doc id={id} sortValue={sort_value} value={value}");
            }
            w.add_document(&doc)?;

            docs.push(OneSortDoc::new(id, value, sort_value as i64));
        } else {
            // update existing doc value
            let id_to_update = random().next_int_bound(docs.len() as i32);
            if verbose() {
                println!("TEST: iter={iter} update doc id={id_to_update} new value={value}");
            }
            w.update_binary_doc_value(
                Term::from_text("id", &id_to_update.to_string()),
                "number",
                value.clone(),
            )?;

            docs[id_to_update as usize].value = value;
        }

        if random().next_int_bound(delete_chance) == 0 {
            let id_to_delete = random().next_int_bound(docs.len() as i32);
            if verbose() {
                println!("TEST: delete doc id={id_to_delete}");
            }
            w.delete_documents(Term::from_text("id", &id_to_delete.to_string()))?;
            if !docs[id_to_delete as usize].deleted {
                docs[id_to_delete as usize].deleted = true;
                deleted_count += 1;
            }
        }

        if random().next_int_bound(refresh_chance) == 0 {
            if verbose() {
                println!("TEST: now get reader; old reader={r}");
            }
            let r2 = w.get_reader()?;
            r.close()?;
            r = r2;

            if verbose() {
                println!("TEST: got reader={r}");
            }

            let mut live_count = 0;

            for ctx in Arc::clone(&r).leaves()? {
                let leaf_reader = ctx.leaf_reader();
                let mut values = binary(&leaf_reader, "number")?;
                let mut sort_values = leaf_reader
                    .get_numeric_doc_values("sort")?
                    .unwrap_or_else(|| npe("sortValues"));
                let live_docs = leaf_reader.get_live_docs();
                let stored_fields = leaf_reader.stored_fields()?;

                let mut last_sort_value = i64::MIN;
                for i in 0..leaf_reader.max_doc() {
                    let doc = stored_fields.document(i)?;
                    let sort_doc =
                        &docs[parse_int(&doc.get("id").unwrap_or_else(|| npe("doc.get(\"id\")")))
                            as usize];

                    assert_equals(&i, &values.next_doc()?);
                    assert_equals(&i, &sort_values.next_doc()?);

                    if let Some(live_docs) = &live_docs {
                        if !live_docs.get(i as usize) {
                            Assert::assert_true(sort_doc.deleted);
                            continue;
                        }
                    }
                    Assert::assert_false(sort_doc.deleted);

                    assert_equals(&sort_doc.value, &values.binary_value()?);

                    let sort_value = sort_values.long_value()?;
                    assert_equals(&sort_doc.sort_value, &sort_value);

                    Assert::assert_true(sort_value >= last_sort_value);
                    last_sort_value = sort_value;
                    live_count += 1;
                }
            }

            assert_equals(&(docs.len() as i32 - deleted_count), &live_count);
        }
    }

    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestBinaryDocValuesUpdates.testManyReopensAndFields`.
fn test_many_reopens_and_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let random = random();
    let mut conf = new_iwc_with(&*random);
    let lmp = LuceneTestCase::new_log_merge_policy();
    lmp.set_merge_factor(3)?; // merge often
    conf.set_merge_policy(Arc::new(lmp))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let is_nrt = random.next_boolean();
    let mut reader = if is_nrt {
        directory_reader::open_with_writer(&*writer)?
    } else {
        writer.commit()?;
        directory_reader::open(dir.as_directory())?
    };

    let num_fields = random.next_int_bound(4) + 3; // 3-7
    let mut field_values = vec![1i64; num_fields as usize];

    let num_rounds = LuceneTestCase::at_least(15);
    let mut doc_id = 0;
    for _ in 0..num_rounds {
        let num_docs = LuceneTestCase::at_least(5);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "id",
                format!("doc-{doc_id}"),
                Store::NO,
            )?));
            doc.add(Box::new(StringField::new("key", "all".into(), Store::NO)?)); // update key
                                                                                  // add all fields with their current value
            for f in 0..field_values.len() {
                doc.add(bdv_field(&format!("f{f}"), to_bytes(field_values[f])));
            }
            writer.add_document(&doc)?;
            doc_id += 1;
        }

        let field_idx = random.next_int_bound(field_values.len() as i32) as usize;
        let update_field = format!("f{field_idx}");
        field_values[field_idx] += 1;
        writer.update_binary_doc_value(
            Term::from_text("key", "all"),
            &update_field,
            to_bytes(field_values[field_idx]),
        )?;

        if random.next_double() < 0.2 {
            let delete_doc = random.next_int_bound(doc_id); // might also delete an already deleted document, ok!
            writer.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &format!("doc-{delete_doc}"),
            )])?;
        }

        // verify reader
        if !is_nrt {
            writer.commit()?;
        }

        let new_reader = directory_reader::open_if_changed(Arc::clone(&reader))?;
        assert_not_null(&new_reader);
        reader.close()?;
        reader = new_reader.unwrap_or_else(|| npe("newReader"));
        Assert::assert_true(reader.num_docs() > 0); // we delete at most one document per round
        for context in Arc::clone(&reader).leaves()? {
            let r = context.leaf_reader();
            let live_docs = r.get_live_docs();
            for field in 0..field_values.len() {
                let f = format!("f{field}");
                let bdv = r.get_binary_doc_values(&f)?;
                assert_not_null(&bdv);
                let mut bdv = bdv.unwrap_or_else(|| npe("bdv"));
                let max_doc = r.max_doc();
                for doc in 0..max_doc {
                    if live_docs.as_ref().is_none_or(|l| l.get(doc as usize)) {
                        assert_equals(&doc, &bdv.advance(doc)?);
                        assert_equals_msg(
                            Some(&format!(
                                "invalid value for doc={doc}, field={f}, reader={r}"
                            )),
                            &field_values[field],
                            &get_value(&*bdv)?,
                        );
                    }
                }
            }
        }
    }

    writer.close()?;
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateSegmentWithNoDocValues`.
fn test_update_segment_with_no_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    // prevent merges, otherwise by the time updates are applied
    // (writer.close()), the segments might have merged and that update becomes
    // legit.
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // first segment with BDV
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc0".into(), Store::NO)?));
    doc.add(bdv_field("bdv", to_bytes(3)));
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc4".into(), Store::NO)?)); // document without 'bdv' field
    writer.add_document(&doc)?;
    writer.commit()?;

    // second segment with no BDV
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc1".into(), Store::NO)?));
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc2".into(), Store::NO)?)); // document that isn't updated
    writer.add_document(&doc)?;
    writer.commit()?;

    // update document in the first segment - should not affect docsWithField of
    // the document without BDV field
    writer.update_binary_doc_value(Term::from_text("id", "doc0"), "bdv", to_bytes(5))?;

    // update document in the second segment - field should be added and we should
    // be able to handle the other document correctly (e.g. no NPE)
    writer.update_binary_doc_value(Term::from_text("id", "doc1"), "bdv", to_bytes(5))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        let r = context.leaf_reader();
        let mut bdv = binary(&r, "bdv")?;
        assert_equals(&0, &bdv.next_doc()?);
        assert_equals(&5, &get_value(&*bdv)?);
        assert_equals(&NO_MORE_DOCS, &bdv.next_doc()?);
    }
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateSegmentWithPostingButNoDocValues`.
fn test_update_segment_with_posting_but_no_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    // prevent merges, otherwise by the time updates are applied
    // (writer.close()), the segments might have merged and that update becomes
    // legit.
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // first segment with BDV
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc0".into(), Store::NO)?));
    doc.add(Box::new(StringField::new(
        "bdv",
        "mock-value".into(),
        Store::NO,
    )?));
    doc.add(bdv_field("bdv", to_bytes(5)));
    writer.add_document(&doc)?;
    writer.commit()?;

    // second segment with no BDV
    let mut doc2 = Document::new();
    doc2.add(Box::new(StringField::new("id", "doc1".into(), Store::NO)?));
    doc2.add(Box::new(StringField::new(
        "bdv",
        "mock-value".into(),
        Store::NO,
    )?));
    let exception = expect_throws(IAE, || writer.add_document(&doc2));
    let expected_err_msg =
        "cannot change field \"bdv\" from doc values type=BINARY to inconsistent doc values type=NONE";
    assert_equals(&Some(expected_err_msg), &exception.message());

    doc2.add(bdv_field("bdv", to_bytes(10)));
    writer.add_document(&doc2)?;

    // update doc values of bdv field in the second segment
    let exception = expect_throws(IAE, || {
        writer.update_binary_doc_value(Term::from_text("id", "doc1"), "bdv", to_bytes(5))
    });
    let expected_err_msg = "Can't update [BINARY] doc values; the field [bdv] must be doc values only field, but is also indexed with postings.";
    assert_equals(&Some(expected_err_msg), &exception.message());

    writer.commit()?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let r1 = leaf(&reader, 0)?;
    let mut bdv1 = binary(&r1, "bdv")?;
    assert_equals(&0, &bdv1.next_doc()?);
    assert_equals(&5, &get_value(&*bdv1)?);
    let r2 = leaf(&reader, 1)?;
    let mut bdv2 = binary(&r2, "bdv")?;
    assert_equals(&1, &bdv2.next_doc()?);
    assert_equals(&10, &get_value(&*bdv2)?);
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateBinaryDVFieldWithSameNameAsPostingField`.
fn test_update_binary_dv_field_with_same_name_as_posting_field() -> Result<()> {
    // this used to fail because FieldInfos.Builder neglected to update
    // globalFieldMaps.docValuesTypes map
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "f",
        "mock-value".into(),
        Store::NO,
    )?));
    doc.add(bdv_field("f", to_bytes(5)));
    writer.add_document(&doc)?;
    writer.commit()?;

    let exception = expect_throws(IAE, || {
        writer.update_binary_doc_value(Term::from_text("f", "mock-value"), "f", to_bytes(17))
    });
    let expected_err_msg = "Can't update [BINARY] doc values; the field [f] must be doc values only field, but is also indexed with postings.";
    assert_equals(&Some(expected_err_msg), &exception.message());
    writer.close()?;

    let r = directory_reader::open(dir.as_directory())?;
    let mut bdv = binary(&leaf(&r, 0)?, "f")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&5, &get_value(&*bdv)?);
    r.close()?;

    dir.close()
}

/// The group a `random.nextDouble()` picks in `testStressMultiThreading`.
fn group_of(group: f64) -> &'static str {
    if group < 0.1 {
        "g0"
    } else if group < 0.5 {
        "g1"
    } else if group < 0.8 {
        "g2"
    } else {
        "g3"
    }
}

/// Port of `TestBinaryDocValuesUpdates.testStressMultiThreading`.
fn test_stress_multi_threading() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // create index
    let num_fields = TestUtil::next_int(&*random(), 1, 4);
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(2000)
    } else {
        LuceneTestCase::at_least(200)
    };
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            format!("doc{i}"),
            Store::NO,
        )?));
        let group = random().next_double();
        let g = group_of(group);
        doc.add(Box::new(StringField::new("updKey", g.into(), Store::NO)?));
        for j in 0..num_fields {
            let value = random().next_int() as i64;
            doc.add(bdv_field(&format!("f{j}"), to_bytes(value)));
            doc.add(bdv_field(
                &format!("cf{j}"),
                to_bytes(value.wrapping_mul(2)),
            )); // control, always updated to f * 2
        }
        writer.add_document(&doc)?;
    }

    let num_threads = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 3, 6)
    } else {
        2
    };
    let done = Arc::new(CountDownLatch::new(num_threads as i64));
    let num_updates = Arc::new(AtomicI32::new(LuceneTestCase::at_least(100)));

    // same thread updates a field as well as reopens
    let mut threads = Vec::with_capacity(num_threads as usize);
    for i in 0..num_threads {
        let writer = Arc::clone(&writer);
        let done = Arc::clone(&done);
        let num_updates = Arc::clone(&num_updates);
        threads.push(Thread::new(format!("UpdateThread-{i}"), move || {
            let mut reader: Option<Arc<dyn DirectoryReader>> = None;
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let random = random();
                    while num_updates.fetch_sub(1, Ordering::SeqCst) > 0 {
                        let group = random.next_double();
                        let t = Term::from_text("updKey", group_of(group));
                        let field = random.next_int_bound(num_fields);
                        let f = format!("f{field}");
                        let cf = format!("cf{field}");
                        let upd_value = random.next_int() as i64;
                        writer.update_doc_values(
                            t,
                            &[
                                bdv_update(&f, to_bytes(upd_value))?,
                                bdv_update(&cf, to_bytes(upd_value * 2))?,
                            ],
                        )?;

                        if random.next_double() < 0.2 {
                            // delete a random document
                            let doc = random.next_int_bound(num_docs);
                            writer.delete_documents_by_terms(vec![Term::from_text(
                                "id",
                                &format!("doc{doc}"),
                            )])?;
                        }

                        if random.next_double() < 0.05 {
                            // commit every 20 updates on average
                            writer.commit()?;
                        }

                        if random.next_double() < 0.1 {
                            // reopen NRT reader (apply updates), on average once every 10 updates
                            match &reader {
                                None => {
                                    reader = Some(directory_reader::open_with_writer(&*writer)?);
                                }
                                Some(current) => {
                                    let r2 = directory_reader::open_if_changed_with_writer(
                                        Arc::clone(current),
                                        &*writer,
                                        true,
                                    )?;
                                    if let Some(r2) = r2 {
                                        current.close()?;
                                        reader = Some(r2);
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }));
            let success = matches!(outcome, Ok(Ok(())));
            // `finally`
            let mut close_failure = None;
            if let Some(reader) = reader.take() {
                if let Err(e) = reader.close() {
                    if success {
                        // suppress this exception only if there was another exception
                        close_failure = Some(e);
                    }
                }
            }
            done.count_down();
            match outcome {
                Ok(Ok(())) => {
                    if let Some(e) = close_failure {
                        Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e))
                            .throw()
                    }
                }
                Ok(Err(e)) if is_instance(&e, "java.io.IOException") => {
                    Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw()
                }
                Ok(Err(e)) => java_throwable(&e).throw(),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }));
    }

    for t in &threads {
        t.start();
    }
    done.await_();
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        let r = context.leaf_reader();
        for i in 0..num_fields {
            let mut bdv = binary(&r, &format!("f{i}"))?;
            let mut control = binary(&r, &format!("cf{i}"))?;
            let live_docs = r.get_live_docs();
            for j in 0..r.max_doc() {
                if live_docs.as_ref().is_none_or(|l| l.get(j as usize)) {
                    assert_equals(&j, &bdv.advance(j)?);
                    assert_equals(&j, &control.advance(j)?);
                    assert_equals(&get_value(&*control)?, &(get_value(&*bdv)?.wrapping_mul(2)));
                }
            }
        }
    }
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateDifferentDocsInDifferentGens`.
fn test_update_different_docs_in_different_gens() -> Result<()> {
    // update same document multiple times across generations
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_max_buffered_docs(4)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let num_docs = LuceneTestCase::at_least(10);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            format!("doc{i}"),
            Store::NO,
        )?));
        let value = random().next_int() as i64;
        doc.add(bdv_field("f", to_bytes(value)));
        doc.add(bdv_field("cf", to_bytes(value * 2)));
        writer.add_document(&doc)?;
    }

    let num_gens = LuceneTestCase::at_least(5);
    for _ in 0..num_gens {
        let doc = random().next_int_bound(num_docs);
        let t = Term::from_text("id", &format!("doc{doc}"));
        let value = random().next_long();
        writer.update_doc_values(
            t,
            &[
                bdv_update("f", to_bytes(value))?,
                bdv_update("cf", to_bytes(value.wrapping_mul(2)))?,
            ],
        )?;
        let reader = directory_reader::open_with_writer(&*writer)?;
        for context in Arc::clone(&reader).leaves()? {
            let r = context.leaf_reader();
            let mut fbdv = binary(&r, "f")?;
            let mut cfbdv = binary(&r, "cf")?;
            for j in 0..r.max_doc() {
                assert_equals(&j, &fbdv.next_doc()?);
                assert_equals(&j, &cfbdv.next_doc()?);
                assert_equals(&get_value(&*cfbdv)?, &(get_value(&*fbdv)?.wrapping_mul(2)));
            }
        }
        reader.close()?;
    }
    writer.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testChangeCodec`.
fn test_change_codec() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut conf = new_iwc();
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?; // disable merges to simplify test assertions.
    conf.set_codec(Arc::new(asserting_codec_with_dv_format(
        TestUtil::get_default_doc_values_format,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "d0".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(5)));
    doc.add(bdv_field("f2", to_bytes(13)));
    writer.add_document(&doc)?;
    writer.close()?;

    // change format
    let mut conf = new_iwc();
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?; // disable merges to simplify test assertions.
    conf.set_codec(Arc::new(asserting_codec_with_dv_format(|| {
        Arc::new(AssertingDocValuesFormat::new()) as Arc<dyn DocValuesFormat>
    })))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "d1".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(17)));
    doc.add(bdv_field("f2", to_bytes(2)));
    writer.add_document(&doc)?;
    writer.update_binary_doc_value(Term::from_text("id", "d0"), "f1", to_bytes(12))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let mut f1 = multi_binary(&reader, "f1")?;
    let mut f2 = multi_binary(&reader, "f2")?;
    assert_equals(&0, &f1.next_doc()?);
    assert_equals(&0, &f2.next_doc()?);
    assert_equals(&12, &get_value(&*f1)?);
    assert_equals(&13, &get_value(&*f2)?);
    assert_equals(&1, &f1.next_doc()?);
    assert_equals(&1, &f2.next_doc()?);
    assert_equals(&17, &get_value(&*f1)?);
    assert_equals(&2, &get_value(&*f2)?);
    reader.close()?;
    dir.close()
}

/// `RandomPicks.randomFrom(random, set)` over a Java `HashSet<String>`, in
/// its iteration order.
fn random_from_set(random: &dyn Random, set: &JavaHashSet<String>) -> String {
    RandomPicks::random_from_list(random, &set.to_vec()).clone()
}

/// Port of `TestBinaryDocValuesUpdates.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir1.as_directory(), conf)?;

    let num_docs = LuceneTestCase::at_least(50);
    let num_terms = TestUtil::next_int(&*random(), 1, num_docs / 5);
    let mut random_terms: JavaHashSet<String> = JavaHashSet::new();
    while (random_terms.size() as i32) < num_terms {
        random_terms.add(TestUtil::random_simple_string(&*random()));
    }

    // create first index
    for _ in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            random_from_set(&*random(), &random_terms),
            Store::NO,
        )?));
        doc.add(bdv_field("bdv", to_bytes(4)));
        doc.add(bdv_field("control", to_bytes(8)));
        writer.add_document(&doc)?;
    }

    if random().next_boolean() {
        writer.commit()?;
    }

    // update some docs to a random value
    let value = random().next_int() as i64;
    let term = Term::from_text("id", &random_from_set(&*random(), &random_terms));
    writer.update_doc_values(
        term,
        &[
            bdv_update("bdv", to_bytes(value))?,
            bdv_update("control", to_bytes(value * 2))?,
        ],
    )?;
    writer.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir2.as_directory(), conf)?;
    if random().next_boolean() {
        writer.add_indexes_dirs(&[dir1.as_directory()])?;
    } else {
        let reader = directory_reader::open(dir1.as_directory())?;
        TestUtil::add_indexes_slowly(&writer, &[Arc::clone(&reader)])?;
        reader.close()?;
    }
    writer.close()?;

    let reader = directory_reader::open(dir2.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        let r = context.leaf_reader();
        let mut bdv = binary(&r, "bdv")?;
        let mut control = binary(&r, "control")?;
        for i in 0..r.max_doc() {
            assert_equals(&i, &bdv.next_doc()?);
            assert_equals(&i, &control.next_doc()?);
            assert_equals(&(get_value(&*bdv)? * 2), &get_value(&*control)?);
        }
    }
    reader.close()?;

    IOUtils::close([
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestBinaryDocValuesUpdates.testDeleteUnusedUpdatesFiles`.
fn test_delete_unused_updates_files() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "d0".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(1)));
    doc.add(bdv_field("f2", to_bytes(1)));
    writer.add_document(&doc)?;

    // update each field twice to make sure all unneeded files are deleted
    for f in ["f1", "f2"] {
        writer.update_binary_doc_value(Term::from_text("id", "d0"), f, to_bytes(2))?;
        writer.commit()?;
        let num_files = dir.as_directory().list_all()?.len();

        // update again, number of files shouldn't change (old field's gen is
        // removed)
        writer.update_binary_doc_value(Term::from_text("id", "d0"), f, to_bytes(3))?;
        writer.commit()?;

        assert_equals(&num_files, &dir.as_directory().list_all()?.len());
    }

    writer.close()?;
    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testTonsOfUpdates`: LUCENE-5248: make
/// sure that when there are many updates, we don't use too much RAM.
fn test_tons_of_updates() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let random = random();
    let mut conf = new_iwc_with(&*random);
    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB)?;
    conf.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?; // don't flush by doc
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    // test data: lots of documents (few 10Ks) and lots of update terms (few hundreds)
    let num_docs = LuceneTestCase::at_least(20000);
    let num_binary_fields = LuceneTestCase::at_least(5);
    let num_terms = TestUtil::next_int(&*random, 10, 100); // terms should affect many docs
    let mut update_terms: JavaHashSet<String> = JavaHashSet::new();
    while (update_terms.size() as i32) < num_terms {
        update_terms.add(TestUtil::random_simple_string(&*random));
    }

    // build a large index with many BDV fields and update terms
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let num_update_terms = TestUtil::next_int(&*random, 1, num_terms / 10);
        for _ in 0..num_update_terms {
            doc.add(Box::new(StringField::new(
                "upd",
                random_from_set(&*random, &update_terms),
                Store::NO,
            )?));
        }
        for j in 0..num_binary_fields {
            let val = random.next_int() as i64;
            doc.add(bdv_field(&format!("f{j}"), to_bytes(val)));
            doc.add(bdv_field(&format!("cf{j}"), to_bytes(val * 2)));
        }
        writer.add_document(&doc)?;
    }

    writer.commit()?; // commit so there's something to apply to

    // set to flush every 2048 bytes (approximately every 12 updates), so we get
    // many flushes during binary updates
    writer
        .get_config()
        .set_ram_buffer_size_mb(2048.0 / 1024.0 / 1024.0)?;
    let num_updates = LuceneTestCase::at_least(100);
    for _ in 0..num_updates {
        let field = random.next_int_bound(num_binary_fields);
        let update_term = Term::from_text("upd", &random_from_set(&*random, &update_terms));
        let value = random.next_int() as i64;
        writer.update_doc_values(
            update_term,
            &[
                bdv_update(&format!("f{field}"), to_bytes(value))?,
                bdv_update(&format!("cf{field}"), to_bytes(value * 2))?,
            ],
        )?;
    }

    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    for context in Arc::clone(&reader).leaves()? {
        for i in 0..num_binary_fields {
            let r = context.leaf_reader();
            let mut f = binary(&r, &format!("f{i}"))?;
            let mut cf = binary(&r, &format!("cf{i}"))?;
            for j in 0..r.max_doc() {
                assert_equals(&j, &f.next_doc()?);
                assert_equals(&j, &cf.next_doc()?);
                assert_equals_msg(
                    Some(&format!("reader={r}, field=f{i}, doc={j}")),
                    &get_value(&*cf)?,
                    &(get_value(&*f)? * 2),
                );
            }
        }
    }
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdatesOrder`.
fn test_updates_order() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("upd", "t1".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("upd", "t2".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(1)));
    doc.add(bdv_field("f2", to_bytes(1)));
    writer.add_document(&doc)?;
    writer.update_binary_doc_value(Term::from_text("upd", "t1"), "f1", to_bytes(2))?; // update f1 to 2
    writer.update_binary_doc_value(Term::from_text("upd", "t1"), "f2", to_bytes(2))?; // update f2 to 2
    writer.update_binary_doc_value(Term::from_text("upd", "t2"), "f1", to_bytes(3))?; // update f1 to 3
    writer.update_binary_doc_value(Term::from_text("upd", "t2"), "f2", to_bytes(3))?; // update f2 to 3
    writer.update_binary_doc_value(Term::from_text("upd", "t1"), "f1", to_bytes(4))?; // update f1 to 4 (but not f2)
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let mut bdv = binary(&leaf(&reader, 0)?, "f1")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&4, &get_value(&*bdv)?);
    let mut bdv = binary(&leaf(&reader, 0)?, "f2")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&3, &get_value(&*bdv)?);
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateAllDeletedSegment`.
fn test_update_all_deleted_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(1)));
    writer.add_document(&doc)?;
    writer.add_document(&doc)?;
    writer.commit()?;
    writer.delete_documents_by_terms(vec![Term::from_text("id", "doc")])?; // delete all docs in the first segment
    writer.add_document(&doc)?;
    writer.update_binary_doc_value(Term::from_text("id", "doc"), "f1", to_bytes(2))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    let mut bdv = binary(&leaf(&reader, 0)?, "f1")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&2, &get_value(&*bdv)?);
    reader.close()?;

    dir.close()
}

/// Port of `TestBinaryDocValuesUpdates.testUpdateTwoNonexistingTerms`.
fn test_update_two_nonexisting_terms() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = new_iwc();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("id", "doc".into(), Store::NO)?));
    doc.add(bdv_field("f1", to_bytes(1)));
    writer.add_document(&doc)?;
    // update w/ multiple nonexisting terms in same field
    writer.update_binary_doc_value(Term::from_text("c", "foo"), "f1", to_bytes(2))?;
    writer.update_binary_doc_value(Term::from_text("c", "bar"), "f1", to_bytes(2))?;
    writer.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    let mut bdv = binary(&leaf(&reader, 0)?, "f1")?;
    assert_equals(&0, &bdv.next_doc()?);
    assert_equals(&1, &get_value(&*bdv)?);
    reader.close()?;

    dir.close()
}

/// The configuration both writers of `testIOContext` use.
fn io_context_config() -> Result<IndexWriterConfig> {
    let mut conf = new_iwc();
    // we want a single large enough segment so that a doc-values update writes a large file
    conf.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
    conf.set_max_buffered_docs(i32::MAX)?; // manually flush
    conf.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
    Ok(conf)
}

/// Port of `TestBinaryDocValuesUpdates.testIOContext`: LUCENE-5591: make
/// sure we pass an IOContext with an approximate segmentSize in FlushInfo.
///
/// `cachingDir.close()` closes the directory it wraps; the
/// `NRTCachingDirectory` here wraps a shared handle of it, whose close does
/// nothing, so the test closes the directory right after the caching one.
fn test_io_context() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), io_context_config()?)?;
    for i in 0..100 {
        writer.add_document(&doc(i)?)?;
    }
    writer.commit()?;
    writer.close()?;

    let mut caching_dir = Arc::new(NRTCachingDirectory::new(
        Box::new(SharedDirectory(dir.as_directory())),
        100.0,
        1.0 / (1024.0 * 1024.0),
    ));
    let writer = IndexWriter::new(
        Arc::clone(&caching_dir) as Arc<dyn Directory>,
        io_context_config()?,
    )?;
    writer.update_binary_doc_value(Term::from_text("id", "doc-0"), "val", to_bytes(100))?;
    let reader = directory_reader::open_with_writer(&*writer)?; // flush
    assert_equals(&0, &caching_dir.list_cached_files()?.len());

    let closed_reader = reader.close();
    let closed_writer = writer.close();
    drop(writer);
    drop(reader);
    let closed_caching = match Arc::get_mut(&mut caching_dir) {
        Some(caching) => caching.close(),
        None => Err(rucene::error::LuceneError::illegal_state(
            "the NRTCachingDirectory is still shared",
        )),
    };
    let closed_dir = dir.close();
    closed_reader?;
    closed_writer?;
    closed_caching?;
    closed_dir
}
