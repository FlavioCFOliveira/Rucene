//! Port of `lucene/core/src/test/org/apache/lucene/search/TestConstantScoreScorer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::{into_scorer_iterator, Scorer};
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{
    BooleanQueryBuilder, ConstantScoreQuery, ConstantScoreScorer, Occur, Query, ScoreMode,
    ScorerIterator, NO_MORE_DOCS,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, close_reader, tq};
use crate::core_support::{assert_equals, class, random, test};

const FIELD: &str = "f";
const VALUES: [&str; 7] = [
    "foo",
    "bar",
    "foo bar",
    "bar foo",
    "foo not bar",
    "bar foo bar",
    "azerty",
];

/// `TERM_QUERY`.
fn term_query() -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    b.add(tq(FIELD, "foo"), Occur::MUST)?;
    b.add(tq(FIELD, "bar"), Occur::MUST)?;
    Ok(Arc::new(b.build()))
}

/// `PHRASE_QUERY`.
fn phrase_query() -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(0, FIELD, &["foo", "bar"])?))
}

/// `TestConstantScoreScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestConstantScoreScorer",
        vec![
            test("testMatching_ScoreMode_COMPLETE", || {
                test_matching(ScoreMode::COMPLETE)
            }),
            test("testMatching_ScoreMode_COMPLETE_NO_SCORES", || {
                test_matching(ScoreMode::COMPLETE_NO_SCORES)
            }),
            test(
                "testMatching_ScoreMode_TOP_SCORES",
                test_matching_top_scores,
            ),
            test("testTwoPhaseMatching_ScoreMode_COMPLETE", || {
                test_two_phase_matching(ScoreMode::COMPLETE)
            }),
            test("testTwoPhaseMatching_ScoreMode_COMPLETE_NO_SCORES", || {
                test_two_phase_matching(ScoreMode::COMPLETE_NO_SCORES)
            }),
            test(
                "testTwoPhaseMatching_ScoreMode_TOP_SCORES",
                test_two_phase_matching_top_scores,
            ),
            test("testEarlyTermination", test_early_termination),
        ],
    )
}

/// `assertEquals(1f, scorer.score(), 0)`.
fn assert_score_one(scorer: &mut ConstantScoreScorer) -> Result<()> {
    Assert::assert_equals_float_msg(None, 1.0, scorer.score()?, 0.0);
    Ok(())
}

/// The opening steps every matching test shares: the first doc, its score,
/// and the effect of `setMinCompetitiveScore(2f)` on the current doc.
fn first_doc_then_min_competitive(scorer: &mut ConstantScoreScorer) -> Result<i32> {
    let doc = scorer.iterator().next_doc()?;
    assert_equals(&2, &doc);
    assert_score_one(scorer)?;

    // Lower bound of the scores is 1f, so setting minCompetitiveScore higher
    // than 1f should not stop the iterator from the current doc
    scorer.set_min_competitive_score(2.0)?;
    assert_equals(&doc, &Scorer::doc_id(scorer));
    assert_equals(&doc, &scorer.iterator().doc_id());
    assert_score_one(scorer)?;
    Ok(doc)
}

fn test_matching(score_mode: ScoreMode) -> Result<()> {
    let mut index = TestConstantScoreScorerIndex::new()?;
    let result = (|| -> Result<()> {
        let mut scorer = index.constant_score_scorer(term_query()?, 1.0, score_mode)?;

        // "foo bar" match
        first_doc_then_min_competitive(&mut scorer)?;

        // "bar foo" match
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&3, &doc);
        assert_score_one(&mut scorer)?;

        // "foo not bar" match
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&4, &doc);
        assert_score_one(&mut scorer)?;

        // "foo bar foo" match
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&5, &doc);
        assert_score_one(&mut scorer)?;

        let doc = scorer.iterator().next_doc()?;
        assert_equals(&NO_MORE_DOCS, &doc);
        Ok(())
    })();
    result.and(index.close())
}

fn test_matching_top_scores() -> Result<()> {
    let mut index = TestConstantScoreScorerIndex::new()?;
    let result = (|| -> Result<()> {
        let mut scorer = index.constant_score_scorer(term_query()?, 1.0, ScoreMode::TOP_SCORES)?;

        // "foo bar" match
        first_doc_then_min_competitive(&mut scorer)?;

        // Iterator was already set to empty by setMinCompetitiveScore, so
        // next doc will be NO_MORE_DOCS
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&NO_MORE_DOCS, &doc);
        Ok(())
    })();
    result.and(index.close())
}

