//! The bindings of the analysis test bases of rmp #357 that have no other
//! home in this port, each run as a suite by the ported runner:
//!
//! * `lucene/core/src/test/org/apache/lucene/analysis/standard/TestStandardFactories.java`,
//!   the `lucene-core` subclass of `BaseTokenStreamFactoryTestCase`: core
//!   `StandardTokenizerFactory` reached through the SPI lookup
//!   (`TokenizerFactory.lookupClass("Standard")`).
//! * `CollationTestBase`, which has no `lucene-core` subclass in Lucene
//!   10.5.0: its four methods over a core analyzer (see
//!   [`test_collation_binding`]).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, JUnitCore, Request, Test};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, StringCharReader, TokenStream,
    TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, Att};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase as F, BaseTokenStreamTestCase as B,
    CollationTestBase,
};
use rucene_test_framework::util::lucene_test_case::LuceneTestCase;

type TestFn = fn() -> Result<()>;

/// Runs `methods` as the test methods of a suite named `name` under the
/// ported runner, with `LuceneTestCase`'s class rules and the class
/// environment, and fails with every failure's message.
fn run_suite(name: &str, methods: &[(&'static str, TestFn)]) {
    let failures = suite_failures(name, methods);
    assert!(
        failures.is_empty(),
        "{name} failures:\n{}",
        failures.join("\n")
    );
}

/// Runs `methods` as [`run_suite`] does and returns every failure, as
/// `description: exception`.
fn suite_failures(name: &str, methods: &[(&'static str, TestFn)]) -> Vec<String> {
    // The runner's thread groups and handlers are process-wide: one suite at
    // a time.
    static SUITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut builder = Class::builder(name)
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()));
    for (method, body) in methods {
        let body = *body;
        builder = builder.method(
            Method::new(*method)
                .annotate(Test::new())
                .body_any(move || {
                    if let Err(e) = body() {
                        let (class, message) = java_class_and_message(&e);
                        Throwable::new(class, message).throw();
                    }
                }),
        );
    }
    let class = Arc::new(builder.build());
    let result = JUnitCore::new().run(&Request::a_class(&class));
    let failures: Vec<String> = result
        .failures()
        .iter()
        .map(|f| format!("{}: {}", f.description(), f.exception()))
        .collect();
    assert_eq!(result.run_count(), methods.len(), "every test method ran");
    failures
}

// ---------------------------------------------------------------------------
// TestStandardFactories
// ---------------------------------------------------------------------------

/// `testStandardTokenizer`: Test StandardTokenizerFactory.
fn test_standard_tokenizer() -> Result<()> {
    let reader = StringCharReader::new("Wha\u{0301}t's this thing do?");
    let mut stream =
        F::tokenizer_factory("Standard", &[])?.create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(reader))?;
    B::assert_token_stream_contents(&mut stream, &["Wha\u{0301}t's", "this", "thing", "do"])
}

/// `testStandardTokenizerMaxTokenLength`.
fn test_standard_tokenizer_max_token_length() -> Result<()> {
    let long_word = "abcdefg".repeat(100); // 7 * 100 = 700 char "word"
    let content = format!("one two three {long_word} four five six");
    let reader = StringCharReader::new(content);
    let mut stream = F::tokenizer_factory("Standard", &["maxTokenLength", "1000"])?
        .create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(reader))?;
    B::assert_token_stream_contents(
        &mut stream,
        &["one", "two", "three", &long_word, "four", "five", "six"],
    )
}

/// `testBogusArguments`: Test that bogus arguments result in exception.
fn test_bogus_arguments() -> Result<()> {
    let expected = LuceneTestCase::expect_throws("java.lang.IllegalArgumentException", || {
        F::tokenizer_factory("Standard", &["bogusArg", "bogusValue"])
            .map(drop)
            .map_err(|e| {
                let (class, message) = java_class_and_message(&e);
                Throwable::new(class, message)
            })
    });
    Assert::assert_true(
        expected
            .message()
            .is_some_and(|m| m.contains("Unknown parameters")),
    );
    Ok(())
}

