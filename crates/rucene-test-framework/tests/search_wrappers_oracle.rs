//! Parity of the search wrappers of `org.apache.lucene.tests.search`
//! (rmp #345) with Lucene 10.5.0: `search_wrappers.tsv`, written by
//! `crates/randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/tests/search/SearchWrappersOracle.java`
//! over the default-codec corpus of `mdw_corpus.tsv` (`nocfs` and `cfs`).
//!
//! Each row is `case, scenario, step, outcome` (`ok<TAB>value` or
//! `err<TAB>class<TAB>message`). The scenarios run inside the suite
//! `TestSearchWrappersScenarios` under the runner (`tests.seed` `5EA4C4E5`),
//! because `AssertingLeafCollector` draws `rarely()` from the test context:
//! the top hits, total hits, count and top explanation of core queries (and
//! of the test-framework query wrappers) through `AssertingIndexSearcher`
//! (three seeds, each ending with the searcher's next draw), through
//! `ScorerIndexSearcher` and through a searcher using
//! `AssertingSimilarity(RandomSimilarity)`; the misuse of each asserting class
//! with Java's message; and, outside the runner, `RandomSimilarity`'s pick per
//! field, `toString()` and next draw over a seed grid.

mod oracle_support;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use oracle_support::{assert_same, rows};
use randomizedtesting::jdk::exception::QuietPanics;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{ClassRule, JUnitCore, Request, Test};
use rucene::document::IntPoint;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::{IndexReader, LeafReaderContext, Term};
use rucene::search::similarities::{BM25Similarity, Explanation, Similarity};
use rucene::search::{
    all, BooleanQueryBuilder, BoostQuery, BulkScorer, CollectionResult, ConstantScoreQuery,
    DefaultBulkScorer, DisjunctionMaxQuery, FieldExistsQuery, IndexSearcher, KnnFloatVectorQuery,
    LeafCollector, LeafReaderContextPartition, MatchAllDocsQuery, MatchNoDocsQuery, Occur,
    PhraseQuery, PrefixQuery, Query, Scorable, ScoreMode, Scorer, ScorerSupplier, SegmentCacheable,
    SimpleScorable, TermQuery, TopDocs, TwoPhaseIterator, Weight, WildcardQuery, NO_MORE_DOCS,
};
use rucene::store::java_nio_file::{java_class_of, java_message_of};
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene_test_framework::search::similarities::{AssertingSimilarity, RandomSimilarity};
use rucene_test_framework::search::{
    AssertingBulkScorer, AssertingCollector, AssertingIndexSearcherOverrides,
    AssertingLeafCollector, AssertingMatches, AssertingQuery, AssertingScorer, AssertingWeight,
    BlockScoreQueryWrapper, BulkScorerWrapperScorer, DisablingBulkScorerQuery,
    DummyTotalHitCountCollector, FixedBitSetCollector, RandomApproximationQuery,
    RandomTwoPhaseView, ScorerIndexSearcher,
};
use rucene_test_framework::util::lucene_test_case::{LuceneTestCase, CLASS_NAME_RULE};

const SEED: &str = "5EA4C4E5";

// ---------------------------------------------------------------------------
// Rows (as IndexWrappersOracle's)
// ---------------------------------------------------------------------------

/// `SearchWrappersOracle.m`: identity hashes masked, line breaks and tabs
/// escaped.
fn m(s: &str) -> String {
    oracle_support::mask(s, "\u{0}", None)
}

fn or_null(message: &str) -> String {
    if message.is_empty() {
        "<null>".to_string()
    } else {
        message.to_string()
    }
}

fn class_and_message(e: &LuceneError) -> (String, String) {
    match e {
        LuceneError::Io(io) => (
            java_class_of(io).to_string(),
            java_message_of(io).unwrap_or_else(|| "<null>".to_string()),
        ),
        LuceneError::Unchecked { class, message, .. } => (
            class.to_string(),
            message.clone().unwrap_or_else(|| "<null>".to_string()),
        ),
        LuceneError::AssertionError(message) => {
            ("java.lang.AssertionError".to_string(), or_null(message))
        }
        other => (
            format!("rucene::{}", other.java_simple_name()),
            other.to_string(),
        ),
    }
}

