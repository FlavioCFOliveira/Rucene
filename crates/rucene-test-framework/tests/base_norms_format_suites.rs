//! The `BaseNormsFormatTestCase` suites (rmp #352, #359): the inherited
//! cases of `BaseIndexFileFormatTestCase` and `BaseNormsFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codecs
//! Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90NormsFormat`
//!   (`lucene/core/src/test`): `private final Codec codec =
//!   TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90NormsFormatMergeInstance`
//!   (`lucene/core/src/test`): extends `TestLucene90NormsFormat`, overriding
//!   `shouldTestMergeInstance()` to return `true`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingNormsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_norms_format_test_case::{self, BaseNormsFormatTestCase};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A norms suite whose `codec` field holds the codec `getCodec()` returns,
/// overriding `shouldTestMergeInstance()` when `merge_instance` is set.
struct NormsSuite {
    codec: Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for NormsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.norms_add_random_fields(doc);
    }

    fn as_norms(&self) -> Option<&dyn BaseNormsFormatTestCase> {
        Some(self)
    }
}

impl BaseNormsFormatTestCase for NormsSuite {}

/// `TestLucene90NormsFormat`: `private final Codec codec =
/// TestUtil.getDefaultCodec()`.
fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90NormsFormat")
        .extends(&base_norms_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(NormsSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }))
        .build()
}

/// `TestLucene90NormsFormatMergeInstance`: `extends TestLucene90NormsFormat`
/// and overrides `shouldTestMergeInstance()` to return `true`.
fn merge_instance_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90NormsFormatMergeInstance")
        .extends(&lucene90_class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(NormsSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: true,
            }))
        }))
        .build()
}

/// `TestAssertingNormsFormat`: `private final Codec codec = new
/// AssertingCodec()`.
fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingNormsFormat")
        .extends(&base_norms_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(NormsSuite {
                codec: Arc::new(AssertingCodec::new()),
                merge_instance: false,
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestNormsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestNormsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestNormsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90NormsFormat`, recording the next `random().nextLong()` after
/// each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestNormsSuiteDraws")
        .extends(&base_norms_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(NormsSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
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

/// Seeded-draw parity with Lucene (rmp #352): every row of
/// `base_norms_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_norms_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_norms_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_norms_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_norms_format_merge_instance() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(merge_instance_class);
}

#[test]
fn test_asserting_norms_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
