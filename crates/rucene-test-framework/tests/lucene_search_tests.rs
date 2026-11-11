//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/search/`
//! (task #359 part 2): `TestAssertingLeafCollector`, `TestPerThreadPKLookup`
//! and `TestQueryUtils`, each run as a suite (`extends LuceneTestCase`) by the
//! ported runner, and each reporting what Lucene's own class reported on the
//! JVM (`framework_tests.tsv`, written by `FrameworkTestsOracle`).
//! `TestBaseExplanationTestCase` belongs to task #356.

mod oracle_support;

use std::any::Any;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, KeywordField, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::collection_terminated_exception::{CollectionError, CollectionResult};
use rucene::search::collector::LeafCollector;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::doc_id_set_iterator::{self, DocIdSetIterator};
use rucene::search::doc_id_stream::{CheckedIntConsumer, DocIdStream};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::{Scorer, ScorerAsScorable};
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::explanation::Explanation;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::{DefaultScorerSupplier, FilterWeight, Weight};
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{PerThreadPKLookup, RandomIndexWriter};
use rucene_test_framework::search::{AssertingLeafCollector, QueryUtils};
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_ASSERTS_ENABLED};
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

fn run_suite(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    oracle_support::assert_framework_outcomes(&builder.build());
}

/// Throws a Rucene error out of a test method, as Java's `throws Exception`:
/// an assertion error carried as an error (`java_assert!`) unwinds as the
/// `java.lang.AssertionError` Java throws.
fn rethrow(e: LuceneError) -> ! {
    if let LuceneError::AssertionError(message) = e {
        Throwable::new(ASSERTION_ERROR, message).throw()
    }
    let (class, message) =
        rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message(&e);
    Throwable::new(class, message).throw()
}

/// [`rethrow`] for a collection call.
fn rethrow_collection(e: CollectionError) -> ! {
    match e {
        CollectionError::Lucene(e) => rethrow(e),
        other => Throwable::new(
            "org.apache.lucene.search.CollectionTerminatedException",
            other.to_string(),
        )
        .throw(),
    }
}

// ---------------------------------------------------------------------------
// TestAssertingLeafCollector
// ---------------------------------------------------------------------------

/// `scoreCallingCollector()`: a leaf collector whose every collection call
/// calls `scorer.score()` (the port lends the scorer to each call).
struct ScoreCallingCollector;

impl LeafCollector for ScoreCallingCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        scorer.score()?;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        _stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        scorer.score()?;
        Ok(())
    }

    fn collect_range(
        &mut self,
        _min: i32,
        _max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        scorer.score()?;
        Ok(())
    }
}

/// `scorable()`: a `Scorable` that is not a `Scorer`.
struct PlainScorable;

impl Scorable for PlainScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(1.0)
    }
}

/// `singletonStream(doc)`.
struct SingletonStream {
    doc: i32,
    consumed: bool,
}

impl DocIdStream for SingletonStream {
    fn for_each_up_to(
        &mut self,
        up_to: i32,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        if !self.consumed && self.doc < up_to {
            self.consumed = true;
            consumer.accept(self.doc)?;
        }
        Ok(())
    }

    fn count_up_to(&mut self, up_to: i32) -> Result<i32> {
        if self.consumed || self.doc >= up_to {
            return Ok(0);
        }
        self.consumed = true;
        Ok(1)
    }

    fn into_array_up_to(&mut self, up_to: i32, array: &mut [i32]) -> usize {
        assert!(!array.is_empty());
        if self.consumed || self.doc >= up_to {
            return 0;
        }
        self.consumed = true;
        array[0] = self.doc;
        1
    }

    fn may_have_remaining(&self) -> bool {
        !self.consumed
    }
}

/// `new ConstantScoreScorer(1f, ScoreMode.COMPLETE, DocIdSetIterator.all(10))`.
fn constant_scorer() -> ConstantScoreScorer {
    ConstantScoreScorer::from_iterator(
        1.0,
        ScoreMode::COMPLETE,
        Box::new(doc_id_set_iterator::all(10).unwrap_or_else(|e| rethrow(e))),
    )
}

