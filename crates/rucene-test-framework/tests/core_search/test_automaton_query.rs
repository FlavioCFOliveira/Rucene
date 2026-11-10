//! Port of `lucene/core/src/test/org/apache/lucene/search/TestAutomatonQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Field, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{EmptyTermsEnum, FilteredTermsEnum, SingleTermsEnum, TermsEnum};
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    scoring_boolean_rewrite,
};
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{AutomatonQuery, Query};
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::{Automata, Automaton, Operations};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    doc_of, fixture_class, ftest, set_up, tear_down, CountDownLatch,
};
use crate::core_support::{assert_equals, det, random, verbose};

const FN: &str = "field";

/// The instance of `TestAutomatonQuery` (and of `TestAutomatonQueryUnicode`).
#[derive(Default)]
pub struct AutomatonFixture {
    pub directory: Option<TestDirectory>,
    pub reader: Option<Arc<dyn DirectoryReader>>,
    pub searcher: Option<IndexSearcher>,
}

impl AutomatonFixture {
    pub fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }
}

/// `TestAutomatonQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<AutomatonFixture>(
        "org.apache.lucene.search.TestAutomatonQuery",
        vec![
            set_up(automaton_set_up),
            tear_down(tear_down_fixture),
            ftest("testAutomata", test_automata),
            ftest("testEquals", test_equals),
            ftest("testRewriteSingleTerm", test_rewrite_single_term),
            ftest("testRewritePrefix", test_rewrite_prefix),
            ftest("testEmptyOptimization", test_empty_optimization),
            ftest("testHashCodeWithThreads", test_hash_code_with_threads),
            ftest("testBiggishAutomaton", test_biggish_automaton),
        ],
    )
}

/// Indexes `title`, `field` (with each of `values` in turn) and `footer`
/// into one document per value.
pub fn index_values(f: &mut AutomatonFixture, initial: &str, values: &[&str]) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let title_field = LuceneTestCase::new_text_field("title", "some title", Store::NO);
    let mut field: Field = LuceneTestCase::new_text_field(FN, initial, Store::NO);
    let footer_field = LuceneTestCase::new_text_field("footer", "a footer", Store::NO);
    for (i, value) in values.iter().enumerate() {
        if i > 0 || initial != *value {
            field.set_string_value(value.to_string())?;
        }
        writer.add_document(&doc_of(&[&title_field, &field, &footer_field])?)?;
    }
    let reader = writer.get_reader()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    writer.close()?;
    Ok(())
}

fn automaton_set_up(f: &mut AutomatonFixture) -> Result<()> {
    index_values(
        f,
        "this is document one 2345",
        &[
            "this is document one 2345",
            "some text from doc two a short piece 5678.91",
            "doc three has some different stuff with numbers 1234 5678.9 and letter b",
        ],
    )
}

pub fn tear_down_fixture(f: &mut AutomatonFixture) -> Result<()> {
    drop(f.searcher.take());
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut directory) = f.directory.take() {
        directory.close()?;
    }
    Ok(())
}

fn new_term(value: &str) -> Term {
    Term::from_text(FN, value)
}

fn automaton_query_nr_hits(f: &AutomatonFixture, query: Arc<dyn Query>) -> Result<i64> {
    if verbose() {
        println!(
            "TEST: run aq={}",
            rucene::search::query::query_to_string(&*query)
        );
    }
    Ok(f.searcher().search(query, 5)?.total_hits.value())
}

/// `assertAutomatonHits(int, Automaton)`.
pub fn assert_automaton_hits(
    f: &AutomatonFixture,
    expected: i64,
    automaton: Automaton,
    nr_hits: fn(&AutomatonFixture, Arc<dyn Query>) -> Result<i64>,
) -> Result<()> {
    for rewrite in [
        scoring_boolean_rewrite(),
        constant_score_rewrite(),
        constant_score_blended_rewrite(),
        constant_score_boolean_rewrite(),
    ] {
        assert_equals(
            &expected,
            &nr_hits(
                f,
                Arc::new(AutomatonQuery::with_rewrite_method(
                    new_term("bogus"),
                    automaton.clone(),
                    false,
                    rewrite,
                )?),
            )?,
        );
    }
    Ok(())
}

