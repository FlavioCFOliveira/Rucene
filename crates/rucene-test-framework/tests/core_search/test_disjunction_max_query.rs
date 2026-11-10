//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDisjunctionMaxQuery.java`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{Analyzer, StringCharReader};
use rucene::document::{Document, FieldType, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::{
    tfidf_scorer, CollectionStatistics, Explanation, SimScorer, Similarity, TFIDFSimilarity,
    TermStatistics,
};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    query_to_string, BooleanQueryBuilder, BoostQuery, DisjunctionMaxQuery, MatchNoDocsQuery, Occur,
    Query, RegexpQuery, ScoreDoc, ScoreMode, WildcardQuery, NO_MORE_DOCS,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    fixture_class_annotated, ftest, object_to_string, set_up, tear_down, term,
};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{assert_equals, assert_equals_msg, random, try_catch, unchecked};

/// threshold for comparing floats
pub const SCORE_COMP_THRESH: f32 = 0.0000;

/// Similarity to eliminate tf, idf and lengthNorm effects to isolate test
/// case.
///
/// same as TestRankingSimilarity in TestRanking.zip from
/// http://issues.apache.org/jira/browse/LUCENE-323
///
/// (`private static class TestSimilarity extends ClassicSimilarity`.)
#[derive(Debug, Default)]
struct TestSimilarity;