/// `TestAssertingLeafCollector`: `score()` is refused while a doc-ID stream
/// or a range is collected with a `Scorer`, and allowed with a plain
/// `Scorable`.
///
/// The collector receives the scorable on every call (see
/// `AssertingLeafCollector`'s docs); a `Scorer` is lent through
/// `ScorerAsScorable`, which answers Java's `instanceof Scorer`.
#[test]
fn test_asserting_leaf_collector() {
    run_suite(
        Class::builder("org.apache.lucene.tests.search.TestAssertingLeafCollector")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testScoreRejectedDuringDocIdStreamCollection")
                    .throws(&["java.io.IOException"])
                    .body_any(|| {
                        let mut collector =
                            AssertingLeafCollector::new(Box::new(ScoreCallingCollector), 0, 10);
                        let mut scorer = constant_scorer();
                        collector
                            .set_scorer(&mut ScorerAsScorable::new(&mut scorer))
                            .unwrap_or_else(|e| rethrow(e));
                        let mut stream = SingletonStream {
                            doc: 1,
                            consumed: false,
                        };
                        if *TEST_ASSERTS_ENABLED {
                            LuceneTestCase::expect_throws(ASSERTION_ERROR, || {
                                collector
                                    .collect_stream(
                                        &mut stream,
                                        &mut ScorerAsScorable::new(&mut scorer),
                                    )
                                    .unwrap_or_else(|e| rethrow_collection(e));
                                Ok(())
                            });
                        } else {
                            collector
                                .collect_stream(
                                    &mut stream,
                                    &mut ScorerAsScorable::new(&mut scorer),
                                )
                                .unwrap_or_else(|e| rethrow_collection(e));
                        }
                    }),
            )
            .method(
                Method::new("testScoreRejectedDuringRangeCollection")
                    .throws(&["java.io.IOException"])
                    .body_any(|| {
                        let mut collector =
                            AssertingLeafCollector::new(Box::new(ScoreCallingCollector), 0, 10);
                        let mut scorer = constant_scorer();
                        collector
                            .set_scorer(&mut ScorerAsScorable::new(&mut scorer))
                            .unwrap_or_else(|e| rethrow(e));
                        if *TEST_ASSERTS_ENABLED {
                            LuceneTestCase::expect_throws(ASSERTION_ERROR, || {
                                collector
                                    .collect_range(1, 3, &mut ScorerAsScorable::new(&mut scorer))
                                    .unwrap_or_else(|e| rethrow_collection(e));
                                Ok(())
                            });
                        } else {
                            collector
                                .collect_range(1, 3, &mut ScorerAsScorable::new(&mut scorer))
                                .unwrap_or_else(|e| rethrow_collection(e));
                        }
                    }),
            )
            .method(
                Method::new("testScoreAllowedDuringRangeCollectionWithNonScorer")
                    .throws(&["java.io.IOException"])
                    .body_any(|| {
                        let mut collector =
                            AssertingLeafCollector::new(Box::new(ScoreCallingCollector), 0, 10);
                        collector
                            .set_scorer(&mut PlainScorable)
                            .unwrap_or_else(|e| rethrow(e));
                        collector
                            .collect_range(1, 3, &mut PlainScorable)
                            .unwrap_or_else(|e| rethrow_collection(e));
                    }),
            ),
    );
}

// ---------------------------------------------------------------------------
// TestPerThreadPKLookup
// ---------------------------------------------------------------------------

/// `new KeywordField(name, value, Field.Store.NO)` in a new document.
fn keyword_doc(fields: &[(&str, &str)]) -> Document {
    let mut doc = Document::new();
    for (name, value) in fields {
        doc.add(Box::new(
            KeywordField::new(name, value.to_string(), Store::NO).unwrap_or_else(|e| rethrow(e)),
        ));
    }
    doc
}

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random()))
/// .setMergePolicy(NoMergePolicy.INSTANCE))`.
fn pk_writer(dir: &TestDirectory) -> Arc<IndexWriter> {
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*LuceneTestCase::random(),
    ))));
    config
        .set_merge_policy(Some(NoMergePolicy::instance()))
        .unwrap_or_else(|e| rethrow(e));
    IndexWriter::new(dir.as_directory(), config).unwrap_or_else(|e| rethrow(e))
}

fn as_reader(r: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(r) as Arc<dyn IndexReader>
}

fn lookup(pk: &mut PerThreadPKLookup, id: &str) -> i32 {
    pk.lookup(&LuceneTestCase::new_bytes_ref_str(id))
        .unwrap_or_else(|e| rethrow(e))
}

