//! The `BaseCompoundFormatTestCase` suites (rmp #352): the inherited cases
//! of `BaseIndexFileFormatTestCase` and `BaseCompoundFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codec
//! Lucene's own subclass binds:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90CompoundFormat`
//!   (`lucene/core/src/test`): `private final Codec codec =
//!   TestUtil.getDefaultCodec()`.
//!
//! The extra test method of `TestLucene90CompoundFormat`
//! (`testFileLengthOrdering`) belongs to the core-test port (sprint 7) and is
//! not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::index::base_compound_format_test_case::{
    self, BaseCompoundFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// A compound suite whose `codec` field holds the codec `getCodec()` returns,
/// overriding `shouldTestMergeInstance()` when `merge_instance` is set.
struct CompoundSuite {
    codec: Arc<dyn Codec>,
    merge_instance: bool,
}

impl BaseIndexFileFormatTestCase for CompoundSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn should_test_merge_instance(&self) -> bool {
        self.merge_instance
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.compound_add_random_fields(doc);
    }

    fn as_compound(&self) -> Option<&dyn BaseCompoundFormatTestCase> {
        Some(self)
    }
}

impl BaseCompoundFormatTestCase for CompoundSuite {}

/// `TestLucene90CompoundFormat`: `private final Codec codec =
/// TestUtil.getDefaultCodec()`.
fn lucene90_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene90.TestLucene90CompoundFormat")
        .extends(&base_compound_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompoundSuite {
                codec: TestUtil::get_default_codec(),
                merge_instance: false,
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestCompoundSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestCompoundSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestCompoundSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90CompoundFormat`, recording the next `random().nextLong()` after
/// each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestCompoundSuiteDraws")
        .extends(&base_compound_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(CompoundSuite {
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
/// `base_compound_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_compound_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_compound_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene90_compound_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}