impl Similarity for TestSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestDisjunctionMaxQuery$TestSimilarity"
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for TestSimilarity {
    fn tf(&self, freq: f32) -> f32 {
        if freq > 0.0 {
            1.0
        } else {
            0.0
        }
    }

    fn length_norm(&self, _length: i32) -> f32 {
        // Disable length norm
        1.0
    }

    fn idf(&self, _doc_freq: i64, _doc_count: i64) -> f32 {
        1.0
    }

    /// `ClassicSimilarity.idfExplain`, inherited.
    fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf, computed as log((docCount+1)/(docFreq+1)) + 1 from:",
            vec![
                Explanation::matched(df, "docFreq, number of documents containing term", vec![]),
                Explanation::matched(
                    doc_count,
                    "docCount, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }
}

/// `private static final FieldType nonAnalyzedType` and its `static` block.
static NON_ANALYZED_TYPE: LazyLock<FieldType> = LazyLock::new(|| {
    let mut t = FieldType::new_from(TextField::type_stored());
    t.set_tokenized(false)
        .expect("a new FieldType is not frozen");
    t
});

/// The instance of `TestDisjunctionMaxQuery`.
pub struct DisjunctionMax {
    sim: Arc<dyn Similarity>,
    index: Option<TestDirectory>,
    r: Option<Arc<dyn LeafReader>>,
    s: Option<IndexSearcher>,
}

impl Default for DisjunctionMax {
    fn default() -> Self {
        DisjunctionMax {
            sim: Arc::new(TestSimilarity),
            index: None,
            r: None,
            s: None,
        }
    }
}

impl DisjunctionMax {
    fn s(&self) -> &IndexSearcher {
        self.s.as_ref().expect("setUp ran")
    }

    fn r(&self) -> &Arc<dyn LeafReader> {
        self.r.as_ref().expect("setUp ran")
    }
}

/// `TestDisjunctionMaxQuery` (`@LuceneTestCase.SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    fixture_class_annotated::<DisjunctionMax>(
        "org.apache.lucene.search.TestDisjunctionMaxQuery",
        vec![
            set_up(dmq_set_up),
            tear_down(|f: &mut DisjunctionMax| {
                drop(f.s.take());
                if let Some(r) = f.r.take() {
                    r.as_index_reader().close()?;
                }
                if let Some(mut index) = f.index.take() {
                    index.close()?;
                }
                Ok(())
            }),
            ftest("testSkipToFirsttimeMiss", test_skip_to_firsttime_miss),
            ftest("testSkipToFirsttimeHit", test_skip_to_firsttime_hit),
            ftest("testSimpleEqualScores1", test_simple_equal_scores1),
            ftest("testSimpleEqualScores2", test_simple_equal_scores2),
            ftest("testSimpleEqualScores3", test_simple_equal_scores3),
            ftest("testSimpleTiebreaker", test_simple_tiebreaker),
            ftest(
                "testBooleanRequiredEqualScores",
                test_boolean_required_equal_scores,
            ),
            ftest(
                "testBooleanOptionalNoTiebreaker",
                test_boolean_optional_no_tiebreaker,
            ),
            ftest(
                "testBooleanOptionalWithTiebreaker",
                test_boolean_optional_with_tiebreaker,
            ),
            ftest(
                "testBooleanOptionalWithTiebreakerAndBoost",
                test_boolean_optional_with_tiebreaker_and_boost,
            ),
            ftest("testRewriteBoolean", test_rewrite_boolean),
            ftest("testRewriteEmpty", test_rewrite_empty),
            ftest("testRewriteAllMatchNoDocs", test_rewrite_all_match_no_docs),
            ftest("testRewriteSingleSurvivor", test_rewrite_single_survivor),
            ftest(
                "testRewriteFilterMatchNoDocs",
                test_rewrite_filter_match_no_docs,
            ),
            ftest(
                "testRewriteFilterMatchNoDocsWithTieBreaker",
                test_rewrite_filter_match_no_docs_with_tie_breaker,
            ),
            ftest("testDisjunctOrderAndEquals", test_disjunct_order_and_equals),
            ftest(
                "testCasesWhenDisjunctOrderMatters",
                test_cases_when_disjunct_order_matters,
            ),
            ftest("testRandomTopDocs", test_random_top_docs),
            ftest("testExplainMatch", test_explain_match),
            ftest("testExplainNoMatch", test_explain_no_match),
            ftest(
                "testExplainMatch_OneNonMatchingSubQuery_NotIncludedInExplanation",
                test_explain_match_one_non_matching_sub_query_not_included_in_explanation,
            ),
            ftest("testGenerics", test_generics),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

fn dmq_set_up(f: &mut DisjunctionMax) -> Result<()> {
    let index = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(analyzer);
    config.set_similarity(Arc::clone(&f.sim))?;
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), index.as_directory(), config)?;

    // hed is the most important field, dek is secondary

    // d1 is an "ok" match for: albino elephant
    {
        let mut d1 = Document::new();
        d1.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d1",
            &NON_ANALYZED_TYPE,
        ))); // Field.Keyword("id",
             // "d1"));
        d1.add(Box::new(LuceneTestCase::new_text_field(
            "hed",
            "elephant",
            Store::YES,
        ))); // Field.Text("hed", "elephant"));
        d1.add(Box::new(LuceneTestCase::new_text_field(
            "dek",
            "elephant",
            Store::YES,
        ))); // Field.Text("dek", "elephant"));
        writer.add_document(&d1)?;
    }

    // d2 is a "good" match for: albino elephant
    {
        let mut d2 = Document::new();
        d2.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d2",
            &NON_ANALYZED_TYPE,
        ))); // Field.Keyword("id",
             // "d2"));
        d2.add(Box::new(LuceneTestCase::new_text_field(
            "hed",
            "elephant",
            Store::YES,
        ))); // Field.Text("hed", "elephant"));
        d2.add(Box::new(LuceneTestCase::new_text_field(
            "dek",
            "albino",
            Store::YES,
        ))); // Field.Text("dek",
             // "albino"));
        d2.add(Box::new(LuceneTestCase::new_text_field(
            "dek",
            "elephant",
            Store::YES,
        ))); // Field.Text("dek", "elephant"));
        writer.add_document(&d2)?;
    }

    // d3 is a "better" match for: albino elephant
    {
        let mut d3 = Document::new();
        d3.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d3",
            &NON_ANALYZED_TYPE,
        ))); // Field.Keyword("id",
             // "d3"));
        d3.add(Box::new(LuceneTestCase::new_text_field(
            "hed",
            "albino",
            Store::YES,
        ))); // Field.Text("hed",
             // "albino"));
        d3.add(Box::new(LuceneTestCase::new_text_field(
            "hed",
            "elephant",
            Store::YES,
        ))); // Field.Text("hed", "elephant"));
        writer.add_document(&d3)?;
    }

    // d4 is the "best" match for: albino elephant
    {
        let mut d4 = Document::new();
        d4.add(Box::new(LuceneTestCase::new_field(
            "id",
            "d4",
            &NON_ANALYZED_TYPE,
        ))); // Field.Keyword("id",
             // "d4"));
        d4.add(Box::new(LuceneTestCase::new_text_field(
            "hed",
            "albino",
            Store::YES,
        ))); // Field.Text("hed",
             // "albino"));
        d4.add(Box::new(LuceneTestCase::new_field(
            "hed",
            "elephant",
            &NON_ANALYZED_TYPE,
        ))); // Field.Text("hed", "elephant"));
        d4.add(Box::new(LuceneTestCase::new_text_field(
            "dek",
            "albino",
            Store::YES,
        ))); // Field.Text("dek",
             // "albino"));
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

/// `protected TermQuery tq(String f, String t)` (macro).
fn tq(f: &str, t: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(f, t)))
}