fn hits(f: &AutomatonFixture, expected: i64, automaton: Automaton) -> Result<()> {
    assert_automaton_hits(f, expected, automaton, automaton_query_nr_hits)
}

/// Test some very simple automata.
fn test_automata(f: &mut AutomatonFixture) -> Result<()> {
    hits(f, 0, Automata::make_empty())?;
    hits(f, 0, Automata::make_empty_string())?;
    hits(f, 2, Automata::make_any_char())?;
    hits(f, 3, Automata::make_any_string())?;
    hits(f, 2, Automata::make_string("doc"))?;
    hits(f, 1, Automata::make_char('a' as i32))?;
    hits(f, 2, Automata::make_char_range('a' as i32, 'b' as i32))?;
    hits(f, 2, Automata::make_decimal_interval(1233, 2346, 0)?)?;
    hits(
        f,
        1,
        det(Operations::determinize(
            &Automata::make_decimal_interval(0, 2000, 0)?,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )),
    )?;
    hits(
        f,
        2,
        Operations::union(&[
            Automata::make_char('a' as i32),
            Automata::make_char('b' as i32),
        ]),
    )?;
    hits(
        f,
        0,
        Operations::intersection(
            &Automata::make_char('a' as i32),
            &Automata::make_char('b' as i32),
        ),
    )?;
    hits(
        f,
        1,
        det(Operations::minus(
            &Automata::make_char_range('a' as i32, 'b' as i32),
            &Automata::make_char('a' as i32),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )),
    )?;
    Ok(())
}

fn test_equals(_f: &mut AutomatonFixture) -> Result<()> {
    let a1: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
        new_term("foobar"),
        Automata::make_string("foobar"),
    )?);
    // reference to a1
    let a2 = Arc::clone(&a1);
    // same as a1 (accepts the same language, same term)
    let a3: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
        new_term("foobar"),
        Operations::concatenate(&[Automata::make_string("foo"), Automata::make_string("bar")]),
    )?);
    // different than a1 (same term, but different language)
    let a4: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
        new_term("foobar"),
        Automata::make_string("different"),
    )?);
    // different than a1 (different term, same language)
    let a5: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
        new_term("blah"),
        Automata::make_string("foobar"),
    )?);

    assert_equals(&a1.query_hash(), &a2.query_hash());
    Assert::assert_true(a1.query_eq(&*a2));

    assert_equals(&a1.query_hash(), &a3.query_hash());
    Assert::assert_true(a1.query_eq(&*a3));

    // different class
    let w1: Arc<dyn Query> = Arc::new(WildcardQuery::new(new_term("foobar"))?);
    // different class
    let w2: Arc<dyn Query> = Arc::new(RegexpQuery::new(new_term("foobar"))?);

    Assert::assert_false(a1.query_eq(&*w1));
    Assert::assert_false(a1.query_eq(&*w2));
    Assert::assert_false(w1.query_eq(&*w2));
    Assert::assert_false(a1.query_eq(&*a4));
    Assert::assert_false(a1.query_eq(&*a5));
    // `a1.equals(null)`: a `&dyn Query` is never null, and `equals(null)`
    // is false.
    Assert::assert_false(false);
    Ok(())
}

/// `MultiTerms.getTerms(searcher.getIndexReader(), FN)`.
fn field_terms(f: &AutomatonFixture) -> Result<Box<dyn rucene::index::Terms>> {
    let reader: Arc<dyn IndexReader> = Arc::clone(f.searcher().get_index_reader());
    Ok(MultiTerms::get_terms(&reader, FN)?.expect("field is indexed"))
}

