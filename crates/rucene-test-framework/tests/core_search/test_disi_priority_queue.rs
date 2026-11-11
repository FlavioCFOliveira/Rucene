//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDisiPriorityQueue.java`.
//!
//! **Divergence from Lucene 10.5.0.** Rucene's `DisiPriorityQueue` holds the
//! positions of its entries in a caller-owned `DisiWrapper` slice (an arena)
//! instead of references to the wrappers, so `assertSame(w, pq.…)` compares
//! positions, the wrappers' `doc` fields are written through the slice, and
//! `topList()`'s `next` chain is the list of positions it returns.

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::multi_reader::MultiReader;
use rucene::index::IndexReader;
use rucene::index::LeafReaderContext;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::disi_priority_queue::DisiPriorityQueue;
use rucene::search::disi_wrapper::DisiWrapper;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{Query, ScoreMode, NO_MORE_DOCS};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;

use crate::core_search::support::null_leaf_context;
use crate::core_support::{assert_equals, assert_null, class, expect_throws_panic, random, test};

/// `TestDisiPriorityQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDisiPriorityQueue",
        vec![
            test("testDisiPriorityQueue2", test_disi_priority_queue2),
            test("testRandom", test_random),
        ],
    )
}

fn test_disi_priority_queue2() -> Result<()> {
    let r = random();
    // w1, w2, w3: positions 0, 1, 2 of the wrapper slice.
    let mut wrappers = vec![
        wrapper(random_disi(&*r))?,
        wrapper(random_disi(&*r))?,
        wrapper(random_disi(&*r))?,
    ];
    let (w1, w2, w3) = (0usize, 1usize, 2usize);

    let mut pq = DisiPriorityQueue::of_max_size(2);
    wrappers[w1].doc = 1;
    wrappers[w2].doc = 0;
    assert_null(&pq.top());
    assert_equals(&0, &pq.size());
    assert_equals(&w1, &pq.add(&wrappers, w1));
    assert_equals(&Some(w1), &pq.top());
    assert_equals(&1, &pq.size());
    assert_equals(&w2, &pq.add(&wrappers, w2));
    assert_equals(&Some(w2), &pq.top());
    assert_equals(&2, &pq.size());
    expect_throws_panic("java.lang.IllegalStateException", || {
        pq.add(&wrappers, w3);
    });

    wrappers[w2].doc = 1;
    assert_equals(&Some(w2), &pq.update_top(&wrappers));
    let top_list = pq.top_list(&wrappers);
    assert_equals(&w1, &top_list[0]);
    assert_equals(&w2, &top_list[1]);
    assert_equals(&2, &top_list.len()); // assertNull(topList.next.next)

    wrappers[w2].doc = 2;
    assert_equals(&Some(w1), &pq.update_top(&wrappers));
    let top_list = pq.top_list(&wrappers);
    assert_equals(&w1, &top_list[0]);
    assert_equals(&1, &top_list.len()); // assertNull(topList.next)

    assert_equals(&Some(w1), &pq.pop(&wrappers));
    assert_equals(&Some(w2), &pq.top());
    Ok(())
}

fn test_random() -> Result<()> {
    let r = random();

    let size = r.next_int_range(1, if *TEST_NIGHTLY { 1000 } else { 10 });
    let mut all: Vec<DisiWrapper> = Vec::with_capacity(size as usize);
    for _i in 0..size {
        let it = random_disi(&*r);
        let w = wrapper(it)?;
        all.push(w);
    }
    // `all` is the arena; `order` is Java's `DisiWrapper[] all`, which the
    // loop below sorts.
    let mut order: Vec<usize> = (0..size as usize).collect();

    let mut pq = DisiPriorityQueue::of_max_size(size as usize);
    if r.next_boolean() {
        for &w in &order {
            pq.add(&all, w);
        }
    } else if r.next_int_bound(10) < 2 && size > 1 {
        let len = random().next_int_range(1, size) as usize;
        for i in 0..len {
            pq.add(&all, order[i]);
        }
        pq.add_all(&all, &order[len..size as usize]);
    } else {
        pq.add_all(&all, &order[0..size as usize]);
    }

    while pq.size() > 0 {
        // `Arrays.sort(all, Comparator.comparingInt(w -> w.doc))`: stable.
        order.sort_by_key(|&w| all[w].doc);
        let top = pq.top().expect("size > 0");
        assert_equals(&all[order[0]].doc, &all[top].doc);
        all[top].doc = all[top].iterator().next_doc()?;
        if all[top].doc == NO_MORE_DOCS {
            pq.pop(&all);
        } else {
            pq.update_top(&all);
        }
    }
    Ok(())
}

