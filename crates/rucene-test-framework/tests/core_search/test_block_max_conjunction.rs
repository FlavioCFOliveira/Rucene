//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBlockMaxConjunction.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQueryBuilder, Occur, Query};
use rucene_test_framework::search::asserting_query::AssertingQuery;
use rucene_test_framework::search::block_score_query_wrapper::BlockScoreQueryWrapper;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::random_approximation_query::RandomApproximationQuery;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};

/// `TestBlockMaxConjunction`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBlockMaxConjunction",
        vec![test("testRandom", test_random)],
    )
}

fn maybe_wrap(mut query: Arc<dyn Query>) -> Arc<dyn Query> {
    if random().next_boolean() {
        query = Arc::new(BlockScoreQueryWrapper::new(
            query,
            TestUtil::next_int(&*random(), 2, 8),
        ));
        query = Arc::new(AssertingQuery::new(random(), query));
    }
    query
}

fn maybe_wrap_two_phase(mut query: Arc<dyn Query>) -> Arc<dyn Query> {
    if random().next_boolean() {
        query = Arc::new(RandomApproximationQuery::new(query, random()));
        query = Arc::new(AssertingQuery::new(random(), query));
    }
    query
}

fn foo(i: i32) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text("foo", &i.to_string())))
}

fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(1 << random().next_int_bound(5));
        let start = random().next_int_bound(10);
        for j in 0..num_values {
            doc.add(Box::new(StringField::new(
                "foo",
                (start + j).to_string(),
                Store::NO,
            )?));
        }
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    // Disable search concurrency for this test: it requires a single segment, and no intra-segment
    // concurrency for its assertions to always be valid
    let maybe_wrap_reader = random().next_boolean();
    let wrap_with_assertions = random().next_boolean();
    let searcher = LuceneTestCase::new_searcher_threads(
        Arc::clone(&reader),
        maybe_wrap_reader,
        wrap_with_assertions,
        false,
    );

    for _iter in 0..100 {
        let start = random().next_int_bound(10);
        let num_clauses = random().next_int_bound(1 << random().next_int_bound(5));
        let mut builder = BooleanQueryBuilder::new();
        for i in 0..num_clauses {
            builder.add(maybe_wrap(foo(start + i)), Occur::MUST)?;
        }
        let query: Arc<dyn Query> = Arc::new(builder.build());

        CheckHits::check_top_scores(&*random(), &query, &searcher)?;

        let filter_term = random().next_int_bound(30);
        let mut filtered = BooleanQueryBuilder::new();
        filtered.add(Arc::clone(&query), Occur::MUST)?;
        filtered.add(foo(filter_term), Occur::FILTER)?;
        let filtered_query: Arc<dyn Query> = Arc::new(filtered.build());

        CheckHits::check_top_scores(&*random(), &filtered_query, &searcher)?;

        let mut builder = BooleanQueryBuilder::new();
        for i in 0..num_clauses {
            builder.add(maybe_wrap_two_phase(foo(start + i)), Occur::MUST)?;
        }

        // Java builds `twoPhaseQuery` over `query`, not over `builder`.
        let mut two_phase = BooleanQueryBuilder::new();
        two_phase.add(Arc::clone(&query), Occur::MUST)?;
        two_phase.add(foo(filter_term), Occur::FILTER)?;
        let two_phase_query: Arc<dyn Query> = Arc::new(two_phase.build());

        CheckHits::check_top_scores(&*random(), &two_phase_query, &searcher)?;
    }

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}
