//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBooleanScorer.java`.

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, LongPoint, Store, StringField};
use rucene::error::Result;
use rucene::index::{IndexOptions, IndexReader, LeafReaderContext};
use rucene::search::batch_score_bulk_scorer::BatchScoreBulkScorer;
use rucene::search::boolean_scorer::BooleanScorer;
use rucene::search::boolean_scorer_supplier::BooleanScorerSupplier;
use rucene::search::bulk_scorer::DefaultBulkScorer;
use rucene::search::constant_score_bulk_scorer::ConstantScoreBulkScorer;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::doc_id_stream::DocIdStream;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::req_excl_bulk_scorer::ReqExclBulkScorer;
use rucene::search::scorable::{Scorable, SimpleScorable};
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::top_docs_collector::TopDocsCollector;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use rucene::search::weight::Weight;
use rucene::search::CollectorManager;
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, BulkScorer, CollectionResult, LeafCollector, Occur, Query,
    ScoreMode, NO_MORE_DOCS,
};
use rucene::util::{ArrayUtil, Bits, FixedBitSet};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::tq;
use crate::core_support::{
    assert_array_equals, assert_equals, assert_equals_msg, class, random, test, IAE,
};

const FIELD: &str = "category";

/// `TestBooleanScorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBooleanScorer",
        vec![
            test("testMethod", test_method),
            test("testEmbeddedBooleanScorer", test_embedded_boolean_scorer),
            test(
                "testOptimizeTopLevelClauseOrNull",
                test_optimize_top_level_clause_or_null,
            ),
            test(
                "testConstantScoreScorerSupplierUsesIntoBitSet",
                test_constant_score_scorer_supplier_uses_into_bit_set,
            ),
            test(
                "testConstantScoreBulkScorerRejectsScoreModeThatNeedsScores",
                test_constant_score_bulk_scorer_rejects_score_mode_that_needs_scores,
            ),
            test(
                "testConstantScoreScorerSupplierBulkCollectsTwoPhaseWithMatcher",
                test_constant_score_scorer_supplier_bulk_collects_two_phase_with_matcher,
            ),
            test(
                "testDefaultBulkScorerDoesNotUseDocIdStreamForTopScores",
                test_default_bulk_scorer_does_not_use_doc_id_stream_for_top_scores,
            ),
            test(
                "testDefaultBulkScorerDoesNotUseDocIdStreamWhenScoresAreNeeded",
                test_default_bulk_scorer_does_not_use_doc_id_stream_when_scores_are_needed,
            ),
            test(
                "testOptimizeProhibitedClauses",
                test_optimize_prohibited_clauses,
            ),
            test(
                "testSparseClauseOptimization",
                test_sparse_clause_optimization,
            ),
            test("testFilterConstantScore", test_filter_constant_score),
            test(
                "testCollectNoThresholdWhenOnlyFilter",
                test_collect_no_threshold_when_only_filter,
            ),
            test(
                "testTopScoresWithMinShouldMatchFallsBackToWand",
                test_top_scores_with_min_should_match_falls_back_to_wand,
            ),
        ],
    )
}

/// `assertThat(actual, instanceOf(class))`, the instance test done by the
/// caller through the opt-in `as_any` pattern.
fn assert_instance_of(is: bool, class: &str) {
    if !is {
        Assert::fail(&format!(
            "\nExpected: an instance of {class}\n     but: an instance of another class"
        ));
    }
}

/// `assertThat(actual, not(instanceOf(class)))`.
fn assert_not_instance_of(is: bool, class: &str) {
    if is {
        Assert::fail(&format!(
            "\nExpected: not an instance of {class}\n     but: an instance of {class}"
        ));
    }
}

/// `UnsupportedOperationException` (no message), thrown from a method whose
/// Rust signature has no error channel.
fn throw_uoe() -> ! {
    Throwable::new("java.lang.UnsupportedOperationException", "").throw()
}

