//! Port of `lucene/core/src/test/org/apache/lucene/search/TestScorerPerf.java`.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::LeafReaderContext;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{
    BooleanQueryBuilder, CollectionResult, Collector, CollectorManager, LeafCollector, Occur,
    Query, ScoreMode,
};
use rucene::util::bit_set::{BitSet, BitSetIterator};
use rucene::util::FixedBitSet;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, verbose};

/// `TestScorerPerf`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestScorerPerf",
        vec![test("testConjunctions", test_conjunctions)],
    )
}

/// `private final boolean validate = true; // set to false when doing performance testing`
const VALIDATE: bool = true;

/// `private static FixedBitSet randBitSet(int sz, int numBitsToSet)`.
fn rand_bit_set(sz: i32, num_bits_to_set: i32) -> FixedBitSet {
    let mut set = FixedBitSet::new(sz as usize);
    for _i in 0..num_bits_to_set {
        set.set(random().next_int_bound(sz) as usize);
    }
    set
}

/// `private static FixedBitSet[] randBitSets(int numSets, int setSize)`.
fn rand_bit_sets(num_sets: i32, set_size: i32) -> Vec<Arc<FixedBitSet>> {
    let mut sets = Vec::with_capacity(num_sets as usize);
    for _i in 0..num_sets {
        sets.push(Arc::new(rand_bit_set(
            set_size,
            random().next_int_bound(set_size),
        )));
    }
    sets
}

/// `private record CountingHitCollectorManager()`.
struct CountingHitCollectorManager;

impl CollectorManager for CountingHitCollectorManager {
    type Collector = CountingHitCollector;
    type Output = CountingHitCollector;

    fn new_collector(&self) -> Result<CountingHitCollector> {
        Ok(CountingHitCollector::default())
    }

    fn reduce(&self, collectors: Vec<CountingHitCollector>) -> Result<CountingHitCollector> {
        let mut result = CountingHitCollector::default();
        for collector in collectors {
            result.leaf.count += collector.leaf.count;
            result.leaf.sum = result.leaf.sum.wrapping_add(collector.leaf.sum);
        }
        Ok(result)
    }
}

/// The leaf side of `CountingHitCollector`, a `SimpleCollector` that is its
/// own leaf collector.
#[derive(Debug, Default)]
struct CountingLeaf {
    count: i32,
    sum: i32,
    doc_base: i32,
}

impl LeafCollector for CountingLeaf {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.count += 1;
        self.sum = self.sum.wrapping_add(self.doc_base + doc); // use it to avoid any possibility of being eliminated by hotspot
        Ok(())
    }
}

/// `private static class CountingHitCollector extends SimpleCollector`.
#[derive(Debug, Default)]
struct CountingHitCollector {
    leaf: CountingLeaf,
}

impl CountingHitCollector {
    fn get_count(&self) -> i32 {
        self.leaf.count
    }
}