fn outcome(f: impl FnOnce() -> Result<String>) -> String {
    let result = {
        let _quiet = QuietPanics::new();
        catch_unwind(AssertUnwindSafe(f))
    };
    match result {
        Ok(Ok(v)) => format!("ok\t{}", m(&v)),
        Ok(Err(e)) => {
            let (class, message) = class_and_message(&e);
            format!("err\t{class}\t{}", m(&message))
        }
        Err(payload) => {
            let (class, message) = oracle_support::panic_error(payload);
            format!("err\t{class}\t{}", m(&message))
        }
    }
}

static ROWS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn row(c: &str, scenario: &str, step: &str, value: String) {
    let key = format!("{c}\t{scenario}\t{step}");
    let previous = ROWS.lock().unwrap().insert(key.clone(), value);
    assert!(previous.is_none(), "duplicate row {key}");
}

fn rec(c: &str, scenario: &str, step: &str, f: impl FnOnce() -> Result<String>) {
    let value = outcome(f);
    row(c, scenario, step, value);
}

fn bits(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

// ---------------------------------------------------------------------------
// The corpus and the queries
// ---------------------------------------------------------------------------

fn load_corpus(case: &str) -> Arc<dyn Directory> {
    let dir = ByteBuffersDirectory::new();
    for row in rows("mdw_corpus.tsv") {
        if row[0] != case {
            continue;
        }
        let b: Vec<u8> = (0..row[2].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&row[2][i..i + 2], 16).unwrap())
            .collect();
        let mut out = dir.create_output(&row[1], &*DEFAULT_IO_CONTEXT).unwrap();
        out.write_bytes(&b, 0, b.len()).unwrap();
        out.close().unwrap();
    }
    Arc::new(dir)
}

fn term(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text("body", text)))
}

fn bool_query(clauses: &[(Arc<dyn Query>, Occur)]) -> Arc<dyn Query> {
    let mut b = BooleanQueryBuilder::new();
    for (q, occur) in clauses {
        b.add(Arc::clone(q), *occur).unwrap();
    }
    Arc::new(b.build())
}

fn should_w1_w2() -> Arc<dyn Query> {
    bool_query(&[(term("w1"), Occur::SHOULD), (term("w2"), Occur::SHOULD)])
}

/// `SearchWrappersOracle.queries(random)`.
fn queries(random: &dyn Random) -> Vec<(&'static str, Arc<dyn Query>)> {
    let mut q: Vec<(&'static str, Arc<dyn Query>)> = vec![
        ("term", term("w7")),
        ("should", should_w1_w2()),
        (
            "must_should",
            bool_query(&[(term("w3"), Occur::MUST), (term("w4"), Occur::SHOULD)]),
        ),
        (
            "must_filter",
            bool_query(&[(term("w5"), Occur::MUST), (term("w12"), Occur::FILTER)]),
        ),
        (
            "should_not",
            bool_query(&[(term("w8"), Occur::SHOULD), (term("w9"), Occur::MUST_NOT)]),
        ),
        (
            "phrase",
            Arc::new(PhraseQuery::with_slop(0, "body", &["w0", "w17"]).unwrap()),
        ),
        (
            "prefix",
            Arc::new(PrefixQuery::new(Term::from_text("body", "w1")).unwrap()),
        ),
        (
            "wildcard",
            Arc::new(WildcardQuery::new(Term::from_text("body", "w?3")).unwrap()),
        ),
        (
            "point_range",
            IntPoint::new_range_query("p_int", &[10], &[50]).unwrap(),
        ),
        ("exists", Arc::new(FieldExistsQuery::new("dv_num"))),
        ("constant", Arc::new(ConstantScoreQuery::new(term("w7")))),
        ("boost", Arc::new(BoostQuery::new(term("w7"), 2.5).unwrap())),
        (
            "dismax",
            Arc::new(DisjunctionMaxQuery::new(vec![term("w1"), term("w2")], 0.1).unwrap()),
        ),
        ("all", Arc::new(MatchAllDocsQuery)),
        ("none", Arc::new(MatchNoDocsQuery::new("none"))),
        (
            "knn",
            Arc::new(KnnFloatVectorQuery::new("vf", vec![1.0, -0.5, 1.5, 1.0], 5).unwrap()),
        ),
    ];
    let asserting: Arc<dyn Random> = Arc::new(JdkRandom::new(random.next_long()));
    q.push((
        "asserting",
        Arc::new(AssertingQuery::new(asserting, term("w7"))),
    ));
    let approx: Arc<dyn Random> = Arc::new(JdkRandom::new(random.next_long()));
    q.push((
        "random_approx",
        Arc::new(RandomApproximationQuery::new(should_w1_w2(), approx)),
    ));
    q.push((
        "block_score",
        Arc::new(BlockScoreQueryWrapper::new(should_w1_w2(), 2)),
    ));
    q.push((
        "disabling_bulk",
        Arc::new(DisablingBulkScorerQuery::new(should_w1_w2())),
    ));
    q
}

