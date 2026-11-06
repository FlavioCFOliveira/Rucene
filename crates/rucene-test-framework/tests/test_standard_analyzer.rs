//! The bindings of Lucene's Unicode test data in
//! `lucene/core/src/test/org/apache/lucene/analysis/standard/TestStandardAnalyzer.java`:
//! `testUnicodeWordBreaks` (`WordBreakTestUnicode_12_1_0.test(a)`) and
//! `testUnicodeEmojiTests` (`EmojiTokenizationTestUnicode_12_1.test(a)`),
//! with the `setUp`/`tearDown` that build and close their analyzer, run as a
//! suite extending `BaseTokenStreamTestCase` by the ported runner against
//! core `StandardTokenizer`.

use std::cell::RefCell;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{JUnitCore, Request};
use rucene::analysis::{
    new_standard_tokenizer_with_factory, Analyzer, AnalyzerState, GlobalReuseStrategy,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::standard::{
    EmojiTokenizationTestUnicode_12_1, WordBreakTestUnicode_12_1_0,
};
use rucene_test_framework::analysis::{tokenizer_components, BaseTokenStreamTestCase};
use rucene_test_framework::util::LuceneTestCase;

/// `TestStandardAnalyzer.a`: the anonymous `Analyzer` of `setUp`, whose
/// components are a `StandardTokenizer(newAttributeFactory())`.
#[derive(Debug)]
struct StandardTokenizerAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for StandardTokenizerAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.standard.TestStandardAnalyzer$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer =
            new_standard_tokenizer_with_factory(BaseTokenStreamTestCase::new_attribute_factory())?;
        Ok(tokenizer_components(std::rc::Rc::new(
            std::cell::RefCell::new(tokenizer),
        )))
    }
}

thread_local! {
    /// `TestStandardAnalyzer.a`, the instance field `setUp` sets and
    /// `tearDown` closes; `setUp`, the test and `tearDown` run on the test's
    /// thread.
    static A: RefCell<Option<StandardTokenizerAnalyzer>> = const { RefCell::new(None) };
}

/// `setUp()`: `super.setUp()`, then the analyzer.
fn set_up() {
    LuceneTestCase::super_set_up();
    A.with(|a| {
        *a.borrow_mut() = Some(StandardTokenizerAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        })
    });
}

/// `tearDown()`: `a.close()`, then `super.tearDown()`.
fn tear_down() {
    if let Some(a) = A.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
}

/// The body of a test over `a`.
fn with_analyzer(body: fn(&dyn Analyzer) -> Result<()>) -> Result<()> {
    A.with(|a| body(a.borrow().as_ref().expect("setUp created the analyzer")))
}

/// `testUnicodeWordBreaks`.
fn test_unicode_word_breaks() -> Result<()> {
    with_analyzer(WordBreakTestUnicode_12_1_0::test)
}

/// `testUnicodeEmojiTests`.
fn test_unicode_emoji_tests() -> Result<()> {
    with_analyzer(EmojiTokenizationTestUnicode_12_1::test)
}

type TestFn = fn() -> Result<()>;

/// `TestStandardAnalyzer extends BaseTokenStreamTestCase`, its two Unicode
/// data tests found as `LuceneTestCase`'s JUnit 3 provider finds them, with
/// its `setUp`/`tearDown` overrides, under the full `LuceneTestCase` rule
/// chain.
#[test]
fn test_standard_analyzer_unicode_data() {
    let methods: [(&'static str, TestFn); 2] = [
        ("testUnicodeWordBreaks", test_unicode_word_breaks),
        ("testUnicodeEmojiTests", test_unicode_emoji_tests),
    ];
    let mut builder = Class::builder("org.apache.lucene.analysis.standard.TestStandardAnalyzer")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        // As in Java, the overrides carry no `@Before`/`@After`: the runner
        // invokes `LuceneTestCase.setUp`/`tearDown`, which dispatch to them.
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body_any(set_up),
        )
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body_any(tear_down),
        );
    for (method, body) in methods {
        builder = builder.method(Method::new(method).body_any(move || {
            if let Err(e) = body() {
                let (class, message) = java_class_and_message(&e);
                Throwable::new(class, message).throw();
            }
        }));
    }
    let class = builder.build();
    let result = JUnitCore::new().run(&Request::a_class(&class));
    let failures: Vec<String> = result
        .failures()
        .iter()
        .map(|f| format!("{}: {}", f.description(), f.exception()))
        .collect();
    assert_eq!(result.run_count(), 2, "both test methods ran");
    assert!(
        failures.is_empty(),
        "TestStandardAnalyzer failures:\n{}",
        failures.join("\n")
    );
}
