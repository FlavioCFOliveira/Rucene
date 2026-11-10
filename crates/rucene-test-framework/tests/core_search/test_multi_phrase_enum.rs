//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiPhraseEnum.java`.
//!
//! simple tests for unionpostingsenum

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_POSITIONS};
use rucene::index::IndexReader;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::multi_phrase_query::UnionPostingsEnum;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::term;
use crate::core_support::{assert_equals, class, test};

/// `TestMultiPhraseEnum`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiPhraseEnum",
        vec![
            test("testOneDocument", test_one_document),
            test("testSomeDocuments", test_some_documents),
        ],
    )
}

fn text(value: &str) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "field",
        value.to_string(),
        Store::NO,
    )?));
    Ok(doc)
}

/// The union of the `POSITIONS` postings of `foo` and `bar` in the only leaf.
fn union_of(
    ir: &Arc<dyn rucene::index::directory_reader::DirectoryReader>,
) -> Result<UnionPostingsEnum> {
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(ir) as Arc<dyn IndexReader>)?;
    let p1 = leaf
        .postings(&term("field", "foo"), POSTINGS_ENUM_POSITIONS)?
        .expect("postings");
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(ir) as Arc<dyn IndexReader>)?;
    let p2 = leaf
        .postings(&term("field", "bar"), POSTINGS_ENUM_POSITIONS)?
        .expect("postings");
    Ok(UnionPostingsEnum::new(vec![p1, p2]))
}

/// Tests union on one document
fn test_one_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    writer.add_document(&text("foo bar")?)?;

    let ir = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    let mut union = union_of(&ir)?;

    assert_equals(&-1, &union.doc_id());

    assert_equals(&0, &union.next_doc()?);
    assert_equals(&2, &union.freq()?);
    assert_equals(&0, &union.next_position()?);
    assert_equals(&1, &union.next_position()?);

    assert_equals(&NO_MORE_DOCS, &union.next_doc()?);

    drop(union);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Tests union on a few documents
fn test_some_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    writer.add_document(&text("foo")?)?;

    writer.add_document(&Document::new())?;

    writer.add_document(&text("foo bar")?)?;

    writer.add_document(&text("bar")?)?;

    writer.force_merge(1)?;
    let ir = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    let mut union = union_of(&ir)?;

    assert_equals(&-1, &union.doc_id());

    assert_equals(&0, &union.next_doc()?);
    assert_equals(&1, &union.freq()?);
    assert_equals(&0, &union.next_position()?);

    assert_equals(&2, &union.next_doc()?);
    assert_equals(&2, &union.freq()?);
    assert_equals(&0, &union.next_position()?);
    assert_equals(&1, &union.next_position()?);

    assert_equals(&3, &union.next_doc()?);
    assert_equals(&1, &union.freq()?);
    assert_equals(&0, &union.next_position()?);

    assert_equals(&NO_MORE_DOCS, &union.next_doc()?);

    drop(union);
    ir.close()?;
    dir.close()?;
    Ok(())
}
