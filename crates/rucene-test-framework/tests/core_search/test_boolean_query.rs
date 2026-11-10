//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBooleanQuery.java`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Field, LongPoint, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_reader::MultiReader;
use rucene::index::{IndexReader, LeafReaderContext, Term};
use rucene::search::conjunction_scorer::ConjunctionScorer;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::disjunction_max_scorer::DisjunctionMaxScorer;
use rucene::search::disjunction_sum_scorer::DisjunctionSumScorer;
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::multi_term_query::scoring_boolean_rewrite;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::phrase_scorer::PhraseScorer;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::req_excl_scorer::ReqExclScorer;
use rucene::search::req_opt_sum_scorer::ReqOptSumScorer;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{
    query_to_string, BooleanClause, BooleanQuery, BooleanQueryBuilder, BoostQuery,
    CollectionResult, CollectorManager, LeafCollector, Occur, Query, ScoreDoc, ScoreMode,
    SimpleCollector, SimpleCollectorImpl, NO_MORE_DOCS,
};
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::FixedBitSet;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::index::threaded_indexing_and_searching_test_case::CachedThreadPoolExecutor;
use rucene_test_framework::search::dummy_total_hit_count_collector::DummyTotalHitCountCollector;
use rucene_test_framework::search::fixed_bit_set_collector::FixedBitSetCollector;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{field_mut, field_mut_as, term, tq};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, verbose};

/// `TestBooleanQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBooleanQuery",
        vec![
            test("testEquality", test_equality),
            test(
                "testEqualityDoesNotDependOnOrder",
                test_equality_does_not_depend_on_order,
            ),
            test(
                "testEqualityOnDuplicateShouldClauses",
                test_equality_on_duplicate_should_clauses,
            ),
            test(
                "testEqualityOnDuplicateMustClauses",
                test_equality_on_duplicate_must_clauses,
            ),
            test(
                "testEqualityOnDuplicateFilterClauses",
                test_equality_on_duplicate_filter_clauses,
            ),
            test(
                "testEqualityOnDuplicateMustNotClauses",
                test_equality_on_duplicate_must_not_clauses,
            ),
            test("testHashCodeIsStable", test_hash_code_is_stable),
            test("testTooManyClauses", test_too_many_clauses),
            test("testNullOrSubScorer", test_null_or_sub_scorer),
            test("testDeMorgan", test_de_morgan),
            test(
                "testBS2DisjunctionNextVsAdvance",
                test_bs2_disjunction_next_vs_advance,
            ),
            test("testMinShouldMatchLeniency", test_min_should_match_leniency),
            test(
                "testFILTERClauseBehavesLikeMUST",
                test_filter_clause_behaves_like_must,
            ),
            test(
                "testFilterClauseDoesNotImpactScore",
                test_filter_clause_does_not_impact_score,
            ),
            test(
                "testConjunctionPropagatesApproximations",
                test_conjunction_propagates_approximations,
            ),
            test(
                "testDisjunctionPropagatesApproximations",
                test_disjunction_propagates_approximations,
            ),
            test(
                "testBoostedScorerPropagatesApproximations",
                test_boosted_scorer_propagates_approximations,
            ),
            test(
                "testExclusionPropagatesApproximations",
                test_exclusion_propagates_approximations,
            ),
            test(
                "testReqOptPropagatesApproximations",
                test_req_opt_propagates_approximations,
            ),
            test("testQueryMatchesCount", test_query_matches_count),
            test(
                "testConjunctionMatchesCount",
                test_conjunction_matches_count,
            ),
            test(
                "testDisjunctionMatchesCount",
                test_disjunction_matches_count,
            ),
            test(
                "testTwoClauseTermDisjunctionCountOptimization",
                test_two_clause_term_disjunction_count_optimization,
            ),
            test(
                "testDisjunctionTwoClausesMatchesCountAndScore",
                test_disjunction_two_clauses_matches_count_and_score,
            ),
            test(
                "testDisjunctionRandomClausesMatchesCount",
                test_disjunction_random_clauses_matches_count,
            ),
            test("testProhibitedMatchesCount", test_prohibited_matches_count),
            test(
                "testRandomBooleanQueryMatchesCount",
                test_random_boolean_query_matches_count,
            ),
            test("testToString", test_to_string),
            test("testQueryVisitor", test_query_visitor),
            test("testClauseSetsImmutability", test_clause_sets_immutability),
        ],
    )
}

/// `Occur.values()`, in declaration order.
const OCCUR_VALUES: [Occur; 4] = [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT];

fn bq_of(b: &BooleanQueryBuilder) -> Arc<dyn Query> {
    Arc::new(b.build())
}

fn test_equality() -> Result<()> {
    let mut bq1 = BooleanQueryBuilder::new();
    bq1.add(tq("field", "value1"), Occur::SHOULD)?;
    bq1.add(tq("field", "value2"), Occur::SHOULD)?;
    let mut nested1 = BooleanQueryBuilder::new();
    nested1.add(tq("field", "nestedvalue1"), Occur::SHOULD)?;
    nested1.add(tq("field", "nestedvalue2"), Occur::SHOULD)?;
    bq1.add(bq_of(&nested1), Occur::SHOULD)?;

    let mut bq2 = BooleanQueryBuilder::new();
    bq2.add(tq("field", "value1"), Occur::SHOULD)?;
    bq2.add(tq("field", "value2"), Occur::SHOULD)?;
    let mut nested2 = BooleanQueryBuilder::new();
    nested2.add(tq("field", "nestedvalue1"), Occur::SHOULD)?;
    nested2.add(tq("field", "nestedvalue2"), Occur::SHOULD)?;
    bq2.add(bq_of(&nested2), Occur::SHOULD)?;

    assert_query_equals(&bq_of(&bq1), &bq_of(&bq2));
    Ok(())
}

