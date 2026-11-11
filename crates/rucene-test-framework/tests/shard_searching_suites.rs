//! The binding of `ShardSearchingTestBase` (rmp #356): Lucene's own
//! subclass `org.apache.lucene.search.TestShardSearching`
//! (`lucene/core/src/test/org/apache/lucene/search/TestShardSearching.java`),
//! ported in full, since the base class declares no test and the
//! subclass's `testSimple` is the only test that runs on it.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs the suite on the first `n` seeds of
//! `suite_seeds::SEEDS`.
//!
//! No seeded-draw oracle binds this suite: `testSimple` runs for a wall-clock
//! time (`endTimeNanos`) against a concurrent `ChangeIndices` thread, so the
//! number of draws of a seed is not reproducible, in Java either.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, MultiReader};
use rucene::search::{
    IndexSearcher, PrefixQuery, Query, ScoreDoc, Sort, SortField, SortFieldType, TermQuery,
    TopDocs, TopFieldDocs, TotalHits,
};
use rucene::util::BytesRef;
use rucene_test_framework::search::shard_searching_test_base::{
    self, is_searcher_expired, nano_time, searcher_expired, ShardIndexSearcher,
    ShardSearchingTestBase,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::test_util::TopDocsView;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_seeds;
use suite_seeds::run_suite;

/// The instance of `TestShardSearching`: the fields of
/// `ShardSearchingTestBase`.
struct ShardSearchingInstance {
    base: Arc<ShardSearchingTestBase>,
}

/// `PreviousSearchState` (`private static class`).
struct PreviousSearchState {
    search_time_nanos: i64,
    versions: Vec<i64>,
    search_after_local: Option<ScoreDoc>,
    search_after_shard: Option<ScoreDoc>,
    sort: Option<Sort>,
    query: Arc<dyn Query>,
    num_hits_paged: i32,
}

impl PreviousSearchState {
    fn new(
        query: Arc<dyn Query>,
        sort: Option<Sort>,
        search_after_local: Option<ScoreDoc>,
        search_after_shard: Option<ScoreDoc>,
        versions: &[i64],
        num_hits_paged: i32,
    ) -> Self {
        PreviousSearchState {
            versions: versions.to_vec(),
            search_after_local,
            search_after_shard,
            sort,
            query,
            num_hits_paged,
            search_time_nanos: nano_time(),
        }
    }
}

/// `random()`.
fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

/// `String.valueOf(o)` of an optional object.
fn or_null<T: std::fmt::Display>(o: &Option<T>) -> String {
    o.as_ref()
        .map_or_else(|| "null".to_string(), ToString::to_string)
}

/// The hits of a search: `TopDocs`, or `TopFieldDocs` for a sorted one.
enum Hits {
    Docs(TopDocs),
    Fields(TopFieldDocs),
}

impl Hits {
    fn total_hits(&self) -> &TotalHits {
        match self {
            Hits::Docs(d) => &d.total_hits,
            Hits::Fields(f) => &f.total_hits,
        }
    }

    fn len(&self) -> usize {
        match self {
            Hits::Docs(d) => d.score_docs.len(),
            Hits::Fields(f) => f.score_docs.len(),
        }
    }

    fn score_doc(&self, i: usize) -> &ScoreDoc {
        match self {
            Hits::Docs(d) => &d.score_docs[i],
            Hits::Fields(f) => &f.score_docs[i].score_doc,
        }
    }

    fn score_doc_mut(&mut self, i: usize) -> &mut ScoreDoc {
        match self {
            Hits::Docs(d) => &mut d.score_docs[i],
            Hits::Fields(f) => &mut f.score_docs[i].score_doc,
        }
    }

    fn view(&self) -> TopDocsView<'_> {
        match self {
            Hits::Docs(d) => TopDocsView::from(d),
            Hits::Fields(f) => TopDocsView::from(f),
        }
    }
}

