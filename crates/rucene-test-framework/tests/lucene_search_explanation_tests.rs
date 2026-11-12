//! Port of `lucene/test-framework/src/test/org/apache/lucene/tests/search/TestBaseExplanationTestCase.java`
//! (task #356): tests that the `BaseExplanationTestCase` helper code, as
//! well as `CheckHits#checkNoMatchExplanations` are checking what they are
//! suppose to. Run as a suite by the ported runner, it reports what
//! Lucene's own class reported on the JVM (`framework_tests.tsv`, written by
//! `FrameworkTestsOracle`).

mod oracle_support;

use std::any::Any;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::terms::Term;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::explanation::Explanation;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::{DefaultScorerSupplier, FilterWeight, Weight};
use rucene::util::BytesRef;
use rucene_test_framework::search::base_explanation_test_case::{
    self, qtest, ExplanationTestInstance, FIELD,
};
use rucene_test_framework::search::CheckHits;
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

fn run_suite(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    oracle_support::assert_framework_outcomes(&builder.build());
}

/// Throws a Rucene error out of a test method as the Java exception it
/// stands for.
fn rethrow(e: LuceneError) -> ! {
    rucene_test_framework::util::java_exceptions::rethrow_java(e)
}

/// `new TermQuery(new Term(FIELD, text))`.
fn term(text: &str) -> Term {
    Term::new(FIELD, BytesRef::from_char_sequence(text))
}

/// `expectThrows(AssertionError.class, runnable)` over a call that fails
/// by panicking with a `Throwable` or by returning an error.
fn expect_assertion_error(runnable: impl FnOnce() -> Result<()>) {
    LuceneTestCase::expect_throws(ASSERTION_ERROR, || {
        runnable().map_err(|e| {
            if let LuceneError {
                kind: LuceneErrorKind::AssertionError(message),
                ..
            } = e
            {
                Throwable::new(ASSERTION_ERROR, message)
            } else {
                rucene_test_framework::util::java_exceptions::java_throwable(&e)
            }
        })
    });
}

/// `BrokenExplainTermQuery extends TermQuery`.
#[derive(Debug, Clone)]
struct BrokenExplainTermQuery {
    inner: TermQuery,
    /// `toggleExplainMatch`.
    toggle_explain_match: bool,
    /// `breakExplainScores`.
    break_explain_scores: bool,
}

impl BrokenExplainTermQuery {
    fn new(t: Term, toggle_explain_match: bool, break_explain_scores: bool) -> Self {
        BrokenExplainTermQuery {
            inner: TermQuery::new(t),
            toggle_explain_match,
            break_explain_scores,
        }
    }
}

impl Query for BrokenExplainTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `createWeight` (`@Override`): `new BrokenExplainWeight(this,
    /// super.createWeight(searcher, scoreMode, boost))`.
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let inner = self.inner.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(BrokenExplainWeight {
            filter: FilterWeight::with_query(Arc::new(self.clone()), inner),
            toggle_explain_match: self.toggle_explain_match,
            break_explain_scores: self.break_explain_scores,
        }))
    }

    /// `TermQuery.equals`: `sameClassAs(other) && term.equals(other.term)`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<BrokenExplainTermQuery>()
            .is_some_and(|o| o.inner.get_term() == self.inner.get_term())
    }

    /// `TermQuery.hashCode`: `classHash() ^ term.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ self.inner.query_hash() ^ self.inner.class_hash()
    }
}

/// `BrokenExplainWeight extends FilterWeight`.
#[derive(Debug)]
struct BrokenExplainWeight {
    filter: FilterWeight,
    toggle_explain_match: bool,
    break_explain_scores: bool,
}

impl SegmentCacheable for BrokenExplainWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.filter.is_cacheable(ctx)
    }
}

impl Weight for BrokenExplainWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.filter.get_query()
    }

    /// `explain` (`@Override`).
    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let mut result = self.filter.inner.explain(context, doc)?;
        if result.is_match() {
            if self.break_explain_scores {
                let value = -(result.value().double_value() as f32);
                result = Explanation::matched(value, "Broken Explanation Score", vec![result]);
            }
            if self.toggle_explain_match {
                result = Explanation::no_match("Broken Explanation Matching", vec![result]);
            }
        } else if self.toggle_explain_match {
            result = Explanation::matched(-42.0f32, "Broken Explanation Matching", vec![result]);
        }
        Ok(result)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.filter.matches(context, doc)
    }

    /// `scorerSupplier` (`@Override`): `in.scorer(context)` wrapped in a
    /// `DefaultScorerSupplier`, `null` when there is no scorer.
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(scorer) = self.filter.inner.scorer(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(scorer))))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.filter.count(context)
    }
}

/// `testQueryNoMatchWhenExpected()`.
fn test_query_no_match_when_expected() {
    expect_assertion_error(|| {
        qtest(Arc::new(TermQuery::new(term("BOGUS"))), &[3 /* none */])
    });
}

/// `testQueryMatchWhenNotExpected()`.
fn test_query_match_when_not_expected() {
    expect_assertion_error(|| {
        qtest(Arc::new(TermQuery::new(term("w1"))), &[0, 1 /*, 2, 3 */])
    });
}

/// `testIncorrectExplainScores()`.
fn test_incorrect_explain_scores() -> Result<()> {
    // sanity check what a real TermQuery matches
    qtest(Arc::new(TermQuery::new(term("zz"))), &[1, 3])?;

    // ensure when the Explanations are broken, we get an error about those matches
    expect_assertion_error(|| {
        qtest(
            Arc::new(BrokenExplainTermQuery::new(term("zz"), false, true)),
            &[1, 3],
        )
    });
    Ok(())
}

/// `testIncorrectExplainMatches()`.
fn test_incorrect_explain_matches() -> Result<()> {
    // sanity check what a real TermQuery matches
    qtest(Arc::new(TermQuery::new(term("zz"))), &[1, 3])?;

    // ensure when the Explanations are broken, we get an error about the non matches
    expect_assertion_error(|| {
        let q: Arc<dyn Query> = Arc::new(BrokenExplainTermQuery::new(term("zz"), true, false));
        let state = base_explanation_test_case::state();
        let searcher = &state
            .as_ref()
            .unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            })
            .searcher;
        CheckHits::check_no_match_explanations(&q, Some(FIELD), searcher, &[1, 3])
    });
    Ok(())
}

fn test(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |_inst: &mut ExplanationTestInstance| body().unwrap_or_else(|e| rethrow(e)))
}

fn base_explanation_test_case_class() -> ClassBuilder {
    Class::builder("org.apache.lucene.tests.search.TestBaseExplanationTestCase")
        .extends(&base_explanation_test_case::class())
        .constructor(Constructor::no_args(ExplanationTestInstance::default))
        .method(test("testQueryNoMatchWhenExpected", || {
            test_query_no_match_when_expected();
            Ok(())
        }))
        .method(test("testQueryMatchWhenNotExpected", || {
            test_query_match_when_not_expected();
            Ok(())
        }))
        .method(test(
            "testIncorrectExplainScores",
            test_incorrect_explain_scores,
        ))
        .method(test(
            "testIncorrectExplainMatches",
            test_incorrect_explain_matches,
        ))
}

#[test]
fn test_base_explanation_test_case() {
    run_suite(base_explanation_test_case_class());
}