fn test_method() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();

    let values = ["1", "2", "3", "4"];

    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    for i in 0..values.len() {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            FIELD,
            values[i],
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let ir = writer.get_reader()?;
    writer.close()?;

    let mut boolean_query1 = BooleanQueryBuilder::new();
    boolean_query1.add(tq(FIELD, "1"), Occur::SHOULD)?;
    boolean_query1.add(tq(FIELD, "2"), Occur::SHOULD)?;

    let mut query = BooleanQueryBuilder::new();
    query.add(Arc::new(boolean_query1.build()), Occur::MUST)?;
    query.add(tq(FIELD, "9"), Occur::MUST_NOT)?;

    let index_searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let hits = index_searcher
        .search(Arc::new(query.build()), 1000)?
        .score_docs;
    assert_equals_msg(Some("Number of matched documents"), &2, &hits.len());
    ir.close()?;
    directory.close()?;
    Ok(())
}

/// Throws UOE if Weight.scorer is called
#[derive(Debug)]
struct CrazyMustUseBulkScorerQuery;

impl Query for CrazyMustUseBulkScorerQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "MustUseBulkScorerQuery".to_string()
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        // Java's anonymous `Weight` captures `CrazyMustUseBulkScorerQuery.this`;
        // the query is a unit value, so a new instance stands for it.
        Ok(Arc::new(CrazyWeight))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        // `this == obj`
        std::ptr::addr_eq(self as *const Self, other as *const dyn Query)
    }

    fn query_hash(&self) -> u64 {
        // `System.identityHashCode(this)`: the address stands for it.
        self as *const Self as usize as u64
    }
}

/// The anonymous `Weight` of `CrazyMustUseBulkScorerQuery.createWeight`.
#[derive(Debug)]
struct CrazyWeight;

impl SegmentCacheable for CrazyWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for CrazyWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::new(CrazyMustUseBulkScorerQuery)
    }

    fn explain(&self, _context: &LeafReaderContext, _doc: i32) -> Result<Explanation> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(Some(Box::new(CrazyScorerSupplier)))
    }
}

/// The anonymous `ScorerSupplier` of `CrazyWeight.scorerSupplier`.
struct CrazyScorerSupplier;

impl ScorerSupplier for CrazyScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn cost(&self) -> i64 {
        throw_uoe()
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(CrazyBulkScorer))
    }
}

/// The anonymous `BulkScorer` of `CrazyScorerSupplier.bulkScorer`.
struct CrazyBulkScorer;

impl BulkScorer for CrazyBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        _accept_docs: Option<&dyn Bits>,
        min: i32,
        _max: i32,
    ) -> CollectionResult<i32> {
        // Java `assert`, enabled under the test runner.
        Assert::assert_true(min == 0);
        let mut scorable = SimpleScorable::new();
        collector.set_scorer(&mut scorable)?;
        collector.collect(0, &mut scorable)?;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        1
    }
}

/// Make sure BooleanScorer can embed another BooleanScorer.
fn test_embedded_boolean_scorer() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "doctors are people who prescribe medicines of which they know little, to cure diseases of which they know less, in human beings of whom they know nothing",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r = w.get_reader()?;
    w.close()?;

    let s = IndexSearcher::new(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let mut q1 = BooleanQueryBuilder::new();
    q1.add(tq("field", "little"), Occur::SHOULD)?;
    q1.add(tq("field", "diseases"), Occur::SHOULD)?;

    let mut q2 = BooleanQueryBuilder::new();
    q2.add(Arc::new(q1.build()), Occur::SHOULD)?;
    q2.add(Arc::new(CrazyMustUseBulkScorerQuery), Occur::SHOULD)?;

    assert_equals(&1, &s.count(Arc::new(q2.build()))?);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// `((BooleanScorerSupplier) ss).booleanScorer()`.
fn boolean_scorer_of(ss: &mut dyn ScorerSupplier) -> Result<Option<Box<dyn BulkScorer>>> {
    let any = ss
        .as_any_mut()
        .expect("ClassCastException: not a BooleanScorerSupplier");
    any.downcast_mut::<BooleanScorerSupplier>()
        .expect("ClassCastException: not a BooleanScorerSupplier")
        .boolean_scorer()
}

fn bulk_is<T: Any>(scorer: &Option<Box<dyn BulkScorer>>) -> bool {
    scorer
        .as_ref()
        .and_then(|s| s.as_any())
        .is_some_and(|a| a.is::<T>())
}

fn test_optimize_top_level_clause_or_null() -> Result<()> {
    // When there is a single non-null scorer, this scorer should be used
    // directly
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    w.add_document(&doc)?;
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None); // so that weights are not wrapped
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];
    let mut b = BooleanQueryBuilder::new();
    b.add(tq("foo", "bar"), Occur::SHOULD)?; // existing term
    b.add(tq("missing_field", "baz"), Occur::SHOULD)?; // missing term
    let query: Arc<dyn Query> = Arc::new(b.build());

    // no scores -> constant-score term scorer
    let weight =
        searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let mut ss = weight.scorer_supplier(ctx)?.expect("supplier");
    let scorer = boolean_scorer_of(&mut *ss)?;
    assert_instance_of(
        bulk_is::<ConstantScoreBulkScorer>(&scorer),
        "org.apache.lucene.search.ConstantScoreBulkScorer",
    ); // term scorer

    // scores -> term scorer too
    let mut b = BooleanQueryBuilder::new();
    b.add(tq("foo", "bar"), Occur::SHOULD)?; // existing term
    b.add(tq("foo", "baz"), Occur::SHOULD)?; // missing term
    let query: Arc<dyn Query> = Arc::new(b.build());
    let weight = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE, 1.0)?;
    let mut ss = weight.scorer_supplier(ctx)?.expect("supplier");
    let scorer = boolean_scorer_of(&mut *ss)?;
    assert_instance_of(
        bulk_is::<BatchScoreBulkScorer>(&scorer),
        "org.apache.lucene.search.BatchScoreBulkScorer",
    ); // term scorer

    w.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// A `LeafCollector` whose `collect(int)` and `collect(DocIdStream)` are the
