//! The `BaseLiveDocsFormatTestCase` suites (rmp #352): the inherited cases
//! of `BaseLiveDocsFormatTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the codec Lucene's own subclass binds:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90LiveDocsFormat`
//!   (`lucene/core/src/test`): `TestUtil.getDefaultCodec()`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene_test_framework::index::base_live_docs_format_test_case::{
    self, tear_down, BaseLiveDocsFormatTestCase, LiveDocsTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestLucene90LiveDocsFormat`: `getCodec()` is
/// `TestUtil.getDefaultCodec()`.
struct TestLucene90LiveDocsFormat;

impl BaseLiveDocsFormatTestCase for TestLucene90LiveDocsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        TestUtil::get_default_codec()
    }
}

fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90LiveDocsFormat")
        .extends(&base_live_docs_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            LiveDocsTestInstance::new(Arc::new(TestLucene90LiveDocsFormat))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestLiveDocsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestLiveDocsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestLiveDocsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90LiveDocsFormat`, recording the next `random().nextLong()`
/// after each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestLiveDocsSuiteDraws")
        .extends(&base_live_docs_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            LiveDocsTestInstance::new(Arc::new(TestLucene90LiveDocsFormat))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut LiveDocsTestInstance| {
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

/// Seeded-draw parity with Lucene (rmp #352): on the oracle's seeds, every
/// test of the suite leaves its randomness where Java's does, and the class
/// leaves its own where Java's does (`base_live_docs_draws.tsv`,
/// `BaseFormatSuitesOracle`).
#[test]
fn test_live_docs_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_live_docs_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_live_docs_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}