impl Collector for CountingHitCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader(context)`.
        self.leaf.doc_base = context.doc_base();
        Ok(Box::new(&mut self.leaf))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// `private static class BitSetQuery extends Query`.
#[derive(Debug, Clone)]
struct BitSetQuery {
    docs: Arc<FixedBitSet>,
}

/// The anonymous `ConstantScoreWeight` of `BitSetQuery.createWeight`.
#[derive(Debug)]
struct BitSetWeight {
    docs: Arc<FixedBitSet>,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for BitSetWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let cost = self.docs.approximate_cardinality() as i64;
        let scorer = ConstantScoreScorer::from_iterator(
            score,
            self.score_mode,
            Box::new(BitSetIterator::new(
                Arc::clone(&self.docs) as Arc<dyn BitSet>,
                cost,
            )?),
        );
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Query for BitSetQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            BitSetWeight {
                docs: Arc::clone(&self.docs),
                score_mode,
            },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        "randomBitSetFilter".to_string()
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<BitSetQuery>()
                .is_some_and(|o| *self.docs == *o.docs)
    }

    fn query_hash(&self) -> u64 {
        (31u64.wrapping_mul(self.class_hash())).wrapping_add(self.docs.hash_code() as u32 as u64)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `private FixedBitSet addClause(FixedBitSet[] sets, BooleanQuery.Builder bq, FixedBitSet result)`.
fn add_clause(
    sets: &[Arc<FixedBitSet>],
    bq: &mut BooleanQueryBuilder,
    mut result: Option<FixedBitSet>,
) -> Result<Option<FixedBitSet>> {
    let rnd = Arc::clone(&sets[random().next_int_bound(sets.len() as i32) as usize]);
    let q: Arc<dyn Query> = Arc::new(BitSetQuery {
        docs: Arc::clone(&rnd),
    });
    bq.add(q, Occur::MUST)?;
    if VALIDATE {
        match result.as_mut() {
            None => result = Some((*rnd).clone()),
            Some(r) => r.and(&rnd),
        }
    }
    Ok(result)
}

/// `private void doConjunctions(IndexSearcher s, FixedBitSet[] sets, int iter, int maxClauses)`.
fn do_conjunctions(
    s: &IndexSearcher,
    sets: &[Arc<FixedBitSet>],
    iter: i32,
    max_clauses: i32,
) -> Result<()> {
    for _i in 0..iter {
        let n_clauses = random().next_int_bound(max_clauses - 1) + 2; // min 2 clauses
        let mut bq = BooleanQueryBuilder::new();
        let mut result = None;
        for _j in 0..n_clauses {
            result = add_clause(sets, &mut bq, result)?;
        }
        let hc = s.search_with_manager(Arc::new(bq.build()), &CountingHitCollectorManager)?;

        if VALIDATE {
            assert_equals(
                &(result.as_ref().expect("two clauses or more").cardinality() as i32),
                &hc.get_count(),
            );
        }
    }
    Ok(())
}

/// `private void doNestedConjunctions(IndexSearcher s, FixedBitSet[] sets, int iter, int maxOuterClauses, int maxClauses)`.
fn do_nested_conjunctions(
    s: &IndexSearcher,
    sets: &[Arc<FixedBitSet>],
    iter: i32,
    max_outer_clauses: i32,
    max_clauses: i32,
) -> Result<()> {
    let mut n_matches: i64 = 0;

    for _i in 0..iter {
        let o_clauses = random().next_int_bound(max_outer_clauses - 1) + 2;
        let mut oq = BooleanQueryBuilder::new();
        let mut result = None;

        for _o in 0..o_clauses {
            let n_clauses = random().next_int_bound(max_clauses - 1) + 2; // min 2 clauses
            let mut bq = BooleanQueryBuilder::new();
            for _j in 0..n_clauses {
                result = add_clause(sets, &mut bq, result)?;
            }

            oq.add(Arc::new(bq.build()), Occur::MUST)?;
        } // outer

        let hc = s.search_with_manager(Arc::new(oq.build()), &CountingHitCollectorManager)?;
        n_matches += i64::from(hc.get_count());
        if VALIDATE {
            assert_equals(
                &(result.as_ref().expect("two clauses or more").cardinality() as i32),
                &hc.get_count(),
            );
        }
    }
    if verbose() {
        println!("Average number of matches={}", n_matches / i64::from(iter));
    }
    Ok(())
}

/// Port of `TestScorerPerf.testConjunctions`.
fn test_conjunctions() -> Result<()> {
    // test many small sets... the bugs will be found on boundary conditions
    let mut d = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let iw = IndexWriter::new(
            d.as_directory(),
            LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(
                &*random(),
            ))),
        )?;
        iw.add_document(&Document::new())?;
        iw.close()?;

        let r = directory_reader::open(d.as_directory())?;
        let inner = (|| -> Result<()> {
            let mut s = LuceneTestCase::new_searcher(Arc::clone(&r));
            s.set_query_cache(None);

            let num_sets = LuceneTestCase::at_least(1000);
            let set_size = LuceneTestCase::at_least(10);
            let sets = rand_bit_sets(num_sets, set_size);

            let iterations = if *TEST_NIGHTLY {
                LuceneTestCase::at_least(10000)
            } else {
                LuceneTestCase::at_least(500)
            };
            let max_clauses = LuceneTestCase::at_least(5);
            do_conjunctions(&s, &sets, iterations, max_clauses)?;
            let max_outer = LuceneTestCase::at_least(3);
            let max_inner = LuceneTestCase::at_least(3);
            do_nested_conjunctions(&s, &sets, iterations, max_outer, max_inner)?;
            Ok(())
        })();
        let closed = r.close();
        inner.and(closed)
    })();
    let closed = d.close();
    result.and(closed)
}
