//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiTermConstantScore.java`.
//!
//! `TestMultiTermConstantScore extends TestBaseRangeFilter`: the superclass's
//! static index (`signedIndexReader`, `signedIndexDir`, `maxId`) and its
//! inherited `testPad` come from `core_search::test_base_range_filter`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::{FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    RewriteMethod,
};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{query_to_string, BooleanQueryBuilder, Occur, Query};
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, close_reader, static_hook, term, tq};
use crate::core_search::test_base_range_filter::{self, pad, F, MIN_ID, STATICS, T};
use crate::core_support::{assert_equals_msg, random, test, verbose};

/// threshold for comparing floats
pub const SCORE_COMP_THRESH: f32 = 1e-6;

/// `CONSTANT_SCORE_REWRITES`:
/// `Set.of(CONSTANT_SCORE_REWRITE, CONSTANT_SCORE_BLENDED_REWRITE)`.
///
/// Java's `Set.of` iterates in an order that varies from one JVM run to the
/// next; the loops over it do not depend on that order, which here is the
/// declaration order.
fn constant_score_rewrites() -> [Arc<dyn RewriteMethod>; 2] {
    [constant_score_rewrite(), constant_score_blended_rewrite()]
}

/// The static fields `small` and `reader`.
struct Statics {
    small: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
}

static SMALL: Mutex<Option<Statics>> = Mutex::new(None);

/// `reader`.
fn small_reader() -> Arc<dyn DirectoryReader> {
    let guard = SMALL.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").reader)
}

/// `assertEquals(String m, int e, int a)`.
fn assert_equals_int(m: &str, e: i32, a: i32) {
    assert_equals_msg(Some(m), &e, &a);
}

/// `TestMultiTermConstantScore`.
pub fn class_def() -> Arc<Class> {
    let methods = vec![
        static_hook("beforeClass", BeforeClass, before_class),
        static_hook("afterClass", AfterClass, after_class),
        // inherited from TestBaseRangeFilter
        test("testPad", test_base_range_filter::test_pad),
        test("testBasics", test_basics),
        test("testEqualScores", test_equal_scores),
        test("testEqualScoresWhenNoHits", test_equal_scores_when_no_hits),
        test("testBooleanOrderUnAffected", test_boolean_order_un_affected),
        test("testRangeQueryId", test_range_query_id),
        test("testRangeQueryRand", test_range_query_rand),
    ];
    let mut builder = Class::builder("org.apache.lucene.search.TestMultiTermConstantScore")
        .extends(&test_base_range_filter::base_class())
        .constructor(Constructor::no_args(|| ()));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

fn before_class() -> Result<()> {
    let data: [Option<&str>; 8] = [
        Some("A 1 2 3 4 5 6"),
        Some("Z       4 5 6"),
        None,
        Some("B   2   4 5 6"),
        Some("Y     3   5 6"),
        None,
        Some("C     3     6"),
        Some("X       4 5 6"),
    ];

    let small = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&WHITESPACE), false),
    )));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), small.as_directory(), iwc)?;

    let mut custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_tokenized(false)?;
    for (i, d) in data.iter().enumerate() {
        let mut doc = rucene::document::Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "id",
            &i.to_string(),
            &custom_type,
        ))); // Field.Keyword("id",String.valueOf(i)));
        doc.add(Box::new(LuceneTestCase::new_field(
            "all",
            "all",
            &custom_type,
        ))); // Field.Keyword("all","all"));
        if let Some(d) = d {
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "data",
                d,
                Store::YES,
            ))); // Field.Text("data",data[i]));
        }
        writer.add_document(&doc)?;
    }

    let reader = writer.get_reader()?;
    writer.close()?;
    *SMALL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics { small, reader });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = SMALL.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics { small, reader }) = statics {
        close_reader(Some(reader))?;
        close_dir(Some(small))?;
    }
    Ok(())
}

/// macro for readability
pub fn csrq(
    f: &str,
    l: Option<&str>,
    h: Option<&str>,
    il: bool,
    ih: bool,
    method: Arc<dyn RewriteMethod>,
) -> Result<Arc<dyn Query>> {
    let query =
        TermRangeQuery::new_string_range_with_rewrite_method(f, l, h, il, ih, Arc::clone(&method))?;
    if verbose() {
        println!(
            "TEST: query={} method={:?}",
            query_to_string(&query),
            method
        );
    }
    Ok(Arc::new(query))
}