#[test]
fn test_standard_factories() {
    run_suite(
        "org.apache.lucene.analysis.standard.TestStandardFactories",
        &[
            ("testStandardTokenizer", test_standard_tokenizer),
            (
                "testStandardTokenizerMaxTokenLength",
                test_standard_tokenizer_max_token_length,
            ),
            ("testBogusArguments", test_bogus_arguments),
        ],
    );
}

// ---------------------------------------------------------------------------
// CollationTestBase
// ---------------------------------------------------------------------------

/// The whole input as one token, as `KeywordTokenizer` (analysis-common)
/// emits it: one token even for an empty input; `copies` times over, for an
/// analyzer that breaks `CollationTestBase`'s single-token contract.
#[derive(Debug)]
struct KeywordLogic {
    copies: u32,
    emitted: u32,
    text: String,
}

impl TokenizerLogic for KeywordLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.emitted == self.copies {
            return Ok(false);
        }
        source.clear_attributes();
        if self.emitted == 0 {
            let mut buf = [0u16; 256];
            let mut units = Vec::new();
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                units.extend_from_slice(&buf[..n]);
            }
            self.text.push_str(&String::from_utf16_lossy(&units));
        }
        self.emitted += 1;
        let text = &self.text;
        let length = text.encode_utf16().count() as i32;
        Att::get(source, char_term_id())
            .expect("the tokenizer's term")
            .set_term(text);
        Att::get(source, offset_id())
            .expect("the tokenizer's offsets")
            .offset_mut()
            .set_offset(0, length);
        Ok(true)
    }

    fn end(&mut self, source: &mut AttributeSource, _input: Option<&dyn CharReader>) -> Result<()> {
        let end = Att::get(source, offset_id())
            .expect("the tokenizer's offsets")
            .offset()
            .end_offset();
        Att::get(source, offset_id())
            .expect("the tokenizer's offsets")
            .offset_mut()
            .set_offset(end, end);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.emitted = 0;
        self.text.clear();
        Ok(())
    }
}

/// What a collating analyzer is to `CollationTestBase` (one token per value,
/// the term bytes being its sort key), with the binary order of UTF-8 as its
/// collation: `CollationKeyAnalyzer` (analysis-common) over a collator that
/// orders by code point, built from core `Tokenizer`.
#[derive(Debug)]
struct BinaryKeyAnalyzer {
    state: AnalyzerState,
    /// Tokens per value: 1, or more to break the contract.
    copies: u32,
}

impl Analyzer for BinaryKeyAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.collation.TestCollationBinding$BinaryKeyAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = Tokenizer::new(KeywordLogic {
            copies: self.copies,
            emitted: 0,
            text: String::new(),
        });
        Att::add(tokenizer.attribute_source_mut(), offset_id())?;
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            tokenizer,
        ))))
    }
}

fn binary_key_analyzer() -> Arc<dyn Analyzer> {
    analyzer_with_copies(1)
}

fn analyzer_with_copies(copies: u32) -> Arc<dyn Analyzer> {
    Arc::new(BinaryKeyAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        copies,
    })
}