fn test_two_phase_matching(score_mode: ScoreMode) -> Result<()> {
    let mut index = TestConstantScoreScorerIndex::new()?;
    let result = (|| -> Result<()> {
        let mut scorer = index.constant_score_scorer(phrase_query()?, 1.0, score_mode)?;

        // "foo bar" match
        first_doc_then_min_competitive(&mut scorer)?;

        // "foo not bar" will match the approximation but not the two phase
        // iterator

        // "foo bar foo" match
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&5, &doc);
        assert_score_one(&mut scorer)?;

        let doc = scorer.iterator().next_doc()?;
        assert_equals(&NO_MORE_DOCS, &doc);
        Ok(())
    })();
    result.and(index.close())
}

fn test_two_phase_matching_top_scores() -> Result<()> {
    let mut index = TestConstantScoreScorerIndex::new()?;
    let result = (|| -> Result<()> {
        let mut scorer =
            index.constant_score_scorer(phrase_query()?, 1.0, ScoreMode::TOP_SCORES)?;

        // "foo bar" match
        first_doc_then_min_competitive(&mut scorer)?;

        // Iterator was already set to empty by setMinCompetitiveScore, so
        // next doc will be NO_MORE_DOCS
        let doc = scorer.iterator().next_doc()?;
        assert_equals(&NO_MORE_DOCS, &doc);
        Ok(())
    })();
    result.and(index.close())
}

/// `static class TestConstantScoreScorerIndex implements AutoCloseable`.
struct TestConstantScoreScorerIndex {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
}

impl TestConstantScoreScorerIndex {
    fn new() -> Result<Self> {
        let directory = LuceneTestCase::new_directory();

        let mut config = LuceneTestCase::new_index_writer_config();
        config.set_merge_policy(LuceneTestCase::new_log_merge_policy_use_cfs(
            random().next_boolean(),
        ))?;
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

        for value in VALUES {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                FIELD,
                value,
                Store::YES,
            )));
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;

        let reader = writer.get_reader()?;
        writer.close()?;
        Ok(TestConstantScoreScorerIndex {
            directory: Some(directory),
            reader: Some(reader),
        })
    }

    fn constant_score_scorer(
        &self,
        query: Arc<dyn Query>,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<ConstantScoreScorer> {
        let searcher =
            LuceneTestCase::new_searcher(Arc::clone(self.reader.as_ref().expect("open")));
        let weight =
            searcher.create_weight(Arc::new(ConstantScoreQuery::new(query)), score_mode, 1.0)?;
        let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
        assert_equals(&1, &leaves.len());

        let context = &leaves[0];
        let scorer = weight.scorer(context)?.expect("matches");

        // `scorer.twoPhaseIterator() == null ? scorer.iterator() :
        // scorer.twoPhaseIterator()`: the scorer's own iteration, which the
        // new scorer shares.
        Ok(match into_scorer_iterator(scorer) {
            ScorerIterator::Simple(iterator) => {
                ConstantScoreScorer::from_iterator(score, score_mode, iterator)
            }
            ScorerIterator::TwoPhase(two_phase) => {
                ConstantScoreScorer::from_two_phase(score, score_mode, two_phase)
            }
        })
    }

    fn close(&mut self) -> Result<()> {
        close_reader(self.reader.take())?;
        close_dir(self.directory.take())
    }
}

fn test_early_termination() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut dir = LuceneTestCase::new_directory();
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    config.set_max_buffered_docs(2)?;
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let iw = IndexWriter::new(dir.as_directory(), config)?;
    let num_docs = 50;
    for i in 0..num_docs {
        let mut doc = Document::new();
        let f = LuceneTestCase::new_text_field(
            "key",
            if i % 2 == 0 { "foo bar" } else { "baz" },
            Store::YES,
        );
        doc.add(Box::new(f));
        iw.add_document(&doc)?;
    }
    let ir = directory_reader::open_with_writer(&*iw)?;

    let is = LuceneTestCase::new_searcher_threads(Arc::clone(&ir), true, true, false);

    let c = TopScoreDocCollectorManager::with_num_hits(10, 10)?;
    let top_docs =
        is.search_with_manager(Arc::new(ConstantScoreQuery::new(tq("key", "foo"))), &c)?;
    assert_equals(&11, &top_docs.total_hits.value());
    assert_equals(
        &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
        &top_docs.total_hits.relation(),
    );

    let c = TopScoreDocCollectorManager::with_num_hits(10, 10)?;
    let mut b = BooleanQueryBuilder::new();
    b.add(
        Arc::new(ConstantScoreQuery::new(tq("key", "foo"))),
        Occur::SHOULD,
    )?;
    b.add(
        Arc::new(ConstantScoreQuery::new(tq("key", "bar"))),
        Occur::FILTER,
    )?;
    let query: Arc<dyn Query> = Arc::new(b.build());
    let top_docs = is.search_with_manager(query, &c)?;
    assert_equals(&11, &top_docs.total_hits.value());
    assert_equals(
        &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
        &top_docs.total_hits.relation(),
    );

    iw.close()?;
    drop(is);
    ir.close()?;
    dir.close()?;
    Ok(())
}