fn test_equality_does_not_depend_on_order() -> Result<()> {
    let queries: [Arc<dyn Query>; 2] = [tq("foo", "bar"), tq("foo", "baz")];
    for _iter in 0..10 {
        let mut clauses: Vec<BooleanClause> = Vec::new();
        let num_clauses = random().next_int_bound(20);
        for _i in 0..num_clauses {
            let mut query: Arc<dyn Query> =
                Arc::clone(RandomPicks::random_from(&*random(), &queries));
            if random().next_boolean() {
                query = Arc::new(BoostQuery::new(query, random().next_float())?);
            }
            let occur = *RandomPicks::random_from(&*random(), &OCCUR_VALUES);
            clauses.push(BooleanClause::new(query, occur));
        }

        let min_should_match = random().next_int_bound(5);
        let mut bq1_builder = BooleanQueryBuilder::new();
        bq1_builder.set_minimum_number_should_match(min_should_match);
        for clause in &clauses {
            bq1_builder.add_clause(clause.clone())?;
        }
        let bq1 = bq1_builder.build();

        shuffle(&mut clauses, &*random());
        let mut bq2_builder = BooleanQueryBuilder::new();
        bq2_builder.set_minimum_number_should_match(min_should_match);
        for clause in &clauses {
            bq2_builder.add_clause(clause.clone())?;
        }
        let bq2 = bq2_builder.build();

        QueryUtils::check_equal(&bq1, &bq2);
    }
    Ok(())
}

/// `bq1` with one `occur` clause on `foo:bar`, `bq2` with two, both with
/// `bq1`'s random `minimumNumberShouldMatch`.
fn duplicate_clauses(occur: Occur) -> Result<(BooleanQuery, BooleanQuery)> {
    let bq1 = BooleanQueryBuilder::new()
        .set_minimum_number_should_match(random().next_int_bound(2))
        .add(tq("foo", "bar"), occur)?
        .build();
    let bq2 = BooleanQueryBuilder::new()
        .set_minimum_number_should_match(bq1.get_minimum_number_should_match())
        .add(tq("foo", "bar"), occur)?
        .add(tq("foo", "bar"), occur)?
        .build();
    Ok((bq1, bq2))
}

fn test_equality_on_duplicate_should_clauses() -> Result<()> {
    let (bq1, bq2) = duplicate_clauses(Occur::SHOULD)?;
    QueryUtils::check_unequal(&bq1, &bq2);
    Ok(())
}

fn test_equality_on_duplicate_must_clauses() -> Result<()> {
    let (bq1, bq2) = duplicate_clauses(Occur::MUST)?;
    QueryUtils::check_unequal(&bq1, &bq2);
    Ok(())
}

fn test_equality_on_duplicate_filter_clauses() -> Result<()> {
    let (bq1, bq2) = duplicate_clauses(Occur::FILTER)?;
    QueryUtils::check_equal(&bq1, &bq2);
    Ok(())
}

fn test_equality_on_duplicate_must_not_clauses() -> Result<()> {
    let bq1 = BooleanQueryBuilder::new()
        .set_minimum_number_should_match(random().next_int_bound(2))
        .add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?
        .add(tq("foo", "bar"), Occur::FILTER)?
        .build();
    let bq2 = BooleanQueryBuilder::new()
        .set_minimum_number_should_match(bq1.get_minimum_number_should_match())
        .add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?
        .add(tq("foo", "bar"), Occur::FILTER)?
        .add(tq("foo", "bar"), Occur::FILTER)?
        .build();
    QueryUtils::check_equal(&bq1, &bq2);
    Ok(())
}

fn test_hash_code_is_stable() -> Result<()> {
    let bq = BooleanQueryBuilder::new()
        .add(
            tq("foo", &TestUtil::random_simple_string(&*random())),
            Occur::SHOULD,
        )?
        .add(
            tq("foo", &TestUtil::random_simple_string(&*random())),
            Occur::SHOULD,
        )?
        .build();
    let hash_code = bq.query_hash();
    assert_equals(&hash_code, &bq.query_hash());
    Ok(())
}

fn test_too_many_clauses() -> Result<()> {
    // Bad code (such as in a Query.rewrite() impl) should be prevented from creating a BooleanQuery
    // that directly exceeds the maxClauseCount (prior to needing IndexSearcher.rewrite() to do a
    // full walk of the final result)
    let mut bq = BooleanQueryBuilder::new();
    for i in 0..IndexSearcher::get_max_clause_count() {
        bq.add(tq("foo", &format!("bar-{i}")), Occur::SHOULD)?;
    }
    expect_throws(
        "org.apache.lucene.search.IndexSearcher$TooManyClauses",
        || bq.add(tq("foo", "bar-MAX"), Occur::SHOULD).map(|_| ()),
    );
    Ok(())
}

// LUCENE-1630
fn test_null_or_sub_scorer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c d",
        Store::NO,
    )));
    w.add_document(&doc)?;

    let r = w.get_reader()?;
    let mut s = LuceneTestCase::new_searcher(Arc::clone(&r));
    // this test relies upon coord being the default implementation,
    // otherwise scores are different!
    s.set_similarity(Arc::new(ClassicSimilarity::new()));

    let mut q = BooleanQueryBuilder::new();
    q.add(tq("field", "a"), Occur::SHOULD)?;

    // PhraseQuery w/ no terms added returns a null scorer
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &[])?);
    q.add(Arc::clone(&pq), Occur::SHOULD)?;
    assert_equals(&1, &s.search(bq_of(&q), 10)?.total_hits.value());

    // A required clause which returns null scorer should return null scorer to
    // IndexSearcher.
    let mut q = BooleanQueryBuilder::new();
    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &[])?);
    q.add(tq("field", "a"), Occur::SHOULD)?;
    q.add(Arc::clone(&pq), Occur::MUST)?;
    assert_equals(&0, &s.search(bq_of(&q), 10)?.total_hits.value());

    let dmq: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(vec![tq("field", "a"), pq], 1.0)?);
    assert_equals(&1, &s.search(dmq, 10)?.total_hits.value());

    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_de_morgan() -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let iw1 = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir1.as_directory())?;
    let mut doc1 = Document::new();
    doc1.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "foo bar",
        Store::NO,
    )));
    iw1.add_document(&doc1)?;
    let reader1 = iw1.get_reader()?;
    iw1.close()?;

    let mut dir2 = LuceneTestCase::new_directory();
    let iw2 = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir2.as_directory())?;
    let mut doc2 = Document::new();
    doc2.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "foo baz",
        Store::NO,
    )));
    iw2.add_document(&doc2)?;
    let reader2 = iw2.get_reader()?;
    iw2.close()?;

    let mut query = BooleanQueryBuilder::new(); // Query: +foo -ba*
    query.add(tq("field", "foo"), Occur::MUST)?;
    let wildcard_query: Arc<dyn Query> = Arc::new(WildcardQuery::with_rewrite_method(
        term("field", "ba*"),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        scoring_boolean_rewrite(),
    )?);
    query.add(wildcard_query, Occur::MUST_NOT)?;

    let multireader = Arc::new(MultiReader::new(
        vec![
            Arc::clone(&reader1) as Arc<dyn IndexReader>,
            Arc::clone(&reader2) as Arc<dyn IndexReader>,
        ],
        true,
    )?);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&multireader));
    assert_equals(&0, &searcher.search(bq_of(&query), 10)?.total_hits.value());

    let es = Arc::new(CachedThreadPoolExecutor::new("NRT search threads"));
    let searcher = IndexSearcher::with_executor(
        Arc::clone(&multireader) as Arc<dyn IndexReader>,
        Some(Arc::clone(&es) as Arc<dyn rucene::search::Executor>),
    )?;
    if verbose() {
        println!(
            "rewritten form: {}",
            query_to_string(&*searcher.rewrite(bq_of(&query))?)
        );
    }
    assert_equals(&0, &searcher.search(bq_of(&query), 10)?.total_hits.value());
    es.shutdown();
    es.await_termination(Duration::from_secs(1));

    multireader.close()?;
    reader1.close()?;
    reader2.close()?;
    dir1.close()?;
    dir2.close()?;
    Ok(())
}

