//! Port of `lucene/core/src/test/org/apache/lucene/search/TestWANDScorer.java`.
//!
//! `WANDScorerQuery.createWeight` builds the `BooleanWeight` of its query in
//! each `scorerSupplier(context)` call, through the searcher it captured; the
//! port's weight holds a clone of that searcher, which shares its state, for
//! the same per-leaf weight construction.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{LeafReaderContext, Term};
use rucene::search::boolean_weight::BooleanWeight;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::{ChildScorable, Scorable};
use rucene::search::scorer::{FilterScorer, Scorer};
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::term_query::TermQuery;
use rucene::search::wand_scorer::{
    scale_max_score, scaling_factor, WANDScorer, FLOAT_MANTISSA_BITS,
};
use rucene::search::weight::{DefaultScorerSupplier, FilterWeight, Weight};
use rucene::search::{
    BooleanQuery, BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, DocIdSetIterator, Occur,
    Query, ScoreMode, NO_MORE_DOCS,
};
use rucene_test_framework::search::asserting_query::AssertingQuery;
use rucene_test_framework::search::block_score_query_wrapper::BlockScoreQueryWrapper;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestWANDScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestWANDScorer",
        vec![
            test("testScalingFactor", test_scaling_factor),
            test("testScaleMaxScore", test_scale_max_score),
            test("testBasics", test_basics),
            test(
                "testBasicsWithDisjunctionAndMinShouldMatch",
                test_basics_with_disjunction_and_min_should_match,
            ),
            test(
                "testBasicsWithDisjunctionAndMinShouldMatchAndTailSizeCondition",
                test_basics_with_disjunction_and_min_should_match_and_tail_size_condition,
            ),
            test(
                "testBasicsWithDisjunctionAndMinShouldMatchAndNonScoringMode",
                test_basics_with_disjunction_and_min_should_match_and_non_scoring_mode,
            ),
            test(
                "testBasicsWithFilteredDisjunctionAndMinShouldMatch",
                test_basics_with_filtered_disjunction_and_min_should_match,
            ),
            test(
                "testBasicsWithFilteredDisjunctionAndMinShouldMatchAndNonScoringMode",
                test_basics_with_filtered_disjunction_and_min_should_match_and_non_scoring_mode,
            ),
            test(
                "testBasicsWithFilteredDisjunctionAndMustNotAndMinShouldMatch",
                test_basics_with_filtered_disjunction_and_must_not_and_min_should_match,
            ),
            test(
                "testBasicsWithFilteredDisjunctionAndMustNotAndMinShouldMatchAndNonScoringMode",
                test_basics_with_filtered_disjunction_and_must_not_and_min_should_match_and_non_scoring_mode,
            ),
            test("testRandom", test_random),
            test("testRandomWithZeroScores", test_random_with_zero_scores),
            test("testRandomWithInfiniteMaxScore", test_random_with_infinite_max_score),
            test("testRandomWithMaxScoreOverflow", test_random_with_max_score_overflow),
        ],
    )
}

/// `Math.scalb(f, scaleFactor)` for a `float`.
fn scalb(f: f32, scale_factor: i32) -> f32 {
    (f64::from(f) * 2f64.powi(scale_factor)) as f32
}

/// Port of `TestWANDScorer.testScalingFactor`.
fn test_scaling_factor() -> Result<()> {
    do_test_scaling_factor(1.0)?;
    do_test_scaling_factor(2.0)?;
    do_test_scaling_factor(1f32.next_down())?;
    do_test_scaling_factor(1f32.next_up())?;
    do_test_scaling_factor(f32::from_bits(1))?;
    do_test_scaling_factor(f32::from_bits(1).next_up())?;
    do_test_scaling_factor(f32::MAX)?;
    do_test_scaling_factor(f32::MAX.next_down())?;
    assert_equals(
        &(scaling_factor(f32::from_bits(1))? + 1),
        &scaling_factor(0.0)?,
    );
    assert_equals(
        &(scaling_factor(f32::MAX)? - 1),
        &scaling_factor(f32::INFINITY)?,
    );

    // Greater scores produce lower scaling factors
    Assert::assert_true(scaling_factor(1f32)? > scaling_factor(10f32)?);
    Assert::assert_true(scaling_factor(f32::MAX)? > scaling_factor(f32::INFINITY)?);
    Assert::assert_true(scaling_factor(0f32)? > scaling_factor(f32::from_bits(1))?);
    Ok(())
}