fn hits(td: &TopDocs) -> String {
    let mut sb = td.total_hits.to_string();
    for sd in &td.score_docs {
        sb.push_str(&format!(" {}:{}", sd.doc, bits(sd.score)));
    }
    sb
}

fn expl(e: &Explanation) -> String {
    format!(
        "{} {} {}",
        e.is_match(),
        bits(e.value().float_value()),
        e.description()
    )
}

fn searches(c: &str, scenario: &str, s: &mut IndexSearcher, random: &dyn Random) {
    s.set_query_cache(None);
    for (name, q) in queries(random) {
        let mut top = -1;
        rec(c, scenario, &format!("{name}/top"), || {
            let td = s.search(Arc::clone(&q), 10)?;
            if let Some(first) = td.score_docs.first() {
                top = first.doc;
            }
            Ok(hits(&td))
        });
        rec(c, scenario, &format!("{name}/count"), || {
            Ok(s.count(Arc::clone(&q))?.to_string())
        });
        if top >= 0 {
            rec(c, scenario, &format!("{name}/explain"), || {
                Ok(expl(&s.explain(Arc::clone(&q), top)?))
            });
        }
    }
}

fn asserting_searcher(
    seed: i64,
    reader: &Arc<dyn IndexReader>,
) -> (IndexSearcher, Arc<AssertingIndexSearcherOverrides>) {
    let r = JdkRandom::new(seed);
    let overrides = Arc::new(AssertingIndexSearcherOverrides::new(&r));
    let mut s = IndexSearcher::new(Arc::clone(reader)).unwrap();
    s.set_overrides(Arc::clone(&overrides) as _);
    (s, overrides)
}

/// `SearchWrappersOracle.scenarios()`.
fn scenarios() {
    for c in ["nocfs", "cfs"] {
        let reader: Arc<dyn IndexReader> = directory_reader::open(load_corpus(c)).unwrap();
        for seed in [1i64, 2, 3] {
            let (mut s, overrides) = asserting_searcher(seed, &reader);
            let scenario = format!("asserting{seed}");
            searches(c, &scenario, &mut s, &JdkRandom::new(seed + 100));
            rec(c, &scenario, "next", || {
                Ok(overrides.random().next_long().to_string())
            });
            rec(c, &scenario, "toString", || {
                Ok(format!("{s:?}")
                    .starts_with("AssertingIndexSearcher(IndexSearcher(")
                    .to_string())
            });
        }
        let mut scorer = ScorerIndexSearcher::new(Arc::clone(&reader)).unwrap();
        searches(c, "scorer", &mut scorer, &JdkRandom::new(7));
        let (mut sim, overrides) = asserting_searcher(11, &reader);
        let rs = Arc::new(RandomSimilarity::new(&JdkRandom::new(12)));
        let asserting: Arc<dyn Similarity> = Arc::new(AssertingSimilarity::new(Arc::clone(&rs)));
        sim.set_similarity(asserting);
        searches(c, "random_sim", &mut sim, &JdkRandom::new(13));
        rec(c, "random_sim", "toString", || Ok(rs.to_string()));
        rec(c, "random_sim", "next", || {
            Ok(overrides.random().next_long().to_string())
        });
        misuse(c, &reader);
        reader.close().unwrap();
    }
}

