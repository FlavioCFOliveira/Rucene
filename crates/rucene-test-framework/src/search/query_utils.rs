/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.search.QueryUtils`.
//!
//! The JUnit assertions of Java (`assertEquals`, `assertFalse`, …) fail by
//! throwing an unchecked `AssertionError`; here they panic with that
//! `Throwable`, as [`randomizedtesting::junit::Assert`] does. Java's
//! `RuntimeException` around an `IOException` is a panic with that class whose
//! message is the I/O error.
//!
//! **Divergences.**
//!
//! * The searcher arguments that Java allows to be `null` are `Option`s.
//! * A `null` scorer (supplier) where Java dereferences it throws Java's
//!   `NullPointerException` (without a message).
//! * The diagnostics that print a scorer, a query class or a searcher (only
//!   under `VERBOSE`) print `<scorer>`, `<query class>` and `<searcher>`.

use std::any::Any;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{
    java_assert_no_message, throw, throw_without_message, NULL_POINTER_EXCEPTION, RUNTIME_EXCEPTION,
};
use randomizedtesting::jdk::lang::float_to_string;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::junit::Assert;
use rucene::codecs::stub::StoredFieldVisitor;
use rucene::error::Result;
use rucene::index::{
    BinaryDocValues, ByteVectorValues, CacheHelper, DocValuesSkipper, FieldInfos,
    FloatVectorValues, IndexReader, IndexReaderCore, LeafMetaData, LeafReader, LeafReaderContext,
    MultiReader, NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, StoredFields, TermVectors, Terms,
};
use rucene::search::{
    query_to_string, AcceptDocs, CollectionError, CollectionResult, Collector, DocIdStream,
    IndexSearcher, KnnCollector, LeafCollector, Query, QueryVisitor, Scorable, ScoreMode, Scorer,
    Weight, NO_MORE_DOCS,
};
use rucene::util::extra::Version;
use rucene::util::{Bits, MatchNoBits};

use crate::search::check_hits::CheckHits;
use crate::util::lucene_test_case::{LuceneTestCase, VERBOSE};

/// Java's `catch (IOException e) { throw new RuntimeException(e); }`: an
/// I/O error becomes a `RuntimeException`; an unchecked exception or an
/// assertion error carried as an error propagates as itself.
pub(crate) fn runtime_exception(e: rucene::error::LuceneError) -> ! {
    match e {
        rucene::error::LuceneError {
            kind: rucene::error::LuceneErrorKind::Unchecked { class, message, .. },
            ..
        } => match message {
            Some(message) => throw(class, message),
            None => throw_without_message(class),
        },
        rucene::error::LuceneError {
            kind: rucene::error::LuceneErrorKind::AssertionError(message),
            ..
        } => throw(randomizedtesting::jdk::exception::ASSERTION_ERROR, message),
        other => throw(RUNTIME_EXCEPTION, other),
    }
}

/// A `null` dereference.
fn null_pointer() -> ! {
    throw_without_message(NULL_POINTER_EXCEPTION)
}

/// Whether two readers are the same object (`==`).
fn same_reader(a: &Arc<dyn LeafReader>, b: &Arc<dyn LeafReader>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Utility methods for checking queries.
///
/// Equivalent to `org.apache.lucene.tests.search.QueryUtils`.
pub struct QueryUtils;

/// The anonymous "whacky" query of `checkHashEquals`: equal only to itself.
#[derive(Debug)]
struct WhackyQuery;

impl Query for WhackyQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "My Whacky Query".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `o == this`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        std::ptr::addr_eq(self as *const Self, other as *const dyn Query)
    }

    /// `System.identityHashCode(this)`.
    fn query_hash(&self) -> u64 {
        self as *const Self as usize as u64
    }
}

impl QueryUtils {
    /// `check(q)`: checks the basic contract of `hashCode`/`equals`.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when the contract does not hold.
    pub fn check(q: &dyn Query) {
        Self::check_hash_equals(q);
    }

    /// `checkHashEquals(q)`: `q` equals itself with the same hash, and an
    /// unrelated query compares unequal both ways without failing.
    ///
    /// **Divergence.** Java also asserts `q.equals(null)` is false; a
    /// `&dyn Query` cannot be null, so that check has no counterpart.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when the contract does not hold.
    pub fn check_hash_equals(q: &dyn Query) {
        Self::check_equal(q, q);
        // test that a class check is done so that no exception is thrown in
        // the implementation of equals()
        let whacky: Arc<dyn Query> = Arc::new(WhackyQuery);
        Self::check_unequal(q, &*whacky);
    }

    /// `checkEqual(q1, q2)`: `assertEquals(q1, q2)` and equal hash codes.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` (`expected:<q1> but was:<q2>`).
    pub fn check_equal(q1: &dyn Query, q2: &dyn Query) {
        if !q1.query_eq(q2) {
            Assert::fail(&format!(
                "expected:<{}> but was:<{}>",
                query_to_string(q1),
                query_to_string(q2)
            ));
        }
        Assert::assert_equals(q1.query_hash(), q2.query_hash());
    }

    /// `checkUnequal(q1, q2)`: neither query equals the other.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` (`<q1> equal to <q2>`).
    pub fn check_unequal(q1: &dyn Query, q2: &dyn Query) {
        Assert::assert_true_msg(
            &format!("{} equal to {}", query_to_string(q1), query_to_string(q2)),
            !q1.query_eq(q2),
        );
        Assert::assert_true_msg(
            &format!("{} equal to {}", query_to_string(q2), query_to_string(q1)),
            !q2.query_eq(q1),
        );
    }

    /// `checkExplanations(q, s)`: deep check that explanations of a query
    /// "score" correctly, `CheckHits.checkExplanations(q, null, s, true)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_explanations(q: &Arc<dyn Query>, s: &IndexSearcher) -> Result<()> {
        CheckHits::check_explanations_deep(q, None, s, true)
    }

    /// `check(random, q1, s)`: `check(random, q1, s, true)`.
    ///
    /// # Panics
    ///
    /// As [`check_searcher_wrap`](Self::check_searcher_wrap).
    pub fn check_searcher(random: &dyn Random, q1: &Arc<dyn Query>, s: Option<&IndexSearcher>) {
        Self::check_searcher_wrap(random, q1, s, true);
    }

    /// `check(random, q1, s, wrap)`: various query sanity checks on a
    /// searcher — [`check`](Self::check), `checkFirstSkipTo`, `checkSkipTo`,
    /// `checkBulkScorerSkipTo`, `checkCount`, the same checks over the three
    /// `wrapUnderlyingReader` searchers when `wrap`, `checkExplanations` and
    /// `CheckHits.checkMatches`.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when a check fails, and a
    /// `RuntimeException` around an I/O error.
    pub fn check_searcher_wrap(
        random: &dyn Random,
        q1: &Arc<dyn Query>,
        s: Option<&IndexSearcher>,
        wrap: bool,
    ) {
        let result = (|| -> Result<()> {
            Self::check(&**q1);
            if let Some(s) = s {
                Self::check_first_skip_to(q1, s)?;
                Self::check_skip_to(q1, s)?;
                Self::check_bulk_scorer_skip_to(random, q1, s)?;
                Self::check_count(q1, s)?;
                if wrap {
                    let w = Self::wrap_underlying_reader(random, s, -1)?;
                    Self::check_searcher_wrap(random, q1, Some(&w), false);
                    let w = Self::wrap_underlying_reader(random, s, 0)?;
                    Self::check_searcher_wrap(random, q1, Some(&w), false);
                    let w = Self::wrap_underlying_reader(random, s, 1)?;
                    Self::check_searcher_wrap(random, q1, Some(&w), false);
                }
                Self::check_explanations(q1, s)?;
                CheckHits::check_matches(q1, s)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            runtime_exception(e);
        }
    }

    /// `wrapUnderlyingReader(random, s, edge)`: a new searcher whose reader is
    /// a `MultiReader` containing the reader of `s` and several "empty"
    /// readers, some of which have deleted documents; it behaves exactly as
    /// `s`. `s`'s reader is the first sub when `edge` is negative, in the
    /// middle when it is 0 and the last sub when it is positive.
    ///
    /// # Errors
    ///
    /// Propagates the `MultiReader` construction errors.
    pub fn wrap_underlying_reader(
        _random: &dyn Random,
        s: &IndexSearcher,
        edge: i32,
    ) -> Result<IndexSearcher> {
        let r = Arc::clone(s.get_index_reader());
        let multi = |subs: Vec<Arc<dyn IndexReader>>| -> Result<Arc<dyn IndexReader>> {
            Ok(Arc::new(MultiReader::new_closing(subs)?))
        };

        // we can't put deleted docs before the nested reader, because
        // it will throw off the docIds
        let readers: Vec<Arc<dyn IndexReader>> = vec![
            if edge < 0 {
                Arc::clone(&r)
            } else {
                multi(vec![])?
            },
            multi(vec![])?,
            multi(vec![
                if edge < 0 {
                    Self::empty_reader(4)
                } else {
                    multi(vec![])?
                },
                multi(vec![])?,
                if 0 == edge {
                    Arc::clone(&r)
                } else {
                    multi(vec![])?
                },
            ])?,
            if 0 < edge {
                multi(vec![])?
            } else {
                Self::empty_reader(7)
            },
            multi(vec![])?,
            multi(vec![
                if 0 < edge {
                    multi(vec![])?
                } else {
                    Self::empty_reader(5)
                },
                multi(vec![])?,
                if 0 < edge {
                    Arc::clone(&r)
                } else {
                    multi(vec![])?
                },
            ])?,
        ];

        let mut out = LuceneTestCase::new_searcher(Arc::new(MultiReader::new_closing(readers)?));
        out.set_similarity(Arc::clone(s.get_similarity()));
        Ok(out)
    }

    /// The private `emptyReader(maxDoc)`: a leaf with `maxDoc` documents, all
    /// deleted, and no field.
    fn empty_reader(max_doc: i32) -> Arc<dyn IndexReader> {
        Arc::new(EmptyReader {
            core: IndexReaderCore::new(),
            max_doc,
        })
    }

    /// `checkSkipTo(q, s)`: alternates scorer `advance()` and `next()` calls
    /// in seven orders and ensures a hit collector receives the same docs and
    /// scores.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_skip_to(q: &Arc<dyn Query>, s: &IndexSearcher) -> Result<()> {
        let reader_context_array = Arc::clone(s.get_top_reader_context()).leaves();

        const SKIP_OP: i32 = 0;
        const NEXT_OP: i32 = 1;
        const ORDERS: [&[i32]; 7] = [
            &[NEXT_OP],
            &[SKIP_OP],
            &[SKIP_OP, NEXT_OP],
            &[NEXT_OP, SKIP_OP],
            &[SKIP_OP, SKIP_OP, NEXT_OP, NEXT_OP],
            &[NEXT_OP, NEXT_OP, SKIP_OP, SKIP_OP],
            &[SKIP_OP, SKIP_OP, SKIP_OP, NEXT_OP, NEXT_OP],
        ];
        for order in ORDERS {
            let mut collector = SkipToCollector {
                q,
                s,
                reader_context_array: &reader_context_array,
                order,
                opidx: 0,
                last_doc: -1,
                // FUTURE: ensure scorer.doc()==-1
                max_diff: 1e-5,
                last_reader: None,
                scorer: None,
                leaf_ptr: 0,
            };
            s.search_with_collector(Arc::clone(q), &mut collector)?;

            if let Some(last_reader) = collector.last_reader.clone() {
                let live_docs = last_reader.get_live_docs();
                Self::assert_no_past_segment_end(
                    q,
                    s,
                    last_reader,
                    collector.last_doc,
                    live_docs.as_deref(),
                )?;
            }
        }
        Ok(())
    }

    /// `checkFirstSkipTo(q, s)`: the first `advance` on just created scorers
    /// always goes to the right doc.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_first_skip_to(q: &Arc<dyn Query>, s: &IndexSearcher) -> Result<()> {
        let context = Arc::clone(s.get_top_reader_context()).leaves();
        let rewritten = s.rewrite(Arc::clone(q))?;
        let mut collector = FirstSkipToCollector {
            q,
            s,
            context: &context,
            w: s.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?,
            max_diff: 1e-3,
            last_doc: -1,
            last_reader: None,
            leaf_ptr: 0,
            interval_times32: 32,
        };
        s.search_with_collector(Arc::clone(q), &mut collector)?;

        if let Some(last_reader) = collector.last_reader.clone() {
            let live_docs = last_reader.get_live_docs();
            Self::assert_no_past_segment_end(
                q,
                s,
                last_reader,
                collector.last_doc,
                live_docs.as_deref(),
            )?;
        }
        Ok(())
    }

    /// The private `assertNoPastSegmentEnd`: the query's scorer on
    /// `previous_reader` has no more matching live documents after
    /// `last_doc`.
    fn assert_no_past_segment_end(
        q: &Arc<dyn Query>,
        s: &IndexSearcher,
        previous_reader: Arc<dyn LeafReader>,
        last_doc: i32,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()> {
        let mut index_searcher = LuceneTestCase::new_searcher_wrap(previous_reader, false);
        index_searcher.set_similarity(Arc::clone(s.get_similarity()));
        let rewritten = index_searcher.rewrite(Arc::clone(q))?;
        let w = index_searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        // `(LeafReaderContext) indexSearcher.getTopReaderContext()`
        let ctx = Arc::clone(index_searcher.get_top_reader_context())
            .as_leaf()
            .unwrap_or_else(|| {
                throw(
                    "java.lang.ClassCastException",
                    "class org.apache.lucene.index.CompositeReaderContext cannot be cast to \
                     class org.apache.lucene.index.LeafReaderContext",
                )
            });
        if let Some(mut scorer) = w.scorer(&ctx)? {
            let mut more = false;
            let mut d = scorer.iterator().advance(last_doc + 1)?;
            while d != NO_MORE_DOCS {
                if live_docs.is_none_or(|bits| bits.get(d as usize)) {
                    more = true;
                    break;
                }
                d = scorer.iterator().next_doc()?;
            }
            Assert::assert_false_msg(
                &format!(
                    "query's last doc was {} but advance({}) got to {}",
                    last_doc,
                    last_doc + 1,
                    scorer.doc_id()
                ),
                more,
            );
        }
        Ok(())
    }

    /// The private `assertDocAndScore`: the shadow scorer's position and score
    /// match the outer scorer's for `doc`; prints a diagnostic on failure
    /// under `VERBOSE`.
    #[allow(clippy::too_many_arguments)] // Java's signature
    fn assert_doc_and_score(
        doc: i32,
        score: f32,
        more: bool,
        scorer: &mut dyn Scorer,
        max_diff: f32,
        order: &[i32],
        op: i32,
        skip_op: i32,
        q: &Arc<dyn Query>,
    ) -> Result<()> {
        let scorer_doc = scorer.doc_id();
        let scorer_score = scorer.score()?;
        let scorer_score2 = scorer.score()?;
        let score_diff = (score - scorer_score).abs();
        let scorer_diff = (scorer_score2 - scorer_score).abs();
        let checks = catch_unwind(AssertUnwindSafe(|| {
            Assert::assert_true(more);
            Assert::assert_equals_msg(
                Some(&format!("scorerDoc={scorer_doc},doc={doc}")),
                i64::from(scorer_doc),
                i64::from(doc),
            );
            Assert::assert_true_msg(
                &format!(
                    "score={}, scorerScore={}",
                    float_to_string(score),
                    float_to_string(scorer_score)
                ),
                score_diff <= max_diff,
            );
            Assert::assert_true_msg(
                &format!(
                    "scorerScorer={}, scorerScore2={}",
                    float_to_string(scorer_score),
                    float_to_string(scorer_score2)
                ),
                scorer_diff <= max_diff,
            );
        }));
        if let Err(failure) = checks {
            if *VERBOSE {
                let mut sbord = String::new();
                for &o in order {
                    sbord.push_str(if o == skip_op { " skip()" } else { " next()" });
                }
                println!(
                    "ERROR matching docs:\n\t{}doc={}, scorerDoc={}\n\t{}tscorer.more={}\n\t{}\
                     scorerScore={} scoreDiff={} maxDiff={}\n\t{}scorerScore2={} scorerDiff={}\
                     \n\thitCollector.doc={} score={}\n\t Scorer=<scorer>\n\t Query={}  \
                     <query class>\n\t Searcher=<searcher>\n\t Order={}\n\t Op={}",
                    if doc != scorer_doc { "--> " } else { "" },
                    doc,
                    scorer_doc,
                    if !more { "--> " } else { "" },
                    more,
                    if score_diff > max_diff { "--> " } else { "" },
                    float_to_string(scorer_score),
                    float_to_string(score_diff),
                    float_to_string(max_diff),
                    if scorer_diff > max_diff { "--> " } else { "" },
                    float_to_string(scorer_score2),
                    float_to_string(scorer_diff),
                    doc,
                    float_to_string(score),
                    query_to_string(&**q),
                    sbord,
                    if op == skip_op { " skip()" } else { " next()" }
                );
            }
            resume_unwind(failure);
        }
        Ok(())
    }

    /// The private `assertAdvanceTo`: a freshly created scorer can advance to
    /// `doc` from position `i`, with the same score.
    fn assert_advance_to(
        doc: i32,
        i: i32,
        score: f32,
        w: &Arc<dyn Weight>,
        leaf_context: &LeafReaderContext,
        max_diff: f32,
    ) -> Result<()> {
        let mut supplier = w
            .scorer_supplier(leaf_context)?
            .unwrap_or_else(|| null_pointer());
        let mut scorer = supplier.get(1)?; // only checking one doc, so leadCost = 1
        let advanced = scorer.iterator().advance(i)?;
        Assert::assert_true_msg(
            &format!("query collected {doc} but advance({i}) says no more docs!"),
            advanced != NO_MORE_DOCS,
        );
        Assert::assert_equals_msg(
            Some(&format!(
                "query collected {doc} but advance({i}) got to {}",
                scorer.doc_id()
            )),
            i64::from(doc),
            i64::from(scorer.doc_id()),
        );
        let advance_score = scorer.score()?;
        let again = scorer.score()?;
        Assert::assert_equals_float_msg(
            Some(&format!("unstable advance({i}) score!")),
            advance_score,
            again,
            max_diff,
        );
        Assert::assert_equals_float_msg(
            Some(&format!(
                "query assigned doc {doc} a score of <{}> but advance({i}) has <{}>!",
                float_to_string(score),
                float_to_string(advance_score)
            )),
            score,
            advance_score,
            max_diff,
        );
        Ok(())
    }

    /// `checkBulkScorerSkipTo(r, query, searcher)`: the scorer and the bulk
    /// scorer advance consistently over random windows.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_bulk_scorer_skip_to(
        r: &dyn Random,
        query: &Arc<dyn Query>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let query = searcher.rewrite(Arc::clone(query))?;
        let weight = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
        for context in Arc::clone(searcher.get_index_reader()).leaves()? {
            let scorer: Option<Box<dyn Scorer>> = match weight.scorer_supplier(&context)? {
                None => None,
                // For IndexOrDocValuesQuey, the bulk scorer will use the indexed structure query
                // and the scorer with a lead cost of 0 will use the doc values query.
                Some(mut scorer_supplier) => Some(scorer_supplier.get(0)?),
            };
            let bulk_scorer = weight.bulk_scorer(&context)?;
            let (mut scorer, mut bulk_scorer) = match (scorer, bulk_scorer) {
                (None, None) => continue,
                (scorer, None) => {
                    // ensure scorer is exhausted (it just didnt return null)
                    let mut scorer = scorer.unwrap_or_else(|| null_pointer());
                    let next = scorer.iterator().next_doc()?;
                    java_assert_no_message(next == NO_MORE_DOCS);
                    continue;
                }
                (None, Some(_)) => null_pointer(),
                (Some(scorer), Some(bulk_scorer)) => (scorer, bulk_scorer),
            };
            let mut up_to = 0;
            loop {
                let min = up_to + r.next_int_bound(5);
                let bound = if r.next_boolean() { 10 } else { 5000 };
                let max = min + 1 + r.next_int_bound(bound);
                if scorer.doc_id() < min {
                    scorer.iterator().advance(min)?;
                }
                let next = bulk_scorer
                    .score(
                        &mut BulkSkipToCollector {
                            scorer: &mut *scorer,
                            min,
                            max,
                        },
                        None,
                        min,
                        max,
                    )
                    .map_err(collection_to_lucene)?;
                java_assert_no_message(max <= next);
                java_assert_no_message(next <= scorer.doc_id());
                up_to = max;

                if scorer.doc_id() == NO_MORE_DOCS {
                    bulk_scorer
                        .score(&mut NoMoreMatchesCollector, None, up_to, NO_MORE_DOCS)
                        .map_err(collection_to_lucene)?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// `checkCount(query, searcher)`: counting hits through
    /// `DocIdStream.count()` yields the same result as counting naively.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_count(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<()> {
        let query = searcher.rewrite(Arc::clone(query))?;
        let weight = searcher.create_weight(query, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
        for context in Arc::clone(searcher.get_index_reader()).leaves()? {
            let Some(mut scorer) = weight.bulk_scorer(&context)? else {
                continue;
            };
            let mut naive = NaiveCountCollector {
                expected_count: 0,
                doc_id_stream: false,
            };
            let live_docs = context.leaf_reader().get_live_docs();
            scorer
                .score(&mut naive, live_docs.as_deref(), 0, NO_MORE_DOCS)
                .map_err(collection_to_lucene)?;
            if !naive.doc_id_stream {
                // Don't spend cycles running the query one more time, it doesn't use the
                // DocIdStream optimization.
                continue;
            }
            let Some(mut scorer) = weight.bulk_scorer(&context)? else {
                Assert::assert_equals(0i64, i64::from(naive.expected_count));
                continue;
            };
            let mut counting = StreamCountCollector { actual_count: 0 };
            scorer
                .score(&mut counting, live_docs.as_deref(), 0, NO_MORE_DOCS)
                .map_err(collection_to_lucene)?;
            Assert::assert_equals(
                i64::from(naive.expected_count),
                i64::from(counting.actual_count),
            );
        }
        Ok(())
    }
}

/// A collection error as the I/O error Java's `BulkScorer.score` throws.
fn collection_to_lucene(e: CollectionError) -> rucene::error::LuceneError {
    match e {
        CollectionError::Lucene(e) => e,
        other => rucene::error::LuceneError::illegal_state(format!("{other:?}")),
    }
}

/// The anonymous `SimpleCollector` of `checkSkipTo`.
struct SkipToCollector<'s> {
    q: &'s Arc<dyn Query>,
    s: &'s IndexSearcher,
    reader_context_array: &'s [Arc<LeafReaderContext>],
    order: &'static [i32],
    /// `opidx[0]`.
    opidx: usize,
    /// `lastDoc[0]`.
    last_doc: i32,
    max_diff: f32,
    /// `lastReader[0]`.
    last_reader: Option<Arc<dyn LeafReader>>,
    scorer: Option<Box<dyn Scorer>>,
    leaf_ptr: usize,
}

impl LeafCollector for SkipToCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, sc: &mut dyn Scorable) -> CollectionResult<()> {
        let score = sc.score()?;
        self.last_doc = doc;
        if self.scorer.is_none() {
            let rewritten = self.s.rewrite(Arc::clone(self.q))?;
            let w = self.s.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
            let context = &self.reader_context_array[self.leaf_ptr];
            self.scorer = Some(w.scorer(context)?.unwrap_or_else(|| null_pointer()));
        }
        let scorer = self
            .scorer
            .as_deref_mut()
            .expect("INVARIANT: the scorer was created above");

        let op = self.order[self.opidx % self.order.len()];
        self.opidx += 1;
        let more = if op == 0 {
            let target = scorer.doc_id() + 1;
            scorer.iterator().advance(target)? != NO_MORE_DOCS
        } else {
            scorer.iterator().next_doc()? != NO_MORE_DOCS
        };
        QueryUtils::assert_doc_and_score(
            doc,
            score,
            more,
            scorer,
            self.max_diff,
            self.order,
            op,
            0,
            self.q,
        )?;
        Ok(())
    }
}

