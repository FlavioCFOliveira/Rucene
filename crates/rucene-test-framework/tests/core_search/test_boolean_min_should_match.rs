//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBooleanMinShouldMatch.java`.
//!
//! Test that BooleanQuery.setMinimumNumberShouldMatch works.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{BooleanQuery, BooleanQueryBuilder, Occur, Query, ScoreDoc, TopDocs};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, close_reader, static_hook, term, tq};
use crate::core_search::test_boolean2::{rand_bool_query, Callback};
use crate::core_support::{assert_equals_msg, class, random, test};

struct Statics {
    index: TestDirectory,
    r: Arc<dyn DirectoryReader>,
    s: IndexSearcher,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn with_s<T>(body: impl FnOnce(&IndexSearcher) -> Result<T>) -> Result<T> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    body(&guard.as_ref().expect("beforeClass ran").s)
}

/// `TestBooleanMinShouldMatch`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBooleanMinShouldMatch",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testAllOptional", test_all_optional),
            test("testOneReqAndSomeOptional", test_one_req_and_some_optional),
            test(
                "testSomeReqAndSomeOptional",
                test_some_req_and_some_optional,
            ),
            test(
                "testOneProhibAndSomeOptional",
                test_one_prohib_and_some_optional,
            ),
            test(
                "testSomeProhibAndSomeOptional",
                test_some_prohib_and_some_optional,
            ),
            test(
                "testOneReqOneProhibAndSomeOptional",
                test_one_req_one_prohib_and_some_optional,
            ),
            test(
                "testSomeReqOneProhibAndSomeOptional",
                test_some_req_one_prohib_and_some_optional,
            ),
            test(
                "testOneReqSomeProhibAndSomeOptional",
                test_one_req_some_prohib_and_some_optional,
            ),
            test(
                "testSomeReqSomeProhibAndSomeOptional",
                test_some_req_some_prohib_and_some_optional,
            ),
            test(
                "testMinHigherThenNumOptional",
                test_min_higher_then_num_optional,
            ),
            test("testMinEqualToNumOptional", test_min_equal_to_num_optional),
            test("testOneOptionalEqualToMin", test_one_optional_equal_to_min),
            test("testNoOptionalButMin", test_no_optional_but_min),
            test("testNoOptionalButMin2", test_no_optional_but_min2),
            test("testRandomQueries", test_random_queries),
            test("testRewriteMSM1", test_rewrite_msm1),
            test("testRewriteNegate", test_rewrite_negate),
            test(
                "testFlattenInnerDisjunctions",
                test_flatten_inner_disjunctions,
            ),
        ],
    )
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

    let index = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), index.as_directory())?;

    for (i, d) in data.iter().enumerate() {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        ))); // Field.Keyword("id",String.valueOf(i)));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "all",
            "all",
            Store::YES,
        ))); // Field.Keyword("all","all"));
        if let Some(d) = d {
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "data",
                d,
                Store::YES,
            )));
            // Field.Text("data",data[i]));
        }
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    w.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics { index, r, s });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(st) = statics {
        drop(st.s);
        close_reader(Some(st.r))?;
        close_dir(Some(st.index))?;
    }
    Ok(())
}

/// `verifyNrHits(Query, int)`.
fn verify_nr_hits(q: Arc<dyn Query>, expected: usize) -> Result<()> {
    with_s(|s| {
        let h = s.search(Arc::clone(&q), 1000)?.score_docs;
        let test_name = LuceneTestCase::get_test_name().unwrap_or_default();
        if expected != h.len() {
            print_hits(&test_name, &h, s)?;
        }
        assert_equals_msg(Some("result count"), &expected, &h.len());
        let collector_manager = TopScoreDocCollectorManager::with_num_hits(1000, i32::MAX)?;
        let h2 = s
            .search_with_manager(Arc::clone(&q), &collector_manager)?
            .score_docs;
        if expected != h2.len() {
            print_hits(&test_name, &h2, s)?;
        }
        assert_equals_msg(Some("result count (bs2)"), &expected, &h2.len());

        QueryUtils::check_searcher(&*random(), &q, Some(s));
        Ok(())
    })
}

