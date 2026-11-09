//! The doc values suites (rmp #353, #359): the inherited cases of
//! `BaseIndexFileFormatTestCase`, `LegacyBaseDocValuesFormatTestCase`,
//! `BaseDocValuesFormatTestCase` and, for the classes that extend it,
//! `BaseCompressingDocValuesFormatTestCase`, run under the ported runner and
//! `LuceneTestCase`'s rule chain on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormat`
//!   (`lucene/core/src/test`, extends `BaseCompressingDocValuesFormatTestCase`):
//!   `private final Codec codec = TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatMergeInstance`
//!   (`lucene/core/src/test`): extends `TestLucene90DocValuesFormat`,
//!   overriding `shouldTestMergeInstance()` to return `true`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatVariableSkipInterval`
//!   (`lucene/core/src/test`, extends `BaseDocValuesFormatTestCase`):
//!   `getCodec()` returns `TestUtil.alwaysDocValuesFormat(new
//!   Lucene90DocValuesFormat(random().nextInt(4, 16)))`;
//! * `org.apache.lucene.codecs.perfield.TestPerFieldDocValuesFormat`
//!   (`lucene/core/src/test`, extends `BaseDocValuesFormatTestCase`):
//!   `setUp()` sets `codec = new RandomCodec(new Random(random().nextLong()),
//!   Collections.emptySet())` before `super.setUp()`, and
//!   `codecAcceptsHugeBinaryValues(field)` returns
//!   `TestUtil.fieldSupportsHugeBinaryDocValues(field)`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingDocValuesFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! The extra test methods of the core classes (those of
//! `TestLucene90DocValuesFormat` included, which
//! `TestLucene90DocValuesFormatMergeInstance` inherits) belong to the
//! core-test port (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::lucene90::doc_values::Lucene90DocValuesFormat;
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::base_compressing_doc_values_format_test_case::{
    self, BaseCompressingDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_doc_values_format_test_case::{
    self, BaseDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    set_up, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase;
use rucene_test_framework::index::RandomCodec;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A subclass of `BaseCompressingDocValuesFormatTestCase` whose `codec`
/// field holds the codec `getCodec()` returns, overriding
/// `shouldTestMergeInstance()` when `merge_instance` is set.
struct CompressingDocValuesSuite {
    codec: Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for CompressingDocValuesSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.doc_values_add_random_fields(doc);
    }

    fn as_doc_values(&self) -> Option<&dyn LegacyBaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl LegacyBaseDocValuesFormatTestCase for CompressingDocValuesSuite {
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        Some(self)
    }

    fn as_compressing_doc_values(&self) -> Option<&dyn BaseCompressingDocValuesFormatTestCase> {
        Some(self)
    }
}

impl BaseDocValuesFormatTestCase for CompressingDocValuesSuite {}

impl BaseCompressingDocValuesFormatTestCase for CompressingDocValuesSuite {}

/// `TestLucene90DocValuesFormat` (`getCodec()` only).
fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormat")
        .extends(&base_compressing_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }))
        .build()
}

/// `TestLucene90DocValuesFormatMergeInstance`: `extends
/// TestLucene90DocValuesFormat` and overrides `shouldTestMergeInstance()` to
/// return `true`.
fn merge_instance_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatMergeInstance")
        .extends(&lucene90_class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: true,
            }))
        }))
        .build()
}

/// How a `BaseDocValuesFormatTestCase` subclass answers `getCodec()`.
enum CodecOf {
    /// A codec built once (a field initializer).
    Field(Arc<dyn Codec>),
    /// A codec built on every call.
    Call(fn() -> Arc<dyn Codec>),
    /// The codec `setUp()` sets (`TestPerFieldDocValuesFormat`).
    SetUp(Mutex<Option<Arc<dyn Codec>>>),
}

/// A subclass of `BaseDocValuesFormatTestCase`: `getCodec()` as `codec`
/// says; `huge_binary_by_field` overrides `codecAcceptsHugeBinaryValues`
/// with `TestUtil.fieldSupportsHugeBinaryDocValues(field)`.
struct DocValuesSuite {
    codec: CodecOf,
    huge_binary_by_field: bool,
}

