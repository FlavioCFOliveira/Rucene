//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermVectorsWriter.java`.
//!
//! tests for writing term vectors
//!
//! A `Field` the Java test adds to a document several times is one shared
//! field (`Document::add_shared`), as Java adds the same object.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, CachingTokenFilter, SharedTokenStream, TokenStream};
use rucene::document::{Document, Field, FieldType, Store, StoredField, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_ALL};
use rucene::index::terms::TermsEnum;
use rucene::index::{IndexReader, IndexableField};
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::hamcrest::{any_of, starts_with, MatcherAssert};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test};
use crate::support::npe;

/// `TestTermVectorsWriter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermVectorsWriter",
        vec![
            test("testDoubleOffsetCounting", test_double_offset_counting),
            test("testDoubleOffsetCounting2", test_double_offset_counting2),
            test(
                "testEndOffsetPositionCharAnalyzer",
                test_end_offset_position_char_analyzer,
            ),
            test(
                "testEndOffsetPositionWithCachingTokenFilter",
                test_end_offset_position_with_caching_token_filter,
            ),
            test(
                "testEndOffsetPositionStopFilter",
                test_end_offset_position_stop_filter,
            ),
            test(
                "testEndOffsetPositionStandard",
                test_end_offset_position_standard,
            ),
            test(
                "testEndOffsetPositionStandardEmptyField",
                test_end_offset_position_standard_empty_field,
            ),
            test(
                "testEndOffsetPositionStandardEmptyField2",
                test_end_offset_position_standard_empty_field2,
            ),
            test("testTermVectorCorruption", test_term_vector_corruption),
            test("testTermVectorCorruption2", test_term_vector_corruption2),
            test("testTermVectorCorruption3", test_term_vector_corruption3),
            test(
                "testNoTermVectorAfterTermVector",
                test_no_term_vector_after_term_vector,
            ),
            test(
                "testNoTermVectorAfterTermVectorMerge",
                test_no_term_vector_after_term_vector_merge,
            ),
            test(
                "testInconsistentTermVectorOptions",
                test_inconsistent_term_vector_options,
            ),
            test(
                "testNoAbortOnBadTVSettings",
                test_no_abort_on_bad_tv_settings,
            ),
        ],
    )
}

/// `new FieldType(base)` with term vectors, positions and offsets.
fn tv_pos_off_type(base: &FieldType) -> Result<FieldType> {
    let custom_type = FieldType::new_from(base);
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    Ok(custom_type)
}

/// `newField(name, value, type)`, shared between the documents and slots it
/// is added to.
fn shared_field(name: &str, value: &str, field_type: &FieldType) -> Arc<dyn IndexableField> {
    Arc::new(LuceneTestCase::new_field(name, value, field_type))
}

/// `new IndexWriter(dir, newIndexWriterConfig(new MockAnalyzer(random())))`.
fn plain_writer(dir: &Arc<dyn Directory>) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        Arc::clone(dir),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )
}

/// `r.termVectors().get(0).terms("field").iterator()`.
fn field_vector_iterator(r: &Arc<dyn IndexReader>) -> Result<Box<dyn TermsEnum>> {
    r.term_vectors()?
        .get(0)?
        .unwrap_or_else(|| npe("termVectors().get(0)"))
        .terms("field")?
        .unwrap_or_else(|| npe("terms(\"field\")"))
        .iterator()
}

/// `dpEnum.nextPosition()` then the start and end offsets.
fn assert_next_offsets(dp_enum: &mut dyn PostingsEnum, start: i32, end: i32) -> Result<()> {
    dp_enum.next_position()?;
    assert_equals(&start, &dp_enum.start_offset());
    assert_equals(&end, &dp_enum.end_offset());
    Ok(())
}

