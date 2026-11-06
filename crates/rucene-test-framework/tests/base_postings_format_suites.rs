//! The `BasePostingsFormatTestCase` suites (rmp #351, #359): the inherited
//! cases of `BaseIndexFileFormatTestCase` and `BasePostingsFormatTestCase`
//! run, under the ported runner and `LuceneTestCase`'s rule chain, on the
//! codecs Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene104.TestLucene104PostingsFormat`
//!   (`lucene/core/src/test`): `TestUtil.alwaysPostingsFormat(new
//!   Lucene104PostingsFormat())`;
//! * `org.apache.lucene.codecs.perfield.TestPerFieldPostingsFormat`
//!   (`lucene/core/src/test`): a `RandomCodec` per `getCodec()` call, with
//!   its two `assumeTrue(false)` overrides;
//! * `org.apache.lucene.tests.codecs.asserting.TestAssertingPostingsFormat`
//!   (`lucene/test-framework/src/test`, ported in full): `AssertingCodec`,
//!   without `PostingsEnum` reuse.
//!
//! The extra test methods of the two core classes belong to the core-test
//! port (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! [`SEEDS`] instead, reporting the pass counts.

mod oracle_support;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::codecs::lucene104::postings_format::Lucene104PostingsFormat;
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_test_framework::codecs::asserting::asserting_codec::AssertingCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_postings_format_test_case::{
    self, BasePostingsFormatTestCase,
};
use rucene_test_framework::index::random_codec::RandomCodec;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_seeds;
use suite_seeds::run_suite;

// -----------------------------------------------------------------------------
// TestLucene104PostingsFormat
// -----------------------------------------------------------------------------

/// `TestLucene104PostingsFormat` (`getCodec()` only; its own tests belong to
/// the core-test port).
struct TestLucene104PostingsFormat;

impl BaseIndexFileFormatTestCase for TestLucene104PostingsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        TestUtil::always_postings_format(Arc::new(Lucene104PostingsFormat::new()))
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.postings_add_random_fields(doc);
    }

    fn as_postings(&self) -> Option<&dyn BasePostingsFormatTestCase> {
        Some(self)
    }
}

impl BasePostingsFormatTestCase for TestLucene104PostingsFormat {}

fn lucene104_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene104.TestLucene104PostingsFormat")
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestLucene104PostingsFormat))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// TestPerFieldPostingsFormat
// -----------------------------------------------------------------------------

/// `TestPerFieldPostingsFormat`.
struct TestPerFieldPostingsFormat;

impl BaseIndexFileFormatTestCase for TestPerFieldPostingsFormat {
    /// `new RandomCodec(new Random(random().nextLong()), Collections.emptySet())`.
    fn get_codec(&self) -> Arc<dyn Codec> {
        let random = JdkRandom::new(LuceneTestCase::random().next_long());
        Arc::new(RandomCodec::new_avoiding(&random, HashSet::new()))
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.postings_add_random_fields(doc);
    }

    fn as_postings(&self) -> Option<&dyn BasePostingsFormatTestCase> {
        Some(self)
    }
}

impl BasePostingsFormatTestCase for TestPerFieldPostingsFormat {}

/// An override of an inherited test that only assumes: `public void <name>()
/// throws Exception { assumeTrue(<message>, false); }`.
fn assume_false_override(name: &str, message: &'static str) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || LuceneTestCase::assume_true(message, false))
}

fn per_field_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.perfield.TestPerFieldPostingsFormat")
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestPerFieldPostingsFormat))
        }))
        .method(assume_false_override(
            "testMergeStability",
            "The MockRandom PF randomizes content on the fly, so we can't check it",
        ))
        .method(assume_false_override(
            "testPostingsEnumReuse",
            "The MockRandom PF randomizes content on the fly, so we can't check it",
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestAssertingPostingsFormat
// -----------------------------------------------------------------------------

/// `TestAssertingPostingsFormat`: `private final Codec codec = new
/// AssertingCodec()`.
struct TestAssertingPostingsFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestAssertingPostingsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.postings_add_random_fields(doc);
    }

    fn as_postings(&self) -> Option<&dyn BasePostingsFormatTestCase> {
        Some(self)
    }
}

impl BasePostingsFormatTestCase for TestAssertingPostingsFormat {
    fn is_postings_enum_reuse_implemented(&self) -> bool {
        false
    }
}

fn asserting_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.codecs.asserting.TestAssertingPostingsFormat")
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestAssertingPostingsFormat {
                codec: Arc::new(AssertingCodec::new()),
            }))
        }))
        .build()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestPostingsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestPostingsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestPostingsSuiteDraws` of `BaseFormatSuitesOracle`: the codec of
/// `TestLucene104PostingsFormat`, recording the next `random().nextLong()`
/// after each test and after the class.
fn draws_class() -> Arc<Class> {
    use randomizedtesting::jdk::reflect::modifier;
    use rucene_test_framework::index::base_index_file_format_test_case::tear_down;
    Class::builder("org.apache.lucene.tests.index.TestPostingsSuiteDraws")
        .extends(&base_postings_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestLucene104PostingsFormat))
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

/// The rows of one seed of `TestPostingsSuiteDraws`, as
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
/// leaves its own where Java's does (`base_postings_draws.tsv`,
/// `BaseFormatSuitesOracle`): every row must be equal.
#[test]
fn test_postings_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let java: Vec<String> = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/base_postings_draws.tsv"
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
fn test_lucene104_postings_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene104_class);
}

#[test]
fn test_per_field_postings_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(per_field_class);
}

#[test]
fn test_asserting_postings_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(asserting_class);
}