/// A builder holding `clauses`, with `minimumNumberShouldMatch` `msm`.
fn bq(clauses: &[(&str, &str, Occur)], msm: i32) -> Result<Arc<dyn Query>> {
    let mut q = BooleanQueryBuilder::new();
    for (field, text, occur) in clauses {
        q.add(tq(field, text), *occur)?;
    }
    q.set_minimum_number_should_match(msm);
    Ok(Arc::new(q.build()))
}

use Occur::{MUST, MUST_NOT, SHOULD};

fn test_all_optional() -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    for i in 1..=4 {
        q.add(tq("data", &i.to_string()), SHOULD)?; // false, false);
    }
    q.set_minimum_number_should_match(2); // match at least two of 4
    verify_nr_hits(Arc::new(q.build()), 2)
}

fn test_one_req_and_some_optional() -> Result<()> {
    /* one required, some optional */
    let q = bq(
        &[
            ("all", "all", MUST),
            ("data", "5", SHOULD),
            ("data", "4", SHOULD),
            ("data", "3", SHOULD),
        ],
        2, // 2 of 3 optional
    )?;
    verify_nr_hits(q, 5)
}

fn test_some_req_and_some_optional() -> Result<()> {
    /* two required, some optional */
    let q = bq(
        &[
            ("all", "all", MUST),
            ("data", "6", MUST),
            ("data", "5", SHOULD),
            ("data", "4", SHOULD),
            ("data", "3", SHOULD),
        ],
        2, // 2 of 3 optional
    )?;
    verify_nr_hits(q, 5)
}

