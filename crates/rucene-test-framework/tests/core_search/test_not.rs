//! Port of `lucene/core/src/test/org/apache/lucene/search/TestNot.java`.
//!
//! Similarity unit test.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::search::{BooleanQueryBuilder, Occur};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::tq;
use crate::core_support::{assert_equals, class, random, test};

/// `TestNot`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestNot",
        vec![test("testNot", test_not)],
    )
}

/// Port of `TestNot.testNot`.
fn test_not() -> Result<()> {
    let store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), store.as_directory())?;

    let mut d1 = Document::new();
    d1.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b",
        Store::YES,
    )));

    writer.add_document(&d1)?;
    let reader = writer.get_reader()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut query = BooleanQueryBuilder::new();
    query.add(tq("field", "a"), Occur::SHOULD)?;
    query.add(tq("field", "b"), Occur::MUST_NOT)?;

    let hits = searcher.search(Arc::new(query.build()), 1000)?.score_docs;
    assert_equals(&0, &hits.len());
    writer.close()?;
    drop(searcher);
    reader.close()?;
    store.close()?;
    Ok(())
}
