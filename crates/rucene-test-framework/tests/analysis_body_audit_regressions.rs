//! Regression tests of the body-level audit of the test-framework analysis
//! port (rmp #518): each locks in Lucene 10.5.0's behaviour of a
//! `BaseTokenStreamTestCase` method that the port had diverged from.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{JUnitCore, Request, Test};
use rucene::analysis::{
    default_attribute_factory, Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy,
    TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{boost_id, char_term_id, offset_id, Att};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::vocabulary_assert::VocabularyAssert;
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamTestCase as B, CannedTokenStream, MockAnalyzer, Token,
};
use rucene_test_framework::util::lucene_test_case::LuceneTestCase;

type TestFn = fn() -> Result<()>;

/// Runs `methods` as the test methods of a suite named `name` under the
/// ported runner (which provides `random()`), and fails with every failure.
fn run_suite(name: &str, methods: &[(&'static str, TestFn)]) {
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
    assert!(
        failures.is_empty(),
        "{name} failures:\n{}",
        failures.join("\n")
    );
}

/// The throwable a call panicked with.
fn panic_of<T>(f: impl FnOnce() -> T) -> Throwable {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(_) => panic!("expected a throwable"),
        Err(payload) => Throwable::from_panic(payload),
    }
}

/// An anonymous `Analyzer` whose `createComponents` is a closure.
struct FnAnalyzer<F> {
    state: AnalyzerState,
    create: F,
}

impl<F> std::fmt::Debug for FnAnalyzer<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FnAnalyzer")
    }
}

impl<F: Fn() -> Result<TokenStreamComponents> + Send + Sync> FnAnalyzer<F> {
    fn new(create: F) -> Self {
        FnAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            create,
        }
    }
}

impl<F: Fn() -> Result<TokenStreamComponents> + Send + Sync> Analyzer for FnAnalyzer<F> {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.Analyzer$Anonymous"
    }
    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        (self.create)()
    }
}

/// A tokenizer over `CharTermAttribute` and `OffsetAttribute` only (no
/// `PositionIncrementAttribute`) that emits one token `x` per reset, and
/// whose `incrementToken()` without `reset()` throws
/// `new RuntimeException("boom")` (an unchecked exception, thrown in Rust as
/// a panic).
#[derive(Debug)]
struct OneTokenLogic {
    term: Att,
    was_reset: bool,
    emitted: bool,
}

impl TokenizerLogic for OneTokenLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _input: &mut dyn CharReader,
    ) -> Result<bool> {
        if !self.was_reset {
            Throwable::new("java.lang.RuntimeException", "boom").throw();
        }
        source.clear_attributes();
        if self.emitted {
            return Ok(false);
        }
        self.emitted = true;
        self.term.set_term("x");
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.was_reset = true;
        self.emitted = false;
        Ok(())
    }

    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.was_reset = false;
        Ok(())
    }
}

fn one_token_analyzer() -> FnAnalyzer<impl Fn() -> Result<TokenStreamComponents> + Send + Sync> {
    FnAnalyzer::new(|| {
        let mut source = AttributeSource::new_with_factory(default_attribute_factory());
        let term = Att::add(&mut source, char_term_id())?;
        Att::add(&mut source, offset_id())?;
        let logic = OneTokenLogic {
            term,
            was_reset: false,
            emitted: false,
        };
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            Tokenizer::with_source(source, logic),
        ))))
    })
}

/// `checkResetException`: an unchecked exception (not an `Error`) thrown by
/// `incrementToken()` without `reset()` is caught by `catch (Exception
/// unexpected)`, so the method fails with `"got wrong exception when reset()
/// not called: " + unexpected` (`BaseTokenStreamTestCase.java:863-866`), after
/// the `finally` consumed the stream correctly. The port let the panic
/// through unchanged.
#[test]
fn check_reset_exception_reports_an_unchecked_exception_as_wrong() {
    let a = one_token_analyzer();
    let t = panic_of(|| B::check_reset_exception(&a, "x"));
    assert_eq!(t.class_name(), "java.lang.AssertionError");
    assert_eq!(
        t.message(),
        Some("got wrong exception when reset() not called: java.lang.RuntimeException: boom")
    );
}

/// `toString(Analyzer, String)` over a stream without
/// `PositionIncrementAttribute` throws Java's helpful
/// `NullPointerException` naming the local `posIncAtt`
/// (`BaseTokenStreamTestCase.java:1675`), and the try-with-resources closes
/// the stream. The port threw the exception with a `null` message.
#[test]
fn to_string_without_position_increments_throws_the_helpful_npe() {
    let a = one_token_analyzer();
    let t = panic_of(|| B::to_string(&a, "x"));
    assert_eq!(t.class_name(), "java.lang.NullPointerException");
    assert_eq!(
        t.message(),
        Some(
            "Cannot invoke \"org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute.getPositionIncrement()\" because \"posIncAtt\" is null"
        )
    );
}