/// LUCENE-1442
///
/// Port of `TestTermVectorsWriter.testDoubleOffsetCounting`.
fn test_double_offset_counting() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = tv_pos_off_type(StringField::type_not_stored())?;
    let f = shared_field("field", "abcd", &custom_type);
    doc.add_shared(Arc::clone(&f));
    doc.add_shared(Arc::clone(&f));
    let f2 = shared_field("field", "", &custom_type);
    doc.add_shared(f2);
    doc.add_shared(Arc::clone(&f));
    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let vector = r
        .term_vectors()?
        .get(0)?
        .unwrap_or_else(|| npe("termVectors().get(0)"))
        .terms("field")?;
    assert_not_null(&vector);
    let mut terms_enum = vector.expect("checked").iterator()?;
    assert_not_null(&terms_enum.next()?);
    assert_equals(
        "",
        terms_enum
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?
            .as_str(),
    );

    // Token "" occurred once
    assert_equals(&1, &terms_enum.total_term_freq()?);

    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 8, 8)?;
    assert_equals(&NO_MORE_DOCS, &dp_enum.next_doc()?);

    // Token "abcd" occurred three times
    assert_equals(
        &Some(BytesRef::from_char_sequence("abcd")),
        &terms_enum.next()?,
    );
    let mut dp_enum = terms_enum
        .postings(Some(dp_enum), POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&3, &terms_enum.total_term_freq()?);

    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 0, 4)?;
    assert_next_offsets(&mut *dp_enum, 4, 8)?;
    assert_next_offsets(&mut *dp_enum, 8, 12)?;

    assert_equals(&NO_MORE_DOCS, &dp_enum.next_doc()?);
    assert_null(&terms_enum.next()?);
    r.close()?;
    dir.close()
}

/// Indexes `f` twice in one document and checks the two offsets of its one
/// term: the shared body of `testDoubleOffsetCounting2`,
/// `testEndOffsetPositionCharAnalyzer` and `testEndOffsetPositionStopFilter`.
fn check_double_field(
    analyzer: impl FnOnce() -> Arc<dyn Analyzer>,
    value: &str,
    first: (i32, i32),
    second: (i32, i32),
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer())),
    )?;
    let mut doc = Document::new();
    let custom_type = tv_pos_off_type(TextField::type_not_stored())?;
    let f = shared_field("field", value, &custom_type);
    doc.add_shared(Arc::clone(&f));
    doc.add_shared(Arc::clone(&f));
    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms_enum = field_vector_iterator(&r)?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&2, &terms_enum.total_term_freq()?);

    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, first.0, first.1)?;
    assert_next_offsets(&mut *dp_enum, second.0, second.1)?;
    assert_equals(&NO_MORE_DOCS, &dp_enum.next_doc()?);

    r.close()?;
    dir.close()
}

