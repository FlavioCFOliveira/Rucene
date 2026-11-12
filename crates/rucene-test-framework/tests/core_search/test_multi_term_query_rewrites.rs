//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiTermQueryRewrites.java`.
//!
//! `checkBoosts`' anonymous `MultiTermQuery` returns an anonymous
//! `FilteredTermsEnum` whose `accept` sets the enum's `BoostAttribute` to the
//! term's value. Rucene's `FilteredTermsEnumFilter::accept` sees only the
//! term, not the enum's attributes, so the enum is wrapped
//! ([`BoostingTermsEnum`]) and the boost of each term the filter accepted is
//! set on the attributes as `next()` returns it; the attribute holds the same
//! value whenever a collector reads it.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::multi_reader::MultiReader;
use rucene::index::postings_enum::PostingsEnum;
use rucene::index::terms::{AcceptStatus, FilteredTermsEnum, FilteredTermsEnumFilter};
use rucene::index::{ImpactsEnum, IndexReader, SeekStatus, Term, TermState, Terms, TermsEnum};
use rucene::search::boost_attribute::set_boost;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    multi_term_query_eq, multi_term_query_hash, multi_term_rewrite, scoring_boolean_rewrite,
    MultiTermQuery, RewriteMethod, TopTermsBoostOnlyBooleanQueryRewrite,
    TopTermsScoringBooleanQueryRewrite,
};
use rucene::search::term_query::TermQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::{
    query_to_string, BooleanQuery, BoostQuery, ConstantScoreQuery, MatchNoDocsQuery, Query,
    QueryVisitor,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::static_hook;
use crate::core_support::{assert_equals_msg, class, random, test, verbose};

struct Statics {
    dir: TestDirectory,
    sdir1: TestDirectory,
    sdir2: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
    multi_reader: Arc<MultiReader>,
    multi_reader_dupls: Arc<MultiReader>,
    searcher: Arc<IndexSearcher>,
    multi_searcher: Arc<IndexSearcher>,
    multi_searcher_dupls: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// The static `searcher`, `multiSearcher` and `multiSearcherDupls`.
fn searchers() -> [Arc<IndexSearcher>; 3] {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref().expect("beforeClass ran");
    [
        Arc::clone(&s.searcher),
        Arc::clone(&s.multi_searcher),
        Arc::clone(&s.multi_searcher_dupls),
    ]
}

/// `TestMultiTermQueryRewrites`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestMultiTermQueryRewrites",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test(
                "testRewritesWithDuplicateTerms",
                test_rewrites_with_duplicate_terms,
            ),
            test("testBoosts", test_boosts),
            test("testMaxClauseLimitations", test_max_clause_limitations),
        ],
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let sdir1 = LuceneTestCase::new_directory();
    let sdir2 = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::new(MockAnalyzer::new(&*random())),
    )?;
    let swriter1 = RandomIndexWriter::new(
        &*random(),
        sdir1.as_directory(),
        Arc::new(MockAnalyzer::new(&*random())),
    )?;
    let swriter2 = RandomIndexWriter::new(
        &*random(),
        sdir2.as_directory(),
        Arc::new(MockAnalyzer::new(&*random())),
    )?;
    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "data",
            &i.to_string(),
            Store::NO,
        )));
        writer.add_document(&doc)?;
        (if i % 2 == 0 { &swriter1 } else { &swriter2 }).add_document(&doc)?;
    }
    writer.force_merge(1)?;
    swriter1.force_merge(1)?;
    swriter2.force_merge(1)?;
    writer.close()?;
    swriter1.close()?;
    swriter2.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    let multi_reader = Arc::new(MultiReader::new(
        vec![
            directory_reader::open(sdir1.as_directory())? as Arc<dyn IndexReader>,
            directory_reader::open(sdir2.as_directory())? as Arc<dyn IndexReader>,
        ],
        true,
    )?);
    let multi_searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&multi_reader)));
    let multi_reader_dupls = Arc::new(MultiReader::new(
        vec![
            directory_reader::open(sdir1.as_directory())? as Arc<dyn IndexReader>,
            directory_reader::open(dir.as_directory())? as Arc<dyn IndexReader>,
        ],
        true,
    )?);
    let multi_searcher_dupls = Arc::new(LuceneTestCase::new_searcher(Arc::clone(
        &multi_reader_dupls,
    )));
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        sdir1,
        sdir2,
        reader,
        multi_reader,
        multi_reader_dupls,
        searcher,
        multi_searcher,
        multi_searcher_dupls,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        dir,
        sdir1,
        sdir2,
        reader,
        multi_reader,
        multi_reader_dupls,
        searcher,
        multi_searcher,
        multi_searcher_dupls,
    }) = statics
    {
        drop((searcher, multi_searcher, multi_searcher_dupls));
        reader.close()?;
        multi_reader.close()?;
        multi_reader_dupls.close()?;
        dir.close()?;
        sdir1.close()?;
        sdir2.close()?;
    }
    Ok(())
}

