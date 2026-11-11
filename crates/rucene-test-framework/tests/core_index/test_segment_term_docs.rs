//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentTermDocs.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_FREQS};
use rucene::index::segment_reader::SegmentReader;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader, SegmentCommitInfo};
use rucene::search::NO_MORE_DOCS;
use rucene::util::{BytesRef, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::doc_helper::{self, DocHelper};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_null, random};

/// The instance fields of `TestSegmentTermDocs`.
#[derive(Default)]
struct TestSegmentTermDocsInstance {
    /// `private Document testDoc = new Document()`.
    test_doc: Document,
    /// `private Directory dir`.
    dir: Option<TestDirectory>,
    /// `private SegmentCommitInfo info`.
    info: Option<SegmentCommitInfo>,
}

impl TestSegmentTermDocsInstance {
    /// `new SegmentReader(info, Version.LATEST.major, newIOContext(random()))`.
    fn open_reader(&self) -> Result<Arc<dyn LeafReader>> {
        let info = self
            .info
            .clone()
            .unwrap_or_else(|| crate::support::npe("info"));
        let context = LuceneTestCase::new_io_context(&*random());
        Ok(Arc::new(SegmentReader::new(
            info,
            i32::from(Version::LATEST.major),
            &*context,
        )?))
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestSegmentTermDocsInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestSegmentTermDocsInstance| {
            body(inst).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// `TestSegmentTermDocs`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestSegmentTermDocs")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestSegmentTermDocsInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("tearDown", tear_down))
        .method(instance_test("test", test))
        .method(instance_test("testTermDocs", test_term_docs))
        .method(instance_test("testBadSeek", test_bad_seek))
        .method(instance_test("testSkipTo", test_skip_to))
        .build()
}

/// `public void setUp()`.
fn set_up(inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir = LuceneTestCase::new_directory();
    DocHelper::setup_doc(&mut inst.test_doc);
    inst.info = DocHelper::write_doc(&*random(), dir.as_directory(), &inst.test_doc)?;
    inst.dir = Some(dir);
    Ok(())
}

/// `public void tearDown()`.
fn tear_down(inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    if let Some(mut dir) = inst.dir.take() {
        dir.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Port of `TestSegmentTermDocs.test`.
fn test(inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    Assert::assert_true(inst.dir.is_some());
    Ok(())
}

/// Port of `TestSegmentTermDocs.testTermDocs`.
fn test_term_docs(inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    // After adding the document, we should be able to read it back in
    let reader = inst.open_reader()?;

    let mut terms = reader
        .terms(doc_helper::TEXT_FIELD_2_KEY)?
        .unwrap_or_else(|| crate::support::npe("terms"))
        .iterator()?;
    terms.seek_ceil(&BytesRef::new(b"field".to_vec()))?;
    let mut term_docs = TestUtil::docs_of(&*random(), &mut *terms, None, POSTINGS_ENUM_FREQS)?;
    if term_docs.next_doc()? != NO_MORE_DOCS {
        let doc_id = term_docs.doc_id();
        Assert::assert_true(doc_id == 0);
        let freq = term_docs.freq()?;
        Assert::assert_true(freq == 3);
    }
    reader.as_index_reader().close()
}

/// Port of `TestSegmentTermDocs.testBadSeek`.
fn test_bad_seek(inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    {
        // After adding the document, we should be able to read it back in
        let reader = inst.open_reader()?;
        let term_docs = TestUtil::docs(
            &*random(),
            &Arc::clone(&reader).as_index_reader(),
            "textField2",
            &BytesRef::new(b"bad".to_vec()),
            None,
            0,
        )?;

        assert_null(&term_docs.map(|_| "PostingsEnum"));
        reader.as_index_reader().close()?;
    }
    {
        // After adding the document, we should be able to read it back in
        let reader = inst.open_reader()?;
        let term_docs = TestUtil::docs(
            &*random(),
            &Arc::clone(&reader).as_index_reader(),
            "junk",
            &BytesRef::new(b"bad".to_vec()),
            None,
            0,
        )?;
        assert_null(&term_docs.map(|_| "PostingsEnum"));
        reader.as_index_reader().close()?;
    }
    Ok(())
}

/// `assertTrue(tdocs.nextDoc() != NO_MORE_DOCS); assertEquals(doc,
/// tdocs.docID()); assertEquals(freq, tdocs.freq());`
fn next_and_check(tdocs: &mut dyn PostingsEnum, doc: i32, freq: i32) -> Result<()> {
    Assert::assert_true(tdocs.next_doc()? != NO_MORE_DOCS);
    assert_equals(&doc, &tdocs.doc_id());
    assert_equals(&freq, &tdocs.freq()?);
    Ok(())
}

/// `assertTrue(tdocs.advance(target) != NO_MORE_DOCS); assertEquals(doc,
/// tdocs.docID());`
fn advance_and_check(tdocs: &mut dyn PostingsEnum, target: i32, doc: i32) -> Result<()> {
    Assert::assert_true(tdocs.advance(target)? != NO_MORE_DOCS);
    assert_equals(&doc, &tdocs.doc_id());
    Ok(())
}

/// `assertFalse(tdocs.advance(target) != NO_MORE_DOCS)`.
fn advance_exhausted(tdocs: &mut dyn PostingsEnum, target: i32) -> Result<()> {
    Assert::assert_false(tdocs.advance(target)? != NO_MORE_DOCS);
    Ok(())
}

/// `TestUtil.docs(random(), reader, t.field(), new BytesRef(t.text()), null, flags)`.
fn docs(reader: &Arc<dyn IndexReader>, t: &Term, flags: i32) -> Result<Box<dyn PostingsEnum>> {
    Ok(TestUtil::docs(
        &*random(),
        reader,
        t.field(),
        &BytesRef::new(t.text().into_bytes()),
        None,
        flags,
    )?
    .unwrap_or_else(|| crate::support::npe("tdocs")))
}

/// Port of `TestSegmentTermDocs.testSkipTo`.
fn test_skip_to(_inst: &mut TestSegmentTermDocsInstance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let ta = Term::from_text("content", "aaa");
    for _i in 0..10 {
        add_doc(&writer, "aaa aaa aaa aaa")?;
    }

    let tb = Term::from_text("content", "bbb");
    for _i in 0..16 {
        add_doc(&writer, "bbb bbb bbb bbb")?;
    }

    let tc = Term::from_text("content", "ccc");
    for _i in 0..50 {
        add_doc(&writer, "ccc ccc ccc ccc")?;
    }

    // assure that we deal with a single segment
    writer.force_merge(1)?;
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    let mut tdocs = docs(&reader, &ta, POSTINGS_ENUM_FREQS)?;

    // without optimization (assumption skipInterval == 16)

    // with next
    next_and_check(&mut *tdocs, 0, 4)?;
    next_and_check(&mut *tdocs, 1, 4)?;
    advance_and_check(&mut *tdocs, 2, 2)?;
    advance_and_check(&mut *tdocs, 4, 4)?;
    advance_and_check(&mut *tdocs, 9, 9)?;
    advance_exhausted(&mut *tdocs, 10)?;

    // without next
    let mut tdocs = docs(&reader, &ta, 0)?;

    advance_and_check(&mut *tdocs, 0, 0)?;
    advance_and_check(&mut *tdocs, 4, 4)?;
    advance_and_check(&mut *tdocs, 9, 9)?;
    advance_exhausted(&mut *tdocs, 10)?;

    // exactly skipInterval documents and therefore with optimization

    // with next
    let mut tdocs = docs(&reader, &tb, POSTINGS_ENUM_FREQS)?;

    next_and_check(&mut *tdocs, 10, 4)?;
    next_and_check(&mut *tdocs, 11, 4)?;
    advance_and_check(&mut *tdocs, 12, 12)?;
    advance_and_check(&mut *tdocs, 15, 15)?;
    advance_and_check(&mut *tdocs, 24, 24)?;
    advance_and_check(&mut *tdocs, 25, 25)?;
    advance_exhausted(&mut *tdocs, 26)?;

    // without next
    let mut tdocs = docs(&reader, &tb, POSTINGS_ENUM_FREQS)?;

    advance_and_check(&mut *tdocs, 5, 10)?;
    advance_and_check(&mut *tdocs, 15, 15)?;
    advance_and_check(&mut *tdocs, 24, 24)?;
    advance_and_check(&mut *tdocs, 25, 25)?;
    advance_exhausted(&mut *tdocs, 26)?;

    // much more than skipInterval documents and therefore with optimization

    // with next
    let mut tdocs = docs(&reader, &tc, POSTINGS_ENUM_FREQS)?;

    next_and_check(&mut *tdocs, 26, 4)?;
    next_and_check(&mut *tdocs, 27, 4)?;
    advance_and_check(&mut *tdocs, 28, 28)?;
    advance_and_check(&mut *tdocs, 40, 40)?;
    advance_and_check(&mut *tdocs, 57, 57)?;
    advance_and_check(&mut *tdocs, 74, 74)?;
    advance_and_check(&mut *tdocs, 75, 75)?;
    advance_exhausted(&mut *tdocs, 76)?;

    // without next
    let mut tdocs = docs(&reader, &tc, 0)?;
    advance_and_check(&mut *tdocs, 5, 26)?;
    advance_and_check(&mut *tdocs, 40, 40)?;
    advance_and_check(&mut *tdocs, 57, 57)?;
    advance_and_check(&mut *tdocs, 74, 74)?;
    advance_and_check(&mut *tdocs, 75, 75)?;
    advance_exhausted(&mut *tdocs, 76)?;

    reader.close()?;
    dir.close()
}

/// `private void addDoc(IndexWriter writer, String value)`.
fn add_doc(writer: &IndexWriter, value: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        value,
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}