// ---------------------------------------------------------------------------
// Misuse of the asserting classes
// ---------------------------------------------------------------------------

fn term_scorer(s: &IndexSearcher, ctx: &LeafReaderContext, text: &str) -> Box<dyn Scorer> {
    let w = s
        .create_weight(s.rewrite(term(text)).unwrap(), ScoreMode::COMPLETE, 1.0)
        .unwrap();
    w.scorer(ctx).unwrap().unwrap()
}

/// `SearchWrappersOracle.NOOP`.
struct Noop;

impl LeafCollector for Noop {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

/// The anonymous counting collector of the bulk scenario.
struct Counting(i32);

impl LeafCollector for Counting {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.0 += 1;
        Ok(())
    }
}

/// The anonymous `FilterWeight` whose count is one above `numDocs`.
#[derive(Debug)]
struct CountTooLarge(Arc<dyn Weight>);

impl SegmentCacheable for CountTooLarge {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.0.is_cacheable(ctx)
    }
}

impl Weight for CountTooLarge {
    fn get_query(&self) -> Arc<dyn Query> {
        self.0.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.0.explain(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.0.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        Ok(context.leaf_reader().num_docs() + 1)
    }
}

fn collect(lc: &mut dyn LeafCollector, doc: i32) -> Result<String> {
    lc.collect(doc, &mut SimpleScorable::new())
        .map_err(|e| e.into_lucene_error())?;
    Ok("null".to_string())
}

fn misuse(c: &str, reader: &Arc<dyn IndexReader>) {
    let mut plain = IndexSearcher::new(Arc::clone(reader)).unwrap();
    plain.set_query_cache(None);
    let leaves = plain.get_leaf_contexts().to_vec();
    let ctx = Arc::clone(&leaves[0]);
    let max_doc = ctx.leaf_reader().max_doc();

    // AssertingScorer
    let mut sc1 = AssertingScorer::wrap(term_scorer(&plain, &ctx, "w7"), true, false);
    rec(c, "scorer", "score_unpositioned", || {
        Ok(sc1.score()?.to_string())
    });
    rec(c, "scorer", "max_score_unpositioned", || {
        Ok(sc1.get_max_score(10)?.to_string())
    });
    rec(c, "scorer", "shallow_decreasing", || {
        Ok(sc1.advance_shallow(-5)?.to_string())
    });
    rec(c, "scorer", "next", || {
        Ok(sc1.iterator().next_doc()?.to_string())
    });
    rec(c, "scorer", "score", || Ok(bits(sc1.score()?)));
    rec(c, "scorer", "min_competitive", || {
        sc1.set_min_competitive_score(1.0)?;
        Ok("null".to_string())
    });
    rec(c, "scorer", "advance_same", || {
        let doc = sc1.doc_id();
        Ok(sc1.iterator().advance(doc)?.to_string())
    });
    rec(c, "scorer", "children", || {
        let children = sc1.children()?;
        Ok(format!("{} {}", children.len(), children[0].relationship))
    });
    rec(c, "scorer", "exhaust", || {
        Ok(sc1.iterator().advance(NO_MORE_DOCS)?.to_string())
    });
    rec(c, "scorer", "next_after_end", || {
        Ok(sc1.iterator().next_doc()?.to_string())
    });
    rec(c, "scorer", "advance_after_end", || {
        Ok(sc1.iterator().advance(5)?.to_string())
    });
    let mut sc2 = AssertingScorer::wrap(term_scorer(&plain, &ctx, "w7"), false, false);
    rec(c, "scorer", "cannot_score", || {
        sc2.iterator().next_doc()?;
        Ok(sc2.score()?.to_string())
    });
    rec(c, "scorer", "cannot_shallow", || {
        Ok(sc2.advance_shallow(100)?.to_string())
    });
    let mut sc3 = AssertingScorer::wrap(term_scorer(&plain, &ctx, "w7"), true, true);
    rec(c, "scorer", "shallow", || {
        Ok(sc3.advance_shallow(3)?.to_string())
    });
    rec(c, "scorer", "shallow_below_doc", || {
        sc3.iterator().advance(40)?;
        let doc = sc3.doc_id();
        Ok(sc3.advance_shallow(doc - 1)?.to_string())
    });
    rec(c, "scorer", "min_competitive_nan", || {
        sc3.set_min_competitive_score(f32::NAN)?;
        Ok("null".to_string())
    });
    rec(c, "scorer", "min_competitive_decreasing", || {
        sc3.set_min_competitive_score(0.5)?;
        sc3.set_min_competitive_score(0.25)?;
        Ok("null".to_string())
    });
    // AssertingScorer over a two-phase scorer
    let phrase: Arc<dyn Query> =
        Arc::new(PhraseQuery::with_slop(0, "body", &["w0", "w17"]).unwrap());
    let phrase_weight = plain
        .create_weight(plain.rewrite(phrase).unwrap(), ScoreMode::COMPLETE, 1.0)
        .unwrap();
    let mut sc4 = AssertingScorer::wrap(phrase_weight.scorer(&ctx).unwrap().unwrap(), true, false);
    rec(c, "scorer", "two_phase", || {
        Ok(sc4.two_phase_iterator().is_some().to_string())
    });
    // Java keeps the view; each step here borrows it anew from the scorer.
    rec(c, "scorer", "matches_unpositioned", || {
        Ok(sc4.two_phase_iterator().unwrap().matches()?.to_string())
    });
    rec(c, "scorer", "approx_next", || {
        Ok(sc4
            .two_phase_iterator()
            .unwrap()
            .approximation()
            .next_doc()?
            .to_string())
    });
    rec(c, "scorer", "approx_matches", || {
        Ok(sc4.two_phase_iterator().unwrap().matches()?.to_string())
    });
    rec(c, "scorer", "approx_backwards", || {
        Ok(sc4
            .two_phase_iterator()
            .unwrap()
            .approximation()
            .advance(0)?
            .to_string())
    });
    rec(c, "scorer", "approx_end", || {
        Ok(sc4
            .two_phase_iterator()
            .unwrap()
            .approximation()
            .advance(NO_MORE_DOCS)?
            .to_string())
    });
    rec(c, "scorer", "approx_after_end", || {
        Ok(sc4
            .two_phase_iterator()
            .unwrap()
            .approximation()
            .next_doc()?
            .to_string())
    });

    // AssertingCollector / AssertingLeafCollector
    let mut ac = AssertingCollector::wrap(DummyTotalHitCountCollector::new());
    rec(c, "collector", "leaf_before_weight", || {
        use rucene::search::Collector;
        ac.get_leaf_collector(&ctx)
            .map_err(|e| e.into_lucene_error())?;
        Ok("true".to_string())
    });
    let all_weight = plain
        .create_weight(
            Arc::new(MatchAllDocsQuery),
            ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )
        .unwrap();
    {
        use rucene::search::Collector;
        rec(c, "collector", "weight", || {
            ac.set_weight(Arc::clone(&all_weight));
            Ok("null".to_string())
        });
        rec(c, "collector", "weight_twice", || {
            ac.set_weight(Arc::clone(&all_weight));
            Ok("null".to_string())
        });
        let last = Arc::clone(&leaves[leaves.len() - 1]);
        let mut lc = ac.get_leaf_collector(&last).unwrap();
        rec(c, "collector", "collect5", || collect(&mut *lc, 5));
        rec(c, "collector", "collect3", || collect(&mut *lc, 3));
        let finish = outcome(|| {
            lc.finish()?;
            Ok(String::new())
        });
        rec(c, "collector", "finish_twice", || {
            lc.finish()?;
            Ok("null".to_string())
        });
        drop(lc);
        // Java returns the flag right after `finish()`; the second `finish()`
        // sets it to the same value before failing.
        row(
            c,
            "collector",
            "finish",
            if finish.starts_with("ok") {
                format!("ok\t{}", ac.has_finished_collecting_previous_leaf())
            } else {
                finish
            },
        );
        rec(c, "collector", "leaf_out_of_order", || {
            ac.get_leaf_collector(&ctx)
                .map_err(|e| e.into_lucene_error())?;
            Ok("true".to_string())
        });
    }
    let leaf = |min: i32, max: i32| AssertingLeafCollector::new(Box::new(Noop), min, max);
    let mut l1 = leaf(0, 10);
    rec(c, "leaf", "out_of_range_max", || collect(&mut l1, 10));
    let mut l2 = leaf(2, 10);
    rec(c, "leaf", "out_of_range_min", || collect(&mut l2, 1));
    let mut l3 = leaf(0, 10);
    rec(c, "leaf", "out_of_order", || {
        collect(&mut l3, 4)?;
        collect(&mut l3, 4)
    });
    let mut l4 = leaf(0, 10);
    rec(c, "leaf", "range_empty", || {
        l4.collect_range(5, 5, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?;
        Ok("null".to_string())
    });
    let mut l5 = leaf(0, 10);
    rec(c, "leaf", "range_beyond", || {
        l5.collect_range(3, 12, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?;
        Ok("null".to_string())
    });
    let mut l6 = leaf(0, 10);
    rec(c, "leaf", "range_then_collect", || {
        l6.collect_range(3, 6, &mut SimpleScorable::new())
            .map_err(|e| e.into_lucene_error())?;
        collect(&mut l6, 5)
    });
    let mut l7 = leaf(0, 10);
    rec(c, "leaf", "finish_twice", || {
        l7.finish()?;
        l7.finish()?;
        Ok("null".to_string())
    });

    // AssertingBulkScorer
    let bulk = |seed: i64| {
        AssertingBulkScorer::wrap(
            Arc::new(JdkRandom::new(seed)),
            Some(Box::new(DefaultBulkScorer::new(term_scorer(
                &plain, &ctx, "w7",
            )))),
            max_doc,
        )
        .unwrap()
    };
    let bs = bulk(21);
    let before = &*bs as *const dyn BulkScorer as *const ();
    let mut bs = AssertingBulkScorer::wrap(Arc::new(JdkRandom::new(1)), Some(bs), max_doc).unwrap();
    let after = &*bs as *const dyn BulkScorer as *const ();
    rec(c, "bulk", "rewrap", || Ok((before == after).to_string()));
    let mut counting = Counting(0);
    rec(c, "bulk", "score_first", || {
        let next = bs
            .score(&mut counting, None, 0, 100)
            .map_err(|e| e.into_lucene_error())?;
        Ok(format!("{next} {}", counting.0))
    });
    rec(c, "bulk", "score_rest", || {
        let next = bs
            .score(&mut counting, None, 100, NO_MORE_DOCS)
            .map_err(|e| e.into_lucene_error())?;
        Ok(format!("{next} {}", counting.0))
    });
    let mut bs2 = bulk(22);
    let mut bulk_score = |min: i32, max: i32| -> Result<String> {
        Ok(bs2
            .score(&mut Noop, None, min, max)
            .map_err(|e| e.into_lucene_error())?
            .to_string())
    };
    rec(c, "bulk", "window", || bulk_score(10, 20));
    rec(c, "bulk", "backward", || bulk_score(5, 30));
    rec(c, "bulk", "inverted", || bulk_score(40, 35));

    // AssertingWeight / AssertingMatches
    let (mut asserting, _overrides) = asserting_searcher(31, reader);
    asserting.set_query_cache(None);
    let mw = asserting
        .create_weight(
            asserting.rewrite(term("w7")).unwrap(),
            ScoreMode::COMPLETE,
            1.0,
        )
        .unwrap();
    let doc = term_scorer(&plain, &ctx, "w7")
        .iterator()
        .next_doc()
        .unwrap();
    let matches = mw.matches(&ctx, doc).unwrap().unwrap();
    rec(c, "matches", "class", || {
        let name = if matches.as_any().is::<AssertingMatches>() {
            "AssertingMatches"
        } else {
            "?"
        };
        let unwrapped = AssertingMatches::un_wrap(Arc::clone(&matches));
        Ok(format!("{name} {}", !Arc::ptr_eq(&unwrapped, &matches)))
    });
    let mut mi = matches.get_matches("body").unwrap().unwrap();
    rec(c, "matches", "start_unpositioned", || {
        Ok(mi.start_position().to_string())
    });
    rec(c, "matches", "offset_unpositioned", || {
        Ok(mi.start_offset()?.to_string())
    });
    rec(c, "matches", "next", || Ok(mi.next()?.to_string()));
    rec(c, "matches", "position", || {
        Ok(format!("{}-{}", mi.start_position(), mi.end_position()))
    });
    rec(c, "matches", "drain", || {
        let mut n = 0;
        while mi.next()? {
            n += 1;
        }
        Ok(n.to_string())
    });
    rec(c, "matches", "next_exhausted", || {
        Ok(mi.next()?.to_string())
    });
    rec(c, "matches", "query_exhausted", || {
        Ok(mi.get_query().to_query_string(""))
    });
    rec(c, "matches", "other_field", || {
        Ok(match matches.get_matches("id")? {
            None => "null".to_string(),
            Some(_) => "?".to_string(),
        })
    });
    let count_weight = AssertingWeight::new(
        Arc::new(JdkRandom::new(32)),
        Arc::new(CountTooLarge(Arc::clone(&all_weight))),
        ScoreMode::COMPLETE_NO_SCORES,
    );
    rec(c, "weight", "count_too_large", || {
        Ok(count_weight.count(&ctx)?.to_string())
    });
    rec(c, "weight", "count", || Ok(mw.count(&ctx)?.to_string()));
    rec(c, "weight", "supplier_get_twice", || {
        let mut supplier = mw.scorer_supplier(&ctx)?.unwrap();
        supplier.get(1)?;
        supplier.get(1)?;
        Ok("?".to_string())
    });
    rec(c, "weight", "supplier_negative_lead", || {
        mw.scorer_supplier(&ctx)?.unwrap().get(-1)?;
        Ok("?".to_string())
    });
    rec(c, "weight", "supplier_top_level_after_get", || {
        let mut supplier = mw.scorer_supplier(&ctx)?.unwrap();
        supplier.get(1)?;
        supplier.set_top_level_scoring_clause()?;
        Ok("null".to_string())
    });
    rec(c, "query", "negative_boost", || {
        AssertingQuery::new(Arc::new(JdkRandom::new(33)), term("w7")).create_weight(
            &plain,
            ScoreMode::COMPLETE,
            -1.0,
        )?;
        Ok("?".to_string())
    });
    rec(c, "query", "wrap_twice", || {
        let aq = AssertingQuery::wrap(Arc::new(JdkRandom::new(34)), term("w7"));
        let again = AssertingQuery::wrap(Arc::new(JdkRandom::new(35)), Arc::clone(&aq));
        Ok(Arc::ptr_eq(&aq, &again).to_string())
    });
    rec(c, "searcher", "search_plain_weight", || {
        let partitions: Vec<LeafReaderContextPartition> = leaves
            .iter()
            .map(|leaf| LeafReaderContextPartition::create_for_entire_segment(Arc::clone(leaf)))
            .collect::<Result<_>>()?;
        asserting.search_partitions(
            &partitions,
            &all_weight,
            &mut DummyTotalHitCountCollector::new(),
        )?;
        Ok("null".to_string())
    });

    // RandomTwoPhaseView
    let mut view =
        RandomTwoPhaseView::new(Arc::new(JdkRandom::new(41)), Box::new(all(20).unwrap()));
    rec(c, "two_phase_view", "match_cost", || {
        Ok(bits(view.match_cost()))
    });
    rec(c, "two_phase_view", "matches_unpositioned", || {
        Ok(view.matches()?.to_string())
    });
    rec(c, "two_phase_view", "next", || {
        Ok(view.approximation().next_doc()?.to_string())
    });
    rec(c, "two_phase_view", "matches", || {
        Ok(view.matches()?.to_string())
    });
    rec(c, "two_phase_view", "matches_twice", || {
        Ok(view.matches()?.to_string())
    });
    rec(c, "two_phase_view", "advance", || {
        let advanced = view.approximation().advance(7)?;
        Ok(format!("{advanced} {}", view.matches()?))
    });
    rec(c, "two_phase_view", "run_end", || {
        Ok(view.doc_id_run_end()?.to_string())
    });

    // The small collectors and wrappers
    rec(c, "small", "fixed_bit_set", || {
        let bits = plain.search_with_manager(
            term("w7"),
            &FixedBitSetCollector::create_manager(reader.max_doc()),
        )?;
        let mut sb = bits.cardinality().to_string();
        for i in 0..bits.length() {
            if bits.get(i) {
                sb.push_str(&format!(" {i}"));
            }
        }
        Ok(sb)
    });
    rec(c, "small", "dummy_total", || {
        Ok(plain
            .search_with_manager(term("w7"), &DummyTotalHitCountCollector::create_manager())?
            .to_string())
    });
    rec(c, "small", "disabling_no_match", || {
        Ok(plain
            .search(Arc::new(DisablingBulkScorerQuery::new(term("zzz"))), 10)?
            .total_hits
            .to_string())
    });
    let bulk_wrapper = || -> Result<BulkScorerWrapperScorer> {
        let w = plain.create_weight(plain.rewrite(should_w1_w2())?, ScoreMode::COMPLETE, 1.0)?;
        Ok(BulkScorerWrapperScorer::new(
            w.bulk_scorer(&ctx)?.unwrap(),
            3,
        ))
    };
    rec(c, "small", "bulk_wrapper_iterate", || {
        let mut bw = bulk_wrapper()?;
        let mut sb = bw.iterator().cost().to_string();
        let mut d = bw.iterator().next_doc()?;
        while d != NO_MORE_DOCS {
            sb.push_str(&format!(" {d}:{}", bits(bw.score()?)));
            d = bw.iterator().next_doc()?;
        }
        Ok(sb)
    });
    rec(c, "small", "bulk_wrapper_advance", || {
        let mut bw = bulk_wrapper()?;
        let a = bw.iterator().advance(50)?;
        let b = bw.iterator().next_doc()?;
        let e = bw.iterator().advance(NO_MORE_DOCS - 1)?;
        Ok(format!("{a} {b} {e} {}", Scorer::doc_id(&bw)))
    });
}

/// `SearchWrappersOracle.similarities()`.
fn similarities() {
    let fields = [
        "id", "body", "tv", "p_int", "dv_num", "f0", "f1", "f2", "f3", "f4", "f5",
    ];
    for seed in [0i64, 1, 2, 3, 42, -7, 123_456_789] {
        let r = JdkRandom::new(seed);
        let rs = RandomSimilarity::new(&r);
        let scenario = format!("random{seed}");
        for f in fields {
            rec("sim", &scenario, f, || Ok(rs.get_to_string(f).to_string()));
        }
        rec("sim", &scenario, "toString", || Ok(rs.to_string()));
        rec("sim", &scenario, "next", || Ok(r.next_long().to_string()));
    }
    rec("sim", "asserting", "toString", || {
        Ok(AssertingSimilarity::new(Arc::new(BM25Similarity::default())).to_string())
    });
}

#[test]
fn search_wrappers_match_lucene() {
    let class = Class::builder("org.apache.lucene.tests.search.TestSearchWrappersScenarios")
        .constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("classNameRule", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::clone(&CLASS_NAME_RULE) as Arc<dyn randomizedtesting::junit::TestRule>
                )
            })
            .annotate(ClassRule),
        )
        .method(
            Method::new("testScenarios")
                .annotate(Test::new())
                .body_any(|| {
                    scenarios();
                    rec("ctx", "context", "next", || {
                        Ok(LuceneTestCase::random().next_long().to_string())
                    });
                }),
        )
        .build();
    let class = Arc::new(class);
    ROWS.lock().unwrap().clear();
    system::set_property("tests.seed", SEED);
    let result = JUnitCore::new().run(&Request::a_class(&class));
    system::clear_property("tests.seed");
    if let Some(f) = result.failures().first() {
        panic!("suite failure: {}", f.exception());
    }
    similarities();

    let mut expected = BTreeMap::new();
    for r in rows("search_wrappers.tsv") {
        expected.insert(format!("{}\t{}\t{}", r[0], r[1], r[2]), r[3..].join("\t"));
    }
    let actual = ROWS.lock().unwrap().clone();
    assert_same("search wrappers", &expected, &actual);
}