fn test_bs2_disjunction_next_vs_advance() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let num_docs = LuceneTestCase::at_least(300);
    for _doc_upto in 0..num_docs {
        let mut contents = String::from("a");
        if random().next_int_bound(20) <= 16 {
            contents += " b";
        }
        if random().next_int_bound(20) <= 8 {
            contents += " c";
        }
        if random().next_int_bound(20) <= 4 {
            contents += " d";
        }
        if random().next_int_bound(20) <= 2 {
            contents += " e";
        }
        if random().next_int_bound(20) <= 1 {
            contents += " f";
        }
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new("field", contents, Store::NO)?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let r = w.get_reader()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    w.close()?;

    for iter in 0..10 * *RANDOM_MULTIPLIER {
        if verbose() {
            println!("iter={iter}");
        }
        let mut terms: Vec<&str> = vec!["a", "b", "c", "d", "e", "f"];
        let num_terms = TestUtil::next_int(&*random(), 1, terms.len() as i32) as usize;
        while terms.len() > num_terms {
            terms.remove(random().next_int_bound(terms.len() as i32) as usize);
        }

        if verbose() {
            println!("  terms=[{}]", terms.join(", "));
        }

        let mut q = BooleanQueryBuilder::new();
        for t in &terms {
            q.add_clause(BooleanClause::new(tq("field", t), Occur::SHOULD))?;
        }

        let weight = s.create_weight(s.rewrite(bq_of(&q))?, ScoreMode::COMPLETE, 1.0)?;

        let mut scorer = weight.scorer(&s.get_leaf_contexts()[0])?.expect("scorer");

        // First pass: just use .nextDoc() to gather all hits
        let mut hits: Vec<ScoreDoc> = Vec::new();
        while scorer.iterator().next_doc()? != NO_MORE_DOCS {
            hits.push(ScoreDoc::new(scorer.doc_id(), scorer.score()?));
        }

        if verbose() {
            println!("  {} hits", hits.len());
        }

        // Now, randomly next/advance through the list and
        // verify exact match:
        for iter2 in 0..10 {
            let weight = s.create_weight(s.rewrite(bq_of(&q))?, ScoreMode::COMPLETE, 1.0)?;
            let mut scorer = weight.scorer(&s.get_leaf_contexts()[0])?.expect("scorer");

            if verbose() {
                println!("  iter2={iter2}");
            }

            let mut upto: i32 = -1;
            while upto < hits.len() as i32 {
                let next_upto;
                let next_doc;
                let left = hits.len() as i32 - upto;
                if left == 1 || random().next_boolean() {
                    // next
                    next_upto = 1 + upto;
                    next_doc = scorer.iterator().next_doc()?;
                } else {
                    // advance
                    let inc = TestUtil::next_int(&*random(), 1, left - 1);
                    next_upto = inc + upto;
                    next_doc = scorer.iterator().advance(hits[next_upto as usize].doc)?;
                }

                if next_upto == hits.len() as i32 {
                    assert_equals(&NO_MORE_DOCS, &next_doc);
                } else {
                    let hit = &hits[next_upto as usize];
                    assert_equals(&hit.doc, &next_doc);
                    // Test for precise float equality:
                    let actual = scorer.score()?;
                    Assert::assert_true_msg(
                        &format!(
                            "doc {} has wrong score: expected={} actual={}",
                            hit.doc, hit.score, actual
                        ),
                        hit.score == actual,
                    );
                }
                upto = next_upto;
            }
        }
    }

    r.close()?;
    d.close()?;
    Ok(())
}

