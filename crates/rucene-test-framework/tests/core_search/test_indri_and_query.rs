//! Port of `lucene/core/src/test/org/apache/lucene/search/TestIndriAndQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::{IndriDirichletSimilarity, Similarity};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    query_to_string, BooleanClause, BoostQuery, IndriAndQuery, Occur, Query, ScoreDoc,
};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down, term};
use crate::core_support::{assert_equals_msg, random, try_catch};

/// threshold for comparing floats
#[allow(dead_code)]
pub const SCORE_COMP_THRESH: f32 = 0.0000;

/// The instance of `TestIndriAndQuery`.
pub struct IndriAnd {
    sim: Arc<dyn Similarity>,
    index: Option<TestDirectory>,
    r: Option<Arc<dyn LeafReader>>,
    s: Option<IndexSearcher>,
}

impl Default for IndriAnd {
    fn default() -> Self {
        IndriAnd {
            sim: Arc::new(IndriDirichletSimilarity::new()),
            index: None,
            r: None,
            s: None,
        }
    }
}

impl IndriAnd {
    fn s(&self) -> &IndexSearcher {
        self.s.as_ref().expect("setUp ran")
    }
}

/// `TestIndriAndQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<IndriAnd>(
        "org.apache.lucene.search.TestIndriAndQuery",
        vec![
            set_up(indri_set_up),
            tear_down(|f: &mut IndriAnd| {
                drop(f.s.take());
                if let Some(r) = f.r.take() {
                    r.as_index_reader().close()?;
                }
                if let Some(mut index) = f.index.take() {
                    index.close()?;
                }
                Ok(())
            }),
            ftest("testSimpleQuery1", test_simple_query1),
            ftest("testSimpleQuery2", test_simple_query2),
        ],
    )
}

fn indri_set_up(f: &mut IndriAnd) -> Result<()> {
    let index = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&WHITESPACE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    ));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_similarity(Some(Arc::clone(&f.sim)))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), index.as_directory(), config)?;
    // Query is "President Washington"
    {
        let mut d1 = Document::new();
        d1.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d1",
            TextField::type_stored(),
        )));
        d1.add(Box::new(LuceneTestCase::new_text_field(
            "body",
            "President Washington was the first leader of the US",
            Store::YES,
        )));
        writer.add_document(&d1)?;
    }

    {
        let mut d2 = Document::new();
        d2.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d2",
            TextField::type_stored(),
        )));
        d2.add(Box::new(LuceneTestCase::new_text_field(
            "body",
            "The president is head of the executive branch of government",
            Store::YES,
        )));
        writer.add_document(&d2)?;
    }

    {
        let mut d3 = Document::new();
        d3.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d3",
            TextField::type_stored(),
        )));
        d3.add(Box::new(LuceneTestCase::new_text_field(
            "body",
            "George Washington was a general in the Revolutionary War",
            Store::YES,
        )));
        writer.add_document(&d3)?;
    }

    {
        let mut d4 = Document::new();
        d4.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d4",
            TextField::type_stored(),
        )));
        d4.add(Box::new(LuceneTestCase::new_text_field(
            "body",
            "A company or college can have a president",
            Store::YES,
        )));
        writer.add_document(&d4)?;
    }

    writer.force_merge(1)?;
    let r = LuceneTestCase::get_only_leaf_reader(writer.get_reader()? as Arc<dyn IndexReader>)?;
    writer.close()?;
    let mut s = IndexSearcher::new(Arc::clone(&r).as_index_reader())?;
    s.set_similarity(Arc::clone(&f.sim));
    f.index = Some(index);
    f.r = Some(r);
    f.s = Some(s);
    Ok(())
}

fn test_simple_query1(f: &mut IndriAnd) -> Result<()> {
    let s = f.s();
    let clause1 = BooleanClause::new(tq("body", "george"), Occur::SHOULD);
    let clause2 = BooleanClause::new(tq("body", "washington"), Occur::SHOULD);

    let q: Arc<dyn Query> = Arc::new(IndriAndQuery::new(vec![clause1, clause2]));

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    if let Err(e) = try_catch("java.lang.Error", || {
        assert_equals_msg(
            Some(&format!("2 docs should match {}", query_to_string(&*q))),
            &2,
            &h.len(),
        );
    }) {
        print_hits("testSimpleEqualScores1", &h, s)?;
        e.throw()
    }
    Ok(())
}

fn test_simple_query2(f: &mut IndriAnd) -> Result<()> {
    let s = f.s();
    let clause1 = BooleanClause::new(tq("body", "president"), Occur::SHOULD);
    let clause2 = BooleanClause::new(tq("body", "washington"), Occur::SHOULD);

    let q: Arc<dyn Query> = Arc::new(IndriAndQuery::new(vec![clause1, clause2]));

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    if let Err(e) = try_catch("java.lang.Error", || {
        assert_equals_msg(
            Some(&format!("all docs should match {}", query_to_string(&*q))),
            &4,
            &h.len(),
        );
    }) {
        print_hits("testSimpleEqualScores1", &h, s)?;
        e.throw()
    }
    Ok(())
}

/// macro
fn tq(f: &str, t: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(f, t)))
}

/// macro
#[allow(dead_code)]
fn tq_boost(f: &str, t: &str, b: f32) -> Result<Arc<dyn Query>> {
    let q = tq(f, t);
    Ok(Arc::new(BoostQuery::new(q, b)?))
}

fn print_hits(test: &str, h: &[ScoreDoc], searcher: &IndexSearcher) -> Result<()> {
    eprintln!("------- {test} -------");

    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in h.iter().enumerate() {
        let d = stored_fields.document(hit.doc)?;
        let score = hit.score;
        eprintln!(
            "#{i}: {score} - {}",
            d.get("body").unwrap_or_else(|| "null".into())
        );
    }
    Ok(())
}
