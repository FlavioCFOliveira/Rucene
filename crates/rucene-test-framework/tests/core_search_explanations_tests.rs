//! The `lucene/core/src/test/org/apache/lucene/search/` subclasses of
//! `TestSimpleExplanations` and `TestComplexExplanations` (rmp #361), with
//! the test methods they inherit and their overrides:
//! `TestSimpleExplanationsOfNonMatches`, `TestSimpleExplanationsWithFillerDocs`
//! and `TestComplexExplanationsOfNonMatches`.
//!
//! The two parent classes are declared and run here with their test methods
//! (`core_search_explanations`), over an instance that holds the subclass's
//! `qtest` override ([`m::ExplanationsCase`]); this is their only binding.

mod core_search_explanations;
mod suite_seeds;

use std::fmt::Display;
use std::sync::{Arc, LazyLock, Mutex};

use core_search_explanations as m;
use m::ExplanationsCase;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::BeforeClass;
use randomizedtesting::RandomizedTest;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::ClassicSimilarity;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::base_explanation_test_case::{
    self, create_doc, term_query, ExplanationState, DOC_FIELDS, FIELD,
};
use rucene_test_framework::search::CheckHits;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use suite_seeds::run_suite;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// The instance of a subclass: the case whose `qtest` it overrides.
pub struct CaseInstance(Arc<dyn ExplanationsCase>);

/// The instance of a parent class, which keeps `BaseExplanationTestCase`'s
/// `qtest`.
struct ParentCase;

impl ExplanationsCase for ParentCase {}

/// A test method of the parent classes, run on the instance's case.
fn case_test(name: &str, body: fn(&dyn ExplanationsCase) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut CaseInstance| body(&*inst.0).unwrap_or_else(|e| rethrow_java(e)))
}

/// `CheckHits.checkNoMatchExplanations(q, FIELD, searcher, expDocNrs)`, the
/// `qtest` override of the two `...OfNonMatches` classes.
fn check_no_match_explanations(q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
    let state = base_explanation_test_case::state();
    let searcher = &state
        .as_ref()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        })
        .searcher;
    CheckHits::check_no_match_explanations(&q, Some(FIELD), searcher, exp_doc_nrs)
}

// -----------------------------------------------------------------------------
// TestSimpleExplanations (parent)
// -----------------------------------------------------------------------------