/// `wrapper(iterator)`.
///
/// **Divergence from Lucene 10.5.0.** Java calls
/// `q.createWeight(null, …).scorer(null)`; Rucene's `create_weight` takes a
/// searcher and `scorer` a context by reference, so a searcher over an empty
/// `MultiReader` and the one leaf of a fixed single-document index stand
/// for `null`; `DummyQuery`'s weight reads neither.
fn wrapper(iterator: Box<dyn DocIdSetIterator + Send>) -> Result<DisiWrapper> {
    let q: Arc<dyn Query> = Arc::new(DummyQuery::new(iterator));
    let searcher =
        IndexSearcher::new(Arc::new(MultiReader::new(Vec::new(), true)?) as Arc<dyn IndexReader>)?;
    let (_reader, leaves) = null_leaf_context()?;
    let s = q
        .create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?
        .scorer(&leaves[0])?
        .expect("DummyQuery always has a scorer");
    Ok(DisiWrapper::new(s, random().next_boolean()))
}

/// `randomDisi(r)`: an iterator over `r.ints(maxSize, 0, NO_MORE_DOCS -
/// 1).sorted().distinct()`.
///
/// `Random.ints(long, int, int)` draws each value with `nextInt(origin,
/// bound)`; `sorted()` consumes the stream when the iterator is created, so
/// the draws happen here, in order.
fn random_disi(r: &dyn Random) -> Box<dyn DocIdSetIterator + Send> {
    let max_size = r.next_int_bound(50);
    let mut ints: Vec<i32> = (0..max_size)
        .map(|_| r.next_int_range(0, NO_MORE_DOCS - 1))
        .collect();
    ints.sort_unstable();
    ints.dedup();
    Box::new(RandomDisi {
        doc: -1,
        ints: ints.into_iter(),
        max_size,
    })
}

/// The anonymous `DocIdSetIterator` of `randomDisi`.
struct RandomDisi {
    doc: i32,
    ints: std::vec::IntoIter<i32>,
    max_size: i32,
}

impl DocIdSetIterator for RandomDisi {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = self.ints.next().unwrap_or(NO_MORE_DOCS);
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        while self.doc < target {
            self.next_doc()?;
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.max_size as i64
    }
}

/// `DummyQuery.COUNTER`.
static COUNTER: AtomicI32 = AtomicI32::new(0);

/// `DummyQuery extends Query`.
///
/// Java's query shares its `disi` with the scorer it builds; here the query
/// hands the iterator over to the one scorer the test builds from it.
struct DummyQuery {
    id: i32,
    disi: Arc<Mutex<Option<Box<dyn DocIdSetIterator + Send>>>>,
}

impl DummyQuery {
    fn new(disi: Box<dyn DocIdSetIterator + Send>) -> DummyQuery {
        DummyQuery {
            id: COUNTER.fetch_add(1, Ordering::SeqCst),
            disi: Arc::new(Mutex::new(Some(disi))),
        }
    }
}

impl std::fmt::Debug for DummyQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DummyQuery ({})", self.id)
    }
}

impl std::fmt::Debug for DummyWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DummyWeight")
    }
}

impl Query for DummyQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(DummyQuery {
                id: self.id,
                disi: Arc::clone(&self.disi),
            }),
            boost,
            DummyWeight {
                score_mode,
                disi: Arc::clone(&self.disi),
            },
        )))
    }

    fn to_query_string(&self, _field: &str) -> String {
        format!("DummyQuery ({})", self.id)
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DummyQuery>()
                .is_some_and(|o| self.id == o.id)
    }

    fn query_hash(&self) -> u64 {
        self.id as u64
    }
}

/// The anonymous `ConstantScoreWeight` of `DummyQuery.createWeight`.
struct DummyWeight {
    score_mode: ScoreMode,
    disi: Arc<Mutex<Option<Box<dyn DocIdSetIterator + Send>>>>,
}

impl ConstantScoreWeightImpl for DummyWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let disi = self
            .disi
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .expect("the test builds one scorer per DummyQuery");
        let scorer = ConstantScoreScorer::from_iterator(score, self.score_mode, disi);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}