fn test_min_should_match_leniency() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c d",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("field", "a"), Occur::SHOULD)?;
    bq.add(tq("field", "b"), Occur::SHOULD)?;

    // No doc can match: BQ has only 2 clauses and we are asking for minShouldMatch=4
    bq.set_minimum_number_should_match(4);
    assert_equals(&0, &s.search(bq_of(&bq), 1)?.total_hits.value());
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn get_matches(searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<FixedBitSet> {
    searcher.search_with_manager(
        query,
        &FixedBitSetCollector::create_manager(searcher.get_index_reader().max_doc()),
    )
}

/// `newTextField(name, value, Store.NO)` as the reused `Field f` of the
/// `setStringValue` tests.
fn text_field(name: &str, value: &str) -> Field {
    LuceneTestCase::new_text_field(name, value, Store::NO)
}

fn test_filter_clause_behaves_like_must() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = text_field("field", "a b c d");
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    field_mut(&mut doc, "field").set_string_value("b d".to_string())?;
    w.add_document(&doc)?;
    field_mut(&mut doc, "field").set_string_value("d".to_string())?;
    w.add_document(&doc)?;
    w.commit()?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let required: [&[&str]; 5] = [&["a", "d"], &["a", "b", "d"], &["d"], &["e"], &[]];
    for required_terms in required {
        let mut bq1 = BooleanQueryBuilder::new();
        let mut bq2 = BooleanQueryBuilder::new();
        for t in required_terms {
            let q = tq("field", t);
            bq1.add(Arc::clone(&q), Occur::MUST)?;
            bq2.add(q, Occur::FILTER)?;
        }

        let matches1 = get_matches(&searcher, bq_of(&bq1))?;
        let matches2 = get_matches(&searcher, bq_of(&bq2))?;
        Assert::assert_true(matches1 == matches2);
    }

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `SimpleCollector` of `assertSameScoresWithoutFilters`.
///
/// Java's collector captures the enclosing `searcher`; this one holds a
/// clone of it, which shares the reader (a collector must be `'static`).
struct SameScoresCollector {
    searcher: IndexSearcher,
    bq2: Arc<dyn Query>,
    matched: Arc<AtomicBool>,
    doc_base: i32,
}

impl LeafCollector for SameScoresCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let actual_score = scorer.score()?;
        let expected_score = self
            .searcher
            .explain(Arc::clone(&self.bq2), self.doc_base + doc)?
            .value()
            .float_value();
        assert_equals_delta(expected_score as f64, actual_score as f64, 10e-5);
        self.matched.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl SimpleCollectorImpl for SameScoresCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }

    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.doc_base = context.doc_base();
        Ok(())
    }
}

/// The anonymous `CollectorManager<SimpleCollector, Void>`.
struct SameScoresManager {
    searcher: IndexSearcher,
    bq2: Arc<dyn Query>,
    matched: Arc<AtomicBool>,
}

impl CollectorManager for SameScoresManager {
    type Collector = SimpleCollector<SameScoresCollector>;
    type Output = ();

    fn new_collector(&self) -> Result<Self::Collector> {
        Ok(SimpleCollector::new(SameScoresCollector {
            searcher: self.searcher.clone(),
            bq2: Arc::clone(&self.bq2),
            matched: Arc::clone(&self.matched),
            doc_base: 0,
        }))
    }

    fn reduce(&self, _collectors: Vec<Self::Collector>) -> Result<()> {
        Ok(())
    }
}

fn assert_same_scores_without_filters(searcher: &IndexSearcher, bq: &BooleanQuery) -> Result<()> {
    let mut bq2_builder = BooleanQueryBuilder::new();
    for c in bq {
        if c.occur() != Occur::FILTER {
            bq2_builder.add_clause(c.clone())?;
        }
    }
    bq2_builder.set_minimum_number_should_match(bq.get_minimum_number_should_match());
    let bq2: Arc<dyn Query> = Arc::new(bq2_builder.build());

    let matched = Arc::new(AtomicBool::new(false));
    searcher.search_with_manager(
        Arc::new(bq.clone()),
        &SameScoresManager {
            searcher: searcher.clone(),
            bq2,
            matched: Arc::clone(&matched),
        },
    )?;

    Assert::assert_true(matched.load(Ordering::SeqCst));
    Ok(())
}