impl BaseIndexFileFormatTestCase for DocValuesSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Field(codec) => Arc::clone(codec),
            CodecOf::Call(codec) => codec(),
            CodecOf::SetUp(codec) => codec
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_else(|| {
                    randomizedtesting::jdk::exception::throw_without_message(
                        "java.lang.NullPointerException",
                    )
                }),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.doc_values_add_random_fields(doc);
    }

    fn as_doc_values(&self) -> Option<&dyn LegacyBaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl LegacyBaseDocValuesFormatTestCase for DocValuesSuite {
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        Some(self)
    }

    fn codec_accepts_huge_binary_values(&self, field: &str) -> bool {
        if self.huge_binary_by_field {
            TestUtil::field_supports_huge_binary_doc_values(field)
        } else {
            true
        }
    }
}

impl BaseDocValuesFormatTestCase for DocValuesSuite {}

/// `TestLucene90DocValuesFormatVariableSkipInterval` (`getCodec()` only).
fn variable_skip_interval_class() -> Arc<Class> {
    Class::builder(
        "org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatVariableSkipInterval",
    )
    .extends(&base_doc_values_format_test_case::class())
    .constructor(Constructor::no_args(|| {
        FormatTestInstance::new(Arc::new(DocValuesSuite {
            codec: CodecOf::Call(|| {
                TestUtil::always_doc_values_format(Arc::new(
                    Lucene90DocValuesFormat::with_interval(
                        LuceneTestCase::random().next_int_range(4, 16),
                    ),
                ))
            }),
            huge_binary_by_field: false,
        }))
    }))
    .build()
}

/// `TestPerFieldDocValuesFormat` (`setUp()`, `getCodec()` and
/// `codecAcceptsHugeBinaryValues(field)` only).
fn per_field_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.perfield.TestPerFieldDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(DocValuesSuite {
                codec: CodecOf::SetUp(Mutex::new(None)),
                huge_binary_by_field: true,
            }))
        }))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut FormatTestInstance| {
                let suite = inst.subclass::<DocValuesSuite>();
                if let CodecOf::SetUp(codec) = &suite.codec {
                    let random = JdkRandom::new(LuceneTestCase::random().next_long());
                    *codec.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(Arc::new(RandomCodec::new_avoiding(&random, HashSet::new())));
                }
                set_up(inst);
            },
        ))
        .build()
}

/// `TestAssertingDocValuesFormat`: `private final Codec codec = new
/// AssertingCodec()`.
fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(DocValuesSuite {
                codec: CodecOf::Field(Arc::new(AssertingCodec::new())),
                huge_binary_by_field: false,
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestDocValuesSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestDocValuesSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestDocValuesSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90DocValuesFormat` (so the cases of the three doc values
/// suites), recording the next `random().nextLong()` after each test and
/// after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestDocValuesSuiteDraws")
        .extends(&base_compressing_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompressingDocValuesSuite {
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

/// Seeded-draw parity with Lucene (rmp #353): every row of
/// `base_doc_values_draws.tsv` (`BaseFormatSuitesOracle`) must be equal,
/// run under the JVM system properties the fixture records (they set the
/// `.si` diagnostics, hence segment sizes and `TieredMergePolicy`'s merge
/// order, hence the merged doc order `testSortedSetVariableLengthSingleValuedVsStoredFields`
/// draws over on DEADBEEF).
///
/// One row is left out: the `next` row of `testNumericDocValuesWithSkipperMedium`
/// on C0FFEE. Java itself does not reproduce it: the test's
/// `RandomIndexWriter` config can pick `ConcurrentMergeScheduler`, whose
/// background merge timing changes the segments the reader sees and so the
/// draws (5 Java runs of that seed gave 5 different values, none the
/// fixture's). The test still runs, and its failure rows are still compared.
#[test]
fn test_doc_values_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match_excluding(
        "base_doc_values_draws.tsv",
        draws_class,
        &DRAWS,
        &[("C0FFEE", "testNumericDocValuesWithSkipperMedium")],
    );
}

#[test]
fn test_lucene90_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_doc_values_format_merge_instance() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(merge_instance_class);
}

#[test]
fn test_lucene90_doc_values_format_variable_skip_interval() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(variable_skip_interval_class);
}

#[test]
fn test_per_field_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(per_field_class);
}

#[test]
fn test_asserting_doc_values_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
