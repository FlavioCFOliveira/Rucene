//! Port of `lucene/core/src/test/org/apache/lucene/search/TestReqOptSumScorer.java`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexOptions, Term};
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::req_opt_sum_scorer::{ReqOptSumScorer, ReqOptSumScorerOverrides};
use rucene::search::scorer::Scorer;
use rucene::search::term_query::TermQuery;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreMode, NO_MORE_DOCS};
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, term_freq_id, Att};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::random_approximation_query::RandomApproximationQuery;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::test_similarity::SimpleSimilarity;
use crate::core_support::{assert_equals, class, random, test};

/// `TestReqOptSumScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestReqOptSumScorer",
        vec![
            test("testBasicsMust", test_basics_must),
            test("testBasicsFilter", test_basics_filter),
            test("testMaxBlock", test_max_block),
            test("testMaxScoreSegment", test_max_score_segment),
            test("testMustRandomFrequentOpt", test_must_random_frequent_opt),
            test("testMustRandomRareOpt", test_must_random_rare_opt),
            test(
                "testFilterRandomFrequentOpt",
                test_filter_random_frequent_opt,
            ),
            test("testFilterRandomRareOpt", test_filter_random_rare_opt),
        ],
    )
}

/// Port of `TestReqOptSumScorer.testBasicsMust`.
fn test_basics_must() -> Result<()> {
    do_test_basics(Occur::MUST)
}

/// Port of `TestReqOptSumScorer.testBasicsFilter`.
fn test_basics_filter() -> Result<()> {
    do_test_basics(Occur::FILTER)
}

/// A document with a `StringField` `f` per value.
fn string_doc(field: &str, values: &[&str]) -> Result<Document> {
    let mut doc = Document::new();
    for value in values {
        doc.add(Box::new(StringField::new(
            field,
            value.to_string(),
            Store::NO,
        )?));
    }
    Ok(doc)
}

/// `new ConstantScoreQuery(new TermQuery(new Term(field, text)))`.
fn cs_term(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::new(
        Term::from_text(field, text),
    ))))
}

/// `NullPointerException` for a `null` scorer.
fn npe<T>() -> T {
    randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
}

/// `private void doTestBasics(Occur reqOccur)`.
fn do_test_basics(req_occur: Occur) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    // retain doc id order
    iwc.set_merge_policy(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    ))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    w.add_document(&string_doc("f", &["foo"])?)?;
    w.add_document(&string_doc("f", &["foo", "bar"])?)?;
    w.add_document(&string_doc("f", &["foo"])?)?;
    w.add_document(&string_doc("f", &["bar"])?)?;
    w.add_document(&string_doc("f", &["foo", "bar"])?)?;
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let mut query = BooleanQueryBuilder::new();
    query.add(cs_term("f", "foo"), req_occur)?;
    query.add(cs_term("f", "bar"), Occur::SHOULD)?;
    let query: Arc<dyn Query> = Arc::new(query.build());
    let weight = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::TOP_SCORES, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let context = &leaves[0];

    let mut scorer = weight.scorer(context)?.unwrap_or_else(npe);
    assert_equals(&0, &scorer.iterator().next_doc()?);
    assert_equals(&1, &scorer.iterator().next_doc()?);
    assert_equals(&2, &scorer.iterator().next_doc()?);
    assert_equals(&4, &scorer.iterator().next_doc()?);
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut ss = weight.scorer_supplier(context)?.unwrap_or_else(npe);
    ss.set_top_level_scoring_clause()?;
    let mut scorer = ss.get(i64::MAX)?;
    scorer.set_min_competitive_score(1f32.next_down())?;
    if req_occur == Occur::MUST {
        assert_equals(&0, &scorer.iterator().next_doc()?);
    }
    assert_equals(&1, &scorer.iterator().next_doc()?);
    if req_occur == Occur::MUST {
        assert_equals(&2, &scorer.iterator().next_doc()?);
    }
    assert_equals(&4, &scorer.iterator().next_doc()?);
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut ss = weight.scorer_supplier(context)?.unwrap_or_else(npe);
    ss.set_top_level_scoring_clause()?;
    let mut scorer = ss.get(i64::MAX)?;
    scorer.set_min_competitive_score(1f32.next_up())?;
    if req_occur == Occur::MUST {
        assert_equals(&1, &scorer.iterator().next_doc()?);
        assert_equals(&4, &scorer.iterator().next_doc()?);
    }
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut ss = weight.scorer_supplier(context)?.unwrap_or_else(npe);
    ss.set_top_level_scoring_clause()?;
    let mut scorer = ss.get(i64::MAX)?;
    assert_equals(&0, &scorer.iterator().next_doc()?);
    scorer.set_min_competitive_score(1f32.next_up())?;
    if req_occur == Occur::MUST {
        assert_equals(&1, &scorer.iterator().next_doc()?);
        assert_equals(&4, &scorer.iterator().next_doc()?);
    }
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    drop(scorer);
    drop(ss);
    drop(weight);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `new Field("foo", new TermFreqTokenStream(term, termFreq), ft)`.