impl Collector for SkipToCollector<'_> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader`: confirm that skipping beyond the last doc, on the
        // previous reader, hits NO_MORE_DOCS
        if let Some(last_reader) = self.last_reader.clone() {
            let live_docs = context.leaf_reader().get_live_docs();
            QueryUtils::assert_no_past_segment_end(
                self.q,
                self.s,
                last_reader,
                self.last_doc,
                live_docs.as_deref(),
            )?;
            self.leaf_ptr += 1;
        }
        let reader = context.leaf_reader();
        java_assert_no_message(same_reader(
            &self.reader_context_array[self.leaf_ptr].leaf_reader(),
            &reader,
        ));
        self.last_reader = Some(reader);
        self.scorer = None;
        self.last_doc = -1;
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `SimpleCollector` of `checkFirstSkipTo`.
struct FirstSkipToCollector<'s> {
    q: &'s Arc<dyn Query>,
    s: &'s IndexSearcher,
    context: &'s [Arc<LeafReaderContext>],
    w: Arc<dyn Weight>,
    max_diff: f32,
    /// `lastDoc[0]`.
    last_doc: i32,
    /// `lastReader[0]`.
    last_reader: Option<Arc<dyn LeafReader>>,
    leaf_ptr: usize,
    interval_times32: i64,
}