fn test_filter_clause_does_not_impact_score() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = text_field("field", "a b c d");
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    field_mut(&mut doc, "field").set_string_value("b d".to_string())?;
    w.add_document(&doc)?;
    field_mut(&mut doc, "field").set_string_value("a d".to_string())?;
    w.add_document(&doc)?;
    w.commit()?;

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut q_builder = BooleanQueryBuilder::new();
    q_builder.add(tq("field", "a"), Occur::FILTER)?;

    // With a single clause, we will rewrite to the underlying
    // query. Make sure that it returns null scores
    assert_same_scores_without_filters(&searcher, &q_builder.build())?;

    // Now with two clauses, we will get a conjunction scorer
    // Make sure it returns null scores
    q_builder.add(tq("field", "b"), Occur::FILTER)?;
    let q = q_builder.build();
    assert_same_scores_without_filters(&searcher, &q)?;

    // Now with a scoring clause, we need to make sure that
    // the boolean scores are the same as those from the term
    // query
    q_builder.add(tq("field", "c"), Occur::SHOULD)?;
    let q = q_builder.build();
    assert_same_scores_without_filters(&searcher, &q)?;

    // FILTER and empty SHOULD
    let mut q_builder = BooleanQueryBuilder::new();
    q_builder.add(tq("field", "a"), Occur::FILTER)?;
    q_builder.add(tq("field", "e"), Occur::SHOULD)?;
    let q = q_builder.build();
    assert_same_scores_without_filters(&searcher, &q)?;

    // mix of FILTER and MUST
    let mut q_builder = BooleanQueryBuilder::new();
    q_builder.add(tq("field", "a"), Occur::FILTER)?;
    q_builder.add(tq("field", "d"), Occur::MUST)?;
    let q = q_builder.build();
    assert_same_scores_without_filters(&searcher, &q)?;

    // FILTER + minShouldMatch
    let mut q_builder = BooleanQueryBuilder::new();
    q_builder.add(tq("field", "b"), Occur::FILTER)?;
    q_builder.add(tq("field", "a"), Occur::SHOULD)?;
    q_builder.add(tq("field", "d"), Occur::SHOULD)?;
    q_builder.set_minimum_number_should_match(1);
    let q = q_builder.build();
    assert_same_scores_without_filters(&searcher, &q)?;

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The shared body of the `*PropagatesApproximations` tests: one document
/// `"a b c"`, a plain `IndexSearcher` without query cache, the query
/// `pq occur1, field:other occur2`, and the class check `is` on the
/// top-level scorer.
fn propagates_approximations(
    occur1: Occur,
    other: &str,
    occur2: Occur,
    is: fn(&dyn std::any::Any) -> bool,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let f = text_field("field", "a b c");
    doc.add(Box::new(f));
    w.add_document(&doc)?;
    w.commit()?;

    let reader = w.get_reader()?;
    // not LuceneTestCase.newSearcher to not have the asserting wrappers
    // and do instanceof checks
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None); // to still have approximations

    let pq: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["a", "b"])?);

    let mut q = BooleanQueryBuilder::new();
    q.add(pq, occur1)?;
    q.add(tq("field", other), occur2)?;

    let weight = searcher.create_weight(searcher.rewrite(bq_of(&q))?, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    let mut scorer: Box<dyn Scorer> = weight.scorer(&leaves[0])?.expect("scorer");
    Assert::assert_true(scorer.as_any().is_some_and(is));
    Assert::assert_true(scorer.two_phase_iterator().is_some());

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_conjunction_propagates_approximations() -> Result<()> {
    propagates_approximations(Occur::MUST, "c", Occur::FILTER, |a| {
        a.is::<ConjunctionScorer>()
    })
}

fn test_disjunction_propagates_approximations() -> Result<()> {
    // `instanceof DisjunctionScorer`: its two concrete subclasses.
    propagates_approximations(Occur::SHOULD, "c", Occur::SHOULD, |a| {
        a.is::<DisjunctionSumScorer>() || a.is::<DisjunctionMaxScorer>()
    })
}

fn test_boosted_scorer_propagates_approximations() -> Result<()> {
    propagates_approximations(Occur::SHOULD, "d", Occur::SHOULD, |a| {
        a.is::<PhraseScorer>()
    })
}

fn test_exclusion_propagates_approximations() -> Result<()> {
    propagates_approximations(Occur::SHOULD, "c", Occur::MUST_NOT, |a| {
        a.is::<ReqExclScorer>()
    })
}

fn test_req_opt_propagates_approximations() -> Result<()> {
    propagates_approximations(Occur::MUST, "c", Occur::SHOULD, |a| {
        a.is::<ReqOptSumScorer>()
    })
}

// LUCENE-9620 Add Weight#count(LeafReaderContext)
fn test_query_matches_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let random_num_docs = TestUtil::next_int(&*random(), 10, 100);
    let mut num_matching_docs = 0;

    for _i in 0..random_num_docs {
        let mut doc = Document::new();
        let f;
        if random().next_boolean() {
            f = LuceneTestCase::new_text_field(
                "field",
                &format!("a b c {}", random().next_int()),
                Store::NO,
            );
            num_matching_docs += 1;
        } else {
            f = LuceneTestCase::new_text_field(
                "field",
                &random().next_int().to_string(),
                Store::NO,
            );
        }
        doc.add(Box::new(f));
        w.add_document(&doc)?;
    }
    w.commit()?;

    let reader = w.get_reader()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let mut q = BooleanQueryBuilder::new();
    q.add(
        Arc::new(PhraseQuery::from_strings("field", &["a", "b"])?),
        Occur::SHOULD,
    )?;
    q.add(tq("field", "c"), Occur::SHOULD)?;

    let built_query = bq_of(&q);

    assert_equals(&searcher.count(built_query)?, &num_matching_docs);

    // IOUtils.close(reader, w, dir)
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The two-document index of the `*MatchesCount` tests: `long=3,
/// string=abc` and `long=10, string=xyz` (with `long3dim` when `three_dim`).
fn two_doc_index(
    three_dim: bool,
) -> Result<(
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<dyn DirectoryReader>,
    IndexSearcher,
)> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let mut doc = Document::new();
    doc.add(Box::new(LongPoint::new("long", &[3])?));
    if three_dim {
        doc.add(Box::new(LongPoint::new("long3dim", &[3, 4, 5])?));
    }
    doc.add(Box::new(StringField::new(
        "string",
        "abc".into(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    field_mut_as::<LongPoint>(&mut doc, "long").set_long_value(10)?;
    if three_dim {
        field_mut_as::<LongPoint>(&mut doc, "long3dim").set_long_values(&[10, 11, 12])?;
    }
    field_mut_as::<StringField>(&mut doc, "string").set_string_value("xyz".to_string())?;
    writer.add_document(&doc)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    Ok((dir, reader, searcher))
}

/// `searcher.createWeight(query, ScoreMode.COMPLETE, 1f).count(reader.leaves().get(0))`.
fn weight_count(
    searcher: &IndexSearcher,
    reader: &Arc<dyn DirectoryReader>,
    query: Arc<dyn Query>,
) -> Result<i32> {
    let weight = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
    let leaves = (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?;
    weight.count(&leaves[0])
}

fn two(a: Arc<dyn Query>, oa: Occur, b: Arc<dyn Query>, ob: Occur) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(
        BooleanQueryBuilder::new().add(a, oa)?.add(b, ob)?.build(),
    ))
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn test_conjunction_matches_count() -> Result<()> {
    let (mut dir, reader, searcher) = two_doc_index(false)?;

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 3)?,
        Occur::FILTER,
    )?;
    // Both queries match a single doc, BooleanWeight can't figure out the count of the conjunction
    assert_equals(&-1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "missing"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 3)?,
        Occur::FILTER,
    )?;
    // One query has a count of 0, the conjunction has a count of 0 too
    assert_equals(&0, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 5)?,
        Occur::FILTER,
    )?;
    // One query has a count of 0, the conjunction has a count of 0 too
    assert_equals(&0, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_range_query("long", &[0], &[10])?,
        Occur::FILTER,
    )?;
    // One query matches all docs, the count of the conjunction is the count of the other query
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        match_all(),
        Occur::MUST,
        LongPoint::new_range_query("long", &[1], &[5])?,
        Occur::FILTER,
    )?;
    // One query matches all docs, the count of the conjunction is the count of the other query
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_disjunction_matches_count() -> Result<()> {
    let (mut dir, reader, searcher) = two_doc_index(true)?;

    let query = two(
        tq("string", "abc"),
        Occur::SHOULD,
        LongPoint::new_exact_query("long", 3)?,
        Occur::SHOULD,
    )?;
    // Both queries match a single doc, BooleanWeight can't figure out the count of the disjunction
    assert_equals(&-1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "missing"),
        Occur::SHOULD,
        LongPoint::new_exact_query("long", 3)?,
        Occur::SHOULD,
    )?;
    // One query has a count of 0, the disjunction count is the other count
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::SHOULD,
        LongPoint::new_exact_query("long", 5)?,
        Occur::SHOULD,
    )?;
    // One query has a count of 0, the disjunction count is the other count
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::SHOULD,
        LongPoint::new_range_query("long", &[0], &[10])?,
        Occur::SHOULD,
    )?;
    // One query matches all docs, the count of the disjunction is the number of docs
    assert_equals(&2, &weight_count(&searcher, &reader, query)?);

    let query = two(
        match_all(),
        Occur::SHOULD,
        LongPoint::new_range_query("long", &[1], &[5])?,
        Occur::SHOULD,
    )?;
    // One query matches all docs, the count of the disjunction is the number of docs
    assert_equals(&2, &weight_count(&searcher, &reader, query)?);

    let lower: [i64; 3] = [4, 5, 6];
    let upper: [i64; 3] = [9, 10, 11];
    let unknown_count_query = LongPoint::new_range_query("long3dim", &lower, &upper)?;
    // Java `assert`s, enabled under the test runner.
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    Assert::assert_true(leaves.len() == 1);
    Assert::assert_true(weight_count(&searcher, &reader, Arc::clone(&unknown_count_query))? == -1);

    let query: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(tq("string", "xyz"), Occur::MUST)?
            .add(Arc::clone(&unknown_count_query), Occur::MUST_NOT)?
            .add(match_all(), Occur::MUST_NOT)?
            .build(),
    );
    // count of the first MUST_NOT clause is unknown, but the second MUST_NOT clause matches all
    // docs
    assert_equals(&0, &weight_count(&searcher, &reader, query)?);

    let query: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(tq("string", "xyz"), Occur::MUST)?
            .add(Arc::clone(&unknown_count_query), Occur::MUST_NOT)?
            .add(tq("string", "abc"), Occur::MUST_NOT)?
            .build(),
    );
    // count of the first MUST_NOT clause is unknown, though the second MUST_NOT clause matche one
    // doc, we can't figure out the number of
    // docs
    assert_equals(&-1, &weight_count(&searcher, &reader, query)?);

    // test pure disjunction
    let query = two(
        Arc::clone(&unknown_count_query),
        Occur::SHOULD,
        match_all(),
        Occur::SHOULD,
    )?;
    // count of the first SHOULD clause is unknown, but the second SHOULD clause matches all docs
    assert_equals(&2, &weight_count(&searcher, &reader, query)?);

    let query = two(
        unknown_count_query,
        Occur::SHOULD,
        tq("string", "abc"),
        Occur::SHOULD,
    )?;
    // count of the first SHOULD clause is unknown, though the second SHOULD clause matche one doc,
    // we can't figure out the number of
    // docs
    assert_equals(&-1, &weight_count(&searcher, &reader, query)?);

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `IndexSearcher` subclass of
/// `testTwoClauseTermDisjunctionCountOptimization`, counting `count` calls.
#[derive(Debug)]
struct CountingSearcher {
    count_invocations: Arc<AtomicI32>,
}