/// anonymous classes' bodies.
struct ClosureLeafCollector<C, S>
where
    C: FnMut(i32),
    S: FnMut(&mut dyn DocIdStream) -> CollectionResult<()>,
{
    collect: C,
    collect_stream: S,
}

impl<C, S> LeafCollector for ClosureLeafCollector<C, S>
where
    C: FnMut(i32),
    S: FnMut(&mut dyn DocIdStream) -> CollectionResult<()>,
{
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        (self.collect)(doc);
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        (self.collect_stream)(stream)
    }
}

fn test_constant_score_scorer_supplier_uses_into_bit_set() -> Result<()> {
    let docs: [i32; 5] = [1, 2, 4097, 5000, 8195];
    let iterator = CountingDocIdSetIterator::new(&docs);
    let into_bit_set_calls = Arc::clone(&iterator.into_bit_set_calls);

    let mut live_docs = FixedBitSet::new(9000);
    live_docs.set_range(0, 9000);
    live_docs.clear(2);
    live_docs.clear(4097);
    live_docs.clear(5000);

    let collected = Mutex::new(vec![0i32; docs.len()]);
    let count = AtomicI32::new(0);
    let mut collector = ClosureLeafCollector {
        collect: |_doc: i32| {
            Assert::fail("ConstantScoreBulkScorer should collect via DocIdStream");
        },
        collect_stream: |stream: &mut dyn DocIdStream| -> CollectionResult<()> {
            let mut buffer = vec![0i32; docs.len()];
            let mut size = stream.into_array(&mut buffer);
            while size != 0 {
                let c = count.load(Ordering::SeqCst) as usize;
                collected.lock().unwrap_or_else(|e| e.into_inner())[c..c + size]
                    .copy_from_slice(&buffer[..size]);
                count.fetch_add(size as i32, Ordering::SeqCst);
                size = stream.into_array(&mut buffer);
            }
            Ok(())
        },
    };

    let mut bulk_scorer = ConstantScoreScorerSupplier::from_iterator(
        ScorerIterator::Simple(Box::new(iterator)),
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        9000,
    )
    .bulk_scorer()?;
    assert_equals(
        &NO_MORE_DOCS,
        &bulk_scorer
            .score(&mut collector, Some(&live_docs), 0, 9000)
            .map_err(|e| e.into_lucene_error())?,
    );
    drop(collector);
    assert_array_equals(
        &[1, 8195],
        &ArrayUtil::copy_of_sub_array_int(
            &collected.lock().unwrap_or_else(|e| e.into_inner()),
            0,
            count.load(Ordering::SeqCst) as usize,
        ),
    );
    assert_equals(&3, &into_bit_set_calls.load(Ordering::SeqCst));
    Ok(())
}