/// `assertTokenStreamContents(..., float[] boost)` compares with
/// `assertEquals(String, float, float, 0.001)`, whose `double` delta binds
/// JUnit's `double` overload: the message renders both floats widened to
/// `double` (`BaseTokenStreamTestCase.java:254-256`). The port rendered
/// them as `float`s (`0.1`).
#[test]
fn boost_mismatch_renders_the_floats_as_doubles() {
    let mut ts = CannedTokenStream::new(vec![Token::with_offsets("a", 0, 1)]);
    Att::add(ts.attribute_source_mut(), boost_id()).unwrap();
    let t = panic_of(|| {
        B::assert_token_stream_contents_full(
            &mut ts,
            &["a"],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
            None,
            None,
            Some(&[0.1]),
        )
    });
    assert_eq!(t.class_name(), "java.lang.AssertionError");
    assert_eq!(
        t.message(),
        Some("boostAtt 0 term=a expected:<0.10000000149011612> but was:<1.0>")
    );
}

/// `toDotFile` opens the writer (`Files.newBufferedWriter`, which creates
/// the file) before it asks the analyzer for a stream
/// (`BaseTokenStreamTestCase.java:1508-1509`), so an analyzer failure leaves
/// an empty file behind. The port wrote the file only after the analysis.
#[test]
fn to_dot_file_creates_the_file_before_the_analysis() {
    let a = FnAnalyzer::new(|| Err(LuceneError::illegal_argument("no components")));
    let path = std::env::temp_dir().join(format!(
        "rucene-to-dot-file-{}-{:?}.dot",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    let result = B::to_dot_file(&a, "x", path.to_str().unwrap());
    let exists = path.exists();
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err());
    assert!(exists, "Files.newBufferedWriter creates the file first");
}

/// `assertGraphStrings(Analyzer, String, String...)`: the message lists each
/// set sorted by `Collections.sort` (`String.compareTo`, UTF-16 code units:
/// U+10400 before U+FF21), and the try-with-resources closes the stream when
/// the assertion fails, so the analyzer hands out its next stream. The port
/// sorted by UTF-8 bytes and left the stream open on a failure.
fn assert_graph_strings_sorts_utf16_and_closes_on_failure() -> Result<()> {
    let a = MockAnalyzer::new(&*LuceneTestCase::random());
    let t = panic_of(|| B::assert_graph_strings(&a, "a", &["a", "\u{10400}", "\u{FF21}"]));
    assert_eq!(t.class_name(), "java.lang.AssertionError");
    assert_eq!(
        t.message(),
        Some(
            "Analyzer created unexpected string path: \u{10400}\nexpected:\n  a\n  \u{10400}\n  \u{FF21}\n\nactual:\n  a\n"
        )
    );
    let ts = a.token_stream_from_str("dummy", "b")?;
    let mut ts = ts.borrow_mut();
    ts.reset()?;
    while ts.increment_token()? {}
    ts.end()?;
    ts.close()
}

/// `VocabularyAssert.assertVocabulary(Analyzer, InputStream, InputStream)`
/// reads both files with `BufferedReader.readLine()`, for which a lone
/// `\r` ends a line (`VocabularyAssert.java:41-48`): `a\rb` is two words,
/// matching the two lines of the output file. The port's `BufRead::lines`
/// split only on `\n` and compared `a\rb` with `a`.
fn vocabulary_lines_end_at_a_lone_carriage_return() -> Result<()> {
    let a = MockAnalyzer::with_automaton(&*LuceneTestCase::random(), Arc::clone(&KEYWORD), false);
    VocabularyAssert::assert_vocabulary(&a, &b"a\rb\n"[..], &b"a\nb\n"[..])
}

#[test]
fn base_token_stream_test_case_graph_strings() {
    run_suite(
        "org.apache.lucene.tests.analysis.TestBodyAuditGraphStrings",
        &[
            (
                "testAssertGraphStringsSortsUtf16AndClosesOnFailure",
                assert_graph_strings_sorts_utf16_and_closes_on_failure,
            ),
            (
                "testVocabularyLinesEndAtALoneCarriageReturn",
                vocabulary_lines_end_at_a_lone_carriage_return,
            ),
        ],
    );
}

/// `assertVocabulary(Analyzer, InputStream)` splits each line with
/// `split("\t")`, which drops trailing empty strings, so `abc\t` has one
/// field and `words[1]` throws `ArrayIndexOutOfBoundsException: Index 1 out
/// of bounds for length 1` (`VocabularyAssert.java:58-59`). The port kept the
/// empty field and checked `abc` against the empty term.
#[test]
fn vocabulary_tsv_line_with_trailing_tab_throws_array_index_out_of_bounds() {
    let a = MockAnalyzer::new(&randomizedtesting::jdk::random::JdkRandom::new(0));
    let t = panic_of(|| VocabularyAssert::assert_vocabulary_tsv(&a, &b"abc\t\n"[..]));
    assert_eq!(t.class_name(), "java.lang.ArrayIndexOutOfBoundsException");
    assert_eq!(t.message(), Some("Index 1 out of bounds for length 1"));
}