fn test_one_prohib_and_some_optional() -> Result<()> {
    /* one prohibited, some optional */
    let q = bq(
        &[
            ("data", "1", SHOULD),
            ("data", "2", SHOULD),
            ("data", "3", MUST_NOT),
            ("data", "4", SHOULD),
        ],
        2, // 2 of 3 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_some_prohib_and_some_optional() -> Result<()> {
    /* two prohibited, some optional */
    let q = bq(
        &[
            ("data", "1", SHOULD),
            ("data", "2", SHOULD),
            ("data", "3", MUST_NOT),
            ("data", "4", SHOULD),
            ("data", "C", MUST_NOT),
        ],
        2, // 2 of 3 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_one_req_one_prohib_and_some_optional() -> Result<()> {
    /* one required, one prohibited, some optional */
    let q = bq(
        &[
            ("data", "6", MUST),
            ("data", "5", SHOULD),
            ("data", "4", SHOULD),
            ("data", "3", MUST_NOT),
            ("data", "2", SHOULD),
            ("data", "1", SHOULD),
        ],
        3, // 3 of 4 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_some_req_one_prohib_and_some_optional() -> Result<()> {
    /* two required, one prohibited, some optional */
    let q = bq(
        &[
            ("all", "all", MUST),
            ("data", "6", MUST),
            ("data", "5", SHOULD),
            ("data", "4", SHOULD),
            ("data", "3", MUST_NOT),
            ("data", "2", SHOULD),
            ("data", "1", SHOULD),
        ],
        3, // 3 of 4 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_one_req_some_prohib_and_some_optional() -> Result<()> {
    /* one required, two prohibited, some optional */
    let q = bq(
        &[
            ("data", "6", MUST),
            ("data", "5", SHOULD),
            ("data", "4", SHOULD),
            ("data", "3", MUST_NOT),
            ("data", "2", SHOULD),
            ("data", "1", SHOULD),
            ("data", "C", MUST_NOT),
        ],
        3, // 3 of 4 optional
    )?;
    verify_nr_hits(q, 1)
}

/// The clauses of the "two required, two prohibited, some optional" tests.
const TWO_REQ_TWO_PROHIB: [(&str, &str, Occur); 8] = [
    ("all", "all", MUST),
    ("data", "6", MUST),
    ("data", "5", SHOULD),
    ("data", "4", SHOULD),
    ("data", "3", MUST_NOT),
    ("data", "2", SHOULD),
    ("data", "1", SHOULD),
    ("data", "C", MUST_NOT),
];

fn test_some_req_some_prohib_and_some_optional() -> Result<()> {
    /* two required, two prohibited, some optional */
    let q = bq(&TWO_REQ_TWO_PROHIB, 3)?; // 3 of 4 optional
    verify_nr_hits(q, 1)
}

fn test_min_higher_then_num_optional() -> Result<()> {
    /* two required, two prohibited, some optional */
    let q = bq(&TWO_REQ_TWO_PROHIB, 90)?; // 90 of 4 optional ?!?!?!
    verify_nr_hits(q, 0)
}

fn test_min_equal_to_num_optional() -> Result<()> {
    /* two required, two optional */
    let q = bq(
        &[
            ("all", "all", SHOULD),
            ("data", "6", MUST),
            ("data", "3", MUST),
            ("data", "2", SHOULD),
        ],
        2, // 2 of 2 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_one_optional_equal_to_min() -> Result<()> {
    /* two required, one optional */
    let q = bq(
        &[
            ("all", "all", MUST),
            ("data", "3", SHOULD),
            ("data", "2", MUST),
        ],
        1, // 1 of 1 optional
    )?;
    verify_nr_hits(q, 1)
}

fn test_no_optional_but_min() -> Result<()> {
    /* two required, no optional */
    let q = bq(&[("all", "all", MUST), ("data", "2", MUST)], 1)?; // 1 of 0 optional
    verify_nr_hits(q, 0)
}

fn test_no_optional_but_min2() -> Result<()> {
    /* one required, no optional */
    let q = bq(&[("all", "all", MUST)], 1)?; // 1 of 0 optional
    verify_nr_hits(q, 0)
}

/// The anonymous `TestBoolean2.Callback` `minNrCB`.
struct MinNrCallback<'a> {
    field: &'a str,
    vals: &'a [&'a str],
}

impl Callback for MinNrCallback<'_> {
    fn post_create(&self, q: &mut BooleanQueryBuilder) -> Result<()> {
        let mut opt = 0;
        for clause in q.build().clauses() {
            if clause.occur() == SHOULD {
                opt += 1;
            }
        }
        q.set_minimum_number_should_match(random().next_int_bound(opt + 2));
        if random().next_boolean() {
            let random_term = term(
                self.field,
                self.vals[random().next_int_bound(self.vals.len() as i32) as usize],
            );
            q.add(
                Arc::new(rucene::search::term_query::TermQuery::new(random_term)),
                MUST_NOT,
            )?;
        }
        Ok(())
    }
}

fn test_random_queries() -> Result<()> {
    let field = "data";
    let vals = [
        "1", "2", "3", "4", "5", "6", "A", "Z", "B", "Y", "Z", "X", "foo",
    ];
    let max_lev = 4;

    let min_nr_cb = MinNrCallback { field, vals: &vals };

    let num = LuceneTestCase::at_least(20);
    for i in 0..num {
        let lev = random().next_int_bound(max_lev);
        let seed = random().next_long();
        let q1 = rand_bool_query(&JdkRandom::new(seed), true, lev, field, &vals, None)?;
        let mut q2 = rand_bool_query(&JdkRandom::new(seed), true, lev, field, &vals, None)?;
        min_nr_cb.post_create(&mut q2)?;

        let (top1, top2) = with_s(|s| {
            let top1 = s.search(Arc::new(q1.build()), 100)?;
            let top2 = s.search(Arc::new(q2.build()), 100)?;
            if i < 100 {
                QueryUtils::check_searcher(
                    &*random(),
                    &(Arc::new(q1.build()) as Arc<dyn Query>),
                    Some(s),
                );
                QueryUtils::check_searcher(
                    &*random(),
                    &(Arc::new(q2.build()) as Arc<dyn Query>),
                    Some(s),
                );
            }
            Ok((top1, top2))
        })?;
        assert_subset_of_same_scores(&q2.build(), &top1, &top2);
    }
    Ok(())
}

/// `Math.ulp(float)`.
fn ulp(f: f32) -> f32 {
    let a = f.abs();
    if a.is_nan() {
        return f32::NAN;
    }
    if a.is_infinite() {
        return f32::INFINITY;
    }
    if a == f32::MAX {
        // 2^(127 - 23)
        return 2f32.powi(104);
    }
    f32::from_bits(a.to_bits() + 1) - a
}

fn assert_subset_of_same_scores(q: &BooleanQuery, top1: &TopDocs, top2: &TopDocs) {
    let q_string = rucene::search::query::query_to_string(q);
    if top2.total_hits.value() > top1.total_hits.value() {
        Assert::fail(&format!(
            "Constrained results not a subset:\n{}{}for query:{q_string}",
            CheckHits::topdocs_string(top1, 0, 0),
            CheckHits::topdocs_string(top2, 0, 0)
        ));
    }

    let num_scoring_clauses = q.get_clauses(SHOULD).len() + q.get_clauses(MUST).len();

    for hit in 0..top2.total_hits.value() as usize {
        let id = top2.score_docs[hit].doc;
        let score = top2.score_docs[hit].score;
        let mut found = false;
        for other in 0..top1.total_hits.value() as usize {
            if top1.score_docs[other].doc == id {
                found = true;
                let other_score = top1.score_docs[other].score;
                Assert::assert_equals_float_msg(
                    Some(&format!(
                        "Doc {id} scores don't match\n{}{}for query:{q_string}",
                        CheckHits::topdocs_string(top1, 0, 0),
                        CheckHits::topdocs_string(top2, 0, 0)
                    )),
                    score,
                    other_score,
                    ulp(score) * num_scoring_clauses as f32,
                );
            }
        }

        if !found {
            Assert::fail(&format!(
                "Doc {id} not found\n{}{}for query:{q_string}",
                CheckHits::topdocs_string(top1, 0, 0),
                CheckHits::topdocs_string(top2, 0, 0)
            ));
        }
    }
}

fn rewrite_case(q2_extra: Option<(&str, Occur)>, msm: Option<i32>) -> Result<()> {
    let mut q1 = BooleanQueryBuilder::new();
    q1.add(tq("data", "1"), SHOULD)?;
    let mut q2 = BooleanQueryBuilder::new();
    q2.add(tq("data", "1"), SHOULD)?;
    if let Some((text, occur)) = q2_extra {
        q2.add(tq("data", text), occur)?;
    }
    if let Some(msm) = msm {
        q2.set_minimum_number_should_match(msm);
    }
    let (top1, top2) = with_s(|s| {
        Ok((
            s.search(Arc::new(q1.build()), 100)?,
            s.search(Arc::new(q2.build()), 100)?,
        ))
    })?;
    assert_subset_of_same_scores(&q2.build(), &top1, &top2);
    Ok(())
}

fn test_rewrite_msm1() -> Result<()> {
    rewrite_case(None, Some(1))
}

fn test_rewrite_negate() -> Result<()> {
    rewrite_case(Some(("Z", MUST_NOT)), None)
}

fn test_flatten_inner_disjunctions() -> Result<()> {
    let mut b = BooleanQueryBuilder::new();
    b.set_minimum_number_should_match(2);
    b.add(tq("all", "all"), SHOULD)?;
    b.add(tq("data", "1"), SHOULD)?;
    b.add(tq("data", "2"), MUST)?;
    verify_nr_hits(Arc::new(b.build()), 1)?;

    let mut inner = BooleanQueryBuilder::new();
    inner.add(tq("all", "all"), SHOULD)?;
    inner.add(tq("data", "1"), SHOULD)?;
    let inner: Arc<dyn Query> = Arc::new(inner.build());
    let mut b = BooleanQueryBuilder::new();
    b.set_minimum_number_should_match(2);
    b.add(inner, SHOULD)?;
    b.add(tq("data", "2"), MUST)?;

    verify_nr_hits(Arc::new(b.build()), 0)
}

/// `printHits(String, ScoreDoc[], IndexSearcher)`.
fn print_hits(test: &str, h: &[ScoreDoc], searcher: &IndexSearcher) -> Result<()> {
    eprintln!("------- {test} -------");

    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in h.iter().enumerate() {
        let d = stored_fields.document(hit.doc)?;
        let score = hit.score;
        eprintln!(
            "#{i}: {score:.6} - {} - {}",
            d.get("id").unwrap_or_else(|| "null".into()),
            d.get("data").unwrap_or_else(|| "null".into())
        );
    }
    Ok(())
}