/// Test that rewriting to a single term works as expected, preserves
/// MultiTermQuery semantics.
fn test_rewrite_single_term(f: &mut AutomatonFixture) -> Result<()> {
    let aq = Arc::new(AutomatonQuery::new(
        new_term("bogus"),
        Automata::make_string("piece"),
    )?);
    let terms = field_terms(f)?;
    let te = aq.terms_enum(&*terms)?;
    // `te instanceof SingleTermsEnum`: the filter of the FilteredTermsEnum.
    let is_single = te
        .as_any()
        .and_then(|a| a.downcast_ref::<FilteredTermsEnum>())
        .and_then(|fte| fte.filter().as_any())
        .is_some_and(|a| a.is::<SingleTermsEnum>());
    Assert::assert_true(is_single);
    assert_equals(&1, &automaton_query_nr_hits(f, aq)?);
    Ok(())
}

/// Test that rewriting to a prefix query works as expected, preserves
/// MultiTermQuery semantics.
fn test_rewrite_prefix(f: &mut AutomatonFixture) -> Result<()> {
    let pfx = Automata::make_string("do");
    let prefix_automaton = Operations::concatenate(&[pfx, Automata::make_any_string()]);
    let aq = Arc::new(AutomatonQuery::new(new_term("bogus"), prefix_automaton)?);
    assert_equals(&3, &automaton_query_nr_hits(f, aq)?);
    Ok(())
}

/// Test handling of the empty language
fn test_empty_optimization(f: &mut AutomatonFixture) -> Result<()> {
    let aq = Arc::new(AutomatonQuery::new(
        new_term("bogus"),
        Automata::make_empty(),
    )?);
    // not yet available: assertTrue(aq.getEnum(searcher.getIndexReader())
    // instanceof EmptyTermEnum);
    let terms = field_terms(f)?;
    // `assertSame(TermsEnum.EMPTY, aq.getTermsEnum(terms))`: Java's EMPTY is
    // the only instance of its class, so the identity is the class check.
    let te: Box<dyn TermsEnum> = aq.terms_enum(&*terms)?;
    Assert::assert_true(te.as_any().is_some_and(|a| a.is::<EmptyTermsEnum>()));
    assert_equals(&0, &automaton_query_nr_hits(f, aq)?);
    Ok(())
}

fn test_hash_code_with_threads(_f: &mut AutomatonFixture) -> Result<()> {
    let n = LuceneTestCase::at_least(100);
    let mut queries: Vec<AutomatonQuery> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        queries.push(AutomatonQuery::new(
            Term::from_text("bogus", "bogus"),
            det(Operations::determinize(
                &AutomatonTestUtil::random_automaton(&*random()),
                DEFAULT_DETERMINIZE_WORK_LIMIT,
            )),
        )?);
    }
    let queries = Arc::new(queries);
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let num_threads = TestUtil::next_int(&*random(), 2, 5);
    let mut threads = Vec::new();
    for _thread_id in 0..num_threads {
        let queries = Arc::clone(&queries);
        let starting_gun = Arc::clone(&starting_gun);
        let thread = Thread::new("Thread", move || {
            starting_gun.await_();
            for q in queries.iter() {
                q.query_hash();
            }
        });
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }
    Ok(())
}

fn test_biggish_automaton(_f: &mut AutomatonFixture) -> Result<()> {
    let num_terms = if *TEST_NIGHTLY { 3000 } else { 500 };
    let mut terms: Vec<BytesRef> = Vec::new();
    while terms.len() < num_terms {
        terms.push(BytesRef::from_char_sequence(
            &TestUtil::random_unicode_string(&*random()),
        ));
    }
    terms.sort();
    AutomatonQuery::new(
        Term::from_text("foo", "bar"),
        Automata::make_string_union(terms.iter().cloned())?,
    )?;
    Ok(())
}