fn extract_inner_query(q: &Arc<dyn Query>) -> Arc<dyn Query> {
    if let Some(csq) = q.as_any().downcast_ref::<ConstantScoreQuery>() {
        // wrapped as ConstantScoreQuery
        return csq.get_query();
    }
    Arc::clone(q)
}

fn extract_term(q: &Arc<dyn Query>) -> Term {
    let q = extract_inner_query(q);
    q.as_any()
        .downcast_ref::<TermQuery>()
        .expect("java.lang.ClassCastException: not a TermQuery")
        .get_term()
        .clone()
}

fn check_boolean_query_order(q: &Arc<dyn Query>) {
    let q = extract_inner_query(q);
    let bq = q
        .as_any()
        .downcast_ref::<BooleanQuery>()
        .expect("java.lang.ClassCastException: not a BooleanQuery");
    let mut last: Option<Term> = None;
    for clause in bq.clauses() {
        let act = extract_term(clause.query());
        if let Some(last) = &last {
            Assert::assert_true_msg("sort order of terms in BQ violated", *last < act);
        }
        last = Some(act);
    }
}

/// The three rewrites of `mtq` and their equality checks, shared by
/// `checkDuplicateTerms` and `checkBoosts`.
fn rewrite_three(mtq: Arc<dyn Query>) -> Result<[Arc<dyn Query>; 3]> {
    let [searcher, multi_searcher, multi_searcher_dupls] = searchers();
    let q1 = searcher.rewrite(Arc::clone(&mtq))?;
    let q2 = multi_searcher.rewrite(Arc::clone(&mtq))?;
    let q3 = multi_searcher_dupls.rewrite(mtq)?;
    if verbose() {
        println!();
        println!("single segment: {}", query_to_string(&*q1));
        println!("multi segment: {}", query_to_string(&*q2));
        println!("multi segment with duplicates: {}", query_to_string(&*q3));
    }
    if !q1.query_eq(&*q2) {
        Assert::fail(&format!(
            "The multi-segment case must produce same rewritten query expected:<{}> but was:<{}>",
            query_to_string(&*q1),
            query_to_string(&*q2)
        ));
    }
    if !q1.query_eq(&*q3) {
        Assert::fail(&format!(
            "The multi-segment case with duplicates must produce same rewritten query expected:<{}> but was:<{}>",
            query_to_string(&*q1),
            query_to_string(&*q3)
        ));
    }
    Ok([q1, q2, q3])
}

fn check_duplicate_terms(method: Arc<dyn RewriteMethod>) -> Result<()> {
    let mtq: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range_with_rewrite_method(
        "data",
        Some("2"),
        Some("7"),
        true,
        true,
        method,
    )?);
    let [q1, q2, q3] = rewrite_three(mtq)?;
    check_boolean_query_order(&q1);
    check_boolean_query_order(&q2);
    check_boolean_query_order(&q3);
    Ok(())
}

