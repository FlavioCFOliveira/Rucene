//! The `BaseTermVectorsFormatTestCase` suites (rmp #352, #359): the
//! inherited cases of `BaseIndexFileFormatTestCase` and
//! `BaseTermVectorsFormatTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90TermVectorsFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.compressing.TestCompressingTermVectorsFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `CompressingCodec.randomInstance(random())` when `TEST_NIGHTLY`, else
//!   `CompressingCodec.reasonableInstance(random())`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingTermVectorsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`,
//!   and `getReadPastLastPositionExceptionClass()` returns
//!   `AssertionError.class`.
//!
//! `org.apache.lucene.index.TestTermVectors` (`lucene/core/src/test`)
//! extends `LuceneTestCase`, not this suite (it only reuses
//! `RandomTokenStream`), so it binds nothing here. The extra test methods of
//! the core classes (`testSkipRedundantPrefetches`, `testNoOrds`,
//! `testChunkCleanup`) belong to the core-test port (sprint 7) and are not
//! declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::base_index_file_format_test_case::{
    tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_term_vectors_format_test_case::{
    self, BaseTermVectorsFormatTestCase,
};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// How a term vectors suite answers `getCodec()`.
enum CodecOf {
    /// A codec built once (a field initializer).
    Field(Arc<dyn Codec>),
    /// A codec built on every call.
    Call(fn() -> Arc<dyn Codec>),
}

/// A term vectors suite: `getCodec()` as `codec` says, and
/// `getReadPastLastPositionExceptionClass()` returning `read_past`.
struct TermVectorsSuite {
    codec: CodecOf,
    read_past: &'static str,
}

impl BaseIndexFileFormatTestCase for TermVectorsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Field(codec) => Arc::clone(codec),
            CodecOf::Call(codec) => codec(),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.term_vectors_add_random_fields(doc);
    }

    fn as_term_vectors(&self) -> Option<&dyn BaseTermVectorsFormatTestCase> {
        Some(self)
    }
}

impl BaseTermVectorsFormatTestCase for TermVectorsSuite {
    fn get_read_past_last_position_exception_class(&self) -> &'static str {
        self.read_past
    }
}

/// The class `name`, extending `BaseTermVectorsFormatTestCase`, whose
/// instances `suite` builds.
fn suite_class(name: &str, suite: fn() -> TermVectorsSuite) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_term_vectors_format_test_case::class())
        .constructor(Constructor::no_args(move || {
            FormatTestInstance::new(Arc::new(suite()))
        }))
        .build()
}

/// `TestLucene90TermVectorsFormat.getCodec()`: `TestUtil.getDefaultCodec()`.
fn lucene90_suite() -> TermVectorsSuite {
    TermVectorsSuite {
        codec: CodecOf::Call(TestUtil::get_default_codec),
        read_past: "java.lang.IllegalStateException",
    }
}

/// `TestLucene90TermVectorsFormat` (`getCodec()` only).
fn lucene90_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90TermVectorsFormat",
        lucene90_suite,
    )
}

/// `TestCompressingTermVectorsFormat` (`getCodec()` only).
fn compressing_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.compressing.TestCompressingTermVectorsFormat",
        || TermVectorsSuite {
            codec: CodecOf::Call(|| {
                let codec = if *TEST_NIGHTLY {
                    CompressingCodec::random_instance(&*LuceneTestCase::random())
                } else {
                    CompressingCodec::reasonable_instance(&*LuceneTestCase::random())
                };
                Arc::new(codec.unwrap_or_else(|e| rethrow(e)))
            }),
            read_past: "java.lang.IllegalStateException",
        },
    )
}

/// `TestAssertingTermVectorsFormat`: `private final Codec codec = new
/// AssertingCodec()`; reading past the last position throws
/// `AssertionError`.
fn asserting_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.tests.codecs.asserting.TestAssertingTermVectorsFormat",
        || TermVectorsSuite {
            codec: CodecOf::Field(Arc::new(AssertingCodec::new())),
            read_past: "java.lang.AssertionError",
        },
    )
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestTermVectorsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestTermVectorsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestTermVectorsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90TermVectorsFormat`, recording the next `random().nextLong()` after
/// each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestTermVectorsSuiteDraws")
        .extends(&base_term_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(lucene90_suite()))
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
/// `base_term_vectors_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_term_vectors_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_term_vectors_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_compressing_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(compressing_class);
}

#[test]
fn test_asserting_term_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
