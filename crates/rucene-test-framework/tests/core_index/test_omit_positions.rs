//! Port of `lucene/core/src/test/org/apache/lucene/index/TestOmitPositions.java`.
//!
//! @lucene.experimental

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexOptions, IndexReader, LeafReader, POSTINGS_ENUM_FREQS};
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test};
use crate::support::npe;

/// `TestOmitPositions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestOmitPositions",
        vec![
            test("testBasic", test_basic),
            test("testPositions", test_positions),
            test("testNoPrxFile", test_no_prx_file),
        ],
    )
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `indexOptions` set.
fn text_type(index_options: IndexOptions) -> Result<FieldType> {
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_index_options(index_options)?;
    Ok(ft)
}

/// Port of `TestOmitPositions.testBasic`.
fn test_basic() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let ft = text_type(IndexOptions::DOCS_AND_FREQS)?;
    let f = LuceneTestCase::new_field("foo", "this is a test test", &ft);
    doc.add(Box::new(f));
    for _ in 0..100 {
        w.add_document(&doc)?;
    }

    let reader: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;

    assert_not_null(&MultiTerms::get_term_postings_enum_all(
        &reader,
        "foo",
        &BytesRef::from_char_sequence("test"),
    )?);

    let mut de = TestUtil::docs(
        &*random(),
        &reader,
        "foo",
        &BytesRef::from_char_sequence("test"),
        None,
        POSTINGS_ENUM_FREQS,
    )?
    .unwrap_or_else(|| npe("TestUtil.docs"));
    while de.next_doc()? != NO_MORE_DOCS {
        assert_equals(&2, &de.freq()?);
    }

    reader.close()?;
    dir.close()
}

/// `fi.fieldInfo(name).getIndexOptions()`.
fn index_options(reader: &Arc<dyn LeafReader>, name: &str) -> IndexOptions {
    reader
        .get_field_infos()
        .field_info(name)
        .unwrap_or_else(|| npe("fieldInfo"))
        .get_index_options()
}

/// Port of `TestOmitPositions.testPositions`: tests whether the
/// DocumentWriter correctly enable the omitTermFreqAndPositions bit in the
/// FieldInfo.
fn test_positions() -> Result<()> {
    let mut ram = LuceneTestCase::new_directory();
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let writer = IndexWriter::new(
        ram.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
    )?;
    let mut d = Document::new();

    // f1: docs only
    let ft = text_type(IndexOptions::DOCS)?;

    let f1 = LuceneTestCase::new_field("f1", "This field has docs only", &ft);
    d.add(Box::new(f1));

    let ft2 = text_type(IndexOptions::DOCS_AND_FREQS)?;

    // f2: docs and freqs
    let f2 = LuceneTestCase::new_field("f2", "This field has docs and freqs", &ft2);
    d.add(Box::new(f2));

    let ft3 = text_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;

    // f3: docs/freqs/positions
    let f3 = LuceneTestCase::new_field("f3", "This field has docs and freqs and positions", &ft3);
    d.add(Box::new(f3));

    writer.add_document(&d)?;
    writer.force_merge(1)?;
    // flush
    writer.close()?;

    let reader = LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(ram.as_directory())? as Arc<dyn IndexReader>
    )?;
    // docs + docs = docs
    assert_equals(&IndexOptions::DOCS, &index_options(&reader, "f1"));
    // docs/freqs + docs/freqs = docs/freqs
    assert_equals(&IndexOptions::DOCS_AND_FREQS, &index_options(&reader, "f2"));
    // docs/freqs/pos + docs/freqs/pos = docs/freqs/pos
    assert_equals(
        &IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        &index_options(&reader, "f3"),
    );

    Arc::clone(&reader).as_index_reader().close()?;
    ram.close()
}

/// `private void assertNoPrx(Directory dir)`.
fn assert_no_prx(dir: &Arc<dyn Directory>) -> Result<()> {
    let files = dir.list_all()?;
    for file in &files {
        Assert::assert_false(file.ends_with(".prx"));
        Assert::assert_false(file.ends_with(".pos"));
    }
    Ok(())
}

/// Port of `TestOmitPositions.testNoPrxFile`: verifies no *.prx exists when
/// all fields omit term positions.
fn test_no_prx_file() -> Result<()> {
    let mut ram = LuceneTestCase::new_directory();

    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    conf.set_max_buffered_docs(3)?;
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(ram.as_directory(), conf)?;
    let mp = writer.get_config().merge_policy();
    let lmp = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a LogMergePolicy",
            )
            .throw()
        });
    lmp.set_merge_factor(2)?;
    lmp.set_no_cfs_ratio(0.0)?;
    let mut d = Document::new();

    let ft = text_type(IndexOptions::DOCS_AND_FREQS)?;
    let f1 = LuceneTestCase::new_field("f1", "This field has term freqs", &ft);
    d.add(Box::new(f1));

    for _ in 0..30 {
        writer.add_document(&d)?;
    }

    writer.commit()?;

    assert_no_prx(&ram.as_directory())?;

    writer.close()?;
    ram.close()
}