/// `protected BoostQuery tq(String f, String t, float b)` (macro).
fn tqb(f: &str, t: &str, b: f32) -> Result<Arc<dyn Query>> {
    let q = tq(f, t);
    Ok(Arc::new(BoostQuery::new(q, b)?))
}

/// `new DisjunctionMaxQuery(disjuncts, tieBreakerMultiplier)`.
fn dmq(disjuncts: Vec<Arc<dyn Query>>, tie_breaker: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(DisjunctionMaxQuery::new(disjuncts, tie_breaker)?))
}

/// `QueryUtils.check(random(), q, s)`.
fn check(q: &Arc<dyn Query>, s: &IndexSearcher) {
    QueryUtils::check_searcher(&*random(), q, Some(s));
}

/// `searcher.storedFields().document(doc).get("id")`.
fn id_of(s: &IndexSearcher, doc: i32) -> Option<String> {
    unchecked(unchecked(s.stored_fields()).document(doc)).get("id")
}

/// `try { body } catch (Error e) { printHits(test, h, s); throw e; }`.
fn or_print_hits(test: &str, h: &[ScoreDoc], s: &IndexSearcher, body: impl FnOnce()) -> Result<()> {
    if let Err(e) = try_catch("java.lang.Error", body) {
        print_hits(test, h, s)?;
        e.throw()
    }
    Ok(())
}

fn test_skip_to_firsttime_miss(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let dq = dmq(vec![tq("id", "d1"), tq("dek", "DOES_NOT_EXIST")], 0.0)?;

    check(&dq, s);
    Assert::assert_true(Arc::clone(s.get_top_reader_context()).as_leaf().is_some());
    let dw = s.create_weight(s.rewrite(dq)?, ScoreMode::COMPLETE, 1.0)?;
    let context = Arc::clone(s.get_top_reader_context())
        .as_leaf()
        .expect("ClassCastException");
    let mut ds = dw.scorer(&context)?.expect("NullPointerException");
    let skip_ok = ds.iterator().advance(3)? != NO_MORE_DOCS;
    if skip_ok {
        Assert::fail(&format!(
            "firsttime skipTo found a match? ... {}",
            f.r()
                .stored_fields()?
                .document(ds.doc_id())?
                .get("id")
                .unwrap_or_else(|| "null".into())
        ));
    }
    Ok(())
}

fn test_skip_to_firsttime_hit(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let dq = dmq(vec![tq("dek", "albino"), tq("dek", "DOES_NOT_EXIST")], 0.0)?;

    Assert::assert_true(Arc::clone(s.get_top_reader_context()).as_leaf().is_some());
    check(&dq, s);
    let dw = s.create_weight(s.rewrite(dq)?, ScoreMode::COMPLETE, 1.0)?;
    let context = Arc::clone(s.get_top_reader_context())
        .as_leaf()
        .expect("ClassCastException");
    let mut ds = dw.scorer(&context)?.expect("NullPointerException");
    Assert::assert_true_msg(
        "firsttime skipTo found no match",
        ds.iterator().advance(3)? != NO_MORE_DOCS,
    );
    assert_equals_msg(
        Some("found wrong docid"),
        &Some("d4".to_string()),
        &f.r().stored_fields()?.document(ds.doc_id())?.get("id"),
    );
    Ok(())
}