fn test_constant_score_bulk_scorer_rejects_score_mode_that_needs_scores() -> Result<()> {
    expect_throws(IAE, || {
        ConstantScoreBulkScorer::from_iterator(
            1.0,
            ScoreMode::COMPLETE,
            Box::new(CountingDocIdSetIterator::new(&[1])),
        )
    });
    Ok(())
}

/// The anonymous `TwoPhaseIterator` of
/// `testConstantScoreScorerSupplierBulkCollectsTwoPhaseWithMatcher`.
struct ExcludeTwoTwoPhase {
    approximation: CountingDocIdSetIterator,
}

impl TwoPhaseIterator for ExcludeTwoTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.approximation.doc_id() != 2)
    }

    fn match_cost(&self) -> f32 {
        1.0
    }
}

fn test_constant_score_scorer_supplier_bulk_collects_two_phase_with_matcher() -> Result<()> {
    let docs: [i32; 3] = [1, 2, 3];
    let approximation = CountingDocIdSetIterator::new(&docs);
    let into_bit_set_calls = Arc::clone(&approximation.into_bit_set_calls);
    // **Divergence from Lucene 10.5.0.** Java passes
    // `TwoPhaseIterator.asDocIdSetIterator(tpi)` and `fromIterator`
    // unwraps it; Rucene's `from_iterator` takes the unwrapped form, a
    // `ScorerIterator::TwoPhase`.
    let iterator = ScorerIterator::TwoPhase(Box::new(ExcludeTwoTwoPhase { approximation }));

    let collected = Mutex::new(vec![0i32; docs.len()]);
    let count = AtomicI32::new(0);
    let push = |doc: i32| {
        let c = count.fetch_add(1, Ordering::SeqCst) as usize;
        collected.lock().unwrap_or_else(|e| e.into_inner())[c] = doc;
    };
    let mut collector = ClosureLeafCollector {
        collect: push,
        collect_stream: |stream: &mut dyn DocIdStream| -> CollectionResult<()> {
            stream.for_each(&mut |doc: i32| -> CollectionResult<()> {
                push(doc);
                Ok(())
            })
        },
    };

    // A no-score two-phase iterator is bulk-collected through ConstantScoreBulkScorer; its matcher
    // is preserved because TwoPhaseIterator#intoBitSet confirms matches() while loading the window.
    let mut bulk_scorer = ConstantScoreScorerSupplier::from_iterator(
        iterator,
        1.0,
        ScoreMode::COMPLETE_NO_SCORES,
        9000,
    )
    .bulk_scorer()?;
    assert_instance_of(
        bulk_scorer
            .as_any()
            .is_some_and(|a| a.is::<ConstantScoreBulkScorer>()),
        "org.apache.lucene.search.ConstantScoreBulkScorer",
    );
    assert_equals(
        &NO_MORE_DOCS,
        &bulk_scorer
            .score(&mut collector, None, 0, 9000)
            .map_err(|e| e.into_lucene_error())?,
    );
    drop(collector);
    // doc 2 is excluded by matches(), so the matcher is honored even via the bulk path.
    assert_array_equals(
        &[1, 3],
        &ArrayUtil::copy_of_sub_array_int(
            &collected.lock().unwrap_or_else(|e| e.into_inner()),
            0,
            count.load(Ordering::SeqCst) as usize,
        ),
    );
    // Confirmation runs through matches(), not the approximation's own bulk intoBitSet.
    assert_equals(&0, &into_bit_set_calls.load(Ordering::SeqCst));
    Ok(())
}

fn test_default_bulk_scorer_does_not_use_doc_id_stream_for_top_scores() -> Result<()> {
    assert_default_bulk_scorer_does_not_use_doc_id_stream_for_scores(ScoreMode::TOP_SCORES)
}

fn test_default_bulk_scorer_does_not_use_doc_id_stream_when_scores_are_needed() -> Result<()> {
    assert_default_bulk_scorer_does_not_use_doc_id_stream_for_scores(ScoreMode::COMPLETE)
}