fn term_freq_field(term: &str, term_freq: i32, ft: &FieldType) -> Result<Field> {
    let stream: Rc<RefCell<dyn TokenStream>> =
        Rc::new(RefCell::new(TermFreqTokenStream::new(term, term_freq)?));
    Field::new_with_token_stream("foo", stream, ft.clone())
}

/// `assertEquals(expected, actual, 0)` over floats.
fn assert_equals_float(expected: f32, actual: f32) {
    Assert::assert_equals_float_msg(None, expected, actual, 0.0);
}

/// Port of `TestReqOptSumScorer.testMaxBlock`.
fn test_max_block() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut ft = FieldType::new();
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    ft.set_tokenized(true)?;
    ft.freeze();

    for i in 0..1024 {
        // create documents with an increasing number of As and one B
        let mut doc = Document::new();
        doc.add(Box::new(term_freq_field("a", i + 1, &ft)?));
        if random().next_float() < 0.5 {
            doc.add(Box::new(term_freq_field("b", 1, &ft)?));
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;
    let reader = directory_reader::open(dir.as_directory())?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(SimpleSimilarity));
    // freq == score
    // searcher.setSimilarity(new TestSimilarity.SimpleSimilarity());
    let req_q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("foo", "a")));
    let opt_q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("foo", "b")));
    let mut bool_q = BooleanQueryBuilder::new();
    bool_q.add(Arc::clone(&req_q), Occur::MUST)?;
    bool_q.add(Arc::clone(&opt_q), Occur::SHOULD)?;
    let bool_q: Arc<dyn Query> = Arc::new(bool_q.build());
    let mut actual = req_opt_scorer(&searcher, &req_q, &opt_q, true)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let mut expected = searcher
        .create_weight(bool_q, ScoreMode::COMPLETE, 1.0)?
        .scorer(&leaves[0])?
        .unwrap_or_else(npe);
    actual.set_min_competitive_score(1f32.next_up())?;
    // Checks that all blocks are fully visited
    for i in 0..1024 {
        assert_equals(&i, &actual.iterator().next_doc()?);
        assert_equals(&i, &expected.iterator().next_doc()?);
        assert_equals_float(actual.score()?, expected.score()?);
    }
    drop(actual);
    drop(expected);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestReqOptSumScorer.testMaxScoreSegment`.
fn test_max_score_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let all_values: [&[&str]; 8] = [
        &["A"],      // 0
        &["A"],      // 1
        &[],         // 2
        &["A", "B"], // 3
        &["A"],      // 4
        &["B"],      // 5
        &["A", "B"], // 6
        &["B"],      // 7
    ];
    for values in all_values {
        w.add_document(&string_doc("foo", values)?)?;
    }
    w.force_merge(1)?;
    w.close()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let req_q = cs_term("foo", "A");
    let opt_q = cs_term("foo", "B");
    let mut scorer = req_opt_scorer(&searcher, &req_q, &opt_q, false)?;
    assert_equals(&0, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&1, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&3, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&4, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&6, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut scorer = req_opt_scorer(&searcher, &req_q, &opt_q, false)?;
    scorer.set_min_competitive_score(1f32.next_down())?;
    assert_equals(&0, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&1, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&3, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&4, &scorer.iterator().next_doc()?);
    assert_equals_float(1.0, scorer.score()?);
    assert_equals(&6, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut scorer = req_opt_scorer(&searcher, &req_q, &opt_q, false)?;
    scorer.set_min_competitive_score(1f32.next_up())?;
    assert_equals(&3, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&6, &scorer.iterator().next_doc()?);
    assert_equals_float(2.0, scorer.score()?);
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    let mut scorer = req_opt_scorer(&searcher, &req_q, &opt_q, true)?;
    scorer.set_min_competitive_score(2f32.next_up())?;
    assert_equals(&NO_MORE_DOCS, &scorer.iterator().next_doc()?);

    drop(scorer);
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestReqOptSumScorer.testMustRandomFrequentOpt`.
fn test_must_random_frequent_opt() -> Result<()> {
    do_test_random(Occur::MUST, 0.5)
}