/// `TestPerThreadPKLookup.testReopen`.
fn test_reopen() {
    let mut dir = LuceneTestCase::new_directory();
    let writer = pk_writer(&dir);
    let add = |fields: &[(&str, &str)]| {
        writer
            .add_document(&keyword_doc(fields))
            .unwrap_or_else(|e| rethrow(e));
    };
    add(&[("PK", "1")]);
    add(&[("PK", "2")]);
    writer.flush().unwrap_or_else(|e| rethrow(e));

    // Terms in PK is null.
    add(&[("PK2", "3")]);
    add(&[("PK2", "4")]);
    writer.flush().unwrap_or_else(|e| rethrow(e));

    let reader1 = directory_reader::open_with_writer(&*writer).unwrap_or_else(|e| rethrow(e));
    let mut pk_lookup1 =
        PerThreadPKLookup::new(&as_reader(&reader1), "PK").unwrap_or_else(|e| rethrow(e));

    add(&[("PK", "5")]);
    add(&[("PK", "6")]);
    // Update liveDocs.
    writer
        .delete_documents_by_terms(vec![Term::new("PK", BytesRef::from_char_sequence("1"))])
        .unwrap_or_else(|e| rethrow(e));
    writer.flush().unwrap_or_else(|e| rethrow(e));

    // Terms in PK is null.
    add(&[("PK2", "7")]);
    add(&[("PK2", "8")]);
    writer.flush().unwrap_or_else(|e| rethrow(e));

    Assert::assert_equals(0, lookup(&mut pk_lookup1, "1"));
    Assert::assert_equals(1, lookup(&mut pk_lookup1, "2"));
    Assert::assert_equals(-1, lookup(&mut pk_lookup1, "5"));
    Assert::assert_equals(-1, lookup(&mut pk_lookup1, "8"));
    let reader2 = directory_reader::open_if_changed(Arc::clone(&reader1))
        .unwrap_or_else(|e| rethrow(e))
        .expect("the writer changed the index");
    let mut pk_lookup2 = pk_lookup1
        .reopen(Some(&as_reader(&reader2)))
        .unwrap_or_else(|e| rethrow(e))
        .expect("a reader was given");

    Assert::assert_equals(-1, lookup(&mut pk_lookup2, "1"));
    Assert::assert_equals(1, lookup(&mut pk_lookup2, "2"));
    Assert::assert_equals(4, lookup(&mut pk_lookup2, "5"));
    Assert::assert_equals(-1, lookup(&mut pk_lookup2, "8"));

    add(&[("PK", "9")]);
    add(&[("PK", "10")]);
    writer.flush().unwrap_or_else(|e| rethrow(e));

    Assert::assert_equals(-1, lookup(&mut pk_lookup2, "9"));
    let reader3 = directory_reader::open_if_changed(Arc::clone(&reader2))
        .unwrap_or_else(|e| rethrow(e))
        .expect("the writer changed the index");
    let mut pk_lookup3 = pk_lookup2
        .reopen(Some(&as_reader(&reader3)))
        .unwrap_or_else(|e| rethrow(e))
        .expect("a reader was given");
    Assert::assert_equals(8, lookup(&mut pk_lookup3, "9"));

    // `assertNull(pkLookup3.reopen(DirectoryReader.openIfChanged(reader3)))`
    let reader4 =
        directory_reader::open_if_changed(Arc::clone(&reader3)).unwrap_or_else(|e| rethrow(e));
    Assert::assert_true(
        pk_lookup3
            .reopen(reader4.as_ref().map(as_reader).as_ref())
            .unwrap_or_else(|e| rethrow(e))
            .is_none(),
    );

    writer.close().unwrap_or_else(|e| rethrow(e));
    reader1.close().unwrap_or_else(|e| rethrow(e));
    reader2.close().unwrap_or_else(|e| rethrow(e));
    reader3.close().unwrap_or_else(|e| rethrow(e));
    // The last handles on the directory (`MockDirectoryWrapper`'s module
    // docs: `close()` needs them released).
    drop((writer, reader1, reader2, reader3, reader4));
    drop((pk_lookup1, pk_lookup2, pk_lookup3));
    dir.close().unwrap_or_else(|e| rethrow(e));
}