/// `private void doTestScalingFactor(float f)`.
fn do_test_scaling_factor(f: f32) -> Result<()> {
    let scaling_factor = scaling_factor(f)?;
    let scaled = scalb(f, scaling_factor);
    let scaled_string = rucene::util::java_fmt::float_to_string(scaled);
    Assert::assert_true_msg(
        &scaled_string,
        scaled >= (1i32 << (FLOAT_MANTISSA_BITS - 1)) as f32,
    );
    Assert::assert_true_msg(
        &scaled_string,
        scaled < (1i32 << FLOAT_MANTISSA_BITS) as f32,
    );
    Ok(())
}

/// Port of `TestWANDScorer.testScaleMaxScore`.
fn test_scale_max_score() -> Result<()> {
    assert_equals(
        &(1i64 << (FLOAT_MANTISSA_BITS - 1)),
        &scale_max_score(32f32, scaling_factor(32f32)?),
    );
    assert_equals(
        &1,
        &scale_max_score(32f32, scaling_factor(scalb(1f32, 60))?),
    );
    assert_equals(&1, &scale_max_score(32f32, scaling_factor(f32::INFINITY)?));
    Ok(())
}

/// `private Query maybeWrap(Query query)`.
fn maybe_wrap(query: Arc<dyn Query>) -> Arc<dyn Query> {
    let mut query = query;
    if random().next_boolean() {
        query = Arc::new(BlockScoreQueryWrapper::new(
            query,
            TestUtil::next_int(&*random(), 2, 8),
        ));
        query = Arc::new(AssertingQuery::new(random(), query));
    }
    query
}

/// `new TermQuery(new Term("foo", text))`.
fn tq(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text("foo", text)))
}

/// `new ConstantScoreQuery(new TermQuery(new Term("foo", text)))`.
fn cs(text: &str) -> Arc<dyn Query> {
    Arc::new(ConstantScoreQuery::new(tq(text)))
}

/// `new BoostQuery(query, boost)`.
fn boost(query: Arc<dyn Query>, boost: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(query, boost)?))
}

/// `new BooleanQuery.Builder().add(clause, SHOULD)....setMinimumNumberShouldMatch(msm).build()`.
fn should(clauses: Vec<Arc<dyn Query>>, min_should_match: i32) -> Result<BooleanQuery> {
    let mut builder = BooleanQueryBuilder::new();
    for clause in clauses {
        builder.add(clause, Occur::SHOULD)?;
    }
    builder.set_minimum_number_should_match(min_should_match);
    Ok(builder.build())
}

/// `new BooleanQuery.Builder().add(query, MUST).add(new TermQuery(new Term("foo", text)), occur).build()`.
fn must_with(query: Arc<dyn Query>, text: &str, occur: Occur) -> Result<Arc<dyn Query>> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(query, Occur::MUST)?;
    builder.add(tq(text), occur)?;
    Ok(Arc::new(builder.build()))
}

/// `new WANDScorerQuery(query, doBlocks)`.
fn wand(query: BooleanQuery, do_blocks: bool) -> Arc<dyn Query> {
    Arc::new(WANDScorerQuery::new(query, do_blocks))
}

/// The six documents the basic tests index, with `newLogMergePolicy()` and a
/// final `forceMerge(1)`.
fn index_docs(dir: &TestDirectory, docs: &[&[&str]]) -> Result<()> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let result = (|| -> Result<()> {
        for values in docs {
            let mut doc = Document::new();
            for value in *values {
                doc.add(Box::new(StringField::new(
                    "foo",
                    value.to_string(),
                    Store::NO,
                )?));
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)
    })();
    result.and(w.close())
}

/// `try (Directory dir = newDirectory()) { index; try (IndexReader reader =
/// DirectoryReader.open(dir)) { body(newSearcher(reader)) } }`.
fn with_searcher(docs: &[&[&str]], body: impl FnOnce(&IndexSearcher) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        index_docs(&dir, docs)?;
        let reader = directory_reader::open(dir.as_directory())?;
        let result = {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            body(&searcher)
        };
        result.and(reader.close())
    })();
    result.and(dir.close())
}