impl LeafCollector for FirstSkipToCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let score = scorer.score()?;
        // The intervalTimes32 trick helps contain the runtime of this check: first we check
        // every single doc in the interval, then after 32 docs we check every 2 docs, etc.
        let mut i = self.last_doc + 1;
        while i <= doc {
            QueryUtils::assert_advance_to(
                doc,
                i,
                score,
                &self.w,
                &self.context[self.leaf_ptr],
                self.max_diff,
            )?;
            // `i += intervalTimes32++ / 1024`: the compound assignment narrows the long sum.
            let step = self.interval_times32 / 1024;
            self.interval_times32 += 1;
            i = (i64::from(i) + step) as i32;
        }
        self.last_doc = doc;
        Ok(())
    }
}

impl Collector for FirstSkipToCollector<'_> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader`: confirm that skipping beyond the last doc, on the
        // previous reader, hits NO_MORE_DOCS
        if let Some(last_reader) = self.last_reader.clone() {
            let live_docs = context.leaf_reader().get_live_docs();
            QueryUtils::assert_no_past_segment_end(
                self.q,
                self.s,
                last_reader,
                self.last_doc,
                live_docs.as_deref(),
            )?;
            self.leaf_ptr += 1;
        }
        self.last_reader = Some(context.leaf_reader());
        self.last_doc = -1;
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The first anonymous `LeafCollector` of `checkBulkScorerSkipTo`.
struct BulkSkipToCollector<'s> {
    scorer: &'s mut dyn Scorer,
    min: i32,
    max: i32,
}