/// The scores of `h` are all equal to the first one.
fn assert_all_scores_equal(h: &[ScoreDoc], upto: usize) {
    let score = h[0].score;
    for i in 1..upto {
        Assert::assert_equals_float_msg(
            Some(&format!("score #{i} is not the same")),
            score,
            h[i].score,
            SCORE_COMP_THRESH,
        );
    }
}

fn test_simple_equal_scores1(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let q = dmq(vec![tq("hed", "albino"), tq("hed", "elephant")], 0.0)?;
    check(&q, s);

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    or_print_hits("testSimpleEqualScores1", &h, s, || {
        assert_equals_msg(
            Some(&format!("all docs should match {}", query_to_string(&*q))),
            &4,
            &h.len(),
        );

        assert_all_scores_equal(&h, h.len());
    })
}

fn test_simple_equal_scores2(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let q = dmq(vec![tq("dek", "albino"), tq("dek", "elephant")], 0.0)?;
    check(&q, s);

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    or_print_hits("testSimpleEqualScores2", &h, s, || {
        assert_equals_msg(
            Some(&format!("3 docs should match {}", query_to_string(&*q))),
            &3,
            &h.len(),
        );
        assert_all_scores_equal(&h, h.len());
    })
}

fn test_simple_equal_scores3(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let q = dmq(
        vec![
            tq("hed", "albino"),
            tq("hed", "elephant"),
            tq("dek", "albino"),
            tq("dek", "elephant"),
        ],
        0.0,
    )?;
    check(&q, s);

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    or_print_hits("testSimpleEqualScores3", &h, s, || {
        assert_equals_msg(
            Some(&format!("all docs should match {}", query_to_string(&*q))),
            &4,
            &h.len(),
        );
        assert_all_scores_equal(&h, h.len());
    })
}

fn test_simple_tiebreaker(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let q = dmq(vec![tq("dek", "albino"), tq("dek", "elephant")], 0.01)?;
    check(&q, s);

    let h = s.search(Arc::clone(&q), 1000)?.score_docs;

    or_print_hits("testSimpleTiebreaker", &h, s, || {
        assert_equals_msg(
            Some(&format!("3 docs should match {}", query_to_string(&*q))),
            &3,
            &h.len(),
        );
        assert_equals_msg(
            Some("wrong first"),
            &Some("d2".to_string()),
            &id_of(s, h[0].doc),
        );
        let score0 = h[0].score;
        let score1 = h[1].score;
        let score2 = h[2].score;
        Assert::assert_true_msg(
            &format!("d2 does not have better score then others: {score0} >? {score1}"),
            score0 > score1,
        );
        Assert::assert_equals_float_msg(
            Some("d4 and d1 don't have equal scores"),
            score1,
            score2,
            SCORE_COMP_THRESH,
        );
    })
}

/// `"… " + q.toString()` of a `BooleanQuery.Builder`, which does not
/// override `Object.toString()`.
fn builder_to_string(q: &BooleanQueryBuilder) -> String {
    object_to_string("org.apache.lucene.search.BooleanQuery$Builder", q)
}

fn test_boolean_required_equal_scores(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let mut q = BooleanQueryBuilder::new();
    {
        let q1 = dmq(vec![tq("hed", "albino"), tq("dek", "albino")], 0.0)?;
        q.add(Arc::clone(&q1), Occur::MUST)?; // true,false);
        check(&q1, s);
    }
    {
        let q2 = dmq(vec![tq("hed", "elephant"), tq("dek", "elephant")], 0.0)?;
        q.add(Arc::clone(&q2), Occur::MUST)?; // true,false);
        check(&q2, s);
    }

    check(&(Arc::new(q.build()) as Arc<dyn Query>), s);

    let h = s.search(Arc::new(q.build()), 1000)?.score_docs;

    or_print_hits("testBooleanRequiredEqualScores1", &h, s, || {
        assert_equals_msg(
            Some(&format!("3 docs should match {}", builder_to_string(&q))),
            &3,
            &h.len(),
        );
        assert_all_scores_equal(&h, h.len());
    })
}

