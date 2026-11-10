//! The `BasePointsFormatTestCase` suites (rmp #354): the inherited cases of
//! `BaseIndexFileFormatTestCase` and `BasePointsFormatTestCase` run, under
//! the ported runner and `LuceneTestCase`'s rule chain, on the codecs
//! Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90PointsFormat`
//!   (`lucene/core/src/test`): its constructor draws whether to randomize the
//!   BKD parameters; if so the codec is a `FilterCodec` "impersonating" the
//!   default codec whose points format writes with a `maxPointsInLeafNode`
//!   of 50 to 500 and a `maxMBSortInHeap` of 3 to 6, else the default codec;
//!   `testMergeStability` assumes the codec is not a `FilterCodec`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90PointsFormatV0`
//!   (`lucene/core/src/test`): an `AssertingCodec` whose points format is
//!   `new Lucene90PointsFormat(VERSION_START)`, built on every call;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingPointsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! The own test methods of `TestLucene90PointsFormat`
//! (`testEstimatePointCount`, `testEstimatePointCount2Dims`, sprint 9) are
//! declared on its binding.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::points::{
    Lucene90PointsFormat, Lucene90PointsReader, Lucene90PointsWriter, VERSION_CURRENT,
    VERSION_START,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::{BinaryPoint, Document};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, IntersectVisitor, LeafReader, PointValues, Relation};
use rucene::util::bkd::BKDConfig;
use rucene::util::BytesRef;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_points_format_test_case::{self, BasePointsFormatTestCase};
use rucene_test_framework::index::MockRandomMergePolicy;
use rucene_test_framework::util::lucene_test_case::{TEST_NIGHTLY, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// The anonymous `PointsFormat` of `TestLucene90PointsFormat`: a
/// `Lucene90PointsWriter` with the drawn parameters, and the stock reader.
#[derive(Debug)]
struct RandomizedPointsFormat {
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
}

impl PointsFormat for RandomizedPointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, write_state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::with_config(
            write_state,
            self.max_points_in_leaf_node,
            self.max_mb_sort_in_heap,
            VERSION_CURRENT,
        )?))
    }

    fn fields_reader(&self, read_state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(read_state)?))
    }
}

/// How a points suite answers `getCodec()`.
enum CodecOf {
    /// A codec built once (a field initializer or the constructor);
    /// `filter` is `codec instanceof FilterCodec`.
    Field { codec: Arc<dyn Codec>, filter: bool },
    /// A codec built on every call.
    Call(fn() -> Arc<dyn Codec>),
}

/// A points suite.
struct PointsSuite {
    codec: CodecOf,
    /// `TestLucene90PointsFormat.maxPointsInLeafNode` (`private final int`);
    /// `None` for the suites that do not declare it.
    max_points_in_leaf_node: Option<i32>,
}

impl PointsSuite {
    /// `codec instanceof FilterCodec`.
    fn is_filter_codec(&self) -> bool {
        matches!(self.codec, CodecOf::Field { filter: true, .. })
    }
}

impl BaseIndexFileFormatTestCase for PointsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Field { codec, .. } => Arc::clone(codec),
            CodecOf::Call(make) => make(),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.points_add_random_fields(doc);
    }

    fn merge_is_stable(&self) -> bool {
        self.points_merge_is_stable()
    }

    fn as_points(&self) -> Option<&dyn BasePointsFormatTestCase> {
        Some(self)
    }
}

impl BasePointsFormatTestCase for PointsSuite {}

/// `new TestLucene90PointsFormat()`.
fn new_lucene90() -> PointsSuite {
    // standard issue
    let default_codec = TestUtil::get_default_codec();
    let random = LuceneTestCase::random();
    let max_points_in_leaf_node;
    let codec = if random.next_boolean() {
        // randomize parameters
        max_points_in_leaf_node = TestUtil::next_int(&*random, 50, 500);
        let max_mb_sort_in_heap = 3.0 + (3.0 * random.next_double());
        if *VERBOSE {
            println!(
                "TEST: using Lucene60PointsFormat with maxPointsInLeafNode={max_points_in_leaf_node} and maxMBSortInHeap={}",
                rucene::util::java_fmt::double_to_string(max_mb_sort_in_heap)
            );
        }

        // sneaky impersonation!
        CodecOf::Field {
            codec: Arc::new(
                FilterCodec::new(default_codec.name().to_string(), Arc::clone(&default_codec))
                    .with_points_format(RandomizedPointsFormat {
                        max_points_in_leaf_node,
                        max_mb_sort_in_heap,
                    }),
            ),
            filter: true,
        }
    } else {
        // standard issue
        max_points_in_leaf_node = BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE;
        CodecOf::Field {
            codec: default_codec,
            filter: false,
        }
    };
    PointsSuite {
        codec,
        max_points_in_leaf_node: Some(max_points_in_leaf_node),
    }
}

fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90PointsFormat")
        .extends(&base_points_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(new_lucene90()))
        }))
        .method(
            Method::new("testMergeStability")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    let suite = inst.subclass::<PointsSuite>();
                    LuceneTestCase::assume_false(
                        "TODO: mess with the parameters and test gets angry!",
                        suite.is_filter_codec(),
                    );
                    // `super.testMergeStability()`
                    suite.test_merge_stability().unwrap_or_else(|e| rethrow(e));
                }),
        )
        .method(lucene90_test(
            "testEstimatePointCount",
            test_estimate_point_count,
        ))
        .method(lucene90_test(
            "testEstimatePointCount2Dims",
            test_estimate_point_count_2_dims,
        ))
        .build()
}

/// A test method of `TestLucene90PointsFormat`, run with its
/// `maxPointsInLeafNode`.
fn lucene90_test(name: &str, body: fn(i32) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut FormatTestInstance| {
            let max_points_in_leaf_node = inst
                .subclass::<PointsSuite>()
                .max_points_in_leaf_node
                .expect("INVARIANT: TestLucene90PointsFormat sets maxPointsInLeafNode");
            body(max_points_in_leaf_node).unwrap_or_else(|e| rethrow(e));
        })
}

/// An anonymous `IntersectVisitor` of the estimate tests: visits nothing,
/// and compares cells with `relation`.
struct EstimateVisitor<F: Fn(&[u8], &[u8]) -> Relation> {
    relation: F,
}

impl<F: Fn(&[u8], &[u8]) -> Relation> IntersectVisitor for EstimateVisitor<F> {
    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        Ok((self.relation)(min_packed_value, max_packed_value))
    }
}

/// `lr.getPointValues("f")`, with the `NullPointerException` of the next
/// call on `null`.
fn point_values_of(lr: &dyn LeafReader, field: &str) -> Result<Box<dyn PointValues>> {
    Ok(lr.get_point_values(field)?.unwrap_or_else(|| {
        Throwable::new(
            "java.lang.NullPointerException",
            "getPointValues returned null",
        )
        .throw()
    }))
}

/// The multi-valued `docCount` check of both estimate tests:
/// `(long) (docCount * (1d - Math.pow((numDocs - pointCount) / points.size(),
/// points.size() / docCount)))`, with Java's `long` divisions.
fn multi_valued_doc_count(doc_count: i64, num_docs: i32, point_count: i64, size: i64) -> i64 {
    let base = ((i64::from(num_docs) - point_count) / size) as f64;
    let exponent = (size / doc_count) as f64;
    (doc_count as f64 * (1f64 - base.powf(exponent))) as i64
}