/// macro for readability
pub fn cspq(prefix: Term, method: Arc<dyn RewriteMethod>) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PrefixQuery::with_rewrite_method(prefix, method)?))
}

/// macro for readability
pub fn cswcq(wild: Term, method: Arc<dyn RewriteMethod>) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(WildcardQuery::with_rewrite_method(
        wild,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        method,
    )?))
}

/// Port of `TestMultiTermConstantScore.testBasics`.
fn test_basics() -> Result<()> {
    for rw in constant_score_rewrites() {
        let rw = || Arc::clone(&rw);
        QueryUtils::check(&*csrq("data", Some("1"), Some("6"), T, T, rw())?);
        QueryUtils::check(&*csrq("data", Some("A"), Some("Z"), T, T, rw())?);
        QueryUtils::check_unequal(
            &*csrq("data", Some("1"), Some("6"), T, T, rw())?,
            &*csrq("data", Some("A"), Some("Z"), T, T, rw())?,
        );

        QueryUtils::check(&*cspq(term("data", "p*u?"), rw())?);
        QueryUtils::check_unequal(
            &*cspq(term("data", "pre*"), rw())?,
            &*cspq(term("data", "pres*"), rw())?,
        );

        QueryUtils::check(&*cswcq(term("data", "p"), rw())?);
        QueryUtils::check_unequal(
            &*cswcq(term("data", "pre*n?t"), rw())?,
            &*cswcq(term("data", "pr*t?j"), rw())?,
        );
    }
    Ok(())
}

/// The three constant-score rewrites `testEqualScores` and
/// `testEqualScoresWhenNoHits` compare, in Java's order.
fn blended_boolean_constant() -> [Arc<dyn RewriteMethod>; 3] {
    [
        constant_score_blended_rewrite(),
        constant_score_boolean_rewrite(),
        constant_score_rewrite(),
    ]
}

/// Port of `TestMultiTermConstantScore.testEqualScores`.
fn test_equal_scores() -> Result<()> {
    // NOTE: uses index build in *this* setUp

    let search = LuceneTestCase::new_searcher(small_reader());

    // some hits match more terms then others, score should be the same

    let mut score = 0.0f32;
    for (k, rw) in blended_boolean_constant().into_iter().enumerate() {
        let result = search
            .search(csrq("data", Some("1"), Some("6"), T, T, rw)?, 1000)?
            .score_docs;
        let num_hits = result.len() as i32;
        assert_equals_int("wrong number of results", 6, num_hits);
        // the first pass sets the score and compares the others to it; the
        // next passes compare every hit
        let start = if k == 0 {
            score = result[0].score;
            1
        } else {
            0
        };
        for i in start..num_hits as usize {
            assert_equals_msg_delta(
                &format!("score for {i} was not the same"),
                score,
                result[i].score,
                SCORE_COMP_THRESH,
            );
        }
    }
    Ok(())
}

/// `assertEquals(message, expected, actual, delta)` over floats.
fn assert_equals_msg_delta(message: &str, expected: f32, actual: f32, delta: f32) {
    if (expected - actual).abs() > delta {
        randomizedtesting::junit::Assert::fail(&format!(
            "{message} expected:<{expected:?}> but was:<{actual:?}>"
        ));
    }
}

/// Test for LUCENE-5245: Empty MTQ rewrites should have a consistent norm, so
/// always need to return a CSQ!
///
/// Port of `TestMultiTermConstantScore.testEqualScoresWhenNoHits`.
fn test_equal_scores_when_no_hits() -> Result<()> {
    // NOTE: uses index build in *this* setUp

    let search = LuceneTestCase::new_searcher(small_reader());

    let dummy_term = tq("data", "1");

    let mut score = 0.0f32;
    for (k, rw) in blended_boolean_constant().into_iter().enumerate() {
        let mut bq = BooleanQueryBuilder::new();
        bq.add(Arc::clone(&dummy_term), Occur::SHOULD)?; // hits one doc
        bq.add(csrq("data", Some("#"), Some("#"), T, T, rw)?, Occur::SHOULD)?; // hits no docs
        let result = search.search(Arc::new(bq.build()), 1000)?.score_docs;
        let num_hits = result.len() as i32;
        assert_equals_int("wrong number of results", 1, num_hits);
        let start = if k == 0 {
            score = result[0].score;
            1
        } else {
            0
        };
        for i in start..num_hits as usize {
            assert_equals_msg_delta(
                &format!("score for {i} was not the same"),
                score,
                result[i].score,
                SCORE_COMP_THRESH,
            );
        }
    }
    Ok(())
}