/// `org.apache.lucene.search.TestSimpleExplanations extends
/// BaseExplanationTestCase`, with its test methods.
fn simple_explanations_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.search.TestSimpleExplanations")
            .extends(&base_explanation_test_case::class())
            .constructor(Constructor::no_args(|| CaseInstance(Arc::new(ParentCase))))
            .method(case_test("testT1", m::test_t1))
            .method(case_test("testT2", m::test_t2))
            .method(case_test("testMA1", m::test_ma1))
            .method(case_test("testMA2", m::test_ma2))
            .method(case_test("testP1", m::test_p1))
            .method(case_test("testP2", m::test_p2))
            .method(case_test("testP3", m::test_p3))
            .method(case_test("testP4", m::test_p4))
            .method(case_test("testP5", m::test_p5))
            .method(case_test("testP6", m::test_p6))
            .method(case_test("testP7", m::test_p7))
            .method(case_test("testCSQ1", m::test_csq1))
            .method(case_test("testCSQ2", m::test_csq2))
            .method(case_test("testCSQ3", m::test_csq3))
            .method(case_test("testDMQ1", m::test_dmq1))
            .method(case_test("testDMQ2", m::test_dmq2))
            .method(case_test("testDMQ3", m::test_dmq3))
            .method(case_test("testDMQ4", m::test_dmq4))
            .method(case_test("testDMQ5", m::test_dmq5))
            .method(case_test("testDMQ6", m::test_dmq6))
            .method(case_test("testDMQ7", m::test_dmq7))
            .method(case_test("testDMQ8", m::test_dmq8))
            .method(case_test("testDMQ9", m::test_dmq9))
            .method(case_test("testMPQ1", m::test_mpq1))
            .method(case_test("testMPQ2", m::test_mpq2))
            .method(case_test("testMPQ3", m::test_mpq3))
            .method(case_test("testMPQ4", m::test_mpq4))
            .method(case_test("testMPQ5", m::test_mpq5))
            .method(case_test("testMPQ6", m::test_mpq6))
            .method(case_test("testBQ1", m::test_bq1))
            .method(case_test("testBQ2", m::test_bq2))
            .method(case_test("testBQ3", m::test_bq3))
            .method(case_test("testBQ4", m::test_bq4))
            .method(case_test("testBQ5", m::test_bq5))
            .method(case_test("testBQ6", m::test_bq6))
            .method(case_test("testBQ7", m::test_bq7))
            .method(case_test("testBQ8", m::test_bq8))
            .method(case_test("testBQ9", m::test_bq9))
            .method(case_test("testBQ10", m::test_bq10))
            .method(case_test("testBQ11", m::test_bq11))
            .method(case_test("testBQ14", m::test_bq14))
            .method(case_test("testBQ15", m::test_bq15))
            .method(case_test("testBQ16", m::test_bq16))
            .method(case_test("testBQ17", m::test_bq17))
            .method(case_test("testBQ19", m::test_bq19))
            .method(case_test("testBQ20", m::test_bq20))
            .method(case_test("testBQ21", m::test_bq21))
            .method(case_test("testBQ23", m::test_bq23))
            .method(case_test("testBQ24", m::test_bq24))
            .method(case_test("testBQ25", m::test_bq25))
            .method(case_test("testBQ26", m::test_bq26))
            .method(case_test("testMultiFieldBQ1", m::test_multi_field_bq1))
            .method(case_test("testMultiFieldBQ2", m::test_multi_field_bq2))
            .method(case_test("testMultiFieldBQ3", m::test_multi_field_bq3))
            .method(case_test("testMultiFieldBQ4", m::test_multi_field_bq4))
            .method(case_test("testMultiFieldBQ5", m::test_multi_field_bq5))
            .method(case_test("testMultiFieldBQ6", m::test_multi_field_bq6))
            .method(case_test("testMultiFieldBQ7", m::test_multi_field_bq7))
            .method(case_test("testMultiFieldBQ8", m::test_multi_field_bq8))
            .method(case_test("testMultiFieldBQ9", m::test_multi_field_bq9))
            .method(case_test("testMultiFieldBQ10", m::test_multi_field_bq10))
            .method(case_test(
                "testMultiFieldBQofPQ1",
                m::test_multi_field_b_qof_pq1,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ2",
                m::test_multi_field_b_qof_pq2,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ3",
                m::test_multi_field_b_qof_pq3,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ4",
                m::test_multi_field_b_qof_pq4,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ5",
                m::test_multi_field_b_qof_pq5,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ6",
                m::test_multi_field_b_qof_pq6,
            ))
            .method(case_test(
                "testMultiFieldBQofPQ7",
                m::test_multi_field_b_qof_pq7,
            ))
            .method(case_test("testSynonymQuery", m::test_synonym_query))
            .method(case_test("testEquality", m::test_equality))
            .build()
    });
    Arc::clone(&CLASS)
}

// -----------------------------------------------------------------------------
// TestSimpleExplanationsOfNonMatches
// -----------------------------------------------------------------------------

/// `TestSimpleExplanationsOfNonMatches`: subclass of TestSimpleExplanations
/// that verifies non matches.
struct SimpleExplanationsOfNonMatches;

impl ExplanationsCase for SimpleExplanationsOfNonMatches {
    /// Overrides superclass to ignore matches and focus on non-matches.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        check_no_match_explanations(q, exp_doc_nrs)
    }
}

