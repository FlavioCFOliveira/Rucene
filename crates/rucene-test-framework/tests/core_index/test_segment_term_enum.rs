//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentTermEnum.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{is_instance, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, random};

/// The instance fields of `TestSegmentTermEnum`.
#[derive(Default)]
struct TestSegmentTermEnumInstance {
    /// `Directory dir`.
    dir: Option<TestDirectory>,
}

impl TestSegmentTermEnumInstance {
    fn dir(&self) -> Arc<dyn Directory> {
        self.dir
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("dir"))
            .as_directory()
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestSegmentTermEnumInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestSegmentTermEnumInstance| {
            body(inst).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// `TestSegmentTermEnum`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestSegmentTermEnum")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestSegmentTermEnumInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("tearDown", tear_down))
        .method(instance_test("testTermEnum", test_term_enum))
        .method(instance_test("testPrevTermAtEnd", test_prev_term_at_end))
        .build()
}

/// `public void setUp()`.
fn set_up(inst: &mut TestSegmentTermEnumInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    inst.dir = Some(LuceneTestCase::new_directory());
    Ok(())
}

/// `public void tearDown()`.
fn tear_down(inst: &mut TestSegmentTermEnumInstance) -> Result<()> {
    if let Some(mut dir) = inst.dir.take() {
        dir.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Port of `TestSegmentTermEnum.testTermEnum`.
fn test_term_enum(inst: &mut TestSegmentTermEnumInstance) -> Result<()> {
    let writer = IndexWriter::new(
        inst.dir(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    // ADD 100 documents with term : aaa
    // add 100 documents with terms: aaa bbb
    // Therefore, term 'aaa' has document frequency of 200 and term 'bbb' 100
    for _i in 0..100 {
        add_doc(&writer, "aaa")?;
        add_doc(&writer, "aaa bbb")?;
    }

    writer.close()?;

    // verify document frequency of terms in an multi segment index
    verify_doc_freq(inst)?;

    // merge segments
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(inst.dir(), conf)?;
    writer.force_merge(1)?;
    writer.close()?;

    // verify document frequency of terms in a single segment index
    verify_doc_freq(inst)
}

/// Port of `TestSegmentTermEnum.testPrevTermAtEnd`.
fn test_prev_term_at_end(inst: &mut TestSegmentTermEnumInstance) -> Result<()> {
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_codec(TestUtil::always_postings_format(
        TestUtil::get_default_postings_format(),
    ))?;
    let writer = IndexWriter::new(inst.dir(), conf)?;
    add_doc(&writer, "aaa bbb")?;
    writer.close()?;
    let reader = LuceneTestCase::get_only_leaf_reader(directory_reader::open(inst.dir())? as _)?;
    let mut terms = reader
        .terms("content")?
        .unwrap_or_else(|| crate::support::npe("terms"))
        .iterator()?;
    assert_not_null(&terms.next()?);
    assert_equals(
        &"aaa".to_string(),
        &terms
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_not_null(&terms.next()?);
    let ord_b = match terms.ord() {
        Ok(ord) => ord,
        Err(uoe) if is_instance(&uoe, "java.lang.UnsupportedOperationException") => {
            // ok -- codec is not required to support ord
            return Arc::clone(&reader).as_index_reader().close();
        }
        Err(e) => return Err(e),
    };
    assert_equals(
        &"bbb".to_string(),
        &terms
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_null(&terms.next()?);

    terms.seek_ord(ord_b)?;
    assert_equals(
        &"bbb".to_string(),
        &terms
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    reader.as_index_reader().close()
}

/// `private void verifyDocFreq()`.
fn verify_doc_freq(inst: &TestSegmentTermEnumInstance) -> Result<()> {
    let reader: Arc<dyn IndexReader> = directory_reader::open(inst.dir())?;
    let mut term_enum = MultiTerms::get_terms(&reader, "content")?
        .unwrap_or_else(|| crate::support::npe("terms"))
        .iterator()?;

    // create enumeration of all terms
    // go to the first term (aaa)
    term_enum.next()?;
    // assert that term is 'aaa'
    assert_equals(
        &"aaa".to_string(),
        &term_enum
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_equals(&200, &term_enum.doc_freq()?);
    // go to the second term (bbb)
    term_enum.next()?;
    // assert that term is 'bbb'
    assert_equals(
        &"bbb".to_string(),
        &term_enum
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_equals(&100, &term_enum.doc_freq()?);

    // create enumeration of terms after term 'aaa',
    // including 'aaa'
    term_enum.seek_ceil(&BytesRef::new(b"aaa".to_vec()))?;
    // assert that term is 'aaa'
    assert_equals(
        &"aaa".to_string(),
        &term_enum
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_equals(&200, &term_enum.doc_freq()?);
    // go to term 'bbb'
    term_enum.next()?;
    // assert that term is 'bbb'
    assert_equals(
        &"bbb".to_string(),
        &term_enum
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?,
    );
    assert_equals(&100, &term_enum.doc_freq()?);
    reader.close()
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