/// `prefix = t.substring(0, end)` on the UTF-16 code units of `t`, then the
/// UTF-8 bytes `new Term(field, prefix)` holds: an unpaired surrogate
/// becomes U+FFFD, as `UnicodeUtil.UTF16toUTF8` writes it.
fn utf16_prefix(t: &str, end: usize) -> String {
    let units: Vec<u16> = t.encode_utf16().take(end).collect();
    String::from_utf16_lossy(&units)
}

/// `testSimple()`.
fn test_simple(inst: &ShardSearchingInstance) -> Result<()> {
    let base = &inst.base;
    let num_nodes = TestUtil::next_int(&*random(), 1, 10);

    let run_time_sec = f64::from(if *TEST_NIGHTLY {
        LuceneTestCase::at_least(5)
    } else {
        LuceneTestCase::at_least(1)
    });

    let min_docs_to_make_terms = TestUtil::next_int(&*random(), 5, 20);

    let max_searcher_age_seconds = TestUtil::next_int(&*random(), 1, 3);

    if *VERBOSE {
        println!(
            "TEST: numNodes={num_nodes} runTimeSec={} maxSearcherAgeSeconds={max_searcher_age_seconds}",
            rucene::util::java_fmt::double_to_string(run_time_sec)
        );
    }

    base.start(num_nodes, run_time_sec, max_searcher_age_seconds)?;

    let mut prior_searches: Vec<Arc<PreviousSearchState>> = Vec::new();
    let mut terms: Option<Vec<BytesRef>> = None;
    while nano_time() < base.end_time_nanos() {
        let do_followon = !prior_searches.is_empty() && random().next_int_bound(7) == 1;

        // Pick a random node; we will run the query on this node:
        let my_node_id = random().next_int_bound(num_nodes) as usize;

        let local_shard_searcher: Arc<ShardIndexSearcher>;

        let prev_search_state: Option<Arc<PreviousSearchState>>;

        if do_followon {
            // Pretend user issued a followon query:
            let prev = Arc::clone(
                &prior_searches[random().next_int_bound(prior_searches.len() as i32) as usize],
            );

            if *VERBOSE {
                println!(
                    "\nTEST: follow-on query age={}",
                    rucene::util::java_fmt::double_to_string(
                        (nano_time() - prev.search_time_nanos) as f64
                            / Duration::from_secs(1).as_nanos() as f64
                    )
                );
            }

            match base.nodes()[my_node_id].acquire_versions(&prev.versions) {
                Ok(s) => local_shard_searcher = s,
                Err(see) if is_searcher_expired(&see) => {
                    // Expected, sometimes; in a "real" app we would
                    // either forward this error to the user ("too
                    // much time has passed; please re-run your
                    // search") or sneakily just switch to newest
                    // searcher w/o telling them...
                    if *VERBOSE {
                        println!("  searcher expired during local shard searcher init: {see}");
                    }
                    remove_state(&mut prior_searches, &prev);
                    continue;
                }
                Err(e) => return Err(e),
            }
            prev_search_state = Some(prev);
        } else {
            if *VERBOSE {
                println!("\nTEST: fresh query");
            }
            // Do fresh query:
            local_shard_searcher = base.nodes()[my_node_id].acquire();
            prev_search_state = None;
        }

        let mut subs: Vec<Option<Arc<dyn IndexReader>>> = vec![None; num_nodes as usize];

        let mut search_state: Option<PreviousSearchState> = None;

        let body = (|| -> Result<()> {
            // Mock: now make a single reader (MultiReader) from all node
            // searchers.  In a real shard env you can't do this... we
            // do it to confirm results from the shard searcher
            // are correct:
            let mut doc_count = 0;
            let init = (|| -> Result<()> {
                let mut node_id = 0i32;
                while node_id < num_nodes {
                    let sub_version = local_shard_searcher.node_versions()[node_id as usize];
                    let sub = base.nodes()[node_id as usize]
                        .searchers()
                        .acquire(sub_version)?;
                    let Some(sub) = sub else {
                        node_id -= 1;
                        while node_id >= 0 {
                            subs[node_id as usize]
                                .as_ref()
                                .expect("acquired above")
                                .dec_ref()?;
                            subs[node_id as usize] = None;
                            node_id -= 1;
                        }
                        return Err(searcher_expired(format!(
                            "nodeID={node_id} version={sub_version}"
                        )));
                    };
                    let reader = Arc::clone(sub.searcher().get_index_reader());
                    doc_count += reader.max_doc();
                    subs[node_id as usize] = Some(reader);
                    node_id += 1;
                }
                Ok(())
            })();
            match init {
                Ok(()) => {}
                Err(see) if is_searcher_expired(&see) => {
                    // Expected
                    if *VERBOSE {
                        println!("  searcher expired during mock reader init: {see}");
                    }
                    return Ok(());
                }
                Err(e) => return Err(e),
            }

            let mock_reader: Arc<dyn IndexReader> = Arc::new(MultiReader::new(
                subs.iter()
                    .map(|s| Arc::clone(s.as_ref().expect("acquired above")))
                    .collect(),
                true,
            )?);
            let mock_searcher = IndexSearcher::new(Arc::clone(&mock_reader))?;

            let query: Option<Arc<dyn Query>>;
            let sort: Option<Sort>;

            if let Some(prev) = &prev_search_state {
                query = Some(Arc::clone(&prev.query));
                sort = prev.sort.clone();
            } else {
                if terms.is_none() && doc_count > min_docs_to_make_terms {
                    // TODO: try to "focus" on high freq terms sometimes too
                    // TODO: maybe also periodically reset the terms...?
                    let multi_terms =
                        MultiTerms::get_terms(&mock_reader, "body")?.unwrap_or_else(|| {
                            randomizedtesting::jdk::exception::throw_without_message(
                                "java.lang.NullPointerException",
                            )
                        });
                    let mut terms_enum = multi_terms.iterator()?;
                    let mut collected = Vec::new();
                    while let Some(term) = terms_enum.next()? {
                        collected.push(BytesRef::deep_copy_of(&term));
                    }
                    if *VERBOSE {
                        println!("TEST: init terms: {} terms", collected.len());
                    }
                    terms = if collected.is_empty() {
                        None
                    } else {
                        Some(collected)
                    };
                }

                if *VERBOSE {
                    println!("  maxDoc={}", mock_reader.max_doc());
                }

                if let Some(terms) = &terms {
                    if random().next_boolean() {
                        query = Some(Arc::new(TermQuery::new(Term::new(
                            "body",
                            terms[random().next_int_bound(terms.len() as i32) as usize].clone(),
                        ))));
                    } else {
                        let t = terms[random().next_int_bound(terms.len() as i32) as usize]
                            .utf8_to_string()?;
                        let prefix = if t.encode_utf16().count() <= 1 {
                            t
                        } else {
                            utf16_prefix(&t, TestUtil::next_int(&*random(), 1, 2) as usize)
                        };
                        query = Some(Arc::new(PrefixQuery::new(Term::new(
                            "body",
                            BytesRef::from_char_sequence(&prefix),
                        ))?));
                    }

                    if random().next_boolean() {
                        sort = None;
                    } else {
                        // TODO: sort by more than 1 field
                        let what = random().next_int_bound(3);
                        if what == 0 {
                            sort = Some(Sort::new_fields(vec![SortField::FIELD_SCORE.clone()])?);
                        } else if what == 1 {
                            // TODO: this sort doesn't merge
                            // correctly... it's tricky because you
                            // could have > 2.1B docs across all shards:
                            // sort = new Sort(SortField.FIELD_DOC);
                            sort = None;
                        } else if what == 2 {
                            sort = Some(Sort::new_fields(vec![SortField::new_reverse(
                                Some("docid_intDV".to_string()),
                                SortFieldType::Int,
                                random().next_boolean(),
                            )?])?);
                        } else {
                            sort = Some(Sort::new_fields(vec![SortField::new_reverse(
                                Some("titleDV".to_string()),
                                SortFieldType::String,
                                random().next_boolean(),
                            )?])?);
                        }
                    }
                } else {
                    query = None;
                    sort = None;
                }
            }

            if let Some(query) = query {
                match assert_same(
                    &mock_searcher,
                    &local_shard_searcher,
                    query,
                    sort,
                    prev_search_state.as_deref(),
                ) {
                    Ok(state) => search_state = state,
                    Err(see) if is_searcher_expired(&see) => {
                        // Expected; in a "real" app we would
                        // either forward this error to the user ("too
                        // much time has passed; please re-run your
                        // search") or sneakily just switch to newest
                        // searcher w/o telling them...
                        if *VERBOSE {
                            println!("  searcher expired during search: {see}");
                            println!("{see:?}");
                        }
                        // We can't do this in general: on a very slow
                        // computer it's possible the local searcher
                        // expires before we can finish our search:
                        // assert prevSearchState != null;
                        if let Some(prev) = &prev_search_state {
                            remove_state(&mut prior_searches, prev);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })();
        // finally
        let released = base.nodes()[my_node_id].release(&local_shard_searcher);
        let mut dec_refs: Result<()> = Ok(());
        for sub in subs.iter().flatten() {
            if dec_refs.is_ok() {
                dec_refs = sub.dec_ref();
            }
        }
        body?;
        released?;
        dec_refs?;

        if let Some(state) = search_state {
            if state.search_after_local.is_some() && random().next_int_bound(5) == 3 {
                prior_searches.push(Arc::new(state));
                if prior_searches.len() > 200 {
                    shuffle(&mut prior_searches, &*random());
                    prior_searches.truncate(100);
                }
            }
        }
    }

    base.finish()
}

/// `priorSearches.remove(prevSearchState)`: by identity, as
/// `PreviousSearchState` does not override `equals`.
fn remove_state(
    prior_searches: &mut Vec<Arc<PreviousSearchState>>,
    state: &Arc<PreviousSearchState>,
) {
    if let Some(i) = prior_searches.iter().position(|s| Arc::ptr_eq(s, state)) {
        prior_searches.remove(i);
    }
}

/// `assertSame(IndexSearcher mockSearcher, NodeState.ShardIndexSearcher
/// shardSearcher, Query q, Sort sort, PreviousSearchState state)`
/// (`private`).
fn assert_same(
    mock_searcher: &IndexSearcher,
    shard_searcher: &ShardIndexSearcher,
    q: Arc<dyn Query>,
    sort: Option<Sort>,
    state: Option<&PreviousSearchState>,
) -> Result<Option<PreviousSearchState>> {
    let mut num_hits = TestUtil::next_int(&*random(), 1, 100);
    if let Some(state) = state {
        if state.search_after_local.is_none() {
            // In addition to what we last searched:
            num_hits += state.num_hits_paged;
        }
    }

    if *VERBOSE {
        println!(
            "TEST: query={} sort={} numHits={num_hits}",
            q.to_query_string(""),
            or_null(&sort)
        );
        if let Some(state) = state {
            println!(
                "  prev: searchAfterLocal={} searchAfterShard={} numHitsPaged={}",
                or_null(&state.search_after_local),
                or_null(&state.search_after_shard),
                state.num_hits_paged
            );
        }
    }

    // Single (mock local) searcher:
    let hits = match &sort {
        None => match state.and_then(|s| s.search_after_local) {
            Some(after) => {
                Hits::Docs(mock_searcher.search_after(Some(after), Arc::clone(&q), num_hits)?)
            }
            None => Hits::Docs(mock_searcher.search(Arc::clone(&q), num_hits)?),
        },
        Some(sort) => {
            Hits::Fields(mock_searcher.search_with_sort(Arc::clone(&q), num_hits, sort)?)
        }
    };

    // Shard searcher
    let mut shard_hits = match &sort {
        None => match state.and_then(|s| s.search_after_shard) {
            Some(after) => {
                Hits::Docs(shard_searcher.search_after(Some(after), Arc::clone(&q), num_hits)?)
            }
            None => Hits::Docs(shard_searcher.search(Arc::clone(&q), num_hits)?),
        },
        Some(sort) => {
            Hits::Fields(shard_searcher.search_with_sort(Arc::clone(&q), num_hits, sort)?)
        }
    };

    let num_nodes = shard_searcher.node_versions().len();
    let mut base = vec![0i32; num_nodes];
    let subs = mock_searcher
        .get_top_reader_context()
        .children()
        .unwrap_or_default();
    Assert::assert_equals(num_nodes as i32, subs.len() as i32);

    for node_id in 0..num_nodes {
        base[node_id] = subs[node_id].doc_base_in_parent();
    }

    if *VERBOSE {
        /*
        for(int shardID=0;shardID<shardSearchers.length;shardID++) {
          System.out.println("  shard=" + shardID + " maxDoc=" + shardSearchers[shardID].searcher.getIndexReader().maxDoc());
        }
        */
        println!("  single searcher: {}", hits.total_hits().value());
        for i in 0..hits.len() {
            let sd = hits.score_doc(i);
            println!(
                "    doc={} score={}",
                sd.doc,
                rucene::util::java_fmt::float_to_string(sd.score)
            );
        }
        println!("  shard searcher: {}", shard_hits.total_hits().value());
        for i in 0..shard_hits.len() {
            let sd = shard_hits.score_doc(i);
            println!(
                "    doc={} (rebased: {}) score={} shard={}",
                sd.doc,
                sd.doc + base[sd.shard_index as usize],
                rucene::util::java_fmt::float_to_string(sd.score),
                sd.shard_index
            );
        }
    }

    let mut num_hits_paged = hits.len() as i32;
    if let Some(state) = state {
        if state.search_after_local.is_some() {
            num_hits_paged += state.num_hits_paged;
        }
    }

    let more_hits;

    let bottom_hit: Option<ScoreDoc>;
    let bottom_hit_shards: Option<ScoreDoc>;

    if i64::from(num_hits_paged) < hits.total_hits().value() {
        // More hits to page through
        more_hits = true;
        if sort.is_none() {
            bottom_hit = Some(*hits.score_doc(hits.len() - 1));
            let sd = shard_hits.score_doc(shard_hits.len() - 1);
            // Must copy because below we rebase:
            bottom_hit_shards = Some(ScoreDoc::with_shard_index(sd.doc, sd.score, sd.shard_index));
            if *VERBOSE {
                println!("  save bottomHit={}", or_null(&bottom_hit));
            }
        } else {
            bottom_hit = None;
            bottom_hit_shards = None;
        }
    } else {
        Assert::assert_equals(hits.total_hits().value(), i64::from(num_hits_paged));
        bottom_hit = None;
        bottom_hit_shards = None;
        more_hits = false;
    }

    // Must rebase so assertEquals passes:
    for hit_id in 0..shard_hits.len() {
        let sd = shard_hits.score_doc_mut(hit_id);
        sd.doc += base[sd.shard_index as usize];
    }

    TestUtil::assert_consistent(hits.view(), shard_hits.view());

    if more_hits {
        // Return a continuation:
        Ok(Some(PreviousSearchState::new(
            q,
            sort,
            bottom_hit,
            bottom_hit_shards,
            shard_searcher.node_versions(),
            num_hits_paged,
        )))
    } else {
        Ok(None)
    }
}

/// `TestShardSearching` (`@SuppressCodecs({"SimpleText", "Direct"}) public
/// class TestShardSearching extends ShardSearchingTestBase`).
fn shard_searching_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.search.TestShardSearching")
        .extends(&shard_searching_test_base::class())
        .annotate(SuppressCodecs {
            value: vec!["SimpleText".to_string(), "Direct".to_string()],
        })
        .constructor(Constructor::no_args(|| ShardSearchingInstance {
            base: ShardSearchingTestBase::new(),
        }))
        .method(
            Method::new("testSimple")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut ShardSearchingInstance| {
                    test_simple(inst).unwrap_or_else(|e: LuceneError| rethrow_java(e))
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_shard_searching() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(shard_searching_class);
}