fn test_boolean_optional_no_tiebreaker(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let mut q = BooleanQueryBuilder::new();
    {
        let q1 = dmq(vec![tq("hed", "albino"), tq("dek", "albino")], 0.0)?;
        q.add(q1, Occur::SHOULD)?; // false,false);
    }
    {
        let q2 = dmq(vec![tq("hed", "elephant"), tq("dek", "elephant")], 0.0)?;
        q.add(q2, Occur::SHOULD)?; // false,false);
    }
    check(&(Arc::new(q.build()) as Arc<dyn Query>), s);

    let h = s.search(Arc::new(q.build()), 1000)?.score_docs;

    or_print_hits("testBooleanOptionalNoTiebreaker", &h, s, || {
        assert_equals_msg(
            Some(&format!("4 docs should match {}", builder_to_string(&q))),
            &4,
            &h.len(),
        );
        let score = h[0].score;
        /* note: -1 */
        assert_all_scores_equal(&h, h.len() - 1);
        assert_equals_msg(
            Some("wrong last"),
            &Some("d1".to_string()),
            &id_of(s, h[h.len() - 1].doc),
        );
        let score1 = h[h.len() - 1].score;
        Assert::assert_true_msg(
            &format!("d1 does not have worse score then others: {score} >? {score1}"),
            score > score1,
        );
    })
}

fn test_boolean_optional_with_tiebreaker(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let mut q = BooleanQueryBuilder::new();
    {
        let q1 = dmq(vec![tq("hed", "albino"), tq("dek", "albino")], 0.01)?;
        q.add(q1, Occur::SHOULD)?; // false,false);
    }
    {
        let q2 = dmq(vec![tq("hed", "elephant"), tq("dek", "elephant")], 0.01)?;
        q.add(q2, Occur::SHOULD)?; // false,false);
    }
    check(&(Arc::new(q.build()) as Arc<dyn Query>), s);

    let h = s.search(Arc::new(q.build()), 1000)?.score_docs;

    or_print_hits("testBooleanOptionalWithTiebreaker", &h, s, || {
        assert_equals_msg(
            Some(&format!("4 docs should match {}", builder_to_string(&q))),
            &4,
            &h.len(),
        );

        let score0 = h[0].score;
        let score1 = h[1].score;
        let score2 = h[2].score;
        let score3 = h[3].score;

        let doc0 = id_of(s, h[0].doc).expect("NullPointerException");
        let doc1 = id_of(s, h[1].doc).expect("NullPointerException");
        let doc2 = id_of(s, h[2].doc);
        let doc3 = id_of(s, h[3].doc);

        Assert::assert_true_msg(
            &format!("doc0 should be d2 or d4: {doc0}"),
            doc0 == "d2" || doc0 == "d4",
        );
        Assert::assert_true_msg(
            &format!("doc1 should be d2 or d4: {doc0}"),
            doc1 == "d2" || doc1 == "d4",
        );
        Assert::assert_equals_float_msg(
            Some("score0 and score1 should match"),
            score0,
            score1,
            SCORE_COMP_THRESH,
        );
        assert_equals_msg(Some("wrong third"), &Some("d3".to_string()), &doc2);
        Assert::assert_true_msg(
            &format!("d3 does not have worse score then d2 and d4: {score1} >? {score2}"),
            score1 > score2,
        );

        assert_equals_msg(Some("wrong fourth"), &Some("d1".to_string()), &doc3);
        Assert::assert_true_msg(
            &format!("d1 does not have worse score then d3: {score2} >? {score3}"),
            score2 > score3,
        );
    })
}