// TODO: clean up the math/estimation here rather than suppress so many warnings
/// Port of `TestLucene90PointsFormat.testEstimatePointCount`.
fn test_estimate_point_count(max_points_in_leaf_node: i32) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    // Avoid mockRandomMP since it may cause non-optimal merges that make the
    // number of points per leaf hard to predict
    while iwc
        .merge_policy()
        .as_any()
        .is_some_and(|a| a.is::<MockRandomMergePolicy>())
    {
        iwc.set_merge_policy(Some(LuceneTestCase::new_merge_policy()))?;
    }
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut point_value = vec![0u8; 3];
    let mut unique_point_value = vec![0u8; 3];
    LuceneTestCase::random().next_bytes(&mut unique_point_value);
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10000)
    } else {
        LuceneTestCase::at_least(500)
    }; // at night, make sure we have several leaves
    let multi_values = LuceneTestCase::random().next_boolean();
    let mut total_values = 0i32;
    for i in 0..num_docs {
        let mut doc = Document::new();
        if i == num_docs / 2 {
            total_values += 1;
            doc.add(Box::new(BinaryPoint::new(
                "f",
                &[BytesRef::new(unique_point_value.clone())],
            )?));
        } else {
            let num_values = if multi_values {
                TestUtil::next_int(&*LuceneTestCase::random(), 2, 100)
            } else {
                1
            };
            for _ in 0..num_values {
                loop {
                    LuceneTestCase::random().next_bytes(&mut point_value);
                    if point_value != unique_point_value {
                        break;
                    }
                }
                doc.add(Box::new(BinaryPoint::new(
                    "f",
                    &[BytesRef::new(point_value.clone())],
                )?));
                total_values += 1;
            }
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let lr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let points = point_values_of(&*lr, "f")?;

    let mut all_points_visitor = EstimateVisitor {
        relation: |_: &[u8], _: &[u8]| Relation::CellInsideQuery,
    };

    Assert::assert_equals(
        i64::from(total_values),
        points.estimate_point_count(&mut all_points_visitor)?,
    );
    Assert::assert_equals(
        i64::from(num_docs),
        points.estimate_doc_count(&mut all_points_visitor)?,
    );

    let mut no_points_visitor = EstimateVisitor {
        relation: |_: &[u8], _: &[u8]| Relation::CellOutsideQuery,
    };

    // Return 0 if no points match
    Assert::assert_equals(0, points.estimate_point_count(&mut no_points_visitor)?);
    Assert::assert_equals(0, points.estimate_doc_count(&mut no_points_visitor)?);

    let unique = unique_point_value.clone();
    let mut one_point_match_visitor = EstimateVisitor {
        relation: move |min_packed_value: &[u8], max_packed_value: &[u8]| {
            if unique[0..3] > max_packed_value[0..3] || unique[0..3] < min_packed_value[0..3] {
                return Relation::CellOutsideQuery;
            }
            Relation::CellCrossesQuery
        },
    };

    // If only one point matches, then the point count is (maxPointsInLeafNode + 1) / 2
    // in general, or maybe 2x that if the point is a split value
    let point_count = points.estimate_point_count(&mut one_point_match_visitor)?;
    let last_node_point_count = i64::from(total_values % max_points_in_leaf_node);
    let half = i64::from((max_points_in_leaf_node + 1) / 2);
    Assert::assert_true_msg(
        &point_count.to_string(),
        point_count == half // common case
            || point_count == (last_node_point_count + 1) / 2 // not fully populated leaf
            || point_count == 2 * half // if the point is a split value
            || point_count == half + ((last_node_point_count + 1) / 2), // if the point is a split value and one leaf is not fully populated
    );

    let doc_count = points.estimate_doc_count(&mut one_point_match_visitor)?;

    if multi_values {
        Assert::assert_equals(
            doc_count,
            multi_valued_doc_count(doc_count, num_docs, point_count, points.size()?),
        );
    } else {
        Assert::assert_equals(point_count.min(i64::from(num_docs)), doc_count);
    }
    drop(points);
    r.close()?;
    dir.close()
}

// The tree is always balanced in the N dims case, and leaves are
// not all full so things are a bit different
// TODO: clean up the math/estimation here rather than suppress so many warnings
/// Port of `TestLucene90PointsFormat.testEstimatePointCount2Dims`.
fn test_estimate_point_count_2_dims(max_points_in_leaf_node: i32) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut point_value = [vec![0u8; 3], vec![0u8; 3]];
    let mut unique_point_value = [vec![0u8; 3], vec![0u8; 3]];
    LuceneTestCase::random().next_bytes(&mut unique_point_value[0]);
    LuceneTestCase::random().next_bytes(&mut unique_point_value[1]);
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(10000)
    } else {
        LuceneTestCase::at_least(1000)
    }; // in nightly, make sure we have several leaves
    let multi_values = LuceneTestCase::random().next_boolean();
    let mut total_values = 0i32;
    for i in 0..num_docs {
        let mut doc = Document::new();
        if i == num_docs / 2 {
            doc.add(Box::new(BinaryPoint::new(
                "f",
                &[
                    BytesRef::new(unique_point_value[0].clone()),
                    BytesRef::new(unique_point_value[1].clone()),
                ],
            )?));
            total_values += 1;
        } else {
            let num_values = if multi_values {
                TestUtil::next_int(&*LuceneTestCase::random(), 2, 100)
            } else {
                1
            };
            for _ in 0..num_values {
                loop {
                    LuceneTestCase::random().next_bytes(&mut point_value[0]);
                    LuceneTestCase::random().next_bytes(&mut point_value[1]);
                    if !(point_value[0] == unique_point_value[0]
                        || point_value[1] == unique_point_value[1])
                    {
                        break;
                    }
                }
                doc.add(Box::new(BinaryPoint::new(
                    "f",
                    &[
                        BytesRef::new(point_value[0].clone()),
                        BytesRef::new(point_value[1].clone()),
                    ],
                )?));
                total_values += 1;
            }
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let lr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let points = point_values_of(&*lr, "f")?;

    let mut all_points_visitor = EstimateVisitor {
        relation: |_: &[u8], _: &[u8]| Relation::CellInsideQuery,
    };

    Assert::assert_equals(
        i64::from(total_values),
        points.estimate_point_count(&mut all_points_visitor)?,
    );
    Assert::assert_equals(
        i64::from(num_docs),
        points.estimate_doc_count(&mut all_points_visitor)?,
    );

    let mut no_points_visitor = EstimateVisitor {
        relation: |_: &[u8], _: &[u8]| Relation::CellOutsideQuery,
    };

    // Return 0 if no points match
    Assert::assert_equals(0, points.estimate_point_count(&mut no_points_visitor)?);
    Assert::assert_equals(0, points.estimate_doc_count(&mut no_points_visitor)?);

    let unique = unique_point_value.clone();
    let mut one_point_match_visitor = EstimateVisitor {
        relation: move |min_packed_value: &[u8], max_packed_value: &[u8]| {
            for dim in 0..2 {
                if unique[dim][0..3] > max_packed_value[dim * 3..dim * 3 + 3]
                    || unique[dim][0..3] < min_packed_value[dim * 3..dim * 3 + 3]
                {
                    return Relation::CellOutsideQuery;
                }
            }
            Relation::CellCrossesQuery
        },
    };

    let point_count = points.estimate_point_count(&mut one_point_match_visitor)?;
    let last_node_point_count = i64::from(total_values % max_points_in_leaf_node);
    let half = i64::from((max_points_in_leaf_node + 1) / 2);
    Assert::assert_true_msg(
        &point_count.to_string(),
        point_count == half // common case
            || point_count == (last_node_point_count + 1) / 2 // not fully populated leaf
            || point_count == 2 * half // if the point is a split value
            || point_count == half + ((last_node_point_count + 1) / 2)
            // in extreme cases, a point can be shared by 4 leaves
            || point_count == 4 * half
            || point_count == 3 * half + ((last_node_point_count + 1) / 2),
    );

    let doc_count = points.estimate_doc_count(&mut one_point_match_visitor)?;
    if multi_values {
        Assert::assert_equals(
            doc_count,
            multi_valued_doc_count(doc_count, num_docs, point_count, points.size()?),
        );
    } else {
        Assert::assert_equals(point_count.min(i64::from(num_docs)), doc_count);
    }
    drop(points);
    r.close()?;
    dir.close()
}

/// `TestLucene90PointsFormatV0.getCodec()`: `new AssertingCodec() {
/// pointsFormat() { return new Lucene90PointsFormat(VERSION_START); } }`.
fn v0_codec() -> Arc<dyn Codec> {
    Arc::new(
        FilterCodec::new("Asserting", Arc::new(AssertingCodec::new())).with_points_format(
            Lucene90PointsFormat::with_version(VERSION_START).unwrap_or_else(|e| rethrow(e)),
        ),
    )
}

fn v0_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90PointsFormatV0")
        .extends(&base_points_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(PointsSuite {
                max_points_in_leaf_node: None,
                codec: CodecOf::Call(v0_codec),
            }))
        }))
        .build()
}

fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingPointsFormat")
        .extends(&base_points_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(PointsSuite {
                max_points_in_leaf_node: None,
                codec: CodecOf::Field {
                    codec: Arc::new(AssertingCodec::new()),
                    filter: false,
                },
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestPointsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestPointsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestPointsSuiteDraws` of `BaseFormatSuitesOracle`: the suite on
/// `TestUtil.getDefaultCodec()` (no draw in its constructor), recording the
/// next `random().nextLong()` after each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestPointsSuiteDraws")
        .extends(&base_points_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(PointsSuite {
                max_points_in_leaf_node: None,
                codec: CodecOf::Field {
                    codec: TestUtil::get_default_codec(),
                    filter: false,
                },
            }))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    DRAWS.lock().unwrap().push(format!(
                        "{}\tnext\t{}",
                        LuceneTestCase::get_test_name().unwrap_or_default(),
                        LuceneTestCase::random().next_long()
                    ));
                    tear_down(inst);
                }),
        )
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(randomizedtesting::junit::AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #354): every row of
/// `base_points_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_points_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_points_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_points_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_points_format_v0() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(v0_class);
}

#[test]
fn test_asserting_points_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