impl IndexSearcherOverrides for CountingSearcher {
    fn count(&self, searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<i32> {
        self.count_invocations.fetch_add(1, Ordering::SeqCst);
        searcher.super_count(query)
    }
}

/// `new IndexWriter(dir, newIndexWriterConfig().setMergePolicy(newLogMergePolicy()))`
/// indexing one document per entry of `doc_content` (each value a
/// `StringField` of `field`), force-merged to one segment and closed.
fn index_string_docs(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    field: &str,
    doc_content: &[Vec<String>],
) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let result = (|| -> Result<()> {
        for values in doc_content {
            let mut doc = Document::new();
            for value in values {
                doc.add(Box::new(StringField::new(field, value.clone(), Store::NO)?));
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)
    })();
    w.close()?;
    result
}

fn test_two_clause_term_disjunction_count_optimization() -> Result<()> {
    let larger_term_count = RandomNumbers::random_int_between(&*random(), 11, 100);
    let smaller_term_count =
        RandomNumbers::random_int_between(&*random(), 1, (larger_term_count - 1) / 10);

    let mut doc_content: Vec<Vec<String>> =
        Vec::with_capacity((larger_term_count + smaller_term_count) as usize);

    for _i in 0..larger_term_count {
        doc_content.push(vec!["large".to_string()]);
    }

    for _i in 0..smaller_term_count {
        doc_content.push(vec!["small".to_string(), "also small".to_string()]);
    }

    let mut dir = LuceneTestCase::new_directory();
    index_string_docs(&dir, "foo", &doc_content)?;

    let reader = directory_reader::open(dir.as_directory())?;
    let count_invocations = Arc::new(AtomicI32::new(0));
    let mut counting_index_searcher =
        IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    counting_index_searcher.set_overrides(Arc::new(CountingSearcher {
        count_invocations: Arc::clone(&count_invocations),
    }));
    let s = &counting_index_searcher;
    let invocations = || count_invocations.load(Ordering::SeqCst);

    {
        // Test no matches in either term
        count_invocations.store(0, Ordering::SeqCst);
        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "no match"), Occur::SHOULD)?
            .add(tq("foo", "also no match"), Occur::SHOULD)?
            .build();