/// `TestPerThreadPKLookup.testPKLookupWithUpdate`.
fn test_pk_lookup_with_update() {
    let mut dir = LuceneTestCase::new_directory();
    let writer = pk_writer(&dir);

    writer
        .add_document(&keyword_doc(&[("PK", "1"), ("version", "1")]))
        .unwrap_or_else(|e| rethrow(e));
    writer
        .update_document(
            Term::new("PK", BytesRef::from_char_sequence("1")),
            &keyword_doc(&[("PK", "1"), ("version", "2")]),
        )
        .unwrap_or_else(|e| rethrow(e));
    // PK updates will be merged to one update.
    writer
        .update_document(
            Term::new("PK", BytesRef::from_char_sequence("1")),
            &keyword_doc(&[("PK", "1"), ("version", "3")]),
        )
        .unwrap_or_else(|e| rethrow(e));
    writer.flush().unwrap_or_else(|e| rethrow(e));
    writer.close().unwrap_or_else(|e| rethrow(e));
    drop(writer);

    let reader = directory_reader::open(dir.as_directory()).unwrap_or_else(|e| rethrow(e));
    let mut pk = PerThreadPKLookup::new(&as_reader(&reader), "PK").unwrap_or_else(|e| rethrow(e));

    let doc_id = lookup(&mut pk, "1");
    Assert::assert_equals(2, doc_id);

    reader.close().unwrap_or_else(|e| rethrow(e));
    drop((reader, pk));
    dir.close().unwrap_or_else(|e| rethrow(e));
}

/// `TestPerThreadPKLookup`.
#[test]
fn test_per_thread_pk_lookup() {
    run_suite(
        Class::builder("org.apache.lucene.tests.search.TestPerThreadPKLookup")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testReopen")
                    .throws(&["java.lang.Exception"])
                    .body_any(test_reopen),
            )
            .method(
                Method::new("testPKLookupWithUpdate")
                    .throws(&["java.lang.Exception"])
                    .body_any(test_pk_lookup_with_update),
            ),
    );
}

// ---------------------------------------------------------------------------
// TestQueryUtils
// ---------------------------------------------------------------------------

/// The broken query variants of `TestQueryUtils`, each a subclass of its
/// `BrokenQuery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Broken {
    /// `BrokenAdvanceQuery`: `advance()` always returns `NO_MORE_DOCS`.
    Advance,
    /// `SkipTooFarQuery`: `advance()` steps one doc too far.
    SkipTooFar,
    /// `UnstableScoreQuery`: `score()` adds 1 on every even call per doc.
    UnstableScore,
    /// `ScoreMismatchQuery`: `score()` adds 100 when the doc was reached by
    /// `advance()`.
    ScoreMismatch,
    /// `EarlyTerminationQuery`: `nextDoc()` returns `NO_MORE_DOCS` after its
    /// first call.
    EarlyTermination,
}

impl Broken {
    /// `getClass().getSimpleName()`.
    fn simple_name(self) -> &'static str {
        match self {
            Broken::Advance => "BrokenAdvanceQuery",
            Broken::SkipTooFar => "SkipTooFarQuery",
            Broken::UnstableScore => "UnstableScoreQuery",
            Broken::ScoreMismatch => "ScoreMismatchQuery",
            Broken::EarlyTermination => "EarlyTerminationQuery",
        }
    }
}

/// `BrokenQuery`: wraps a delegate and substitutes a broken scorer.
#[derive(Debug, Clone)]
struct BrokenQuery {
    kind: Broken,
    delegate: Arc<dyn Query>,
}

impl BrokenQuery {
    /// `new <Kind>Query(new TermQuery(new Term("f", "v")))`.
    fn of(kind: Broken) -> Arc<dyn Query> {
        Arc::new(BrokenQuery {
            kind,
            delegate: Arc::new(TermQuery::new(Term::new(
                "f",
                BytesRef::from_char_sequence("v"),
            ))),
        })
    }
}

impl Query for BrokenQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "{}({})",
            self.kind.simple_name(),
            self.delegate.to_query_string(field)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.delegate.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let inner = self.delegate.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(BrokenWeight {
            filter: FilterWeight::with_query(Arc::new(self.clone()), inner),
            kind: self.kind,
        }))
    }

    /// `sameClassAs(obj) && delegate.equals(((BrokenQuery) obj).delegate)`:
    /// the variants are distinct Java classes.
    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<BrokenQuery>()
            .is_some_and(|o| o.kind == self.kind && self.delegate.query_eq(&*o.delegate))
    }

    /// `classHash() ^ delegate.hashCode()`, the class hash of each variant
    /// distinct.
    fn query_hash(&self) -> u64 {
        (self.class_hash() ^ (self.kind as u64 + 1)) ^ self.delegate.query_hash()
    }
}

/// The anonymous `FilterWeight` of `BrokenQuery.createWeight`.
#[derive(Debug)]
struct BrokenWeight {
    filter: FilterWeight,
    kind: Broken,
}

impl SegmentCacheable for BrokenWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.filter.is_cacheable(ctx)
    }
}

