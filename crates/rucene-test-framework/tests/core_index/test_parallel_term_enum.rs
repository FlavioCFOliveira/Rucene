//! Port of `lucene/core/src/test/org/apache/lucene/index/TestParallelTermEnum.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{After, Assert, Before};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::parallel_reader::ParallelLeafReader;
use rucene::index::{IndexReader, LeafReader, Terms, POSTINGS_ENUM_NONE};
use rucene::search::NO_MORE_DOCS;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, random};
use crate::support::npe;

/// The instance fields of `TestParallelTermEnum`.
#[derive(Default)]
struct Instance {
    ir1: Option<Arc<dyn LeafReader>>,
    ir2: Option<Arc<dyn LeafReader>>,
    rd1: Option<TestDirectory>,
    rd2: Option<TestDirectory>,
}

/// `TestParallelTermEnum`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestParallelTermEnum")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(
            Method::new("setUp")
                .annotate(Before)
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut Instance| set_up(inst).unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(
            Method::new("tearDown")
                .annotate(After)
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut Instance| tear_down(inst).unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(
            Method::new("test1")
                .throws(&["java.io.IOException"])
                .body(|inst: &mut Instance| test1(inst).unwrap_or_else(|e| rethrow_java(e))),
        )
        .build()
}

/// `newTextField(name, value, Field.Store.YES)`.
fn text(name: &str, value: &str) -> Box<rucene::document::Field> {
    Box::new(LuceneTestCase::new_text_field(name, value, Store::YES))
}

/// `public void setUp()`.
fn set_up(inst: &mut Instance) -> Result<()> {
    let rd1 = inst.rd1.insert(LuceneTestCase::new_directory());
    let iw1 = IndexWriter::new(
        rd1.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    let mut doc = Document::new();
    doc.add(text("field1", "the quick brown fox jumps"));
    doc.add(text("field2", "the quick brown fox jumps"));
    iw1.add_document(&doc)?;

    iw1.close()?;
    let rd2 = inst.rd2.insert(LuceneTestCase::new_directory());
    let iw2 = IndexWriter::new(
        rd2.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;

    let mut doc = Document::new();
    doc.add(text("field1", "the fox jumps over the lazy dog"));
    doc.add(text("field3", "the fox jumps over the lazy dog"));
    iw2.add_document(&doc)?;

    iw2.close()?;

    let rd1 = inst.rd1.as_ref().unwrap_or_else(|| npe("rd1"));
    let rd2 = inst.rd2.as_ref().unwrap_or_else(|| npe("rd2"));
    inst.ir1 = Some(LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(rd1.as_directory())? as Arc<dyn IndexReader>,
    )?);
    inst.ir2 = Some(LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(rd2.as_directory())? as Arc<dyn IndexReader>,
    )?);
    Ok(())
}

/// `public void tearDown()`.
fn tear_down(inst: &mut Instance) -> Result<()> {
    let ir1 = inst.ir1.take().unwrap_or_else(|| npe("ir1"));
    ir1.as_index_reader().close()?;
    let ir2 = inst.ir2.take().unwrap_or_else(|| npe("ir2"));
    ir2.as_index_reader().close()?;
    inst.rd1.take().unwrap_or_else(|| npe("rd1")).close()?;
    inst.rd2.take().unwrap_or_else(|| npe("rd2")).close()
}

/// `private void checkTerms(Terms terms, String... termsList)`.
fn check_terms(terms: Option<Box<dyn Terms>>, terms_list: &[&str]) -> Result<()> {
    assert_not_null(&terms);
    let terms = terms.unwrap_or_else(|| npe("terms"));
    let mut te = terms.iterator()?;

    for t in terms_list {
        let b = te.next()?;
        assert_not_null(&b);
        let b = b.unwrap_or_else(|| npe("b"));
        assert_equals(&t.to_string(), &b.utf8_to_string()?);
        let mut td = TestUtil::docs_of(&*random(), &mut *te, None, POSTINGS_ENUM_NONE)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        Assert::assert_true(td.next_doc()? != NO_MORE_DOCS);
        assert_equals(&0, &td.doc_id());
        assert_equals(&td.next_doc()?, &NO_MORE_DOCS);
    }
    assert_null(&te.next()?);
    Ok(())
}

/// Port of `TestParallelTermEnum.test1`.
fn test1(inst: &Instance) -> Result<()> {
    let ir1 = inst.ir1.as_ref().unwrap_or_else(|| npe("ir1"));
    let ir2 = inst.ir2.as_ref().unwrap_or_else(|| npe("ir2"));
    let pr = ParallelLeafReader::new(vec![Arc::clone(ir1), Arc::clone(ir2)])?;

    assert_equals(&3, &pr.get_field_infos().size());

    check_terms(
        pr.terms("field1")?,
        &["brown", "fox", "jumps", "quick", "the"],
    )?;
    check_terms(
        pr.terms("field2")?,
        &["brown", "fox", "jumps", "quick", "the"],
    )?;
    check_terms(
        pr.terms("field3")?,
        &["dog", "fox", "jumps", "lazy", "over", "the"],
    )
}