        assert_equals(&0, &s.count(Arc::new(query))?);
        assert_equals(&3, &invocations());
    }
    {
        // Test match no match in first term
        count_invocations.store(0, Ordering::SeqCst);
        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "no match"), Occur::SHOULD)?
            .add(tq("foo", "small"), Occur::SHOULD)?
            .build();

        assert_equals(&smaller_term_count, &s.count(Arc::new(query))?);
        assert_equals(&3, &invocations());
    }
    {
        // Test match no match in second term
        count_invocations.store(0, Ordering::SeqCst);
        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "small"), Occur::SHOULD)?
            .add(tq("foo", "no match"), Occur::SHOULD)?
            .build();

        assert_equals(&smaller_term_count, &s.count(Arc::new(query))?);
        assert_equals(&3, &invocations());
    }
    {
        // Test match in both terms that hits optimization threshold with small term first
        count_invocations.store(0, Ordering::SeqCst);

        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "small"), Occur::SHOULD)?
            .add(tq("foo", "large"), Occur::SHOULD)?
            .build();

        let count = s.count(Arc::new(query.clone()))?;

        assert_equals(&(larger_term_count + smaller_term_count), &count);
        assert_equals(&4, &invocations());

        Assert::assert_true(query.is_two_clause_pure_disjunction_with_terms());
        let queries = query.rewrite_two_clause_disjunction_with_terms_for_count(s)?;
        assert_equals(&queries.len(), &3);
        assert_equals(&smaller_term_count, &s.count(Arc::clone(&queries[0]))?);
        assert_equals(&larger_term_count, &s.count(Arc::clone(&queries[1]))?);
    }
    {
        // Test match in both terms that hits optimization threshold with large term first
        count_invocations.store(0, Ordering::SeqCst);

        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "large"), Occur::SHOULD)?
            .add(tq("foo", "small"), Occur::SHOULD)?
            .build();

        let count = s.count(Arc::new(query.clone()))?;

        assert_equals(&(larger_term_count + smaller_term_count), &count);
        assert_equals(&4, &invocations());

        Assert::assert_true(query.is_two_clause_pure_disjunction_with_terms());
        let queries = query.rewrite_two_clause_disjunction_with_terms_for_count(s)?;
        assert_equals(&queries.len(), &3);
        assert_equals(&larger_term_count, &s.count(Arc::clone(&queries[0]))?);
        assert_equals(&smaller_term_count, &s.count(Arc::clone(&queries[1]))?);
    }
    {
        // Test match in both terms that doesn't hit optimization threshold
        count_invocations.store(0, Ordering::SeqCst);
        let query = BooleanQueryBuilder::new()
            .add(tq("foo", "small"), Occur::SHOULD)?
            .add(tq("foo", "also small"), Occur::SHOULD)?
            .build();

        let count = s.count(Arc::new(query))?;

        assert_equals(&smaller_term_count, &count);
        assert_equals(&3, &invocations());
    }

    drop(counting_index_searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

// test BlockMaxMaxscoreScorer
fn test_disjunction_two_clauses_matches_count_and_score() -> Result<()> {
    let doc_content: Vec<Vec<String>> = vec![
        strings(&["A", "B"]),      // 0
        strings(&["A"]),           // 1
        strings(&[]),              // 2
        strings(&["A", "B", "C"]), // 3
        strings(&["B"]),           // 4
        strings(&["B", "C"]),      // 5
    ];

    // result sorted by score
    let match_doc_score: [[i32; 2]; 5] = [[0, 2 + 1], [3, 2 + 1], [1, 2], [4, 1], [5, 1]];

    let mut dir = LuceneTestCase::new_directory();
    index_string_docs(&dir, "foo", &doc_content)?;

    let reader = directory_reader::open(dir.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let query: Arc<dyn Query> = Arc::new(
            BooleanQueryBuilder::new()
                .add(
                    Arc::new(BoostQuery::new(
                        Arc::new(ConstantScoreQuery::new(tq("foo", "A"))),
                        2.0,
                    )?),
                    Occur::SHOULD,
                )?
                .add(
                    Arc::new(ConstantScoreQuery::new(tq("foo", "B"))),
                    Occur::SHOULD,
                )?
                .build(),
        );

        let top_docs = searcher.search(query, 10)?;

        for i in 0..top_docs.score_docs.len() {
            let score_doc = &top_docs.score_docs[i];
            assert_equals(&match_doc_score[i][0], &score_doc.doc);
            assert_equals_delta(match_doc_score[i][1] as f64, score_doc.score as f64, 0.0);
        }
        Ok(())
    })();
    reader.close()?;
    dir.close()?;
    result
}

fn test_disjunction_random_clauses_matches_count() -> Result<()> {
    let num_field_value = RandomNumbers::random_int_between(&*random(), 1, 10);
    let mut num_docs_per_field_value = vec![0i32; num_field_value as usize];
    let mut all_docs_count = 0;

    for i in 0..num_docs_per_field_value.len() {
        let num_docs = RandomNumbers::random_int_between(&*random(), 10, 50);
        num_docs_per_field_value[i] = num_docs;
        all_docs_count += num_docs;
    }

    let mut dir = LuceneTestCase::new_directory();
    let mut doc_content: Vec<Vec<String>> = Vec::new();
    for i in 0..num_field_value {
        for _j in 0..num_docs_per_field_value[i as usize] {
            doc_content.push(vec![i.to_string()]);
        }
    }
    index_string_docs(&dir, "field", &doc_content)?;

    let mut matched_docs_count = 0;
    let reader = directory_reader::open(dir.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let mut builder = BooleanQueryBuilder::new();

        for i in 0..num_field_value {
            if RandomizedTest::random_boolean() {
                matched_docs_count += num_docs_per_field_value[i as usize];
                builder.add(tq("field", &i.to_string()), Occur::SHOULD)?;
            }
        }

        let query = bq_of(&builder);

        let top_docs = searcher.search(query, all_docs_count)?;
        assert_equals(&(matched_docs_count as usize), &top_docs.score_docs.len());
        Ok(())
    })();
    reader.close()?;
    dir.close()?;
    result
}