impl Weight for BrokenWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.filter.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.filter.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.filter.matches(context, doc)
    }

    /// `Scorer inner = in.scorer(context); if (inner == null) return null;
    /// return new DefaultScorerSupplier(wrapScorer(inner));`
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut inner) = self.filter.inner.scorer(context)? else {
            return Ok(None);
        };
        let cost = inner.iterator().cost();
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(
            BrokenScorer {
                kind: self.kind,
                inner,
                cost,
                doc_id: -1,
                last_was_advance: false,
                exhausted: false,
                last_scored_doc: -2,
                score_call_count: 0,
            },
        )))))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.filter.count(context)
    }
}

/// The scorer each variant's `wrapScorer(inner)` builds, with the
/// replacement iterator (`docId[0]` and the flags the anonymous classes
/// capture) as its own state; the inner scorer's iterator is `innerIter`.
struct BrokenScorer {
    kind: Broken,
    inner: Box<dyn Scorer>,
    /// `innerIter.cost()`, read once (a scorer's cost does not change).
    cost: i64,
    /// `docId[0]`.
    doc_id: i32,
    /// `ScoreMismatchQuery`'s `lastWasAdvance[0]`.
    last_was_advance: bool,
    /// `EarlyTerminationQuery`'s `exhausted[0]`.
    exhausted: bool,
    /// `UnstableScoreQuery`'s `lastScoredDoc`.
    last_scored_doc: i32,
    /// `UnstableScoreQuery`'s `scoreCallCount`.
    score_call_count: i32,
}

impl DocIdSetIterator for BrokenScorer {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = match self.kind {
            Broken::ScoreMismatch => {
                self.last_was_advance = false;
                self.inner.iterator().next_doc()?
            }
            Broken::EarlyTermination => {
                if self.exhausted {
                    NO_MORE_DOCS
                } else {
                    self.exhausted = true;
                    self.inner.iterator().next_doc()?
                }
            }
            _ => self.inner.iterator().next_doc()?,
        };
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc_id = match self.kind {
            Broken::Advance => NO_MORE_DOCS,
            Broken::SkipTooFar => {
                let mut doc = self.inner.iterator().advance(target)?;
                if doc != NO_MORE_DOCS {
                    doc = self.inner.iterator().next_doc()?; // skip one too far
                }
                doc
            }
            Broken::ScoreMismatch => {
                self.last_was_advance = true;
                self.inner.iterator().advance(target)?
            }
            Broken::EarlyTermination => {
                self.exhausted = true;
                self.inner.iterator().advance(target)?
            }
            Broken::UnstableScore => self.inner.iterator().advance(target)?,
        };
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl Scorable for BrokenScorer {
    fn score(&mut self) -> Result<f32> {
        match self.kind {
            Broken::UnstableScore => {
                let doc = self.doc_id;
                if doc != self.last_scored_doc {
                    self.last_scored_doc = doc;
                    self.score_call_count = 0;
                }
                self.score_call_count += 1;
                let extra = if self.score_call_count % 2 == 0 {
                    1.0
                } else {
                    0.0
                };
                Ok(self.inner.score()? + extra)
            }
            Broken::ScoreMismatch => {
                if self.last_was_advance {
                    Ok(self.inner.score()? + 100.0)
                } else {
                    self.inner.score()
                }
            }
            _ => self.inner.score(),
        }
    }
}

impl Scorer for BrokenScorer {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let max = self.inner.get_max_score(up_to)?;
        Ok(match self.kind {
            Broken::UnstableScore => max + 1.0,
            Broken::ScoreMismatch => max + 100.0,
            _ => max,
        })
    }
}

/// The fields of `TestQueryUtils`, set by `setUp`.
#[derive(Default)]
struct QueryUtilsFixture {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

/// Five documents `f:v` written by a `RandomIndexWriter`, optionally merged
/// into one segment; the reader it returns.
fn five_docs(dir: &TestDirectory, force_merge: bool) -> Arc<dyn DirectoryReader> {
    let iw =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())
            .unwrap_or_else(|e| rethrow(e));
    for _ in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "f",
            "v",
            Store::NO,
        )));
        iw.add_document(&doc).unwrap_or_else(|e| rethrow(e));
    }
    if force_merge {
        iw.force_merge(1).unwrap_or_else(|e| rethrow(e));
    }
    let reader = iw.get_reader().unwrap_or_else(|e| rethrow(e));
    iw.close().unwrap_or_else(|e| rethrow(e));
    reader
}

