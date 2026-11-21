//! The `BaseFieldInfoFormatTestCase` suites (rmp #355): the inherited cases
//! of `BaseIndexFileFormatTestCase` and `BaseFieldInfoFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codecs
//! Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene94.TestLucene94FieldInfosFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90FieldInfosFormat`
//!   (`lucene/core/src/test`): `getCodec()` returns
//!   `TestUtil.getDefaultCodec()`, and `supportDocValuesSkipIndex()` returns
//!   `false`.
//!
//! The own test method of `TestLucene94FieldInfosFormat`
//! (`testVectorSimilarityFuncs`, sprint 9) is declared on its binding.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::lucene94::field_infos_format::SIMILARITY_FUNCTIONS;
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::index::VectorSimilarityFunction;
use rucene_test_framework::index::base_field_info_format_test_case::{
    self, BaseFieldInfoFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    format_test, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A field infos suite: `getCodec()` returns `TestUtil.getDefaultCodec()`;
/// `supportDocValuesSkipIndex()` returns `skip_index`.
struct FieldInfosSuite {
    skip_index: bool,
}

impl BaseIndexFileFormatTestCase for FieldInfosSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        TestUtil::get_default_codec()
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.field_infos_add_random_fields(doc);
    }

    fn as_field_infos(&self) -> Option<&dyn BaseFieldInfoFormatTestCase> {
        Some(self)
    }
}

impl BaseFieldInfoFormatTestCase for FieldInfosSuite {
    fn support_doc_values_skip_index(&self) -> bool {
        self.skip_index
    }
}

/// `TestLucene94FieldInfosFormat`.
fn lucene94_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene94.TestLucene94FieldInfosFormat")
        .extends(&base_field_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(FieldInfosSuite { skip_index: true }))
        }))
        .method(format_test("testVectorSimilarityFuncs", |_| {
            test_vector_similarity_funcs()
        }))
        .build()
}

/// Ensures that all expected vector similarity functions are translatable
/// in the format.
///
/// Port of `TestLucene94FieldInfosFormat.testVectorSimilarityFuncs`.
fn test_vector_similarity_funcs() -> rucene::error::Result<()> {
    // This does not necessarily have to be all similarity functions, but
    // differences should be considered carefully.
    let expected_values = VectorSimilarityFunction::values().to_vec();

    randomizedtesting::junit::Assert::assert_true_msg(
        &format!(
            "expected:<{:?}> but was:<{:?}>",
            SIMILARITY_FUNCTIONS.to_vec(),
            expected_values
        ),
        SIMILARITY_FUNCTIONS.to_vec() == expected_values,
    );
    Ok(())
}

/// `TestLucene90FieldInfosFormat`: `supportDocValuesSkipIndex()` is `false`.
fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90FieldInfosFormat")
        .extends(&base_field_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(FieldInfosSuite { skip_index: false }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestFieldInfosSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestFieldInfosSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestFieldInfosSuiteDraws` of `BaseFormatSuitesOracle`: the binding of
/// `TestLucene94FieldInfosFormat`, recording the next `random().nextLong()`
/// after each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestFieldInfosSuiteDraws")
        .extends(&base_field_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(FieldInfosSuite { skip_index: true }))
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
/// `base_field_infos_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_field_infos_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_field_infos_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene94_field_infos_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene94_class);
}

#[test]
fn test_lucene90_field_infos_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}
