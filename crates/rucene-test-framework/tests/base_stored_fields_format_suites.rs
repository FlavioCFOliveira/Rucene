//! The `BaseStoredFieldsFormatTestCase` suites (rmp #351, #359): the
//! inherited cases of `BaseIndexFileFormatTestCase` and
//! `BaseStoredFieldsFormatTestCase` run, under the ported runner and
//! `LuceneTestCase`'s rule chain, on the codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormat`
//!   (`lucene/core/src/test`): `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatHighCompression`
//!   (`lucene/core/src/test`): `new Lucene104Codec(Mode.BEST_COMPRESSION)`;
//! * `org.apache.lucene.codecs.lucene90.compressing.TestCompressingStoredFieldsFormat`
//!   (`lucene/core/src/test`): `CompressingCodec.randomInstance(random())`
//!   when `TEST_NIGHTLY`, else `CompressingCodec.reasonableInstance(random())`;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingStoredFieldsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`.
//!
//! The extra test methods of the three core classes belong to the core-test
//! port (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::codecs::lucene104::codec::{Lucene104Codec, Mode};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_stored_fields_format_test_case::{
    self, BaseStoredFieldsFormatTestCase,
};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_seeds;
use suite_seeds::run_suite;

/// A stored-fields suite that only overrides `getCodec()`, which `codec`
/// answers.
struct StoredFieldsSuite {
    codec: fn() -> Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for StoredFieldsSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        (self.codec)()
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.stored_fields_add_random_fields(doc);
    }

    fn as_stored_fields(&self) -> Option<&dyn BaseStoredFieldsFormatTestCase> {
        Some(self)
    }
}

impl BaseStoredFieldsFormatTestCase for StoredFieldsSuite {}

/// The class `name`, extending `BaseStoredFieldsFormatTestCase`, whose
/// `getCodec()` is `codec`.
fn suite_class(name: &str, codec: fn() -> Arc<dyn Codec>) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(move || {
            FormatTestInstance::new(Arc::new(StoredFieldsSuite { codec }))
        }))
        .build()
}

/// `TestLucene90StoredFieldsFormat.getCodec()`: `TestUtil.getDefaultCodec()`.
fn lucene90_codec() -> Arc<dyn Codec> {
    TestUtil::get_default_codec()
}

/// `TestLucene90StoredFieldsFormat` (`getCodec()` only).
fn lucene90_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormat",
        lucene90_codec,
    )
}

/// `TestLucene90StoredFieldsFormatHighCompression` (`getCodec()` only):
/// `new Lucene104Codec(Lucene104Codec.Mode.BEST_COMPRESSION)`.
fn high_compression_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.TestLucene90StoredFieldsFormatHighCompression",
        || Arc::new(Lucene104Codec::with_mode(Mode::BestCompression)),
    )
}

/// `TestCompressingStoredFieldsFormat` (`getCodec()` only).
fn compressing_class() -> Arc<Class> {
    suite_class(
        "org.apache.lucene.codecs.lucene90.compressing.TestCompressingStoredFieldsFormat",
        || {
            let codec = if *TEST_NIGHTLY {
                CompressingCodec::random_instance(&*LuceneTestCase::random())
            } else {
                CompressingCodec::reasonable_instance(&*LuceneTestCase::random())
            };
            Arc::new(codec.unwrap_or_else(|e| rethrow(e)))
        },
    )
}

/// `TestAssertingStoredFieldsFormat`: `private final Codec codec = new
/// AssertingCodec()`.
struct TestAssertingStoredFieldsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestAssertingStoredFieldsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.stored_fields_add_random_fields(doc);
    }

    fn as_stored_fields(&self) -> Option<&dyn BaseStoredFieldsFormatTestCase> {
        Some(self)
    }
}

impl BaseStoredFieldsFormatTestCase for TestAssertingStoredFieldsFormat {}

fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingStoredFieldsFormat")
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestAssertingStoredFieldsFormat {
                codec: Arc::new(AssertingCodec::new()),
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestStoredFieldsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestStoredFieldsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestStoredFieldsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene90StoredFieldsFormat`, recording the next
/// `random().nextLong()` after each test and after the class.
fn draws_class() -> Arc<Class> {
    use randomizedtesting::jdk::reflect::modifier;
    use rucene_test_framework::index::base_index_file_format_test_case::tear_down;
    Class::builder("org.apache.lucene.tests.index.TestStoredFieldsSuiteDraws")
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(StoredFieldsSuite {
                codec: lucene90_codec,
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

/// The rows of one seed of `TestStoredFieldsSuiteDraws`, as
/// `BaseFormatSuitesOracle` writes them (sorted).
fn draw_rows(seed: &str) -> Vec<String> {
    use randomizedtesting::jdk::system;
    use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};
    struct Events(Mutex<Vec<String>>);
    impl RunListener for Events {
        fn test_failure(&self, f: &Failure) {
            let m = f
                .description()
                .method_name()
                .unwrap_or_else(|| "null".into());
            let t = f.exception();
            let first = t
                .message()
                .unwrap_or("null")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            self.0
                .lock()
                .unwrap()
                .push(format!("{m}\tF\t{}: {first}", t.class_name()));
        }
        fn test_assumption_failure(&self, f: &Failure) {
            let m = f
                .description()
                .method_name()
                .unwrap_or_else(|| "null".into());
            let first = f
                .message()
                .unwrap_or("null")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            self.0.lock().unwrap().push(format!("{m}\tA\t{first}"));
        }
    }
    system::set_property("tests.seed", seed);
    DRAWS.lock().unwrap().clear();
    let events = Arc::new(Events(Mutex::new(Vec::new())));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&events) as Arc<dyn RunListener>);
    let r = core.run(&Request::a_class(&draws_class()));
    system::clear_property("tests.seed");
    let mut rows: Vec<String> = DRAWS.lock().unwrap().clone();
    rows.extend(events.0.lock().unwrap().iter().cloned());
    rows.push(format!(
        "#counts\trun={} fail={} assume={}\t-",
        r.run_count(),
        r.failure_count(),
        r.assumption_failure_count()
    ));
    rows.sort();
    rows.into_iter()
        .map(|row| format!("{seed}\t{row}"))
        .collect()
}

/// Seeded-draw parity with Lucene (rmp #351): on the oracle's seeds, every
/// test of the suite leaves its randomness where Java's does, and the class
/// leaves its own where Java's does (`base_stored_fields_draws.tsv`,
/// `BaseFormatSuitesOracle`): every row must be equal.
#[test]
fn test_stored_fields_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let java: Vec<String> = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/base_stored_fields_draws.tsv"
    ))
    .unwrap()
    .lines()
    .filter(|l| !l.starts_with('#'))
    .map(str::to_string)
    .collect();
    let mut seeds: Vec<String> = Vec::new();
    for row in &java {
        let seed = row.split('\t').next().unwrap().to_string();
        if !seeds.contains(&seed) {
            seeds.push(seed);
        }
    }
    let mut rust: Vec<String> = Vec::new();
    for seed in &seeds {
        rust.extend(draw_rows(seed));
    }
    let only_java: Vec<&String> = java.iter().filter(|r| !rust.contains(r)).collect();
    let only_rust: Vec<&String> = rust.iter().filter(|r| !java.contains(r)).collect();
    println!(
        "draw rows: java={} rust={} equal={}",
        java.len(),
        rust.len(),
        java.len() - only_java.len()
    );
    for r in &only_java {
        println!("  java only: {r}");
    }
    for r in &only_rust {
        println!("  rust only: {r}");
    }
    assert!(
        only_java.is_empty() && only_rust.is_empty(),
        "rows differ from Lucene's: java only {only_java:?}, rust only {only_rust:?}"
    );
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_lucene90_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene90_class);
}

#[test]
fn test_lucene90_stored_fields_format_high_compression() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(high_compression_class);
}

#[test]
fn test_compressing_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(compressing_class);
}

#[test]
fn test_asserting_stored_fields_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