/// `searcher.getIndexReader().leaves().get(0)`.
fn leaf0(searcher: &IndexSearcher) -> Result<Arc<LeafReaderContext>> {
    Ok(Arc::clone(searcher.get_index_reader()).leaves()?[0].clone())
}

/// `ss = weight.scorerSupplier(leaf0); ss.setTopLevelScoringClause(); ss.get(Long.MAX_VALUE)`.
fn top_level_scorer(weight: &Arc<dyn Weight>, searcher: &IndexSearcher) -> Result<Box<dyn Scorer>> {
    let ctx = leaf0(searcher)?;
    let mut ss = weight
        .scorer_supplier(&ctx)?
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
    ss.set_top_level_scoring_clause()?;
    ss.get(i64::MAX)
}

/// `weight.scorer(leaf0)`.
fn leaf_scorer(weight: &Arc<dyn Weight>, searcher: &IndexSearcher) -> Result<Box<dyn Scorer>> {
    let ctx = leaf0(searcher)?;
    Ok(weight
        .scorer(&ctx)?
        .unwrap_or_else(|| panic!("java.lang.NullPointerException")))
}

/// `assertEquals(doc, scorer.iterator().nextDoc())`.
fn next_is(scorer: &mut Box<dyn Scorer>, doc: i32) -> Result<()> {
    assert_equals(&doc, &scorer.iterator().next_doc()?);
    Ok(())
}

/// `assertEquals(doc, scorer.iterator().nextDoc()); assertEquals(score, scorer.score(), 0)`.
fn next_scores(scorer: &mut Box<dyn Scorer>, doc: i32, score: f32) -> Result<()> {
    next_is(scorer, doc)?;
    assert_equals_delta(f64::from(score), f64::from(scorer.score()?), 0.0);
    Ok(())
}

/// `searcher.createWeight(searcher.rewrite(query), scoreMode, 1)`.
fn weight_of(
    searcher: &IndexSearcher,
    query: &Arc<dyn Query>,
    score_mode: ScoreMode,
) -> Result<Arc<dyn Weight>> {
    searcher.create_weight(searcher.rewrite(Arc::clone(query))?, score_mode, 1.0)
}

const DOCS_ABC: [&[&str]; 6] = [
    &["A", "B"],      // 0
    &["A"],           // 1
    &[],              // 2
    &["A", "B", "C"], // 3
    &["B"],           // 4
    &["B", "C"],      // 5
];