fn test_rewrites_with_duplicate_terms() -> Result<()> {
    check_duplicate_terms(scoring_boolean_rewrite())?;

    check_duplicate_terms(constant_score_boolean_rewrite())?;

    // use a large PQ here to only test duplicate terms and dont mix up when all scores are equal
    check_duplicate_terms(Arc::new(TopTermsScoringBooleanQueryRewrite::new(1024)))?;
    check_duplicate_terms(Arc::new(TopTermsBoostOnlyBooleanQueryRewrite::new(1024)))?;
    Ok(())
}

fn check_boolean_query_boosts(bq: &Arc<dyn Query>) {
    let bq = bq
        .as_any()
        .downcast_ref::<BooleanQuery>()
        .expect("java.lang.ClassCastException: not a BooleanQuery");
    for clause in bq.clauses() {
        let boost_q = clause
            .query()
            .as_any()
            .downcast_ref::<BoostQuery>()
            .expect("java.lang.ClassCastException: not a BoostQuery");
        let inner = boost_q.get_query();
        let mtq = inner
            .as_any()
            .downcast_ref::<TermQuery>()
            .expect("java.lang.ClassCastException: not a TermQuery");
        let expected: f32 = mtq
            .get_term()
            .text()
            .parse()
            .expect("java.lang.NumberFormatException");
        // assertEquals("Parallel sorting of boosts in rewrite mode broken", expected, boost, 0)
        if f64::from(expected) != f64::from(boost_q.get_boost()) {
            Assert::fail(&format!(
                "Parallel sorting of boosts in rewrite mode broken expected:<{expected}> but was:<{}>",
                boost_q.get_boost()
            ));
        }
    }
}

/// The anonymous `FilteredTermsEnum` of `checkBoosts`: its `accept`.
struct BoostFilter;

impl FilteredTermsEnumFilter for BoostFilter {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        // boostAtt.setBoost(Float.parseFloat(term.utf8ToString())) is set by
        // `BoostingTermsEnum::next` on the accepted term.
        if term.length == 0 {
            return Ok(AcceptStatus::No);
        }
        let c = char::from(term.bytes[term.offset]);
        if c >= '2' {
            if c <= '7' {
                Ok(AcceptStatus::Yes)
            } else {
                Ok(AcceptStatus::End)
            }
        } else {
            Ok(AcceptStatus::No)
        }
    }
}

/// The `FilteredTermsEnum` of `checkBoosts` with its `BoostAttribute` set to
/// `Float.parseFloat(term.utf8ToString())` of each term it returns.
struct BoostingTermsEnum {
    inner: FilteredTermsEnum,
}

impl fmt::Display for BoostingTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for BoostingTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        self.inner.as_any()
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.inner.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        let next = self.inner.next()?;
        if let Some(term) = &next {
            let boost: f32 = term
                .utf8_to_string()?
                .parse()
                .expect("java.lang.NumberFormatException");
            set_boost(self.inner.attributes(), boost);
        }
        Ok(next)
    }
}

/// The anonymous `MultiTermQuery("data", method)` of `checkBoosts`.
#[derive(Debug, Clone)]
struct BoostingQuery {
    rewrite_method: Arc<dyn RewriteMethod>,
}

impl MultiTermQuery for BoostingQuery {
    fn get_field(&self) -> &str {
        "data"
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(&self.rewrite_method)
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        _atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(BoostingTermsEnum {
            // `new FilteredTermsEnum(terms.iterator())`, which is
            // `this(tenum, true)` (`FilteredTermsEnum.java:78-80`): the first
            // step seeks to the null initial seek term and ends the enum.
            inner: FilteredTermsEnum::with_start_with_seek(
                terms.iterator()?,
                Box::new(BoostFilter),
                true,
            ),
        }))
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn to_multi_term_query_arc(&self) -> Arc<dyn MultiTermQuery> {
        Arc::new(self.clone())
    }

    fn to_query_arc(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }
}