fn test_boolean_optional_with_tiebreaker_and_boost(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let mut q = BooleanQueryBuilder::new();
    {
        let q1 = dmq(vec![tqb("hed", "albino", 1.5)?, tq("dek", "albino")], 0.01)?;
        q.add(q1, Occur::SHOULD)?; // false,false);
    }
    {
        let q2 = dmq(
            vec![tqb("hed", "elephant", 1.5)?, tq("dek", "elephant")],
            0.01,
        )?;
        q.add(q2, Occur::SHOULD)?; // false,false);
    }
    check(&(Arc::new(q.build()) as Arc<dyn Query>), s);

    let h = s.search(Arc::new(q.build()), 1000)?.score_docs;

    or_print_hits("testBooleanOptionalWithTiebreakerAndBoost", &h, s, || {
        assert_equals_msg(
            Some(&format!("4 docs should match {}", builder_to_string(&q))),
            &4,
            &h.len(),
        );

        let score0 = h[0].score;
        let score1 = h[1].score;
        let score2 = h[2].score;
        let score3 = h[3].score;

        let doc0 = id_of(s, h[0].doc);
        let doc1 = id_of(s, h[1].doc);
        let doc2 = id_of(s, h[2].doc);
        let doc3 = id_of(s, h[3].doc);

        assert_equals_msg(Some("doc0 should be d4: "), &Some("d4".to_string()), &doc0);
        assert_equals_msg(Some("doc1 should be d3: "), &Some("d3".to_string()), &doc1);
        assert_equals_msg(Some("doc2 should be d2: "), &Some("d2".to_string()), &doc2);
        assert_equals_msg(Some("doc3 should be d1: "), &Some("d1".to_string()), &doc3);

        Assert::assert_true_msg(
            &format!("d4 does not have a better score then d3: {score0} >? {score1}"),
            score0 > score1,
        );
        Assert::assert_true_msg(
            &format!("d3 does not have a better score then d2: {score1} >? {score2}"),
            score1 > score2,
        );
        Assert::assert_true_msg(
            &format!("d3 does not have a better score then d1: {score2} >? {score3}"),
            score2 > score3,
        );
    })
}

fn test_rewrite_boolean(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let sub1 = tq("hed", "albino");
    let sub2 = tq("hed", "elephant");
    let q = dmq(vec![Arc::clone(&sub1), Arc::clone(&sub2)], 1.0)?;
    let rewritten = s.rewrite(q)?;
    let mut expected = BooleanQueryBuilder::new();
    expected.add(sub1, Occur::SHOULD)?;
    expected.add(sub2, Occur::SHOULD)?;
    let expected: Arc<dyn Query> = Arc::new(expected.build());
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

fn test_rewrite_empty(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    let q = dmq(vec![], 0.0)?;
    let rewritten = s.rewrite(q)?;
    let expected: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::instance());
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

fn test_rewrite_all_match_no_docs(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    // All disjuncts rewrite to MatchNoDocsQuery -> should return MatchNoDocsQuery
    let sub1: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("test1"));
    let sub2: Arc<dyn Query> = Arc::new(MatchNoDocsQuery::new("test2"));
    let q = dmq(vec![sub1, sub2], 0.0)?;
    let rewritten = s.rewrite(q)?;
    assert_query_equals(&(Arc::new(MatchNoDocsQuery::instance()) as _), &rewritten);
    Ok(())
}

fn test_rewrite_single_survivor(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    // One disjunct rewrites to MatchNoDocsQuery, other matches -> should return the matching one
    let sub = tq("hed", "albino");
    let q = dmq(
        vec![Arc::clone(&sub), Arc::new(MatchNoDocsQuery::new("test"))],
        0.0,
    )?;
    let rewritten = s.rewrite(q)?;
    assert_query_equals(&sub, &rewritten);
    Ok(())
}