impl LeafCollector for BulkSkipToCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer2: &mut dyn Scorable) -> CollectionResult<()> {
        java_assert_no_message(doc >= self.min);
        java_assert_no_message(doc < self.max);
        Assert::assert_equals(i64::from(self.scorer.doc_id()), i64::from(doc));
        let expected = self.scorer.score()?;
        let actual = scorer2.score()?;
        Assert::assert_equals_float_msg(None, expected, actual, 0.01);
        self.scorer.iterator().next_doc()?;
        Ok(())
    }
}

/// The second anonymous `LeafCollector` of `checkBulkScorerSkipTo`: no more
/// matches.
struct NoMoreMatchesCollector;

impl LeafCollector for NoMoreMatchesCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // no more matches
        java_assert_no_message(false);
        Ok(())
    }
}

/// The first anonymous `LeafCollector` of `checkCount`: counts the slow way.
struct NaiveCountCollector {
    /// `expectedCount[0]`.
    expected_count: i32,
    /// `docIdStream[0]`.
    doc_id_stream: bool,
}

impl LeafCollector for NaiveCountCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.expected_count += 1;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        // Don't use DocIdStream#count, we want to count the slow way here.
        self.doc_id_stream = true;
        // `LeafCollector.super.collect(stream)`: `stream.forEach(this::collect)`.
        let mut consumer = |doc: i32| self.collect(doc, &mut *scorer);
        stream.for_each(&mut consumer)
    }
}