/// Port of `TestMultiTermConstantScore.testBooleanOrderUnAffected`.
fn test_boolean_order_un_affected() -> Result<()> {
    // NOTE: uses index build in *this* setUp

    let search = LuceneTestCase::new_searcher(small_reader());

    for rw in constant_score_rewrites() {
        // first do a regular TermRangeQuery which uses term expansion so
        // docs with more terms in range get higher scores

        let rq: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range_with_rewrite_method(
            "data",
            Some("1"),
            Some("4"),
            T,
            T,
            Arc::clone(&rw),
        )?);

        let expected = search.search(Arc::clone(&rq), 1000)?.score_docs;
        let num_hits = expected.len() as i32;

        // now do a boolean where which also contains a
        // ConstantScoreRangeQuery and make sure the order is the same

        let mut q = BooleanQueryBuilder::new();
        q.add(rq, Occur::MUST)?; // T, F);
        q.add(csrq("data", Some("1"), Some("6"), T, T, rw)?, Occur::MUST)?; // T, F);

        let actual = search.search(Arc::new(q.build()), 1000)?.score_docs;

        assert_equals_int("wrong number of hits", num_hits, actual.len() as i32);
        for i in 0..num_hits as usize {
            assert_equals_int(
                &format!("mismatch in docid for hit#{i}"),
                expected[i].doc,
                actual[i].doc,
            );
        }
    }
    Ok(())
}

/// `(signedIndexReader, maxId, signedIndexDir.minR, signedIndexDir.maxR)`.
fn signed_index() -> (Arc<dyn DirectoryReader>, i32, i32, i32) {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref().expect("beforeClassBaseTestRangeFilter ran");
    (
        Arc::clone(&s.signed_index_reader),
        s.max_id,
        s.signed_index_dir.min_r,
        s.signed_index_dir.max_r,
    )
}

/// `search.search(q, numDocs).scoreDocs.length`.
fn hits(
    search: &rucene::search::index_searcher::IndexSearcher,
    q: Result<Arc<dyn Query>>,
    num_docs: i32,
) -> Result<i32> {
    Ok(search.search(q?, num_docs)?.score_docs.len() as i32)
}

/// Port of `TestMultiTermConstantScore.testRangeQueryId`.
fn test_range_query_id() -> Result<()> {
    // NOTE: uses index build in *super* setUp

    let (reader, max_id, _, _) = signed_index();
    let min_id = MIN_ID;
    let search = LuceneTestCase::new_searcher(Arc::clone(&reader));

    if verbose() {
        println!("TEST: reader={reader}");
    }

    let med_id = (max_id - min_id) / 2;

    let min_ip = pad(min_id);
    let max_ip = pad(max_id);
    let med_ip = pad(med_id);
    let (min, max, med) = (
        Some(min_ip.as_str()),
        Some(max_ip.as_str()),
        Some(med_ip.as_str()),
    );

    let num_docs = (Arc::clone(&reader) as Arc<dyn IndexReader>).num_docs();

    assert_equals_int("num of docs", num_docs, 1 + max_id - min_id);

    for rw in constant_score_rewrites() {
        let q = |l, h, il, ih| csrq("id", l, h, il, ih, Arc::clone(&rw));

        // test id, bounded on both ends

        assert_equals_int(
            "find all",
            num_docs,
            hits(&search, q(min, max, T, T), num_docs)?,
        );
        assert_equals_int(
            "all but last",
            num_docs - 1,
            hits(&search, q(min, max, T, F), num_docs)?,
        );
        assert_equals_int(
            "all but first",
            num_docs - 1,
            hits(&search, q(min, max, F, T), num_docs)?,
        );
        assert_equals_int(
            "all but ends",
            num_docs - 2,
            hits(&search, q(min, max, F, F), num_docs)?,
        );
        assert_equals_int(
            "med and up",
            1 + max_id - med_id,
            hits(&search, q(med, max, T, T), num_docs)?,
        );
        assert_equals_int(
            "up to med",
            1 + med_id - min_id,
            hits(&search, q(min, med, T, T), num_docs)?,
        );

        // unbounded id

        assert_equals_int(
            "min and up",
            num_docs,
            hits(&search, q(min, None, T, F), num_docs)?,
        );
        assert_equals_int(
            "max and down",
            num_docs,
            hits(&search, q(None, max, F, T), num_docs)?,
        );
        assert_equals_int(
            "not min, but up",
            num_docs - 1,
            hits(&search, q(min, None, F, F), num_docs)?,
        );
        assert_equals_int(
            "not max, but down",
            num_docs - 1,
            hits(&search, q(None, max, F, F), num_docs)?,
        );
        assert_equals_int(
            "med and up, not max",
            max_id - med_id,
            hits(&search, q(med, max, T, F), num_docs)?,
        );
        assert_equals_int(
            "not min, up to med",
            med_id - min_id,
            hits(&search, q(min, med, F, T), num_docs)?,
        );

        // very small sets

        assert_equals_int(
            "min,min,F,F",
            0,
            hits(&search, q(min, min, F, F), num_docs)?,
        );
        assert_equals_int(
            "med,med,F,F",
            0,
            hits(&search, q(med, med, F, F), num_docs)?,
        );
        assert_equals_int(
            "max,max,F,F",
            0,
            hits(&search, q(max, max, F, F), num_docs)?,
        );
        assert_equals_int(
            "min,min,T,T",
            1,
            hits(&search, q(min, min, T, T), num_docs)?,
        );
        assert_equals_int(
            "nul,min,F,T",
            1,
            hits(&search, q(None, min, F, T), num_docs)?,
        );
        assert_equals_int(
            "max,max,T,T",
            1,
            hits(&search, q(max, max, T, T), num_docs)?,
        );
        assert_equals_int(
            "max,nul,T,T",
            1,
            hits(&search, q(max, None, T, F), num_docs)?,
        );
        assert_equals_int(
            "med,med,T,T",
            1,
            hits(&search, q(med, med, T, T), num_docs)?,
        );
    }
    Ok(())
}