fn assert_default_bulk_scorer_does_not_use_doc_id_stream_for_scores(
    score_mode: ScoreMode,
) -> Result<()> {
    let docs: [i32; 3] = [1, 2, 4097];
    let iterator = CountingDocIdSetIterator::new(&docs);
    let into_bit_set_calls = Arc::clone(&iterator.into_bit_set_calls);
    let scorer: Box<dyn Scorer> = Box::new(ConstantScoreScorer::from_iterator(
        1.0,
        score_mode,
        Box::new(iterator),
    ));

    let collected = Mutex::new(vec![0i32; docs.len()]);
    let count = AtomicI32::new(0);
    let mut collector = ClosureLeafCollector {
        collect: |doc: i32| {
            let c = count.fetch_add(1, Ordering::SeqCst) as usize;
            collected.lock().unwrap_or_else(|e| e.into_inner())[c] = doc;
        },
        collect_stream: |_stream: &mut dyn DocIdStream| -> CollectionResult<()> {
            Assert::fail(&format!(
                "ScoreMode {score_mode:?} must preserve per-doc collection"
            ))
        },
    };

    let mut bulk_scorer: Box<dyn BulkScorer> = Box::new(DefaultBulkScorer::new(scorer));
    assert_equals(
        &NO_MORE_DOCS,
        &bulk_scorer
            .score(&mut collector, None, 0, 9000)
            .map_err(|e| e.into_lucene_error())?,
    );
    drop(collector);
    assert_array_equals(
        &docs,
        &ArrayUtil::copy_of_sub_array_int(
            &collected.lock().unwrap_or_else(|e| e.into_inner()),
            0,
            count.load(Ordering::SeqCst) as usize,
        ),
    );
    assert_equals(&0, &into_bit_set_calls.load(Ordering::SeqCst));
    Ok(())
}

/// `CountingDocIdSetIterator extends DocIdSetIterator`.
///
/// Java reads `intoBitSetCalls` off the iterator after handing it to a
/// scorer; the counter is shared so that the test keeps a handle on it.
struct CountingDocIdSetIterator {
    docs: Vec<i32>,
    index: i32,
    doc: i32,
    into_bit_set_calls: Arc<AtomicI32>,
}

impl CountingDocIdSetIterator {
    fn new(docs: &[i32]) -> Self {
        CountingDocIdSetIterator {
            docs: docs.to_vec(),
            index: -1,
            doc: -1,
            into_bit_set_calls: Arc::new(AtomicI32::new(0)),
        }
    }
}

impl DocIdSetIterator for CountingDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.index += 1;
        if self.index as usize == self.docs.len() {
            self.doc = NO_MORE_DOCS;
            return Ok(self.doc);
        }
        self.doc = self.docs[self.index as usize];
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        loop {
            self.next_doc()?;
            if self.doc >= target {
                break;
            }
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.into_bit_set_calls.fetch_add(1, Ordering::SeqCst);
        while self.doc < up_to {
            bit_set.set((self.doc - offset) as usize);
            self.next_doc()?;
        }
        Ok(())
    }
}