/// LUCENE-1442
///
/// Port of `TestTermVectorsWriter.testDoubleOffsetCounting2`.
fn test_double_offset_counting2() -> Result<()> {
    check_double_field(
        || Arc::new(MockAnalyzer::new(&*random())),
        "abcd",
        (0, 4),
        (5, 9),
    )
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionCharAnalyzer`.
fn test_end_offset_position_char_analyzer() -> Result<()> {
    check_double_field(
        || Arc::new(MockAnalyzer::new(&*random())),
        "abcd   ",
        (0, 4),
        (8, 12),
    )
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionWithCachingTokenFilter`.
fn test_end_offset_position_with_caching_token_filter() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let mut doc = Document::new();
    {
        let stream: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(CachingTokenFilter::new(Box::new(
                SharedTokenStream::new(analyzer.token_stream_from_str("field", "abcd   ")?),
            ))));
        let custom_type = tv_pos_off_type(TextField::type_not_stored())?;
        let f: Arc<dyn IndexableField> = Arc::new(Field::new_with_token_stream(
            "field",
            Arc::clone(&stream),
            custom_type,
        )?);
        doc.add_shared(Arc::clone(&f));
        doc.add_shared(Arc::clone(&f));
        let added = w.add_document(&doc);
        // try-with-resources: `stream.close()`.
        let closed = stream.borrow_mut().close();
        added?;
        closed?;
    }
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms_enum = field_vector_iterator(&r)?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&2, &terms_enum.total_term_freq()?);

    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 0, 4)?;
    assert_next_offsets(&mut *dp_enum, 8, 12)?;
    assert_equals(&NO_MORE_DOCS, &dp_enum.next_doc()?);

    r.close()?;
    dir.close()
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionStopFilter`.
fn test_end_offset_position_stop_filter() -> Result<()> {
    check_double_field(
        || {
            Arc::new(MockAnalyzer::with_filter(
                &*random(),
                Arc::clone(&SIMPLE),
                true,
                Arc::clone(&ENGLISH_STOPSET),
            ))
        },
        "abcd the",
        (0, 4),
        (9, 13),
    )
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionStandard`.
fn test_end_offset_position_standard() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = tv_pos_off_type(TextField::type_not_stored())?;
    let f = shared_field("field", "abcd the  ", &custom_type);
    let f2 = shared_field("field", "crunch man", &custom_type);
    doc.add_shared(f);
    doc.add_shared(f2);
    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms_enum = field_vector_iterator(&r)?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;

    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 0, 4)?;

    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(Some(dp_enum), POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 11, 17)?;

    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(Some(dp_enum), POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 18, 21)?;

    r.close()?;
    dir.close()
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionStandardEmptyField`.
fn test_end_offset_position_standard_empty_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = tv_pos_off_type(TextField::type_not_stored())?;
    let f = shared_field("field", "", &custom_type);
    let f2 = shared_field("field", "crunch man", &custom_type);
    doc.add_shared(f);
    doc.add_shared(f2);
    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms_enum = field_vector_iterator(&r)?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;

    assert_equals(&1, &(terms_enum.total_term_freq()? as i32));
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 1, 7)?;

    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(Some(dp_enum), POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 8, 11)?;

    r.close()?;
    dir.close()
}

/// LUCENE-1448
///
/// Port of `TestTermVectorsWriter.testEndOffsetPositionStandardEmptyField2`.
fn test_end_offset_position_standard_empty_field2() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = plain_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = tv_pos_off_type(TextField::type_not_stored())?;

    let f = shared_field("field", "abcd", &custom_type);
    doc.add_shared(f);
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "",
        &custom_type,
    )));

    let f2 = shared_field("field", "crunch", &custom_type);
    doc.add_shared(f2);

    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let mut terms_enum = field_vector_iterator(&r)?;
    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;

    assert_equals(&1, &(terms_enum.total_term_freq()? as i32));
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 0, 4)?;

    assert_not_null(&terms_enum.next()?);
    let mut dp_enum = terms_enum
        .postings(Some(dp_enum), POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    Assert::assert_true(dp_enum.next_doc()? != NO_MORE_DOCS);
    assert_next_offsets(&mut *dp_enum, 6, 12)?;

    r.close()?;
    dir.close()
}

/// The config of the `testTermVectorCorruption*` writers:
/// `setMaxBufferedDocs(2)`, `setRAMBufferSizeMB(DISABLE_AUTO_FLUSH)`, a
/// `SerialMergeScheduler` and a `LogDocMergePolicy`.
fn corruption_config() -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_max_buffered_docs(2)?;
    iwc.set_ram_buffer_size_mb(f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH))?;
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    Ok(iwc)
}

/// The indexing of `testTermVectorCorruption` and `testTermVectorCorruption2`:
/// two stored-only documents, then one with a term vector field.
fn index_corruption_docs(writer: &IndexWriter) -> Result<()> {
    let mut document = Document::new();
    let custom_type = FieldType::new();
    custom_type.set_stored(true)?;

    let stored_field = shared_field("stored", "stored", &custom_type);
    document.add_shared(Arc::clone(&stored_field));
    writer.add_document(&document)?;
    writer.add_document(&document)?;

    let mut document = Document::new();
    document.add_shared(Arc::clone(&stored_field));
    let custom_type2 = tv_pos_off_type(StringField::type_not_stored())?;
    let term_vector_field = shared_field("termVector", "termVector", &custom_type2);

    document.add_shared(term_vector_field);
    writer.add_document(&document)?;
    writer.force_merge(1)?;
    writer.close()
}

/// LUCENE-1168
///
/// Port of `TestTermVectorsWriter.testTermVectorCorruption`.
fn test_term_vector_corruption() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    for _ in 0..2 {
        let writer = IndexWriter::new(dir.as_directory(), corruption_config()?)?;
        index_corruption_docs(&writer)?;

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let stored_fields = reader.stored_fields()?;
        let term_vectors = reader.term_vectors()?;
        for i in 0..reader.num_docs() {
            stored_fields.document(i)?;
            term_vectors.get(i)?;
        }
        reader.close()?;

        let writer = IndexWriter::new(dir.as_directory(), corruption_config()?)?;

        let ram_copy: Arc<dyn Directory> = Arc::from(TestUtil::ram_copy_of(&*dir.as_directory())?);
        let mut index_dir = MockDirectoryWrapper::new(&*random(), ram_copy);
        let index_dirs = [index_dir.as_directory()];
        writer.add_indexes_dirs(&index_dirs)?;
        writer.force_merge(1)?;
        writer.close()?;
        drop(index_dirs);
        // Java leaves the copy open: it is never closed.
        let _ = &mut index_dir;
    }
    dir.close()
}

/// LUCENE-1168
///
/// Port of `TestTermVectorsWriter.testTermVectorCorruption2`.
fn test_term_vector_corruption2() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    for _ in 0..2 {
        let writer = IndexWriter::new(dir.as_directory(), corruption_config()?)?;
        index_corruption_docs(&writer)?;

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        assert_null(&reader.term_vectors()?.get(0)?.map(|_| ()));
        assert_null(&reader.term_vectors()?.get(1)?.map(|_| ()));
        assert_not_null(&reader.term_vectors()?.get(2)?);
        reader.close()?;
    }
    dir.close()
}

/// LUCENE-1168
///
/// Port of `TestTermVectorsWriter.testTermVectorCorruption3`.
fn test_term_vector_corruption3() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), corruption_config()?)?;

    let mut document = Document::new();
    let custom_type = FieldType::new();
    custom_type.set_stored(true)?;

    let stored_field = shared_field("stored", "stored", &custom_type);
    document.add_shared(stored_field);
    let custom_type2 = tv_pos_off_type(StringField::type_not_stored())?;
    let term_vector_field = shared_field("termVector", "termVector", &custom_type2);
    document.add_shared(term_vector_field);
    for _ in 0..10 {
        writer.add_document(&document)?;
    }
    writer.close()?;

    let writer = IndexWriter::new(dir.as_directory(), corruption_config()?)?;
    for _ in 0..6 {
        writer.add_document(&document)?;
    }

    writer.force_merge(1)?;
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let stored_fields = reader.stored_fields()?;
    let term_vectors = reader.term_vectors()?;
    for i in 0..10 {
        term_vectors.get(i)?;
        stored_fields.document(i)?;
    }
    reader.close()?;
    dir.close()
}

/// LUCENE-1008
///
/// Port of `TestTermVectorsWriter.testNoTermVectorAfterTermVector`.
fn test_no_term_vector_after_term_vector() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_writer(&dir.as_directory())?;
    let mut document = Document::new();
    let custom_type2 = tv_pos_off_type(TextField::type_not_stored())?;
    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "a b c",
        &custom_type2,
    )));
    iw.add_document(&document)?;
    let mut document = Document::new();
    document.add(Box::new(LuceneTestCase::new_text_field(
        "tvtest",
        "x y z",
        Store::NO,
    )));
    iw.add_document(&document)?;
    // Make first segment
    iw.commit()?;

    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    let mut document = Document::new();
    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "a b c",
        &custom_type,
    )));
    iw.add_document(&document)?;
    // Make 2nd segment
    iw.commit()?;

    iw.force_merge(1)?;
    iw.close()?;
    dir.close()
}

/// LUCENE-1010
///
/// Port of `TestTermVectorsWriter.testNoTermVectorAfterTermVectorMerge`.
fn test_no_term_vector_after_term_vector_merge() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = plain_writer(&dir.as_directory())?;
    let mut document = Document::new();
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "a b c",
        &custom_type,
    )));
    iw.add_document(&document)?;
    iw.commit()?;

    let mut document = Document::new();
    document.add(Box::new(LuceneTestCase::new_text_field(
        "tvtest",
        "x y z",
        Store::NO,
    )));
    iw.add_document(&document)?;
    // Make first segment
    iw.commit()?;

    iw.force_merge(1)?;

    let custom_type2 = FieldType::new_from(TextField::type_not_stored());
    custom_type2.set_store_term_vectors(true)?;
    document.add(Box::new(LuceneTestCase::new_field(
        "tvtest",
        "a b c",
        &custom_type2,
    )));
    let document = Document::new();
    iw.add_document(&document)?;
    // Make 2nd segment
    iw.commit()?;
    iw.force_merge(1)?;

    iw.close()?;
    dir.close()
}

/// In a single doc, for the same field, mix the term vectors up
///
/// Port of `TestTermVectorsWriter.testInconsistentTermVectorOptions`.
fn test_inconsistent_term_vector_options() -> Result<()> {
    // no vectors + vectors
    let a = FieldType::new_from(TextField::type_not_stored());
    let b = FieldType::new_from(TextField::type_not_stored());
    b.set_store_term_vectors(true)?;
    do_test_mixup(a, b)?;

    // vectors + vectors with pos
    let a = FieldType::new_from(TextField::type_not_stored());
    a.set_store_term_vectors(true)?;
    let b = FieldType::new_from(TextField::type_not_stored());
    b.set_store_term_vectors(true)?;
    b.set_store_term_vector_positions(true)?;
    do_test_mixup(a, b)?;

    // vectors + vectors with off
    let a = FieldType::new_from(TextField::type_not_stored());
    a.set_store_term_vectors(true)?;
    let b = FieldType::new_from(TextField::type_not_stored());
    b.set_store_term_vectors(true)?;
    b.set_store_term_vector_offsets(true)?;
    do_test_mixup(a, b)?;

    // vectors with pos + vectors with pos + off
    let a = FieldType::new_from(TextField::type_not_stored());
    a.set_store_term_vectors(true)?;
    a.set_store_term_vector_positions(true)?;
    let b = FieldType::new_from(TextField::type_not_stored());
    b.set_store_term_vectors(true)?;
    b.set_store_term_vector_positions(true)?;
    b.set_store_term_vector_offsets(true)?;
    do_test_mixup(a, b)?;

    // vectors with pos + vectors with pos + pay
    let a = FieldType::new_from(TextField::type_not_stored());
    a.set_store_term_vectors(true)?;
    a.set_store_term_vector_positions(true)?;
    let b = FieldType::new_from(TextField::type_not_stored());
    b.set_store_term_vectors(true)?;
    b.set_store_term_vector_positions(true)?;
    b.set_store_term_vector_payloads(true)?;
    do_test_mixup(a, b)
}

/// `private void doTestMixup(FieldType ft1, FieldType ft2)`.
fn do_test_mixup(ft1: FieldType, ft2: FieldType) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add 3 good docs
    for i in 0..3 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
        iw.add_document(&doc)?;
    }

    // add broken doc
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("field", "value1".to_string(), ft1)?));
    doc.add(Box::new(Field::new("field", "value2".to_string(), ft2)?));

    // ensure broken doc hits exception
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        iw.add_document(&doc)
    });
    let message = expected.message().map(str::to_string);
    assert_not_null(&message);
    let message = message.expect("checked");
    MatcherAssert::assert_that(
        message.as_str(),
        &any_of(vec![
            Box::new(starts_with(
                "all instances of a given field name must have the same term vectors settings",
            )),
            Box::new(starts_with(
                "Inconsistency of field data structures across documents for field [field]",
            )),
        ]),
    );
    // ensure good docs are still ok
    let ir: Arc<dyn IndexReader> = iw.get_reader()?;
    assert_equals(&3, &ir.num_docs());

    ir.close()?;
    iw.close()?;
    dir.close()
}

/// LUCENE-5611: don't abort segment when term vector settings are wrong
///
/// Port of `TestTermVectorsWriter.testNoAbortOnBadTVSettings`.
fn test_no_abort_on_bad_tv_settings() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    // Don't use RandomIndexWriter because we want to be sure both docs go to 1 seg:
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let iw = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    iw.add_document(&doc)?;
    let ft = FieldType::new_from(StoredField::type_());
    ft.set_store_term_vectors(true)?;
    ft.freeze();
    doc.add(Box::new(Field::new("field", "value".to_string(), ft)?));

    expect_throws("java.lang.IllegalArgumentException", || {
        iw.add_document(&doc)
    });

    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*iw)?;

    // Make sure the exc didn't lose our first document:
    assert_equals(&1, &r.num_docs());
    iw.close()?;
    r.close()?;
    dir.close()
}