fn bytes(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// The bounds a subclass passes, in the binary order of the analyzers here:
/// the term `"ساب"` lies outside `[U+0634, U+0698]` and
/// inside `[U+0633, U+0638]`, the second range of the base's fields. (The
/// first range of the fields, `[U+062F, U+0698]`, contains the term in this
/// order; only a Farsi collation excludes it, see
/// [`farsi_methods_fail_on_an_unexcluded_range`].)
fn bounds() -> [BytesRef; 4] {
    let base = CollationTestBase::default();
    [
        bytes("\u{0634}"),
        bytes(&base.first_range_end_original),
        bytes(&base.second_range_beginning_original),
        bytes(&base.second_range_end_original),
    ]
}

/// The three Farsi range methods over `analyzer`.
fn farsi_methods(analyzer: Arc<dyn Analyzer>) -> Result<()> {
    let base = CollationTestBase::default();
    let [a, b, c, d] = bounds();
    base.test_farsi_range_filter_collating(Arc::clone(&analyzer), &a, &b, &c, &d)?;
    base.test_farsi_range_query_collating(Arc::clone(&analyzer), &a, &b, &c, &d)?;
    base.test_farsi_term_range_query(analyzer, &a, &b, &c, &d)
}

/// The Farsi methods over core `StandardAnalyzer`, which keeps the Arabic
/// word whole.
fn farsi_over_standard_analyzer() -> Result<()> {
    farsi_methods(Arc::new(StandardAnalyzer::new()))
}

/// The Farsi methods over the binary key analyzer.
fn farsi_over_binary_keys() -> Result<()> {
    farsi_methods(binary_key_analyzer())
}

/// The signature of `CollationTestBase`'s Farsi methods.
type FarsiMethod = fn(
    &CollationTestBase,
    Arc<dyn Analyzer>,
    &BytesRef,
    &BytesRef,
    &BytesRef,
    &BytesRef,
) -> Result<()>;

/// A Farsi method over the binary key analyzer with the fields' own first
/// range, `[U+062F, U+0698]`, which the binary order does not exclude: the
/// method must fail on its first assertion.
fn farsi_with_unexcluded_first_range(method: FarsiMethod) -> Result<()> {
    let base = CollationTestBase::default();
    let a = bytes(&base.first_range_beginning_original);
    let [_, b, c, d] = bounds();
    method(&base, binary_key_analyzer(), &a, &b, &c, &d)
}

fn filter_with_unexcluded_first_range() -> Result<()> {
    farsi_with_unexcluded_first_range(CollationTestBase::test_farsi_range_filter_collating)
}

fn query_with_unexcluded_first_range() -> Result<()> {
    farsi_with_unexcluded_first_range(CollationTestBase::test_farsi_range_query_collating)
}

fn term_range_with_unexcluded_first_range() -> Result<()> {
    farsi_with_unexcluded_first_range(CollationTestBase::test_farsi_term_range_query)
}

/// `assertThreadSafe` over the binary key analyzer: every random simple
/// string, empty ones included, is one token.
fn assert_thread_safe_over_binary_keys() -> Result<()> {
    CollationTestBase::default().assert_thread_safe(binary_key_analyzer())
}

/// `assertThreadSafe` over an analyzer that gives every string two tokens
/// fails on the calling thread: its single-token check holds.
fn assert_thread_safe_refuses_two_tokens() -> Result<()> {
    LuceneTestCase::expect_throws("java.lang.AssertionError", || {
        CollationTestBase::default()
            .assert_thread_safe(analyzer_with_copies(2))
            .map_err(|e| {
                let (class, message) = java_class_and_message(&e);
                Throwable::new(class, message)
            })
    });
    Ok(())
}

/// `CollationTestBase`'s methods, bound over core analyzers.
#[test]
fn test_collation_binding() {
    run_suite(
        "org.apache.lucene.collation.TestCollationBinding",
        &[
            (
                "testFarsiOverStandardAnalyzer",
                farsi_over_standard_analyzer,
            ),
            ("testFarsiOverBinaryKeys", farsi_over_binary_keys),
            ("testAssertThreadSafe", assert_thread_safe_over_binary_keys),
            (
                "testAssertThreadSafeRefusesTwoTokens",
                assert_thread_safe_refuses_two_tokens,
            ),
        ],
    );
}

/// The positive control of the Farsi methods: with a first range the
/// analyzer's order does not exclude, each fails with its first assertion's
/// message (and, as in Java, leaves its directory open, which the suite's
/// rules then report).
#[test]
fn farsi_methods_fail_on_an_unexcluded_range() {
    let failures = suite_failures(
        "org.apache.lucene.collation.TestCollationBindingControl",
        &[
            ("testFilter", filter_with_unexcluded_first_range),
            ("testQuery", query_with_unexcluded_first_range),
            ("testTermRange", term_range_with_unexcluded_first_range),
        ],
    );
    for method in ["testFilter", "testQuery", "testTermRange"] {
        assert!(
            failures.iter().any(|f| f.starts_with(method)
                && f.ends_with(
                    "java.lang.AssertionError: The index Term should not be included. \
                     expected:<0> but was:<1>"
                )),
            "{method}: {failures:#?}"
        );
    }
}