/// The second anonymous `LeafCollector` of `checkCount`: counts with
/// `DocIdStream.count()`.
struct StreamCountCollector {
    /// `actualCount[0]`.
    actual_count: i32,
}

impl LeafCollector for StreamCountCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.actual_count += 1;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.actual_count += stream.count()?;
        Ok(())
    }
}

/// The anonymous `LeafReader` of `emptyReader(maxDoc)`.
#[derive(Debug)]
struct EmptyReader {
    core: IndexReaderCore,
    max_doc: i32,
}

impl std::fmt::Display for EmptyReader {
    /// `Object.toString()` of the anonymous class (without its hash code).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("org.apache.lucene.tests.search.QueryUtils$1")
    }
}

/// The anonymous `StoredFields` of `emptyReader`: visits nothing.
#[derive(Debug)]
struct EmptyStoredFields;

impl StoredFields for EmptyStoredFields {
    fn document_with_visitor(
        &self,
        _doc_id: i32,
        _visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        Ok(())
    }
}

impl LeafReader for EmptyReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(rucene::index::EmptyTermVectors))
    }

    fn num_docs(&self) -> i32 {
        0
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(EmptyStoredFields))
    }

    fn do_close(&self) -> Result<()> {
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }

    fn get_numeric_doc_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }

    fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(None)
    }

    fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        Ok(None)
    }

    fn get_sorted_numeric_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        Ok(None)
    }

    fn get_sorted_set_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        Ok(None)
    }

    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }

    fn get_doc_values_skipper(&self, _field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Ok(None)
    }

    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(None)
    }

    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(None)
    }

    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_field_infos(&self) -> FieldInfos {
        FieldInfos::empty()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        Some(Box::new(MatchNoBits::new(self.max_doc as usize)))
    }

    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
        Ok(None)
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        LeafMetaData::new(
            Version::LATEST.major as i32,
            Some(Version::LATEST),
            None,
            false,
        )
        .expect("INVARIANT: the latest version is a valid created version")
    }
}
