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
//! The extra test methods of `TestLucene90PointsFormat`
//! (`testEstimatePointCount`, `testEstimatePointCount2Dims`, …) belong to the
//! core-test port (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::lucene90::points::{
    Lucene90PointsFormat, Lucene90PointsReader, Lucene90PointsWriter, VERSION_CURRENT,
    VERSION_START,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::Document;
use rucene::error::Result;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_points_format_test_case::{self, BasePointsFormatTestCase};
use rucene_test_framework::util::lucene_test_case::VERBOSE;
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
    let codec = if random.next_boolean() {
        // randomize parameters
        let max_points_in_leaf_node = TestUtil::next_int(&*random, 50, 500);
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
        CodecOf::Field {
            codec: default_codec,
            filter: false,
        }
    };
    PointsSuite { codec }
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
        .build()
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
