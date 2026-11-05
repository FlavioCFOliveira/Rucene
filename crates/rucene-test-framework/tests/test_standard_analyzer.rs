//! The bindings of Lucene's Unicode test data in
//! `lucene/core/src/test/org/apache/lucene/analysis/standard/TestStandardAnalyzer.java`:
//! `testUnicodeWordBreaks` (`WordBreakTestUnicode_12_1_0.test(a)`) and
//! `testUnicodeEmojiTests` (`EmojiTokenizationTestUnicode_12_1.test(a)`),
//! with the `setUp`/`tearDown` that build and close their analyzer, run as a
//! suite by the ported runner against core `StandardTokenizer`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::junit::{ClassRule, JUnitCore, Request, Test};
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
use rucene_test_framework::util::lucene_test_case::CLASS_NAME_RULE;

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

/// `setUp()`, the test body, then `tearDown()` (`a.close()`).
fn with_analyzer(body: fn(&dyn Analyzer) -> Result<()>) -> Result<()> {
    let a = StandardTokenizerAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    };
    let result = body(&a);
    a.close();
    result
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

#[test]
fn test_standard_analyzer_unicode_data() {
    let methods: [(&'static str, TestFn); 2] = [
        ("testUnicodeWordBreaks", test_unicode_word_breaks),
        ("testUnicodeEmojiTests", test_unicode_emoji_tests),
    ];
    let mut builder = Class::builder("org.apache.lucene.analysis.standard.TestStandardAnalyzer")
        .constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("classNameRule", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::clone(&CLASS_NAME_RULE) as Arc<dyn randomizedtesting::junit::TestRule>
                )
            })
            .annotate(ClassRule),
        );
    for (method, body) in methods {
        builder = builder.method(Method::new(method).annotate(Test::new()).body_any(move || {
            if let Err(e) = body() {
                let (class, message) = java_class_and_message(&e);
                Throwable::new(class, message).throw();
            }
        }));
    }
    let class = Arc::new(builder.build());
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