/// Port of `TestReqOptSumScorer.testMustRandomRareOpt`.
fn test_must_random_rare_opt() -> Result<()> {
    do_test_random(Occur::MUST, 0.05)
}

/// Port of `TestReqOptSumScorer.testFilterRandomFrequentOpt`.
fn test_filter_random_frequent_opt() -> Result<()> {
    do_test_random(Occur::FILTER, 0.5)
}

/// Port of `TestReqOptSumScorer.testFilterRandomRareOpt`.
fn test_filter_random_rare_opt() -> Result<()> {
    do_test_random(Occur::FILTER, 0.05)
}

/// `new BooleanQuery.Builder().add(a, occurA).add(b, occurB).build()`.
fn bool2(
    a: Arc<dyn Query>,
    occur_a: Occur,
    b: Arc<dyn Query>,
    occur_b: Occur,
) -> Result<Arc<dyn Query>> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(a, occur_a)?;
    builder.add(b, occur_b)?;
    Ok(Arc::new(builder.build()))
}

/// `new TermQuery(new Term(field, text))`.
fn term_q(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field, text)))
}

/// `new RandomApproximationQuery(query, random())`.
fn random_approx(query: &Arc<dyn Query>) -> Arc<dyn Query> {
    Arc::new(RandomApproximationQuery::new(Arc::clone(query), random()))
}

/// `private void doTestRandom(Occur reqOccur, double optFreq)`.
fn do_test_random(req_occur: Occur, opt_freq: f64) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _i in 0..num_docs {
        let num_as = if random().next_boolean() {
            0
        } else {
            1 + random().next_int_bound(5)
        };
        let num_bs = if random().next_double() < opt_freq {
            0
        } else {
            1 + random().next_int_bound(5)
        };
        let mut doc = Document::new();
        for _j in 0..num_as {
            doc.add(Box::new(StringField::new("f", "A".to_string(), Store::NO)?));
        }
        for _j in 0..num_bs {
            doc.add(Box::new(StringField::new("f", "B".to_string(), Store::NO)?));
        }
        if random().next_boolean() {
            doc.add(Box::new(StringField::new("f", "C".to_string(), Store::NO)?));
        }
        w.add_document(&doc)?;
    }
    let r = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));

    let must_term = term_q("f", "A");
    let should_term = term_q("f", "B");
    let query = bool2(
        Arc::clone(&must_term),
        req_occur,
        Arc::clone(&should_term),
        Occur::SHOULD,
    )?;

    let collector_manager = TopScoreDocCollectorManager::new(10, None, i32::MAX)?;
    let top_docs = searcher.search_with_manager(Arc::clone(&query), &collector_manager)?;
    let expected = top_docs.score_docs;

    // Also test a filtered query, since it does not compute the score on all
    // matches.
    let query = bool2(query, Occur::MUST, term_q("f", "C"), Occur::FILTER)?;

    let collector_manager = TopScoreDocCollectorManager::new(10, None, i32::MAX)?;
    let top_docs = searcher.search_with_manager(Arc::clone(&query), &collector_manager)?;
    let expected_filtered = top_docs.score_docs;

    CheckHits::check_top_scores(&*random(), &query, &searcher)?;

    {
        let q = bool2(
            random_approx(&must_term),
            req_occur,
            Arc::clone(&should_term),
            Occur::SHOULD,
        )?;

        let collector_manager = TopScoreDocCollectorManager::new(10, None, 1)?;
        let top_docs = searcher.search_with_manager(Arc::clone(&q), &collector_manager)?;
        let actual = top_docs.score_docs;
        CheckHits::check_equal(&*query, &expected, &actual);

        let q = bool2(
            Arc::clone(&must_term),
            req_occur,
            random_approx(&should_term),
            Occur::SHOULD,
        )?;
        let collector_manager = TopScoreDocCollectorManager::new(10, None, 1)?;
        let top_docs = searcher.search_with_manager(Arc::clone(&q), &collector_manager)?;
        let actual = top_docs.score_docs;
        CheckHits::check_equal(&*q, &expected, &actual);

        let q = bool2(
            random_approx(&must_term),
            req_occur,
            random_approx(&should_term),
            Occur::SHOULD,
        )?;
        let collector_manager = TopScoreDocCollectorManager::new(10, None, 1)?;
        let top_docs = searcher.search_with_manager(Arc::clone(&q), &collector_manager)?;
        let actual = top_docs.score_docs;
        CheckHits::check_equal(&*q, &expected, &actual);
    }

    let query = {
        let nested_q = bool2(
            Arc::clone(&query),
            Occur::MUST,
            term_q("f", "C"),
            Occur::FILTER,
        )?;
        CheckHits::check_top_scores(&*random(), &nested_q, &searcher)?;

        let query = bool2(
            query,
            Occur::MUST,
            random_approx(&term_q("f", "C")),
            Occur::FILTER,
        )?;

        let collector_manager = TopScoreDocCollectorManager::new(10, None, 1)?;
        let top_docs = searcher.search_with_manager(Arc::clone(&nested_q), &collector_manager)?;
        let actual_filtered = top_docs.score_docs;
        CheckHits::check_equal(&*nested_q, &expected_filtered, &actual_filtered);
        query
    };

    {
        let query = bool2(query, req_occur, term_q("f", "C"), Occur::SHOULD)?;

        CheckHits::check_top_scores(&*random(), &query, &searcher)?;

        let query = bool2(term_q("f", "C"), req_occur, query, Occur::SHOULD)?;

        CheckHits::check_top_scores(&*random(), &query, &searcher)?;
    }

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `ReqOptSumScorer` subclass of `reqOptScorer` whose
/// `getMaxScore(int)` returns `Float.POSITIVE_INFINITY`.
#[derive(Debug)]
struct InfiniteMaxScore;