/// Port of `TestWANDScorer.testBasics`.
fn test_basics() -> Result<()> {
    with_searcher(&DOCS_ABC, |searcher| {
        let query = wand(
            should(vec![boost(cs("A"), 2.0)?, cs("B"), boost(cs("C"), 3.0)?], 0)?,
            random().next_boolean(),
        );

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;
        next_scores(&mut scorer, 1, 2.0)?;
        next_scores(&mut scorer, 3, 2.0 + 1.0 + 3.0)?;
        next_scores(&mut scorer, 4, 1.0)?;
        next_scores(&mut scorer, 5, 1.0 + 3.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;
        scorer.set_min_competitive_score(4.0)?;

        next_scores(&mut scorer, 3, 2.0 + 1.0 + 3.0)?;
        next_scores(&mut scorer, 5, 1.0 + 3.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;

        scorer.set_min_competitive_score(10.0)?;

        next_is(&mut scorer, NO_MORE_DOCS)?;

        //  test a filtered disjunction
        let query = must_with(
            wand(
                should(vec![boost(cs("A"), 2.0)?, cs("B")], 0)?,
                random().next_boolean(),
            ),
            "C",
            Occur::FILTER,
        )?;

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 3, 2.0 + 1.0)?;
        next_scores(&mut scorer, 5, 1.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        scorer.set_min_competitive_score(2.0)?;

        next_scores(&mut scorer, 3, 2.0 + 1.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        // Now test a filtered disjunction with a MUST_NOT
        let query = must_with(
            wand(
                should(vec![boost(cs("A"), 2.0)?, cs("B")], 0)?,
                random().next_boolean(),
            ),
            "C",
            Occur::MUST_NOT,
        )?;

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;
        next_scores(&mut scorer, 1, 2.0)?;
        next_scores(&mut scorer, 4, 1.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        scorer.set_min_competitive_score(3.0)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

/// Port of `TestWANDScorer.testBasicsWithDisjunctionAndMinShouldMatch`.
fn test_basics_with_disjunction_and_min_should_match() -> Result<()> {
    with_searcher(&DOCS_ABC, |searcher| {
        let query = wand(
            should(vec![boost(cs("A"), 2.0)?, cs("B"), boost(cs("C"), 3.0)?], 2)?,
            random().next_boolean(),
        );

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;
        next_scores(&mut scorer, 3, 2.0 + 1.0 + 3.0)?;
        next_scores(&mut scorer, 5, 1.0 + 3.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;
        scorer.set_min_competitive_score(4.0)?;

        next_scores(&mut scorer, 3, 2.0 + 1.0 + 3.0)?;
        next_scores(&mut scorer, 5, 1.0 + 3.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;

        scorer.set_min_competitive_score(10.0)?;

        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

/// Port of `TestWANDScorer.testBasicsWithDisjunctionAndMinShouldMatchAndTailSizeCondition`.
fn test_basics_with_disjunction_and_min_should_match_and_tail_size_condition() -> Result<()> {
    let docs: [&[&str]; 6] = [
        &["A", "B"],      // 0
        &["A"],           // 1
        &[],              // 2
        &["A", "B", "C"], // 3
        // 2 "B"s here and the non constant score term query below forces the
        // tailMaxScore >= minCompetitiveScore && tailSize < minShouldMatch condition
        &["B", "B"], // 4
        &["B", "C"], // 5
    ];
    with_searcher(&docs, |searcher| {
        let query = wand(
            should(vec![tq("A"), tq("B"), tq("C")], 2)?,
            random().next_boolean(),
        );

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_is(&mut scorer, 0)?;
        let score = scorer.score()?;
        scorer.set_min_competitive_score(score)?;

        next_is(&mut scorer, 3)?;
        Ok(())
    })
}

/// Port of `TestWANDScorer.testBasicsWithDisjunctionAndMinShouldMatchAndNonScoringMode`.
fn test_basics_with_disjunction_and_min_should_match_and_non_scoring_mode() -> Result<()> {
    with_searcher(&DOCS_ABC, |searcher| {
        let query = wand(
            should(vec![boost(cs("A"), 2.0)?, cs("B"), boost(cs("C"), 3.0)?], 2)?,
            random().next_boolean(),
        );

        let weight = weight_of(searcher, &query, ScoreMode::COMPLETE_NO_SCORES)?;
        let mut scorer = leaf_scorer(&weight, searcher)?;

        next_is(&mut scorer, 0)?;
        next_is(&mut scorer, 3)?;
        next_is(&mut scorer, 5)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

const DOCS_ABCD: [&[&str]; 6] = [
    &["A", "B"],           // 0
    &["A", "C", "D"],      // 1
    &[],                   // 2
    &["A", "B", "C", "D"], // 3
    &["B"],                // 4
    &["C", "D"],           // 5
];

/// The `WANDScorerQuery` of `A^2 B D^4` with `minShouldMatch == 2`.
fn wand_abd() -> Result<Arc<dyn Query>> {
    Ok(wand(
        should(vec![boost(cs("A"), 2.0)?, cs("B"), boost(cs("D"), 4.0)?], 2)?,
        random().next_boolean(),
    ))
}

/// Port of `TestWANDScorer.testBasicsWithFilteredDisjunctionAndMinShouldMatch`.
fn test_basics_with_filtered_disjunction_and_min_should_match() -> Result<()> {
    with_searcher(&DOCS_ABCD, |searcher| {
        let query = must_with(wand_abd()?, "C", Occur::FILTER)?;

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = top_level_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 1, 2.0 + 4.0)?;
        next_scores(&mut scorer, 3, 2.0 + 1.0 + 4.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        scorer.set_min_competitive_score(2.0 + 1.0 + 4.0)?;

        next_scores(&mut scorer, 3, 2.0 + 1.0 + 4.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

/// Port of `TestWANDScorer.testBasicsWithFilteredDisjunctionAndMinShouldMatchAndNonScoringMode`.
fn test_basics_with_filtered_disjunction_and_min_should_match_and_non_scoring_mode() -> Result<()> {
    with_searcher(&DOCS_ABCD, |searcher| {
        let query = must_with(wand_abd()?, "C", Occur::FILTER)?;

        let weight = weight_of(searcher, &query, ScoreMode::TOP_DOCS)?;
        let mut scorer = leaf_scorer(&weight, searcher)?;

        next_is(&mut scorer, 1)?;
        next_is(&mut scorer, 3)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

const DOCS_ABCD_BD: [&[&str]; 6] = [
    &["A", "B"],           // 0
    &["A", "C", "D"],      // 1
    &[],                   // 2
    &["A", "B", "C", "D"], // 3
    &["B", "D"],           // 4
    &["C", "D"],           // 5
];

/// Port of `TestWANDScorer.testBasicsWithFilteredDisjunctionAndMustNotAndMinShouldMatch`.
fn test_basics_with_filtered_disjunction_and_must_not_and_min_should_match() -> Result<()> {
    with_searcher(&DOCS_ABCD_BD, |searcher| {
        let query = must_with(wand_abd()?, "C", Occur::MUST_NOT)?;

        let weight = weight_of(searcher, &query, ScoreMode::TOP_SCORES)?;
        let mut scorer = leaf_scorer(&weight, searcher)?;

        next_scores(&mut scorer, 0, 2.0 + 1.0)?;
        next_scores(&mut scorer, 4, 1.0 + 4.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;

        let mut scorer = top_level_scorer(&weight, searcher)?;

        scorer.set_min_competitive_score(4.0)?;

        next_scores(&mut scorer, 4, 1.0 + 4.0)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

/// Port of `TestWANDScorer.testBasicsWithFilteredDisjunctionAndMustNotAndMinShouldMatchAndNonScoringMode`.
fn test_basics_with_filtered_disjunction_and_must_not_and_min_should_match_and_non_scoring_mode(
) -> Result<()> {
    with_searcher(&DOCS_ABCD_BD, |searcher| {
        let query = must_with(wand_abd()?, "C", Occur::MUST_NOT)?;

        let weight = weight_of(searcher, &query, ScoreMode::COMPLETE_NO_SCORES)?;
        let mut scorer = leaf_scorer(&weight, searcher)?;

        next_is(&mut scorer, 0)?;
        next_is(&mut scorer, 4)?;
        next_is(&mut scorer, NO_MORE_DOCS)?;
        Ok(())
    })
}

/// The random index of `testRandom`, `testRandomWithZeroScores` and
/// `doTestRandomSpecialMaxScore`, and a searcher without concurrency over it.
fn random_index(body: impl FnOnce(&IndexSearcher, i32) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _i in 0..num_docs {
        let mut doc = Document::new();
        let num_values = random().next_int_bound(1 << random().next_int_bound(5));
        let start = random().next_int_bound(10);
        for j in 0..num_values {
            doc.add(Box::new(StringField::new(
                "foo",
                (start + j).to_string(),
                Store::NO,
            )?));
        }
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    // turn off concurrent search to avoid Random object used across threads resulting into
    // RuntimeException, as WANDScorerQuery#createWeight has reference to this searcher,
    // but will be called during searching
    let searcher = LuceneTestCase::new_searcher_threads(Arc::clone(&reader), true, true, false);

    let result = body(&searcher, num_docs);
    drop(searcher);
    result?;
    reader.close()?;
    dir.close()
}

/// The 100 iterations the random tests share; `clause` builds the clause
/// for the term `start + i`.
fn check_random_queries(
    searcher: &IndexSearcher,
    mut clause: impl FnMut(i32) -> Result<Arc<dyn Query>>,
) -> Result<()> {
    for _iter in 0..100 {
        let start = random().next_int_bound(10);
        let num_clauses = random().next_int_bound(1 << random().next_int_bound(5));
        let mut builder = BooleanQueryBuilder::new();
        for i in 0..num_clauses {
            builder.add(clause(start + i)?, Occur::SHOULD)?;
        }
        let query = wand(builder.build(), random().next_boolean());

        CheckHits::check_top_scores(&*random(), &query, searcher)?;

        let filter_term = random().next_int_bound(30);
        let filtered_query = must_with(query, &filter_term.to_string(), Occur::FILTER)?;

        CheckHits::check_top_scores(&*random(), &filtered_query, searcher)?;
    }
    Ok(())
}

/// Port of `TestWANDScorer.testRandom`.
fn test_random() -> Result<()> {
    random_index(|searcher, _num_docs| {
        check_random_queries(searcher, |term| Ok(maybe_wrap(tq(&term.to_string()))))
    })
}

/// Degenerate case: all clauses produce a score of 0.
///
/// Port of `TestWANDScorer.testRandomWithZeroScores`.
fn test_random_with_zero_scores() -> Result<()> {
    random_index(|searcher, _num_docs| {
        check_random_queries(searcher, |term| {
            Ok(maybe_wrap(boost(cs(&term.to_string()), 0f32)?))
        })
    })
}

/// Test the case when some clauses produce infinite max scores.
///
/// Port of `TestWANDScorer.testRandomWithInfiniteMaxScore`.
fn test_random_with_infinite_max_score() -> Result<()> {
    do_test_random_special_max_score(f32::INFINITY)
}

/// Test the case when some clauses produce finite max scores, but their sum
/// overflows.
///
/// Port of `TestWANDScorer.testRandomWithMaxScoreOverflow`.
fn test_random_with_max_score_overflow() -> Result<()> {
    do_test_random_special_max_score(f32::MAX)
}

/// `private void doTestRandomSpecialMaxScore(float maxScore)`.
fn do_test_random_special_max_score(max_score: f32) -> Result<()> {
    random_index(|searcher, num_docs| {
        check_random_queries(searcher, |term| {
            let mut query = tq(&term.to_string());
            if random().next_boolean() {
                query = Arc::new(MaxScoreWrapperQuery {
                    query,
                    max_range: num_docs / TestUtil::next_int(&*random(), 1, 100),
                    max_score,
                });
            }
            Ok(query)
        })
    })
}

/// `private static class MaxScoreWrapperScorer extends FilterScorer`.
struct MaxScoreWrapperScorer {
    r#in: FilterScorer,
    max_range: i32,
    max_score: f32,
    last_shallow_target: i32,
}

impl MaxScoreWrapperScorer {
    fn new(scorer: Box<dyn Scorer>, max_range: i32, max_score: f32) -> Self {
        MaxScoreWrapperScorer {
            r#in: FilterScorer::new(scorer),
            max_range,
            max_score,
            last_shallow_target: -1,
        }
    }
}

impl std::fmt::Debug for MaxScoreWrapperScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MaxScoreWrapperScorer")
    }
}

impl Scorable for MaxScoreWrapperScorer {
    fn score(&mut self) -> Result<f32> {
        self.r#in.score()
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        self.r#in.smoothing_score(doc_id)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.r#in.set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.r#in.children()
    }
}

impl Scorer for MaxScoreWrapperScorer {
    fn doc_id(&self) -> i32 {
        self.r#in.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.r#in.iterator()
    }

    fn two_phase_iterator(
        &mut self,
    ) -> Option<&mut dyn rucene::search::two_phase_iterator::TwoPhaseIterator> {
        self.r#in.two_phase_iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.last_shallow_target = target;
        self.r#in.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        if up_to - self.doc_id().max(self.last_shallow_target) >= self.max_range {
            return Ok(self.max_score);
        }
        self.r#in.get_max_score(up_to)
    }
}

/// `private static class MaxScoreWrapperQuery extends Query`.
#[derive(Debug, Clone)]
struct MaxScoreWrapperQuery {
    query: Arc<dyn Query>,
    max_range: i32,
    max_score: f32,
}

/// The anonymous `ScorerSupplier` of the `MaxScoreWrapperQuery` weight.
struct MaxScoreWrapperScorerSupplier {
    supplier: Box<dyn ScorerSupplier>,
    max_range: i32,
    max_score: f32,
}

impl ScorerSupplier for MaxScoreWrapperScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        Ok(Box::new(MaxScoreWrapperScorer::new(
            self.supplier.get(lead_cost)?,
            self.max_range,
            self.max_score,
        )))
    }

    fn cost(&self) -> i64 {
        self.supplier.cost()
    }
}

/// The anonymous `FilterWeight` of `MaxScoreWrapperQuery.createWeight`.
#[derive(Debug)]
struct MaxScoreWrapperWeight {
    base: FilterWeight,
    max_range: i32,
    max_score: f32,
}

impl SegmentCacheable for MaxScoreWrapperWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for MaxScoreWrapperWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base.explain(context, doc)
    }

    fn matches(
        &self,
        context: &LeafReaderContext,
        doc: i32,
    ) -> Result<Option<Arc<dyn rucene::search::matches::Matches>>> {
        self.base.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.base.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(supplier) = self.base.inner.scorer_supplier(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(MaxScoreWrapperScorerSupplier {
            supplier,
            max_range: self.max_range,
            max_score: self.max_score,
        })))
    }
}

impl Query for MaxScoreWrapperQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(field)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let that = other
            .as_any()
            .downcast_ref::<MaxScoreWrapperQuery>()
            .expect("same class");
        self.query.query_eq(&*that.query)
            && self.max_range == that.max_range
            && self.max_score == that.max_score
    }

    fn query_hash(&self) -> i32 {
        let mut hash = self.class_hash();
        hash = hash.wrapping_mul(31).wrapping_add(self.query.query_hash());
        hash = hash.wrapping_mul(31).wrapping_add(self.max_range);
        hash.wrapping_mul(31)
            .wrapping_add(self.max_score.to_bits() as i32)
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if let Some(rewritten) = self.query.rewrite(index_searcher)? {
            return Ok(Some(Arc::new(MaxScoreWrapperQuery {
                query: rewritten,
                max_range: self.max_range,
                max_score: self.max_score,
            })));
        }
        Ok(None)
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let weight = self.query.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(MaxScoreWrapperWeight {
            base: FilterWeight::new(weight),
            max_range: self.max_range,
            max_score: self.max_score,
        }))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `private static class WANDScorerQuery extends Query`.
#[derive(Debug, Clone)]
struct WANDScorerQuery {
    query: Arc<BooleanQuery>,
    do_blocks: bool,
}

impl WANDScorerQuery {
    fn new(query: BooleanQuery, do_blocks: bool) -> Self {
        debug_assert!(
            query.clauses().len() == query.get_clauses(Occur::SHOULD).len(),
            "This test utility query is only used to create WANDScorer for disjunctions."
        );
        WANDScorerQuery {
            query: Arc::new(query),
            do_blocks,
        }
    }
}

/// The anonymous `Weight` of `WANDScorerQuery.createWeight`.
#[derive(Debug)]
struct WANDScorerWeight {
    query: Arc<BooleanQuery>,
    do_blocks: bool,
    /// The searcher `createWeight` was called with.
    searcher: IndexSearcher,
    score_mode: ScoreMode,
    boost: f32,
}

impl SegmentCacheable for WANDScorerWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for WANDScorerWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    /// no-ops: Java returns `null`, which its caller dereferences.
    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        panic!("java.lang.NullPointerException")
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let weight = self
            .query
            .create_weight(&self.searcher, self.score_mode, self.boost)?;
        let boolean_weight = weight
            .as_any()
            .and_then(|a| a.downcast_ref::<BooleanWeight>())
            .unwrap_or_else(|| {
                panic!("java.lang.ClassCastException: the weight is not a BooleanWeight")
            });
        let mut optional_scorers: Vec<Box<dyn Scorer>> = Vec::new();
        for wc in boolean_weight.weighted_clauses() {
            if let Some(mut ss) = wc.weight.scorer_supplier(context)? {
                optional_scorers.push(ss.get(i64::MAX)?);
            }
        }
        let scorer: Box<dyn Scorer> = if !optional_scorers.is_empty() {
            Box::new(WANDScorer::new(
                optional_scorers,
                self.query.get_minimum_number_should_match() as usize,
                self.score_mode,
                if self.do_blocks { i64::MAX } else { 0 },
            )?)
        } else {
            match weight.scorer(context)? {
                Some(scorer) => scorer,
                None => return Ok(None),
            }
        };
        Ok(Some(Box::new(DefaultScorerSupplier::new(scorer))))
    }
}

impl Query for WANDScorerQuery {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(WANDScorerWeight {
            query: Arc::clone(&self.query),
            do_blocks: self.do_blocks,
            searcher: searcher.clone(),
            score_mode,
            boost,
        }))
    }

    fn to_query_string(&self, _field: &str) -> String {
        "WANDScorerQuery".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {
        // no-ops
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<WANDScorerQuery>()
                .is_some_and(|o| self.query.query_eq(&*o.query))
    }

    fn query_hash(&self) -> i32 {
        31i32
            .wrapping_mul(self.class_hash())
            .wrapping_add(self.query.query_hash())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
