//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMatchNoDocsQuery.java`.
//!
//! Tests MatchNoDocsQuery.
//!
//! `query.rewrite(null)`: Rucene's `Query::rewrite` takes a searcher
//! reference, which cannot be `null`; the port hands it a searcher over an
//! empty reader, which `MatchNoDocsQuery.rewrite` does not read.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::{
    query_to_string, BooleanClause, BooleanQueryBuilder, MatchNoDocsQuery, Occur, Query,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, term};
use crate::core_support::{assert_equals, random};

/// The instance of `TestMatchNoDocsQuery`.
#[derive(Default)]
pub struct MatchNoDocs {
    analyzer: Option<Arc<dyn Analyzer>>,
}

/// `TestMatchNoDocsQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<MatchNoDocs>(
        "org.apache.lucene.search.TestMatchNoDocsQuery",
        vec![
            set_up(|f: &mut MatchNoDocs| {
                f.analyzer = Some(Arc::new(MockAnalyzer::new(&*random())));
                Ok(())
            }),
            ftest("testSimple", test_simple),
            ftest("testQuery", test_query),
            ftest("testEquals", test_equals),
        ],
    )
}

/// `query.rewrite(searcher)`: Rucene's `None` is Java's `this`.
fn rewrite_once(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<Arc<dyn Query>> {
    Ok(query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::clone(query)))
}

fn test_simple(_f: &mut MatchNoDocs) -> Result<()> {
    let mut query: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    assert_equals(query_to_string(&*query).as_str(), "MatchNoDocsQuery(\"\")");
    query = Arc::new(MatchNoDocsQuery::new("field 'title' not found"));
    assert_equals(
        query_to_string(&*query).as_str(),
        "MatchNoDocsQuery(\"field 'title' not found\")",
    );
    let null_searcher = IndexSearcher::new(Arc::new(MultiReader::new(Vec::new(), true)?))?;
    let rewrite = rewrite_once(&query, &null_searcher)?;
    Assert::assert_true(rewrite.as_any().is::<MatchNoDocsQuery>());
    assert_equals(
        query_to_string(&*rewrite).as_str(),
        "MatchNoDocsQuery(\"field 'title' not found\")",
    );
    Ok(())
}

fn test_query(f: &mut MatchNoDocs) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Arc::clone(
        f.analyzer.as_ref().expect("setUp ran"),
    ));
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let iw = IndexWriter::new(dir.as_directory(), iwc)?;
    add_doc("one", &iw)?;
    add_doc("two", &iw)?;
    add_doc("three", &iw)?;
    let ir = directory_reader::open_with_writer(&*iw)?;
    let searcher = IndexSearcher::new(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    let mut query: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("field not found"));
    assert_equals(&searcher.count(Arc::clone(&query))?, &0);

    let mut hits = searcher
        .search(Arc::new(MatchNoDocsQuery::instance()), 1000)?
        .score_docs;
    assert_equals(&0, &hits.len());
    assert_equals(
        query_to_string(&*query).as_str(),
        "MatchNoDocsQuery(\"field not found\")",
    );

    let mut bq = BooleanQueryBuilder::new();
    bq.add_clause(BooleanClause::new(
        Arc::new(TermQuery::new(term("key", "five"))),
        Occur::SHOULD,
    ))?;
    bq.add_clause(BooleanClause::new(
        Arc::new(MatchNoDocsQuery::new("field not found")),
        Occur::MUST,
    ))?;
    query = Arc::new(bq.build());
    assert_equals(&searcher.count(Arc::clone(&query))?, &0);
    hits = searcher
        .search(Arc::new(MatchNoDocsQuery::instance()), 1000)?
        .score_docs;
    assert_equals(&0, &hits.len());
    assert_equals(
        query_to_string(&*query).as_str(),
        "key:five +MatchNoDocsQuery(\"field not found\")",
    );

    bq = BooleanQueryBuilder::new();
    bq.add_clause(BooleanClause::new(
        Arc::new(TermQuery::new(term("key", "one"))),
        Occur::SHOULD,
    ))?;
    bq.add_clause(BooleanClause::new(
        Arc::new(MatchNoDocsQuery::new("field not found")),
        Occur::SHOULD,
    ))?;
    query = Arc::new(bq.build());
    assert_equals(
        query_to_string(&*query).as_str(),
        "key:one MatchNoDocsQuery(\"field not found\")",
    );
    assert_equals(&searcher.count(Arc::clone(&query))?, &1);
    hits = searcher.search(Arc::clone(&query), 1000)?.score_docs;
    let rewrite = rewrite_once(&query, &searcher)?;
    assert_equals(&1, &hits.len());
    assert_equals(query_to_string(&*rewrite).as_str(), "key:one");

    iw.close()?;
    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

fn test_equals(_f: &mut MatchNoDocs) -> Result<()> {
    let q1: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    let q2: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    Assert::assert_true(q1.query_eq(&*q2));
    QueryUtils::check(&*q1);
    Ok(())
}

fn add_doc(text: &str, iw: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    let f = LuceneTestCase::new_text_field("key", text, Store::YES);
    doc.add(Box::new(f));
    iw.add_document(&doc)?;
    Ok(())
}