impl ReqOptSumScorerOverrides for InfiniteMaxScore {
    fn get_max_score(
        &self,
        _up_to: i32,
        _inherited: &mut dyn FnMut(i32) -> Result<f32>,
    ) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}

/// `private static Scorer reqOptScorer(IndexSearcher searcher, Query reqQ, Query optQ, boolean withBlockScore)`.
fn req_opt_scorer(
    searcher: &IndexSearcher,
    req_q: &Arc<dyn Query>,
    opt_q: &Arc<dyn Query>,
    with_block_score: bool,
) -> Result<Box<dyn Scorer>> {
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let req_scorer = searcher
        .create_weight(Arc::clone(req_q), ScoreMode::TOP_SCORES, 1.0)?
        .scorer(&leaves[0])?
        .unwrap_or_else(npe);
    let opt_scorer = searcher
        .create_weight(Arc::clone(opt_q), ScoreMode::TOP_SCORES, 1.0)?
        .scorer(&leaves[0])?
        .unwrap_or_else(npe);
    if with_block_score {
        Ok(Box::new(ReqOptSumScorer::new(
            req_scorer,
            opt_scorer,
            ScoreMode::TOP_SCORES,
        )?))
    } else {
        let mut scorer = ReqOptSumScorer::new(req_scorer, opt_scorer, ScoreMode::TOP_SCORES)?;
        scorer.set_overrides(Arc::new(InfiniteMaxScore));
        Ok(Box::new(scorer))
    }
}

/// `private static class TermFreqTokenStream extends TokenStream`.
struct TermFreqTokenStream {
    source: AttributeSource,
    term: String,
    term_freq: i32,
    term_att: Att,
    term_freq_att: Att,
    finish: bool,
}

impl TermFreqTokenStream {
    fn new(term: &str, term_freq: i32) -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let term_freq_att = Att::add(&mut source, term_freq_id())?;
        Ok(Self {
            source,
            term: term.to_string(),
            term_freq,
            term_att,
            term_freq_att,
            finish: false,
        })
    }
}

impl std::fmt::Debug for TermFreqTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermFreqTokenStream")
            .field("term", &self.term)
            .field("term_freq", &self.term_freq)
            .field("finish", &self.finish)
            .finish_non_exhaustive()
    }
}

impl TokenStream for TermFreqTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.finish {
            return Ok(false);
        }

        self.source.clear_attributes();

        // `termAtt.append(term)` on the cleared term.
        self.term_att.set_term(&self.term);
        self.term_freq_att
            .term_freq_mut()
            .set_term_frequency(self.term_freq);

        self.finish = true;
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.finish = false;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