/// `expectThrows(AssertionError.class, () -> QueryUtils.<check>(q, s))`.
fn expect_assertion(
    check: fn(&Arc<dyn Query>, &IndexSearcher) -> Result<()>,
    q: &Arc<dyn Query>,
    s: &IndexSearcher,
) {
    LuceneTestCase::expect_throws(ASSERTION_ERROR, || {
        check(q, s).unwrap_or_else(|e| rethrow(e));
        Ok(())
    });
}

/// `testCheck{Skip,FirstSkip}ToDetectsEarlyTermination`: over a local index
/// merged into one segment, searched without wrapping.
fn early_termination(check: fn(&Arc<dyn Query>, &IndexSearcher) -> Result<()>) {
    let mut local_dir = LuceneTestCase::new_directory();
    let local_reader = five_docs(&local_dir, true);
    let local_searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&local_reader), false);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let q = BrokenQuery::of(Broken::EarlyTermination);
        expect_assertion(check, &q, &local_searcher);
    }));
    // `finally { localReader.close(); localDir.close(); }`
    local_reader.close().unwrap_or_else(|e| rethrow(e));
    drop((local_searcher, local_reader));
    local_dir.close().unwrap_or_else(|e| rethrow(e));
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}

/// `TestQueryUtils`: `QueryUtils.checkSkipTo` and `checkFirstSkipTo`
/// detect broken scorer behaviour.
#[test]
fn test_query_utils() {
    let mut builder = Class::builder("org.apache.lucene.tests.search.TestQueryUtils")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(QueryUtilsFixture::default))
        // The overrides carry no `@Before`/`@After`, as in Java.
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |f: &mut QueryUtilsFixture| {
                LuceneTestCase::super_set_up();
                let dir = LuceneTestCase::new_directory();
                let reader = five_docs(&dir, false);
                f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
                f.reader = Some(reader);
                f.dir = Some(dir);
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|f: &mut QueryUtilsFixture| {
                    let reader = f.reader.take().expect("setUp ran");
                    reader.close().unwrap_or_else(|e| rethrow(e));
                    drop((f.searcher.take(), reader));
                    let mut dir = f.dir.take().expect("setUp ran");
                    dir.close().unwrap_or_else(|e| rethrow(e));
                    LuceneTestCase::super_tear_down();
                }),
        );
    type Check = fn(&Arc<dyn Query>, &IndexSearcher) -> Result<()>;
    let skip_to: Check = QueryUtils::check_skip_to;
    let first_skip_to: Check = QueryUtils::check_first_skip_to;
    let cases: [(&str, Broken, Check); 8] = [
        (
            "testCheckSkipToDetectsBrokenAdvance",
            Broken::Advance,
            skip_to,
        ),
        (
            "testCheckFirstSkipToDetectsBrokenAdvance",
            Broken::Advance,
            first_skip_to,
        ),
        (
            "testCheckSkipToDetectsWrongDocFromAdvance",
            Broken::SkipTooFar,
            skip_to,
        ),
        (
            "testCheckFirstSkipToDetectsWrongDocFromAdvance",
            Broken::SkipTooFar,
            first_skip_to,
        ),
        (
            "testCheckSkipToDetectsUnstableScore",
            Broken::UnstableScore,
            skip_to,
        ),
        (
            "testCheckFirstSkipToDetectsUnstableScore",
            Broken::UnstableScore,
            first_skip_to,
        ),
        (
            "testCheckSkipToDetectsScoreMismatch",
            Broken::ScoreMismatch,
            skip_to,
        ),
        (
            "testCheckFirstSkipToDetectsScoreMismatch",
            Broken::ScoreMismatch,
            first_skip_to,
        ),
    ];
    for (name, kind, check) in cases {
        builder = builder.method(Method::new(name).throws(&["java.io.IOException"]).body(
            move |f: &mut QueryUtilsFixture| {
                let q = BrokenQuery::of(kind);
                expect_assertion(check, &q, f.searcher.as_ref().expect("setUp ran"));
            },
        ));
    }
    builder = builder
        .method(
            Method::new("testCheckSkipToDetectsEarlyTermination")
                .throws(&["java.io.IOException"])
                .body(move |_f: &mut QueryUtilsFixture| early_termination(skip_to)),
        )
        .method(
            Method::new("testCheckFirstSkipToDetectsEarlyTermination")
                .throws(&["java.io.IOException"])
                .body(move |_f: &mut QueryUtilsFixture| early_termination(first_skip_to)),
        );
    run_suite(builder);
}