fn test_rewrite_filter_match_no_docs(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    // Mixed: some MatchNoDocsQuery, some real queries -> should filter out MatchNoDocsQuery
    let sub1 = tq("hed", "albino");
    let sub2 = tq("hed", "elephant");
    let q = dmq(
        vec![
            Arc::clone(&sub1),
            Arc::clone(&sub2),
            Arc::new(MatchNoDocsQuery::new("test")),
        ],
        0.0,
    )?;
    let rewritten = s.rewrite(q)?;
    let expected = dmq(vec![sub1, sub2], 0.0)?;
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

fn test_rewrite_filter_match_no_docs_with_tie_breaker(f: &mut DisjunctionMax) -> Result<()> {
    let s = f.s();
    // Verify tie breaker multiplier is preserved when filtering MatchNoDocsQuery
    let sub1 = tq("hed", "albino");
    let sub2 = tq("hed", "elephant");
    let q = dmq(
        vec![
            Arc::clone(&sub1),
            Arc::clone(&sub2),
            Arc::new(MatchNoDocsQuery::new("test")),
        ],
        0.1,
    )?;
    let rewritten = s.rewrite(q)?;
    let expected = dmq(vec![sub1, sub2], 0.1)?;
    assert_query_equals(&expected, &rewritten);
    Ok(())
}

fn test_disjunct_order_and_equals(_f: &mut DisjunctionMax) -> Result<()> {
    // the order that disjuncts are provided in should not matter for equals() comparisons
    let sub1 = tq("hed", "albino");
    let sub2 = tq("hed", "elephant");
    let q1 = dmq(vec![Arc::clone(&sub1), Arc::clone(&sub2)], 1.0)?;
    let q2 = dmq(vec![sub2, sub1], 1.0)?;
    assert_query_equals(&q1, &q2);
    Ok(())
}

/* Inspired from TestIntervals.testIntervalDisjunctionToStringStability */
fn test_cases_when_disjunct_order_matters(_f: &mut DisjunctionMax) -> Result<()> {
    let clause_nbr = random().next_int_bound(22) + 4; // ensure a reasonably large minimum number of clauses
    let mut terms = Vec::with_capacity(clause_nbr as usize);
    for i in 0..clause_nbr {
        terms.push(char::from(b'a' + i as u8).to_string());
    }

    let expected = format!(
        "({})~1.0",
        terms
            .iter()
            .map(|term| format!("test:{term}"))
            .collect::<Vec<_>>()
            .join(" | ")
    );

    let source =
        DisjunctionMaxQuery::new(terms.iter().map(|term| tq("test", term)).collect(), 1.0)?;

    assert_equals(expected.as_str(), source.to_query_string("").as_str());
    let disjuncts = source.get_disjuncts();
    assert_equals(&terms.len(), &disjuncts.len());
    let mut i = 0;
    for query in disjuncts {
        let tq = query
            .as_any()
            .downcast_ref::<TermQuery>()
            .expect("ClassCastException");
        assert_equals(terms[i].as_str(), tq.get_term().text().as_str());
        i += 1;
    }
    Ok(())
}

fn test_random_top_docs(_f: &mut DisjunctionMax) -> Result<()> {
    do_test_random_top_docs(2, &[0.05, 0.05])?;
    do_test_random_top_docs(2, &[1.0, 0.05])?;
    do_test_random_top_docs(3, &[1.0, 0.5, 0.05])?;
    do_test_random_top_docs(4, &[1.0, 0.5, 0.05, 0.0])?;
    do_test_random_top_docs(4, &[1.0, 0.5, 0.05, 0.0])?;
    Ok(())
}

/// The shared body of the three `testExplain*` methods.
fn explain(f: &DisjunctionMax, sub1: Arc<dyn Query>, sub2: Arc<dyn Query>) -> Result<Explanation> {
    let s = f.s();
    let dq = dmq(vec![sub1, sub2], 0.0)?;

    let dw = s.create_weight(s.rewrite(dq)?, ScoreMode::COMPLETE, 1.0)?;
    let context = Arc::clone(s.get_top_reader_context())
        .as_leaf()
        .expect("ClassCastException");
    dw.explain(&context, 1)
}

fn test_explain_match(f: &mut DisjunctionMax) -> Result<()> {
    // Both match
    let sub1 = tq("hed", "elephant");
    let sub2 = tq("dek", "elephant");

    let explanation = explain(f, sub1, sub2)?;

    assert_equals("max of:", explanation.description());
    // Two matching sub queries should be included in the explanation details
    assert_equals(&2, &explanation.details().len());
    Ok(())
}

fn test_explain_no_match(f: &mut DisjunctionMax) -> Result<()> {
    // No match
    let sub1 = tq("abc", "elephant");
    let sub2 = tq("def", "elephant");

    let explanation = explain(f, sub1, sub2)?;

    assert_equals("No matching clause", explanation.description());
    // Two non-matching sub queries should be included in the explanation details
    assert_equals(&2, &explanation.details().len());
    Ok(())
}

fn test_explain_match_one_non_matching_sub_query_not_included_in_explanation(
    f: &mut DisjunctionMax,
) -> Result<()> {
    // Matches
    let sub1 = tq("hed", "elephant");

    // Doesn't match
    let sub2 = tq("def", "elephant");

    let explanation = explain(f, sub1, sub2)?;

    assert_equals("max of:", explanation.description());
    // Only the matching sub query (sub1) should be included in the explanation details
    assert_equals(&1, &explanation.details().len());
    Ok(())
}

fn do_test_random_top_docs(num_fields: usize, freqs: &[f64]) -> Result<()> {
    randomizedtesting::jdk::exception::java_assert(num_fields == freqs.len(), String::new);
    let mut dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::with_analyzer(Arc::new(StandardAnalyzer::new()));
    let w = IndexWriter::new(dir.as_directory(), config)?;

    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        LuceneTestCase::at_least(100)
    }; // at night, make sure some terms have skip data
    for _ in 0..num_docs {
        let mut doc = Document::new();
        for j in 0..num_fields {
            let mut builder = String::new();
            let num_as = if random().next_double() < freqs[j] {
                0
            } else {
                1 + random().next_int_bound(5)
            };
            for _ in 0..num_as {
                if !builder.is_empty() {
                    builder.push(' ');
                }
                builder.push('a');
            }
            if random().next_boolean() {
                doc.add(Box::new(StringField::new(
                    "field",
                    "c".to_string(),
                    Store::NO,
                )?));
            }
            let num_others = if random().next_boolean() {
                0
            } else {
                1 + random().next_int_bound(5)
            };
            for _ in 0..num_others {
                if !builder.is_empty() {
                    builder.push(' ');
                }
                builder.push_str(&random().next_int().to_string());
            }
            doc.add(Box::new(TextField::new_with_reader(
                &j.to_string(),
                Box::new(StringCharReader::new(&builder)),
            )?));
        }
        w.add_document(&doc)?;
    }
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    for i in 0..4 {
        let mut clauses: Vec<Arc<dyn Query>> = Vec::new();
        for j in 0..num_fields {
            if i % 2 == 1 {
                clauses.push(tq(&j.to_string(), "a"));
            } else {
                let boost = if random().next_boolean() {
                    0.0
                } else {
                    random().next_float()
                };
                if boost > 0.0 {
                    clauses.push(tqb(&j.to_string(), "a", boost)?);
                } else {
                    clauses.push(tq(&j.to_string(), "a"));
                }
            }
        }
        let tie_breaker = random().next_float();
        let query = dmq(clauses.clone(), tie_breaker)?;
        CheckHits::check_top_scores(&*random(), &query, &searcher)?;

        let mut b = BooleanQueryBuilder::new();
        b.add(dmq(clauses, tie_breaker)?, Occur::MUST)?;
        b.add(tq("field", "c"), Occur::FILTER)?;
        let query: Arc<dyn Query> = Arc::new(b.build());
        CheckHits::check_top_scores(&*random(), &query, &searcher)?;
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

// Ensure generics and type inference play nicely together
fn test_generics(_f: &mut DisjunctionMax) -> Result<()> {
    let query =
        DisjunctionMaxQuery::new(["term"].iter().map(|term| tq("test", term)).collect(), 1.0)?;
    assert_equals(&1, &query.get_disjuncts().len());

    let disjuncts: Vec<Arc<dyn Query>> = vec![
        Arc::new(RegexpQuery::new(term("field", "foobar"))?),
        Arc::new(WildcardQuery::new(term("field", "foobar"))?),
    ];
    let query = DisjunctionMaxQuery::new(disjuncts, 1.0)?;
    assert_equals(&2, &query.get_disjuncts().len());
    Ok(())
}

/// `protected void printHits(String test, ScoreDoc[] h, IndexSearcher
/// searcher)`, with `new DecimalFormat("0.000000000")`.
fn print_hits(test: &str, h: &[ScoreDoc], searcher: &IndexSearcher) -> Result<()> {
    eprintln!("------- {test} -------");

    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in h.iter().enumerate() {
        let d = stored_fields.document(hit.doc)?;
        let score = hit.score;
        eprintln!(
            "#{i}: {score:.9} - {}",
            d.get("id").unwrap_or_else(|| "null".into())
        );
    }
    Ok(())
}