fn test_prohibited_matches_count() -> Result<()> {
    let (mut dir, reader, searcher) = two_doc_index(false)?;

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 3)?,
        Occur::MUST_NOT,
    )?;
    // Both queries match a single doc, BooleanWeight can't figure out the count of the query
    assert_equals(&-1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "missing"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 3)?,
        Occur::MUST_NOT,
    )?;
    // the positive clause doesn't match any docs, so the overall query doesn't either
    assert_equals(&0, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_exact_query("long", 5)?,
        Occur::MUST_NOT,
    )?;
    // the negative clause doesn't match any docs, so the overall count is the count of the positive
    // clause
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    let query = two(
        tq("string", "abc"),
        Occur::MUST,
        LongPoint::new_range_query("long", &[0], &[10])?,
        Occur::MUST_NOT,
    )?;
    // the negative clause matches all docs, so the query doesn't match any docs
    assert_equals(&0, &weight_count(&searcher, &reader, query)?);

    let query = two(
        LongPoint::new_range_query("long", &[0], &[10])?,
        Occur::MUST,
        tq("string", "abc"),
        Occur::MUST_NOT,
    )?;
    // The positive clause matches all docs, so we can subtract the number of matches of the
    // negative clause
    assert_equals(&1, &weight_count(&searcher, &reader, query)?);

    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_random_boolean_query_matches_count() -> Result<()> {
    let (mut dir, reader, searcher) = two_doc_index(false)?;
    for _iter in 0..1000 {
        let num_clauses = TestUtil::next_int(&*random(), 2, 5);
        let mut builder = BooleanQueryBuilder::new();
        let mut num_should_clauses = 0;
        for _i in 0..num_clauses {
            let query: Arc<dyn Query> = match random().next_int_bound(6) {
                0 => tq("string", "abc"),
                1 => LongPoint::new_exact_query("long", 3)?,
                2 => tq("string", "missing"),
                3 => LongPoint::new_exact_query("long", 5)?,
                4 => match_all(),
                _ => LongPoint::new_range_query("long", &[0], &[10])?,
            };
            let occur = *RandomPicks::random_from(&*random(), &OCCUR_VALUES);
            if occur == Occur::SHOULD {
                num_should_clauses += 1;
            }
            builder.add(query, occur)?;
        }
        builder.set_minimum_number_should_match(TestUtil::next_int(
            &*random(),
            0,
            num_should_clauses,
        ));
        let boolean_query = bq_of(&builder);
        assert_equals(
            &(searcher.search_with_manager(
                Arc::clone(&boolean_query),
                &DummyTotalHitCountCollector::create_manager(),
            )? as i32),
            &searcher.count(boolean_query)?,
        );
    }
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_to_string() -> Result<()> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("field", "a"), Occur::SHOULD)?;
    bq.add(tq("field", "b"), Occur::MUST)?;
    bq.add(tq("field", "c"), Occur::MUST_NOT)?;
    bq.add(tq("field", "d"), Occur::FILTER)?;
    assert_equals("a +b -c #d", bq.build().to_query_string("field").as_str());
    Ok(())
}

/// The anonymous `QueryVisitor` of `testQueryVisitor`.
struct ExpectingVisitor<'t> {
    a: &'t Term,
    b: &'t Term,
    c: &'t Term,
    d: &'t Term,
    expected: Option<Term>,
}

impl QueryVisitor for ExpectingVisitor<'_> {
    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        _parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        self.expected = Some(
            match occur {
                Occur::SHOULD => self.a,
                Occur::MUST => self.b,
                Occur::FILTER => self.c,
                Occur::MUST_NOT => self.d,
            }
            .clone(),
        );
        Box::new(self)
    }

    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        assert_equals(&self.expected, &Some(terms[0].clone()));
    }
}

fn test_query_visitor() -> Result<()> {
    let a = term("f", "a");
    let b = term("f", "b");
    let c = term("f", "c");
    let d = term("f", "d");
    let mut bq_builder = BooleanQueryBuilder::new();
    bq_builder.add(
        Arc::new(rucene::search::term_query::TermQuery::new(a.clone())),
        Occur::SHOULD,
    )?;
    bq_builder.add(
        Arc::new(rucene::search::term_query::TermQuery::new(b.clone())),
        Occur::MUST,
    )?;
    bq_builder.add(
        Arc::new(rucene::search::term_query::TermQuery::new(c.clone())),
        Occur::FILTER,
    )?;
    bq_builder.add(
        Arc::new(rucene::search::term_query::TermQuery::new(d.clone())),
        Occur::MUST_NOT,
    )?;
    let bq = bq_builder.build();

    bq.visit(&mut ExpectingVisitor {
        a: &a,
        b: &b,
        c: &c,
        d: &d,
        expected: None,
    });
    Ok(())
}

fn test_clause_sets_immutability() -> Result<()> {
    let a = term("f", "a");
    let b = term("f", "b");
    let c = term("f", "c");
    let d = term("f", "d");
    let tqt = |t: &Term| -> Arc<dyn Query> {
        Arc::new(rucene::search::term_query::TermQuery::new(t.clone()))
    };
    let mut bq_builder = BooleanQueryBuilder::new();
    bq_builder.add(tqt(&a), Occur::SHOULD)?;
    bq_builder.add(tqt(&a), Occur::SHOULD)?;
    bq_builder.add(tqt(&b), Occur::MUST)?;
    bq_builder.add(tqt(&b), Occur::MUST)?;
    bq_builder.add(tqt(&c), Occur::FILTER)?;
    bq_builder.add(tqt(&c), Occur::FILTER)?;
    bq_builder.add(tqt(&d), Occur::MUST_NOT)?;
    bq_builder.add(tqt(&d), Occur::MUST_NOT)?;
    let bq = bq_builder.build();
    // should and must are not deduplicated
    assert_equals(&2, &bq.get_clauses(Occur::SHOULD).len());
    assert_equals(&2, &bq.get_clauses(Occur::MUST).len());
    // filter and must not are deduplicated
    assert_equals(&1, &bq.get_clauses(Occur::FILTER).len());
    assert_equals(&1, &bq.get_clauses(Occur::MUST_NOT).len());
    // check immutability
    // **Divergence from Lucene 10.5.0.** Java: `assertThrows(
    // UnsupportedOperationException.class, () ->
    // bq.getClauses(occur).add(MatchNoDocsQuery.INSTANCE))` and the same on
    // `bq.clauses().add(...)`. Rucene: `get_clauses` answers an owned copy
    // and `clauses` a shared slice, so the mutation either touches a copy
    // or does not compile. Why: Rust enforces the immutability in the
    // types; the test checks the property Java's exception protects — the
    // query's clauses are unchanged after the attempt.
    for occur in OCCUR_VALUES {
        let before = bq.get_clauses(occur).len();
        let mut copy = bq.get_clauses(occur);
        copy.push(Arc::new(MatchNoDocsQuery::instance()));
        assert_equals(&before, &bq.get_clauses(occur).len());
    }
    let before = bq.clauses().len();
    let mut copy: Vec<BooleanClause> = bq.clauses().to_vec();
    copy.push(BooleanClause::new(
        Arc::new(MatchNoDocsQuery::instance()),
        Occur::SHOULD,
    ));
    assert_equals(&before, &bq.clauses().len());
    Ok(())
}