/// Port of `TestMultiTermConstantScore.testRangeQueryRand`.
fn test_range_query_rand() -> Result<()> {
    // NOTE: uses index build in *super* setUp

    let (reader, max_id, min_r, max_r) = signed_index();
    let min_id = MIN_ID;
    let search = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let min_rp = pad(min_r);
    let max_rp = pad(max_r);
    let (min, max) = (Some(min_rp.as_str()), Some(max_rp.as_str()));

    let num_docs = (Arc::clone(&reader) as Arc<dyn IndexReader>).num_docs();

    assert_equals_int("num of docs", num_docs, 1 + max_id - min_id);

    for rw in constant_score_rewrites() {
        let q = |l, h, il, ih| csrq("rand", l, h, il, ih, Arc::clone(&rw));

        // test extremes, bounded on both ends

        assert_equals_int(
            "find all",
            num_docs,
            hits(&search, q(min, max, T, T), num_docs)?,
        );
        assert_equals_int(
            "all but biggest",
            num_docs - 1,
            hits(&search, q(min, max, T, F), num_docs)?,
        );
        assert_equals_int(
            "all but smallest",
            num_docs - 1,
            hits(&search, q(min, max, F, T), num_docs)?,
        );
        assert_equals_int(
            "all but extremes",
            num_docs - 2,
            hits(&search, q(min, max, F, F), num_docs)?,
        );

        // unbounded

        assert_equals_int(
            "smallest and up",
            num_docs,
            hits(&search, q(min, None, T, F), num_docs)?,
        );
        assert_equals_int(
            "biggest and down",
            num_docs,
            hits(&search, q(None, max, F, T), num_docs)?,
        );
        assert_equals_int(
            "not smallest, but up",
            num_docs - 1,
            hits(&search, q(min, None, F, F), num_docs)?,
        );
        assert_equals_int(
            "not biggest, but down",
            num_docs - 1,
            hits(&search, q(None, max, F, F), num_docs)?,
        );

        // very small sets

        assert_equals_int(
            "min,min,F,F",
            0,
            hits(&search, q(min, min, F, F), num_docs)?,
        );
        assert_equals_int(
            "max,max,F,F",
            0,
            hits(&search, q(max, max, F, F), num_docs)?,
        );

        assert_equals_int(
            "min,min,T,T",
            1,
            hits(&search, q(min, min, T, T), num_docs)?,
        );
        assert_equals_int(
            "nul,min,F,T",
            1,
            hits(&search, q(None, min, F, T), num_docs)?,
        );

        assert_equals_int(
            "max,max,T,T",
            1,
            hits(&search, q(max, max, T, T), num_docs)?,
        );
        assert_equals_int(
            "max,nul,T,T",
            1,
            hits(&search, q(max, None, T, F), num_docs)?,
        );
    }
    Ok(())
}