fn simple_explanations_of_non_matches_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestSimpleExplanationsOfNonMatches")
        .extends(&simple_explanations_class())
        .constructor(Constructor::no_args(|| {
            CaseInstance(Arc::new(SimpleExplanationsOfNonMatches))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestSimpleExplanationsWithFillerDocs
// -----------------------------------------------------------------------------

/// `NUM_FILLER_DOCS` (`private static final`): num of empty docs injected
/// between every doc in the index.
fn num_filler_docs() -> i32 {
    if *TEST_NIGHTLY {
        rucene::search::boolean_scorer::SIZE
    } else {
        4
    }
}

/// `PRE_FILLER_DOCS` (`private static`): num of empty docs injected prior to
/// the first doc in the (main) index.
static PRE_FILLER_DOCS: Mutex<i32> = Mutex::new(0);

/// `EXTRA` (`private static`): if non-null then the filler docs are not
/// empty, and need to be filtered out from queries using this as both field
/// name & field value.
static EXTRA: Mutex<Option<&'static str>> = Mutex::new(None);

fn extra() -> Option<&'static str> {
    *EXTRA.lock().unwrap_or_else(|e| e.into_inner())
}

fn pre_filler_docs() -> i32 {
    *PRE_FILLER_DOCS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `replaceIndex()` (`@BeforeClass`): replaces the index created by our
/// superclass with a new one that includes a lot of docs filler docs.
fn replace_index() -> Result<()> {
    *EXTRA.lock().unwrap_or_else(|e| e.into_inner()) = if LuceneTestCase::random().next_boolean() {
        None
    } else {
        Some("extra")
    };
    *PRE_FILLER_DOCS.lock().unwrap_or_else(|e| e.into_inner()) =
        TestUtil::next_int(&*LuceneTestCase::random(), 0, num_filler_docs() / 2);

    // free up what our super class created that we won't be using
    let ExplanationState {
        searcher,
        reader,
        mut directory,
        analyzer,
    } = base_explanation_test_case::state()
        .take()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
    drop(searcher);
    (Arc::clone(&reader) as Arc<dyn IndexReader>).close()?;
    drop(reader);
    directory.close()?;

    let directory = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(
        Arc::clone(&analyzer) as Arc<dyn Analyzer>
    );
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let writer = RandomIndexWriter::new_with_config(
        &*LuceneTestCase::random(),
        directory.as_directory(),
        config,
    )?;
    let result = (|| -> Result<_> {
        for _filler in 0..pre_filler_docs() {
            writer.add_document(&make_filler_doc())?;
        }
        for i in 0..DOC_FIELDS.len() {
            writer.add_document(&create_doc(i))?;

            for _filler in 0..num_filler_docs() {
                writer.add_document(&make_filler_doc())?;
            }
        }
        let reader = writer.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        Ok((reader, searcher))
    })();
    let closed = writer.close();
    drop(writer);
    let (reader, searcher) = result?;
    closed?;
    *base_explanation_test_case::state() = Some(ExplanationState {
        searcher,
        reader,
        directory,
        analyzer,
    });
    Ok(())
}

/// `makeFillerDoc()` (`private static`). Java's `EMPTY_DOC` is one shared
/// empty `Document`; an empty document is built here for each use.
fn make_filler_doc() -> Document {
    let Some(extra) = extra() else {
        return Document::new();
    };
    let mut doc =
        create_doc(
            TestUtil::next_int(&*LuceneTestCase::random(), 0, DOC_FIELDS.len() as i32 - 1) as usize,
        );
    doc.add(Box::new(LuceneTestCase::new_string_field(
        extra,
        extra,
        Store::NO,
    )));
    doc
}

/// `TestSimpleExplanationsWithFillerDocs`.
struct SimpleExplanationsWithFillerDocs;

impl ExplanationsCase for SimpleExplanationsWithFillerDocs {
    /// Adjusts `expDocNrs` based on the filler docs injected in the index,
    /// and if neccessary wraps the `q` in a BooleanQuery that will filter out
    /// all filler docs using the `EXTRA` field.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        let mut q = q;
        let mut exp_doc_nrs = exp_doc_nrs.to_vec();
        for doc in exp_doc_nrs.iter_mut() {
            *doc = pre_filler_docs() + ((num_filler_docs() + 1) * *doc);
        }

        if let Some(extra) = extra() {
            let mut builder = BooleanQueryBuilder::new();
            builder.add_clause(BooleanClause::new(q, Occur::MUST))?;
            builder.add_clause(BooleanClause::new(
                term_query(extra, extra),
                Occur::MUST_NOT,
            ))?;
            q = Arc::new(builder.build());
        }
        base_explanation_test_case::qtest(q, &exp_doc_nrs)
    }
}

/// `Assume.assumeNotNull("test is not viable with empty filler docs", EXTRA)`.
fn assume_extra() {
    let message = "test is not viable with empty filler docs";
    let extra = extra();
    RandomizedTest::assume_not_null(Some(&[
        Some(&message as &dyn Display),
        extra.as_ref().map(|e| e as &dyn Display),
    ]));
}

fn simple_explanations_with_filler_docs_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestSimpleExplanationsWithFillerDocs")
        .extends(&simple_explanations_class())
        .constructor(Constructor::no_args(|| {
            CaseInstance(Arc::new(SimpleExplanationsWithFillerDocs))
        }))
        .method(
            Method::new("replaceIndex")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .throws(&["java.lang.Exception"])
                .annotate(BeforeClass)
                .static_body(|| replace_index().unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(case_test("testMA1", |c| {
            assume_extra();
            m::test_ma1(c)
        }))
        .method(case_test("testMA2", |c| {
            assume_extra();
            m::test_ma2(c)
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestComplexExplanations (parent)
// -----------------------------------------------------------------------------

/// `org.apache.lucene.search.TestComplexExplanations extends
/// BaseExplanationTestCase`, with its `setUp()`/`tearDown()` overrides and
/// test methods.
fn complex_explanations_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.search.TestComplexExplanations")
            .extends(&base_explanation_test_case::class())
            .constructor(Constructor::no_args(|| CaseInstance(Arc::new(ParentCase))))
            .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
                |_inst: &mut CaseInstance| {
                    LuceneTestCase::super_set_up();
                    // TODO: switch to BM25?
                    if let Some(state) = base_explanation_test_case::state().as_mut() {
                        state
                            .searcher
                            .set_similarity(Arc::new(ClassicSimilarity::new()));
                    }
                },
            ))
            .method(
                Method::new("tearDown")
                    .throws(&["java.lang.Exception"])
                    .body(|_inst: &mut CaseInstance| {
                        if let Some(state) = base_explanation_test_case::state().as_mut() {
                            state
                                .searcher
                                .set_similarity(IndexSearcher::get_default_similarity());
                        }
                        LuceneTestCase::super_tear_down();
                    }),
            )
            .method(case_test("testT3", m::complex_test_t3))
            .method(case_test("testMA3", m::complex_test_ma3))
            .method(case_test("testFQ5", m::complex_test_fq5))
            .method(case_test("testCSQ4", m::complex_test_csq4))
            .method(case_test("testDMQ10", m::complex_test_dmq10))
            .method(case_test("testMPQ7", m::complex_test_mpq7))
            .method(case_test("testBQ12", m::complex_test_bq12))
            .method(case_test("testBQ13", m::complex_test_bq13))
            .method(case_test("testBQ18", m::complex_test_bq18))
            .method(case_test("testBQ21", m::complex_test_bq21))
            .method(case_test("testBQ22", m::complex_test_bq22))
            .method(case_test(
                "testExplainFailingOptionalClauses",
                m::complex_test_explain_failing_optional_clauses,
            ))
            .build()
    });
    Arc::clone(&CLASS)
}

// -----------------------------------------------------------------------------
// TestComplexExplanationsOfNonMatches
// -----------------------------------------------------------------------------

/// `TestComplexExplanationsOfNonMatches`: subclass of
/// TestComplexExplanations that verifies non matches.
struct ComplexExplanationsOfNonMatches;

impl ExplanationsCase for ComplexExplanationsOfNonMatches {
    /// Overrides superclass to ignore matches and focus on non-matches.
    fn qtest(&self, q: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
        check_no_match_explanations(q, exp_doc_nrs)
    }
}

fn complex_explanations_of_non_matches_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestComplexExplanationsOfNonMatches")
        .extends(&complex_explanations_class())
        .constructor(Constructor::no_args(|| {
            CaseInstance(Arc::new(ComplexExplanationsOfNonMatches))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

fn run(class: fn() -> Arc<Class>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(class);
}

#[test]
fn test_simple_explanations() {
    run(simple_explanations_class);
}

#[test]
fn test_complex_explanations() {
    run(complex_explanations_class);
}

#[test]
fn test_simple_explanations_of_non_matches() {
    run(simple_explanations_of_non_matches_class);
}

#[test]
fn test_simple_explanations_with_filler_docs() {
    run(simple_explanations_with_filler_docs_class);
}

#[test]
fn test_complex_explanations_of_non_matches() {
    run(complex_explanations_of_non_matches_class);
}
