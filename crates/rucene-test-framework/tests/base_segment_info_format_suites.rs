//! The `BaseSegmentInfoFormatTestCase` suites (rmp #355): the inherited cases
//! of `BaseIndexFileFormatTestCase` and `BaseSegmentInfoFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codec
//! Lucene's own subclass binds:
//!
//! * `org.apache.lucene.codecs.lucene99.TestLucene99SegmentInfoFormat`
//!   (`lucene/core/src/test`): `getVersions()` returns `{Version.LATEST}` and
//!   `getCodec()` returns `TestUtil.getDefaultCodec()`.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::util::Version;
use rucene_test_framework::index::base_index_file_format_test_case::{
    tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_segment_info_format_test_case::{
    self, BaseSegmentInfoFormatTestCase,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestLucene99SegmentInfoFormat`.
struct Lucene99SegmentInfoSuite;

impl BaseIndexFileFormatTestCase for Lucene99SegmentInfoSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        TestUtil::get_default_codec()
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.segment_infos_add_random_fields(doc);
    }

    fn as_segment_infos(&self) -> Option<&dyn BaseSegmentInfoFormatTestCase> {
        Some(self)
    }
}

impl BaseSegmentInfoFormatTestCase for Lucene99SegmentInfoSuite {
    fn get_versions(&self) -> Vec<Version> {
        vec![Version::LATEST]
    }
}

fn lucene99_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene99.TestLucene99SegmentInfoFormat")
        .extends(&base_segment_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(Lucene99SegmentInfoSuite))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestSegmentInfosSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestSegmentInfosSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestSegmentInfosSuiteDraws` of `BaseFormatSuitesOracle`: the binding of
/// `TestLucene99SegmentInfoFormat`, recording the next `random().nextLong()`
/// after each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestSegmentInfosSuiteDraws")
        .extends(&base_segment_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(Lucene99SegmentInfoSuite))
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

/// Seeded-draw parity with Lucene (rmp #355): every row of
/// `base_segment_infos_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_segment_infos_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_segment_infos_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene99_segment_info_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene99_class);
}

// -----------------------------------------------------------------------------
// One `getCodec()` per test (pass B2, rmp #520)
// -----------------------------------------------------------------------------

/// The `getCodec()` calls of [`CountingSuite`] since the counter was reset.
static GET_CODEC_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `TestLucene99SegmentInfoFormat` counting its `getCodec()` calls.
struct CountingSuite;

impl BaseIndexFileFormatTestCase for CountingSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        GET_CODEC_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        TestUtil::get_default_codec()
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.segment_infos_add_random_fields(doc);
    }

    fn as_segment_infos(&self) -> Option<&dyn BaseSegmentInfoFormatTestCase> {
        Some(self)
    }
}

impl BaseSegmentInfoFormatTestCase for CountingSuite {
    fn get_versions(&self) -> Vec<Version> {
        vec![Version::LATEST]
    }
}

/// A test running `body` on the counting suite, then asserting that it
/// called `getCodec()` exactly once, as each Java test does (`Codec codec =
/// getCodec();`, then `codec` is passed to `new SegmentInfo`).
fn counting_test(
    name: &str,
    body: fn(&dyn BaseSegmentInfoFormatTestCase) -> rucene::error::Result<()>,
) -> Method {
    rucene_test_framework::index::base_index_file_format_test_case::format_test(name, move |c| {
        GET_CODEC_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        body(c.as_segment_infos().expect("a segment-infos case"))?;
        randomizedtesting::junit::Assert::assert_equals(
            1i64,
            GET_CODEC_CALLS.load(std::sync::atomic::Ordering::SeqCst) as i64,
        );
        Ok(())
    })
}

/// The tests whose port called `getCodec()` a second time (in the shared
/// `new SegmentInfo(...)` helper), one of each fixture shape.
fn counting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene99.TestSegmentInfoGetCodecCalls")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CountingSuite))
        }))
        .method(counting_test("testFiles", |c| c.test_files()))
        .method(counting_test("testHasBlocks", |c| c.test_has_blocks()))
        .method(counting_test("testDiagnostics", |c| c.test_diagnostics()))
        .method(counting_test("testExceptionOnCreateOutput", |c| {
            c.test_exception_on_create_output()
        }))
        .method(counting_test("testExceptionOnCloseInput", |c| {
            c.test_exception_on_close_input()
        }))
        .build()
}

/// Each `BaseSegmentInfoFormatTestCase` test obtains its codec once, before
/// the segment id, and builds its `SegmentInfo` with it (rmp #520): the port
/// called `getCodec()` again inside its `new SegmentInfo(...)` helper.
#[test]
fn test_segment_info_tests_call_get_codec_once() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(counting_class);
}
