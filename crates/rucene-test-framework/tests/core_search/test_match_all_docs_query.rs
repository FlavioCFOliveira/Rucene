//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMatchAllDocsQuery.java`.
//!
//! Tests MatchAllDocsQuery.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, term};
use crate::core_support::{assert_equals, random};

/// The instance of `TestMatchAllDocsQuery`.
#[derive(Default)]
pub struct MatchAllDocs {
    analyzer: Option<Arc<dyn Analyzer>>,
}

/// `TestMatchAllDocsQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<MatchAllDocs>(
        "org.apache.lucene.search.TestMatchAllDocsQuery",
        vec![
            set_up(|f: &mut MatchAllDocs| {
                f.analyzer = Some(Arc::new(MockAnalyzer::new(&*random())));
                Ok(())
            }),
            ftest("testQuery", test_query),
            ftest("testEquals", test_equals),
            ftest("testEarlyTermination", test_early_termination),
        ],
    )
}

/// `new IndexWriter(dir, newIndexWriterConfig(analyzer).setMaxBufferedDocs(2).setMergePolicy(newLogMergePolicy()))`.
fn writer(
    f: &MatchAllDocs,
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Arc::clone(
        f.analyzer.as_ref().expect("setUp ran"),
    ));
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    IndexWriter::new(dir.as_directory(), iwc)
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn test_query(f: &mut MatchAllDocs) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = writer(f, &dir)?;
    add_doc("one", &iw)?;
    add_doc("two", &iw)?;
    add_doc("three four", &iw)?;
    let mut ir = directory_reader::open_with_writer(&*iw)?;
    let mut is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    let mut hits = is.search(match_all(), 1000)?.score_docs;
    assert_equals(&3, &hits.len());
    let key =
        |is: &rucene::search::index_searcher::IndexSearcher, doc: i32| -> Result<Option<String>> {
            Ok(is
                .stored_fields()?
                .document(doc)?
                .get("key")
                .map(|s| s.to_string()))
        };
    assert_equals(&Some("one".to_string()), &key(&is, hits[0].doc)?);
    assert_equals(&Some("two".to_string()), &key(&is, hits[1].doc)?);
    assert_equals(&Some("three four".to_string()), &key(&is, hits[2].doc)?);

    // some artificial queries to trigger the use of skipTo():

    let mut bq = BooleanQueryBuilder::new();
    bq.add(match_all(), Occur::MUST)?;
    bq.add(match_all(), Occur::MUST)?;
    hits = is.search(Arc::new(bq.build()), 1000)?.score_docs;
    assert_equals(&3, &hits.len());

    bq = BooleanQueryBuilder::new();
    bq.add(match_all(), Occur::MUST)?;
    bq.add(Arc::new(TermQuery::new(term("key", "three"))), Occur::MUST)?;
    hits = is.search(Arc::new(bq.build()), 1000)?.score_docs;
    assert_equals(&1, &hits.len());

    iw.delete_documents_by_terms(vec![term("key", "one")])?;
    drop(is);
    ir.close()?;
    ir = directory_reader::open_with_writer(&*iw)?;
    is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    hits = is.search(match_all(), 1000)?.score_docs;
    assert_equals(&2, &hits.len());

    iw.close()?;
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}

fn test_equals(_f: &mut MatchAllDocs) -> Result<()> {
    let q1 = match_all();
    let q2 = match_all();
    Assert::assert_true(q1.query_eq(&*q2));
    Ok(())
}

fn add_doc(text: &str, iw: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    let f = LuceneTestCase::new_text_field("key", text, Store::YES);
    doc.add(Box::new(f));
    iw.add_document(&doc)?;
    Ok(())
}

fn test_early_termination(f: &mut MatchAllDocs) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = writer(f, &dir)?;
    let num_docs = 500;
    for i in 0..num_docs {
        add_doc(&format!("doc{i}"), &iw)?;
    }
    let ir = directory_reader::open_with_writer(&*iw)?;

    let single_threaded_searcher =
        LuceneTestCase::new_searcher_threads(Arc::clone(&ir), true, true, false);
    let total_hits_threshold = 200;
    let collector_manager = TopScoreDocCollectorManager::new(10, None, total_hits_threshold)?;
    let top_docs = single_threaded_searcher.search_with_manager(match_all(), &collector_manager)?;
    assert_equals(
        &i64::from(total_hits_threshold + 1),
        &top_docs.total_hits.value(),
    );
    assert_equals(
        &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
        &top_docs.total_hits.relation(),
    );

    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let collector_manager = TopScoreDocCollectorManager::new(10, None, num_docs)?;
    let top_docs = is.search_with_manager(match_all(), &collector_manager)?;
    assert_equals(&i64::from(num_docs), &top_docs.total_hits.value());
    assert_equals(
        &TotalHitsRelation::EQUAL_TO,
        &top_docs.total_hits.relation(),
    );

    iw.close()?;
    drop((is, single_threaded_searcher));
    ir.close()?;
    dir.close()?;
    Ok(())
}