fn test_optimize_prohibited_clauses() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
    w.add_document(&doc)?;
    w.force_merge(1)?;
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None); // so that weights are not wrapped
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];

    let queries: [Vec<(Arc<dyn Query>, Occur)>; 4] = [
        vec![
            (tq("foo", "baz"), Occur::SHOULD),
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        vec![
            (tq("foo", "baz"), Occur::SHOULD),
            (Arc::new(MatchAllDocsQuery::instance()), Occur::SHOULD),
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        vec![
            (tq("foo", "baz"), Occur::MUST),
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
        vec![
            (tq("foo", "baz"), Occur::FILTER),
            (tq("foo", "bar"), Occur::MUST_NOT),
        ],
    ];
    for clauses in queries {
        let mut b = BooleanQueryBuilder::new();
        for (q, o) in clauses {
            b.add(q, o)?;
        }
        let query: Arc<dyn Query> = Arc::new(b.build());
        let weight = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE, 1.0)?;
        let mut ss = weight.scorer_supplier(ctx)?.expect("supplier");
        let scorer = boolean_scorer_of(&mut *ss)?;
        assert_instance_of(
            bulk_is::<ReqExclBulkScorer>(&scorer),
            "org.apache.lucene.search.ReqExclBulkScorer",
        );
    }

    w.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_sparse_clause_optimization() -> Result<()> {
    // When some windows have only one scorer that can match, the scorer will
    // directly call the collector in this window
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let empty_doc = Document::new();
    let num_docs = LuceneTestCase::at_least(10);
    let mut num_empty_docs = LuceneTestCase::at_least(200);
    for _d in 0..num_docs {
        let mut i = num_empty_docs;
        while i >= 0 {
            w.add_document(&empty_doc)?;
            i -= 1;
        }
        // Java builds this document and never adds it.
        let mut doc = Document::new();
        for value in ["foo", "bar", "baz"] {
            if random().next_boolean() {
                doc.add(Box::new(StringField::new(
                    "field",
                    value.into(),
                    Store::NO,
                )?));
            }
        }
    }
    num_empty_docs = LuceneTestCase::at_least(200);
    let mut i = num_empty_docs;
    while i >= 0 {
        w.add_document(&empty_doc)?;
        i -= 1;
    }
    if random().next_boolean() {
        w.force_merge(1)?;
    }
    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut b = BooleanQueryBuilder::new();
    b.add(
        Arc::new(BoostQuery::new(tq("field", "foo"), 3.0)?),
        Occur::SHOULD,
    )?;
    b.add(
        Arc::new(BoostQuery::new(tq("field", "bar"), 3.0)?),
        Occur::SHOULD,
    )?;
    b.add(
        Arc::new(BoostQuery::new(tq("field", "baz"), 3.0)?),
        Occur::SHOULD,
    )?;
    let query: Arc<dyn Query> = Arc::new(b.build());

    // duel BS1 vs. BS2
    QueryUtils::check_searcher(&*random(), &query, Some(&searcher));

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `ScoreMode.values()`, in declaration order.
const SCORE_MODE_VALUES: [ScoreMode; 5] = [
    ScoreMode::COMPLETE,
    ScoreMode::COMPLETE_NO_SCORES,
    ScoreMode::TOP_SCORES,
    ScoreMode::TOP_DOCS,
    ScoreMode::TOP_DOCS_WITH_SCORES,
];

fn scorer_is_css(scorer: &Option<Box<dyn Scorer>>) -> bool {
    scorer
        .as_ref()
        .and_then(|s| s.as_any())
        .is_some_and(|a| a.is::<ConstantScoreScorer>())
}

fn test_filter_constant_score() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "bat".into(), Store::NO)?));
    doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
    w.add_document(&doc)?;
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None);
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;

    {
        // single filter rewrites to a constant score query
        let mut b = BooleanQueryBuilder::new();
        b.add(tq("foo", "bar"), Occur::FILTER)?;
        let rewrite = searcher.rewrite(Arc::new(b.build()))?;
        let boost = rewrite.as_any().downcast_ref::<BoostQuery>();
        assert_instance_of(boost.is_some(), "org.apache.lucene.search.BoostQuery");
        assert_instance_of(
            boost
                .expect("checked")
                .get_query()
                .as_any()
                .is::<ConstantScoreQuery>(),
            "org.apache.lucene.search.ConstantScoreQuery",
        );
    }

    let build = |clauses: Vec<(Arc<dyn Query>, Occur)>| -> Result<Arc<dyn Query>> {
        let mut b = BooleanQueryBuilder::new();
        for (q, o) in clauses {
            b.add(q, o)?;
        }
        Ok(Arc::new(b.build()))
    };

    let queries: [Arc<dyn Query>; 3] = [
        build(vec![
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::FILTER),
        ])?,
        build(vec![
            (tq("foo", "baz"), Occur::FILTER),
            // non-existing term
            (tq("foo", "arf"), Occur::SHOULD),
        ])?,
        build(vec![
            (tq("foo", "baz"), Occur::FILTER),
            (tq("foo", "baz"), Occur::FILTER),
            // non-existing term
            (tq("foo", "arf"), Occur::SHOULD),
            (tq("foo", "arw"), Occur::SHOULD),
        ])?,
    ];
    for query in queries {
        let rewrite = searcher.rewrite(query)?;
        for score_mode in SCORE_MODE_VALUES {
            let weight = searcher.create_weight(Arc::clone(&rewrite), score_mode, 1.0)?;
            let scorer = weight.scorer(&leaves[0])?;
            if score_mode == ScoreMode::TOP_SCORES {
                assert_instance_of(
                    scorer_is_css(&scorer),
                    "org.apache.lucene.search.ConstantScoreScorer",
                );
            } else {
                assert_not_instance_of(
                    scorer_is_css(&scorer),
                    "org.apache.lucene.search.ConstantScoreScorer",
                );
            }
        }
    }

    let queries: [Arc<dyn Query>; 3] = [
        build(vec![
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::SHOULD),
        ])?,
        build(vec![
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::MUST),
            // non-existing term
            (tq("foo", "arf"), Occur::SHOULD),
        ])?,
        build(vec![
            // non-existing term
            (tq("foo", "bar"), Occur::FILTER),
            (tq("foo", "baz"), Occur::SHOULD),
            // non-existing term
            (tq("foo", "arf"), Occur::MUST),
        ])?,
    ];
    for query in queries {
        let rewrite = searcher.rewrite(query)?;
        for score_mode in SCORE_MODE_VALUES {
            let weight = searcher.create_weight(Arc::clone(&rewrite), score_mode, 1.0)?;
            let scorer = weight.scorer(&leaves[0])?;
            assert_not_instance_of(
                scorer_is_css(&scorer),
                "org.apache.lucene.search.ConstantScoreScorer",
            );
        }
    }

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_collect_no_threshold_when_only_filter() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut field_type = FieldType::new();
    field_type.set_index_options(IndexOptions::DOCS)?;
    for i in 0..50 {
        let mut doc = Document::new();
        doc.add(Box::new(Field::new(
            "foo",
            format!("bar{}", i % 2),
            FieldType::new_from(&field_type),
        )?));
        doc.add(Box::new(LongPoint::new("field", &[(i % 4) as i64])?));
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None);

    let term_query = tq("foo", "bar0");

    let mut builder = BooleanQueryBuilder::new();
    builder.add(term_query, Occur::FILTER)?;

    let index_query = LongPoint::new_range_query("field", &[1], &[i64::MAX])?;
    builder.add(index_query, Occur::FILTER)?;

    let total_hits_threshold = 7;
    let top_score_doc_collector_manager =
        TopScoreDocCollectorManager::new(3, None, total_hits_threshold)?;
    let mut collector = top_score_doc_collector_manager.new_collector()?;
    searcher.search_with_collector(Arc::new(builder.build()), &mut collector)?;
    assert_equals(&(total_hits_threshold + 1), &collector.total_hits());

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Make sure a SHOULD-only BooleanQuery with TOP_SCORES and minShouldMatch > 1 is dispatched to
/// WandScorer
fn test_top_scores_with_min_should_match_falls_back_to_wand() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _i in 0..100 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("foo", "bar".into(), Store::NO)?));
        doc.add(Box::new(StringField::new("foo", "baz".into(), Store::NO)?));
        doc.add(Box::new(StringField::new("foo", "qux".into(), Store::NO)?));
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let mut searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    searcher.set_query_cache(None); // so that weights are not wrapped
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    let ctx = &leaves[0];

    let mut b = BooleanQueryBuilder::new();
    b.add(tq("foo", "bar"), Occur::SHOULD)?;
    b.add(tq("foo", "baz"), Occur::SHOULD)?;
    b.add(tq("foo", "qux"), Occur::SHOULD)?;
    b.set_minimum_number_should_match(2);
    let query: Arc<dyn Query> = Arc::new(b.build());

    let weight = searcher.create_weight(
        searcher.rewrite(Arc::clone(&query))?,
        ScoreMode::TOP_SCORES,
        1.0,
    )?;
    let mut ss = weight.scorer_supplier(ctx)?.expect("supplier");
    let boolean_scorer = boolean_scorer_of(&mut *ss)?;
    Assert::assert_true_msg(
        "TOP_SCORES + minShouldMatch > 1 must skip BooleanScorer and fall back to BS2/WAND",
        boolean_scorer.is_none(),
    );

    // Sanity: bulkScorer() itself returns a non-null BS2 (DefaultBulkScorer
    // wrapping WANDScorer).
    let bulk = ss.bulk_scorer()?;
    Assert::assert_true(bulk.as_any().is_some_and(|a| a.is::<DefaultBulkScorer>()));

    // Non-TOP_SCORES with minShouldMatch > 1 still uses BooleanScorer when
    // matches are dense.
    let weight = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE, 1.0)?;
    let mut ss = weight.scorer_supplier(ctx)?.expect("supplier");
    Assert::assert_true_msg(
        "COMPLETE + minShouldMatch > 1 (dense) should still use BooleanScorer",
        bulk_is::<BooleanScorer>(&boolean_scorer_of(&mut *ss)?),
    );

    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