impl Query for BoostingQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    fn to_query_string(&self, _field: &str) -> String {
        "dummy".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        multi_term_rewrite(self, index_searcher)
    }

    fn query_eq(&self, obj: &dyn Query) -> bool {
        self.same_class_as(obj)
            && obj
                .as_any()
                .downcast_ref::<BoostingQuery>()
                .is_some_and(|that| multi_term_query_eq(self, that))
    }

    fn query_hash(&self) -> i32 {
        multi_term_query_hash(self)
    }
}

fn check_boosts(method: Arc<dyn RewriteMethod>) -> Result<()> {
    let mtq: Arc<dyn Query> = Arc::new(BoostingQuery {
        rewrite_method: method,
    });
    let [q1, q2, q3] = rewrite_three(mtq)?;
    if q1.as_any().is::<MatchNoDocsQuery>() {
        Assert::assert_true(q1.as_any().is::<MatchNoDocsQuery>());
        Assert::assert_true(q3.as_any().is::<MatchNoDocsQuery>());
    } else {
        check_boolean_query_boosts(&q1);
        check_boolean_query_boosts(&q2);
        check_boolean_query_boosts(&q3);
        debug_assert!(false);
    }
    Ok(())
}

fn test_boosts() -> Result<()> {
    check_boosts(scoring_boolean_rewrite())?;

    // use a large PQ here to only test boosts and dont mix up when all scores are equal
    check_boosts(Arc::new(TopTermsScoringBooleanQueryRewrite::new(1024)))?;
    Ok(())
}

/// `try { body } finally { IndexSearcher.setMaxClauseCount(saved); }`.
fn with_max_clause_count(count: i32, body: impl FnOnce() -> Result<()>) -> Result<()> {
    let saved_max_clause_count = IndexSearcher::get_max_clause_count();
    IndexSearcher::set_max_clause_count(count)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    IndexSearcher::set_max_clause_count(saved_max_clause_count)?;
    match result {
        Ok(r) => r,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn check_max_clause_limitation(method: Arc<dyn RewriteMethod>) -> Result<()> {
    with_max_clause_count(3, || {
        let mtq: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range_with_rewrite_method(
            "data",
            Some("2"),
            Some("7"),
            true,
            true,
            method,
        )?);
        let [_, _, multi_searcher_dupls] = searchers();
        let expected = expect_throws(
            "org.apache.lucene.search.IndexSearcher$TooManyClauses",
            || multi_searcher_dupls.rewrite(Arc::clone(&mtq)),
        );
        //  Maybe remove this assert in later versions, when internal API changes:
        assert_equals_msg(
            Some("Should throw BooleanQuery.TooManyClauses with a stacktrace containing checkMaxClauseCount()"),
            "checkMaxClauseCount",
            expected
                .stack_trace()
                .first()
                .map(|e| e.method_name.as_str())
                .unwrap_or_default(),
        );
        Ok(())
    })
}

fn check_no_max_clause_limitation(method: Arc<dyn RewriteMethod>) -> Result<()> {
    with_max_clause_count(3, || {
        let mtq: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range_with_rewrite_method(
            "data",
            Some("2"),
            Some("7"),
            true,
            true,
            method,
        )?);
        let [_, _, multi_searcher_dupls] = searchers();
        multi_searcher_dupls.rewrite(mtq)?;
        Ok(())
    })
}

fn test_max_clause_limitations() -> Result<()> {
    check_max_clause_limitation(scoring_boolean_rewrite())?;
    check_max_clause_limitation(constant_score_boolean_rewrite())?;
    check_no_max_clause_limitation(constant_score_rewrite())?;
    check_no_max_clause_limitation(constant_score_blended_rewrite())?;
    check_no_max_clause_limitation(Arc::new(TopTermsScoringBooleanQueryRewrite::new(1024)))?;
    check_no_max_clause_limitation(Arc::new(TopTermsBoostOnlyBooleanQueryRewrite::new(1024)))?;
    Ok(())
}
