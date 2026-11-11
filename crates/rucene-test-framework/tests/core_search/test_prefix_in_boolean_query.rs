//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPrefixInBooleanQuery.java`.
//!
//! https://issues.apache.org/jira/browse/LUCENE-1974
//!
//! represent the bug of
//!
//! BooleanScorer.score(Collector collector, int max, int firstDocID)
//!
//! Line 273, end=8192, subScorerDocID=11378, then more got false?

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::{BooleanQueryBuilder, Occur, Query};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, close_reader, field_mut, static_hook, term, tq};
use crate::core_support::{assert_equals_msg, class, random, test};

const FIELD: &str = "name";

/// The static fields `directory`, `reader` and `searcher`.
struct Statics {
    directory: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestPrefixInBooleanQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPrefixInBooleanQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testPrefixQuery", test_prefix_query),
            test("testTermQuery", test_term_query),
            test("testTermBooleanQuery", test_term_boolean_query),
            test("testPrefixBooleanQuery", test_prefix_boolean_query),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;

    let mut doc = Document::new();
    let field = LuceneTestCase::new_string_field(FIELD, "meaninglessnames", Store::NO);
    doc.add(Box::new(field));

    for _ in 0..5137 {
        writer.add_document(&doc)?;
    }
    field_mut(&mut doc, FIELD).set_string_value("tangfulin".to_string())?;
    writer.add_document(&doc)?;

    field_mut(&mut doc, FIELD).set_string_value("meaninglessnames".to_string())?;
    for _ in 5138..11377 {
        writer.add_document(&doc)?;
    }
    field_mut(&mut doc, FIELD).set_string_value("tangfulin".to_string())?;
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    writer.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        directory,
        reader,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = statics {
        drop(s.searcher);
        close_reader(Some(s.reader))?;
        close_dir(Some(s.directory))?;
    }
    Ok(())
}

fn assert_matched(query: Arc<dyn Query>) -> Result<()> {
    assert_equals_msg(
        Some("Number of matched documents"),
        &2,
        &searcher().search(query, 1000)?.total_hits.value(),
    );
    Ok(())
}

/// Port of `TestPrefixInBooleanQuery.testPrefixQuery`.
fn test_prefix_query() -> Result<()> {
    assert_matched(Arc::new(PrefixQuery::new(term(FIELD, "tang"))?))
}

/// Port of `TestPrefixInBooleanQuery.testTermQuery`.
fn test_term_query() -> Result<()> {
    assert_matched(tq(FIELD, "tangfulin"))
}

/// Port of `TestPrefixInBooleanQuery.testTermBooleanQuery`.
fn test_term_boolean_query() -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(tq(FIELD, "tangfulin"), Occur::SHOULD)?;
    query.add(tq(FIELD, "notexistnames"), Occur::SHOULD)?;
    assert_matched(Arc::new(query.build()))
}

/// Port of `TestPrefixInBooleanQuery.testPrefixBooleanQuery`.
fn test_prefix_boolean_query() -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(
        Arc::new(PrefixQuery::new(term(FIELD, "tang"))?),
        Occur::SHOULD,
    )?;
    query.add(tq(FIELD, "notexistnames"), Occur::SHOULD)?;
    assert_matched(Arc::new(query.build()))
}
