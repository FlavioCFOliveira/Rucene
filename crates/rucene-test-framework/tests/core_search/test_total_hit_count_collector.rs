//! Port of `lucene/core/src/test/org/apache/lucene/search/TestTotalHitCountCollector.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query, TotalHitCountCollectorManager,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::Concurrency;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestTotalHitCountCollector`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestTotalHitCountCollector",
        vec![test("testBasics", test_basics)],
    )
}

/// Port of `TestTotalHitCountCollector.testBasics`.
fn test_basics() -> Result<()> {
    let index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    for i in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "string",
            format!("a{i}"),
            Store::NO,
        )?));
        doc.add(Box::new(StringField::new(
            "string",
            format!("b{i}"),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    writer.close()?;

    // `Concurrency.values()`.
    let concurrency = *RandomizedTest::random_from(&[
        Concurrency::None,
        Concurrency::InterSegment,
        Concurrency::IntraSegment,
    ]);
    let searcher =
        LuceneTestCase::new_searcher_concurrency(Arc::clone(&reader), true, true, concurrency);
    let collector_manager = TotalHitCountCollectorManager::new(searcher.get_slices()?);
    let total_hits = searcher
        .search_with_manager(Arc::new(MatchAllDocsQuery::instance()), &collector_manager)?;
    assert_equals(&5, &total_hits);

    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(TermQuery::new(Term::from_text("string", "a1"))),
        Occur::SHOULD,
    )?;
    query.add(
        Arc::new(TermQuery::new(Term::from_text("string", "b3"))),
        Occur::SHOULD,
    )?;
    let query: Arc<dyn Query> = Arc::new(query.build());
    let total_hits = searcher.search_with_manager(query, &collector_manager)?;
    assert_equals(&2, &total_hits);

    drop(searcher);
    reader.close()?;
    index_store.close()
}
