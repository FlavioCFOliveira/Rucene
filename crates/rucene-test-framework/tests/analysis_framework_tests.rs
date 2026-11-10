//! Port of `lucene/test-framework/src/test/org/apache/lucene/tests/analysis/`:
//! `TestMockAnalyzer`, `TestMockCharFilter`, `TestMockSynonymFilter`,
//! `TestToken`, `TestLookaheadTokenFilter` and `TrivialLookaheadFilter`, each
//! run as a suite by the ported runner.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, JUnitCore, Request, Test};
use rucene::analysis::tokenattributes::{
    CharTermAttribute, FlagsAttribute, OffsetAttribute, PayloadAttribute,
    PositionIncrementAttribute, PositionLengthAttribute, TermFrequencyAttribute, TypeAttribute,
};
use rucene::analysis::{
    Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic, CharReader,
    GlobalReuseStrategy, PerFieldReuseStrategy, StringCharReader, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::attribute::{
    Attribute, AttributeImpl, AttributeReflector, AttributeSource, DefaultAttributeFactory,
};
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::{Automata, CharacterRunAutomaton, Operations, RegExp};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{
    char_term_id, flags_id, offset_id, payload_id, pos_inc_id, type_id, Att,
};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::lookahead_token_filter::{
    new_lookahead_filter, LookaheadLogic, LookaheadPosition, LookaheadTokenFilter, Position,
};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{
    DEFAULT_MAX_TOKEN_LENGTH, KEYWORD, SIMPLE, WHITESPACE,
};
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, BaseTokenStreamTestCase as B, MockAnalyzer,
    MockCharFilter, MockRandomLookaheadTokenFilter, MockSynonymFilter, MockTokenizer, Token,
};
use rucene_test_framework::util::automaton::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase, RANDOM_MULTIPLIER, TEST_NIGHTLY,
};
use rucene_test_framework::util::TestUtil;

type TestFn = fn() -> Result<()>;

/// Runs `methods` as the test methods of a suite named `name` under the
/// ported runner, and fails with every failure's message.
fn run_suite(name: &str, methods: &[(&'static str, TestFn)]) {
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
    assert!(
        failures.is_empty(),
        "{name} failures:\n{}",
        failures.join("\n")
    );
}

fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

fn arc<A: Analyzer + 'static>(a: A) -> Arc<dyn Analyzer> {
    Arc::new(a)
}

fn automaton(regexp: &str) -> Arc<CharacterRunAutomaton> {
    let a = RegExp::new(regexp).unwrap().to_automaton().unwrap();
    Arc::new(CharacterRunAutomaton::new(a).unwrap())
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

impl<F: Fn(&str) -> Result<TokenStreamComponents> + Send + Sync> FnAnalyzer<F> {
    fn new(create: F) -> Self {
        FnAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            create,
        }
    }
}

impl<F: Fn(&str) -> Result<TokenStreamComponents> + Send + Sync> Analyzer for FnAnalyzer<F> {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.Analyzer$Anonymous"
    }
    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        (self.create)(field_name)
    }
}

// ---------------------------------------------------------------------------
// TestMockAnalyzer
// ---------------------------------------------------------------------------

mod test_mock_analyzer {
    use super::*;

    /// Test a configuration that behaves a lot like WhitespaceAnalyzer.
    pub fn test_whitespace() -> Result<()> {
        let a = MockAnalyzer::new(&*random());
        B::assert_analyzes_to(
            &a,
            "A bc defg hiJklmn opqrstuv wxy z ",
            &["a", "bc", "defg", "hijklmn", "opqrstuv", "wxy", "z"],
        )?;
        B::assert_analyzes_to(&a, "aba cadaba shazam", &["aba", "cadaba", "shazam"])?;
        B::assert_analyzes_to(&a, "break on whitespace", &["break", "on", "whitespace"])
    }

    /// Test a configuration that behaves a lot like SimpleAnalyzer.
    pub fn test_simple() -> Result<()> {
        let a = MockAnalyzer::with_automaton(&*random(), Arc::clone(&SIMPLE), true);
        B::assert_analyzes_to(
            &a,
            "a-bc123 defg+hijklmn567opqrstuv78wxy_z ",
            &["a", "bc", "defg", "hijklmn", "opqrstuv", "wxy", "z"],
        )?;
        B::assert_analyzes_to(&a, "aba4cadaba-Shazam", &["aba", "cadaba", "shazam"])?;
        B::assert_analyzes_to(&a, "break+on/Letters", &["break", "on", "letters"])
    }

    /// Test a configuration that behaves a lot like KeywordAnalyzer.
    pub fn test_keyword() -> Result<()> {
        let a = MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false);
        B::assert_analyzes_to(
            &a,
            "a-bc123 defg+hijklmn567opqrstuv78wxy_z ",
            &["a-bc123 defg+hijklmn567opqrstuv78wxy_z "],
        )?;
        B::assert_analyzes_to(&a, "aba4cadaba-Shazam", &["aba4cadaba-Shazam"])?;
        B::assert_analyzes_to(&a, "break+on/Nothing", &["break+on/Nothing"])?;
        // currently though emits no tokens for empty string: maybe we can do it,
        // but we don't want to emit tokens infinitely...
        B::assert_analyzes_to(&a, "", &[])
    }

    // Test some regular expressions as tokenization patterns
    /// Test a configuration where each character is a term.
    pub fn test_single_char() -> Result<()> {
        let single = automaton(".");
        let a = arc(MockAnalyzer::with_automaton(&*random(), single, false));
        B::assert_analyzes_to_offsets(
            &*a,
            "foobar",
            &["f", "o", "o", "b", "a", "r"],
            &[0, 1, 2, 3, 4, 5],
            &[1, 2, 3, 4, 5, 6],
        )?;
        B::check_random_data(&*random(), &a, 100)
    }

    /// Test a configuration where two characters makes a term.
    pub fn test_two_chars() -> Result<()> {
        let single = automaton("..");
        let a = arc(MockAnalyzer::with_automaton(&*random(), single, false));
        B::assert_analyzes_to_offsets(&*a, "foobar", &["fo", "ob", "ar"], &[0, 2, 4], &[2, 4, 6])?;
        // make sure when last term is a "partial" match that end() is correct
        {
            let ts = a.token_stream_from_str("bogus", "fooba")?;
            let mut ts = ts.borrow_mut();
            B::assert_token_stream_contents_offsets_pos_increments_final_offset(
                &mut *ts,
                &["fo", "ob"],
                &[0, 2],
                &[2, 4],
                &[1, 1],
                Some(5),
            )?;
        }
        B::check_random_data(&*random(), &a, 100)
    }

    /// Test a configuration where three characters makes a term.
    pub fn test_three_chars() -> Result<()> {
        let single = automaton("...");
        let a = arc(MockAnalyzer::with_automaton(&*random(), single, false));
        B::assert_analyzes_to_offsets(&*a, "foobar", &["foo", "bar"], &[0, 3], &[3, 6])?;
        // make sure when last term is a "partial" match that end() is correct
        {
            let ts = a.token_stream_from_str("bogus", "fooba")?;
            let mut ts = ts.borrow_mut();
            B::assert_token_stream_contents_offsets_pos_increments_final_offset(
                &mut *ts,
                &["foo"],
                &[0],
                &[3],
                &[1],
                Some(5),
            )?;
        }
        B::check_random_data(&*random(), &a, 100)
    }

    /// Test a configuration where word starts with one uppercase.
    pub fn test_uppercase() -> Result<()> {
        let single = automaton("[A-Z][a-z]*");
        let a = arc(MockAnalyzer::with_automaton(&*random(), single, false));
        B::assert_analyzes_to_offsets(
            &*a,
            "FooBarBAZ",
            &["Foo", "Bar", "B", "A", "Z"],
            &[0, 3, 6, 7, 8],
            &[3, 6, 7, 8, 9],
        )?;
        B::assert_analyzes_to_offsets(&*a, "aFooBar", &["Foo", "Bar"], &[1, 4], &[4, 7])?;
        B::check_random_data(&*random(), &a, 100)
    }

    /// Test a configuration that behaves a lot like StopAnalyzer.
    pub fn test_stop() -> Result<()> {
        let a = MockAnalyzer::with_filter(
            &*random(),
            Arc::clone(&SIMPLE),
            true,
            Arc::clone(&ENGLISH_STOPSET),
        );
        B::assert_analyzes_to_pos_increments(
            &a,
            "the quick brown a fox",
            &["quick", "brown", "fox"],
            &[2, 1, 2],
        )
    }

    /// Test a configuration that behaves a lot like KeepWordFilter.
    pub fn test_keep() -> Result<()> {
        let keep_words = Arc::new(
            CharacterRunAutomaton::new(
                Operations::complement(
                    &Operations::union(&[
                        Automata::make_string("foo"),
                        Automata::make_string("bar"),
                    ]),
                    DEFAULT_DETERMINIZE_WORK_LIMIT,
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let a = MockAnalyzer::with_filter(&*random(), Arc::clone(&SIMPLE), true, keep_words);
        B::assert_analyzes_to_pos_increments(
            &a,
            "quick foo brown bar bar fox foo",
            &["foo", "bar", "bar", "foo"],
            &[2, 2, 1, 2],
        )
    }

    /// Test a configuration that behaves a lot like LengthFilter.
    pub fn test_length() -> Result<()> {
        let length5 = automaton(".{5,}");
        let a = MockAnalyzer::with_filter(&*random(), Arc::clone(&WHITESPACE), true, length5);
        B::assert_analyzes_to_pos_increments(
            &a,
            "ok toolong fine notfine",
            &["ok", "fine"],
            &[1, 2],
        )
    }

    /// Test MockTokenizer encountering a too long token.
    pub fn test_too_long_token() -> Result<()> {
        let whitespace = FnAnalyzer::new(|_| {
            let t = MockTokenizer::with_max_token_length(Arc::clone(&WHITESPACE), false, 5)?;
            Ok(tokenizer_components(Rc::new(RefCell::new(t))))
        });
        {
            let ts = whitespace.token_stream_from_str("bogus", "test 123 toolong ok ")?;
            let mut ts = ts.borrow_mut();
            B::assert_token_stream_contents_offsets_final_offset(
                &mut *ts,
                &["test", "123", "toolo", "ng", "ok"],
                &[0, 5, 9, 14, 17],
                &[4, 8, 14, 16, 19],
                Some(20),
            )?;
        }
        let ts = whitespace.token_stream_from_str("bogus", "test 123 toolo")?;
        let mut ts = ts.borrow_mut();
        B::assert_token_stream_contents_offsets_final_offset(
            &mut *ts,
            &["test", "123", "toolo"],
            &[0, 5, 9],
            &[4, 8, 14],
            Some(14),
        )
    }

    pub fn test_lucene_3042() -> Result<()> {
        let test_string = "t";

        let analyzer = MockAnalyzer::new(&*random());
        {
            let stream = analyzer.token_stream_from_str("dummy", test_string)?;
            let mut stream = stream.borrow_mut();
            stream.reset()?;
            while stream.increment_token()? {
                // consume
            }
            stream.end()?;
            stream.close()?;
        }

        B::assert_analyzes_to(&analyzer, test_string, &["t"])
    }

    /// blast some random strings through the analyzer
    pub fn test_random_strings() -> Result<()> {
        let a = arc(MockAnalyzer::new(&*random()));
        B::check_random_data(&*random(), &a, LuceneTestCase::at_least(1000))
    }

    /// blast some random strings through differently configured tokenizers
    pub fn test_random_regexps() -> Result<()> {
        let iters = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(30)
        } else {
            LuceneTestCase::at_least(1)
        };
        for _ in 0..iters {
            let automaton = Operations::determinize(
                &AutomatonTestUtil::random_automaton(&*random()),
                DEFAULT_DETERMINIZE_WORK_LIMIT,
            )
            .unwrap();
            let dfa = Arc::new(CharacterRunAutomaton::new(automaton).unwrap());
            let lowercase = random().next_boolean();
            let limit = TestUtil::next_int(&*random(), 0, 500);
            let a = arc(FnAnalyzer::new(move |_| {
                let t = MockTokenizer::with_max_token_length(Arc::clone(&dfa), lowercase, limit)?;
                Ok(tokenizer_components(Rc::new(RefCell::new(t))))
            }));
            B::check_random_data(&*random(), &a, 100)?;
            a.close();
        }
        Ok(())
    }

    pub fn test_forward_offsets() -> Result<()> {
        let num = LuceneTestCase::at_least(1000);
        for _ in 0..num {
            let s = TestUtil::random_htmlish_string(&*random(), 20);
            let reader = Box::new(StringCharReader::new(s));
            let charfilter = MockCharFilter::new(reader, 2)?;
            let analyzer = MockAnalyzer::new(&*random());
            let ts = analyzer.token_stream("bogus", Box::new(charfilter))?;
            let mut ts = ts.borrow_mut();
            ts.reset()?;
            while ts.increment_token()? {}
            ts.end()?;
            ts.close()?;
        }
        Ok(())
    }

    #[derive(Debug)]
    struct WrapReader {
        delegate: Arc<dyn Analyzer>,
    }

    impl AnalyzerWrapperLogic for WrapReader {
        fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
            &*self.delegate
        }
        fn wrap_reader(
            &self,
            _field_name: &str,
            reader: Box<dyn CharReader>,
        ) -> Box<dyn CharReader> {
            Box::new(MockCharFilter::new(reader, 7).unwrap())
        }
    }

    pub fn test_wrap_reader() -> Result<()> {
        // LUCENE-5153: test that wrapping an analyzer's reader is allowed
        let random = random();
        let delegate: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random));
        // delegate.getReuseStrategy(): MockAnalyzer's PER_FIELD_REUSE_STRATEGY
        let a = AnalyzerWrapper::new(
            Box::new(WrapReader { delegate }),
            Box::new(PerFieldReuseStrategy),
        );
        B::check_one_term(&a, "abc", "aabc")
    }

    /// The anonymous `DelegatingAnalyzerWrapper` of `testChangeGaps`.
    #[derive(Debug)]
    struct ChangeGaps {
        delegate: Arc<dyn Analyzer>,
        position_gap: i32,
        offset_gap: i32,
    }

    impl rucene::analysis::DelegatingAnalyzerWrapperLogic for ChangeGaps {
        fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
            &*self.delegate
        }
        fn get_position_increment_gap(&self, _field_name: &str) -> i32 {
            self.position_gap
        }
        fn get_offset_gap(&self, _field_name: &str) -> i32 {
            self.offset_gap
        }
    }

    pub fn test_change_gaps() -> Result<()> {
        use rucene::document::{Document, Field as DocField, FieldType};
        use rucene::index::{IndexOptions, IndexReader, POSTINGS_ENUM_ALL};
        use rucene_test_framework::index::RandomIndexWriter;

        // LUCENE-5324: check that it is possible to change the wrapper's gaps
        let position_gap = random().next_int_bound(1000);
        let offset_gap = random().next_int_bound(1000);
        let delegate: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
        // delegate.getReuseStrategy(): MockAnalyzer's PER_FIELD_REUSE_STRATEGY
        let a: Arc<dyn Analyzer> = Arc::new(rucene::analysis::DelegatingAnalyzerWrapper::new(
            Box::new(PerFieldReuseStrategy),
            Box::new(ChangeGaps {
                delegate,
                position_gap,
                offset_gap,
            }),
        ));

        let mut dir = LuceneTestCase::new_directory();
        let writer = RandomIndexWriter::new(&*random(), dir.as_directory(), a)?;
        let mut doc = Document::new();
        let mut ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS)?;
        ft.set_tokenized(true)?;
        ft.set_store_term_vectors(true)?;
        ft.set_store_term_vector_positions(true)?;
        ft.set_store_term_vector_offsets(true)?;
        doc.add(Box::new(DocField::new("f", "a".to_string(), ft.clone())?));
        doc.add(Box::new(DocField::new("f", "a".to_string(), ft)?));
        writer.add_document(&doc)?;
        let directory_reader = writer.get_reader()?;
        let reader = LuceneTestCase::get_only_leaf_reader(
            Arc::clone(&directory_reader) as Arc<dyn IndexReader>
        )?;
        let fields = reader
            .term_vectors()?
            .get(0)?
            .expect("term vectors of doc 0");
        let terms = fields.terms("f")?.expect("terms of f");
        let mut te = terms.iterator()?;
        let next = te.next()?;
        Assert::assert_true_msg("expected:<[61]> but was:<null>", next.is_some());
        Assert::assert_equals(LuceneTestCase::new_bytes_ref_str("a"), next.unwrap());
        let mut dpe = te.postings(None, POSTINGS_ENUM_ALL)?;
        Assert::assert_equals(0, dpe.next_doc()?);
        Assert::assert_equals(2, dpe.freq()?);
        Assert::assert_equals(0, dpe.next_position()?);
        Assert::assert_equals(0, dpe.start_offset());
        let end_offset = dpe.end_offset();
        Assert::assert_equals(1 + position_gap, dpe.next_position()?);
        Assert::assert_equals(1 + end_offset + offset_gap, dpe.end_offset());
        let next = te.next()?;
        Assert::assert_true_msg(
            &format!(
                "expected:<null> but was:<{}>",
                next.as_ref().map(ToString::to_string).unwrap_or_default()
            ),
            next.is_none(),
        );
        drop(dpe);
        drop(te);
        drop(terms);
        drop(fields);
        drop(reader);
        directory_reader.close()?;
        drop(directory_reader);
        writer.close()?;
        // `writer.w.getDirectory().close()`: the writer's handle goes first.
        drop(writer);
        dir.close()
    }
}

#[test]
fn test_mock_analyzer() {
    use test_mock_analyzer::*;
    run_suite(
        "org.apache.lucene.tests.analysis.TestMockAnalyzer",
        &[
            ("testWhitespace", test_whitespace),
            ("testSimple", test_simple),
            ("testKeyword", test_keyword),
            ("testSingleChar", test_single_char),
            ("testTwoChars", test_two_chars),
            ("testThreeChars", test_three_chars),
            ("testUppercase", test_uppercase),
            ("testStop", test_stop),
            ("testKeep", test_keep),
            ("testLength", test_length),
            ("testTooLongToken", test_too_long_token),
            ("testLUCENE_3042", test_lucene_3042),
            ("testRandomStrings", test_random_strings),
            ("testRandomRegexps", test_random_regexps),
            ("testForwardOffsets", test_forward_offsets),
            ("testWrapReader", test_wrap_reader),
            ("testChangeGaps", test_change_gaps),
        ],
    );
}

// ---------------------------------------------------------------------------
// TestMockCharFilter
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CharFilterAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CharFilterAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.TestMockCharFilter$1"
    }
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        Ok(tokenizer_components(Rc::new(RefCell::new(tokenizer))))
    }
    fn init_reader(&self, _field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        Box::new(MockCharFilter::new(reader, 7).unwrap())
    }
}

fn test_mock_char_filter_test() -> Result<()> {
    let analyzer = CharFilterAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    };
    B::assert_analyzes_to_offsets(&analyzer, "ab", &["aab"], &[0], &[2])?;
    B::assert_analyzes_to_offsets(&analyzer, "aba", &["aabaa"], &[0], &[3])?;
    B::assert_analyzes_to_offsets(&analyzer, "abcdefga", &["aabcdefgaa"], &[0], &[8])
}

#[test]
fn test_mock_char_filter() {
    run_suite(
        "org.apache.lucene.tests.analysis.TestMockCharFilter",
        &[("test", test_mock_char_filter_test)],
    );
}

// ---------------------------------------------------------------------------
// TestMockSynonymFilter
// ---------------------------------------------------------------------------

/// test the mock synonym filter
fn test_mock_synonym_filter_test() -> Result<()> {
    let analyzer = FnAnalyzer::new(|_| {
        let tokenizer = Rc::new(RefCell::new(MockTokenizer::whitespace_lower_case()?));
        let filter = MockSynonymFilter::new(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(filter)))
    });
    #[allow(clippy::type_complexity)]
    let cases: &[(&str, &[&str], &[i32], &[i32], &[&str], &[i32], &[i32])] = &[
        (
            "dogs",
            &["dogs", "dog"],
            &[0, 0],
            &[4, 4],
            &["word", "SYNONYM"],
            &[1, 0],
            &[1, 1],
        ),
        (
            "small dogs",
            &["small", "dogs", "dog"],
            &[0, 6, 6],
            &[5, 10, 10],
            &["word", "word", "SYNONYM"],
            &[1, 1, 0],
            &[1, 1, 1],
        ),
        (
            "dogs running",
            &["dogs", "dog", "running"],
            &[0, 0, 5],
            &[4, 4, 12],
            &["word", "SYNONYM", "word"],
            &[1, 0, 1],
            &[1, 1, 1],
        ),
        (
            "small dogs running",
            &["small", "dogs", "dog", "running"],
            &[0, 6, 6, 11],
            &[5, 10, 10, 18],
            &["word", "word", "SYNONYM", "word"],
            &[1, 1, 0, 1],
            &[1, 1, 1, 1],
        ),
        ("guinea", &["guinea"], &[0], &[6], &["word"], &[1], &[1]),
        ("pig", &["pig"], &[0], &[3], &["word"], &[1], &[1]),
        (
            "guinea pig",
            &["guinea", "cavy", "pig"],
            &[0, 0, 7],
            &[6, 10, 10],
            &["word", "SYNONYM", "word"],
            &[1, 0, 1],
            &[1, 2, 1],
        ),
        (
            "guinea dogs",
            &["guinea", "dogs", "dog"],
            &[0, 7, 7],
            &[6, 11, 11],
            &["word", "word", "SYNONYM"],
            &[1, 1, 0],
            &[1, 1, 1],
        ),
        (
            "dogs guinea",
            &["dogs", "dog", "guinea"],
            &[0, 0, 5],
            &[4, 4, 11],
            &["word", "SYNONYM", "word"],
            &[1, 0, 1],
            &[1, 1, 1],
        ),
        (
            "dogs guinea pig",
            &["dogs", "dog", "guinea", "cavy", "pig"],
            &[0, 0, 5, 5, 12],
            &[4, 4, 11, 15, 15],
            &["word", "SYNONYM", "word", "SYNONYM", "word"],
            &[1, 0, 1, 0, 1],
            &[1, 1, 1, 2, 1],
        ),
        (
            "guinea pig dogs",
            &["guinea", "cavy", "pig", "dogs", "dog"],
            &[0, 0, 7, 11, 11],
            &[6, 10, 10, 15, 15],
            &["word", "SYNONYM", "word", "word", "SYNONYM"],
            &[1, 0, 1, 1, 0],
            &[1, 2, 1, 1, 1],
        ),
        (
            "small dogs and guinea pig running",
            &[
                "small", "dogs", "dog", "and", "guinea", "cavy", "pig", "running",
            ],
            &[0, 6, 6, 11, 15, 15, 22, 26],
            &[5, 10, 10, 14, 21, 25, 25, 33],
            &[
                "word", "word", "SYNONYM", "word", "word", "SYNONYM", "word", "word",
            ],
            &[1, 1, 0, 1, 1, 0, 1, 1],
            &[1, 1, 1, 1, 1, 2, 1, 1],
        ),
        (
            "small guinea pig and dogs running",
            &[
                "small", "guinea", "cavy", "pig", "and", "dogs", "dog", "running",
            ],
            &[0, 6, 6, 13, 17, 21, 21, 26],
            &[5, 12, 16, 16, 20, 25, 25, 33],
            &[
                "word", "word", "SYNONYM", "word", "word", "word", "SYNONYM", "word",
            ],
            &[1, 1, 0, 1, 1, 1, 0, 1],
            &[1, 1, 2, 1, 1, 1, 1, 1],
        ),
    ];
    for (input, output, starts, ends, types, incs, lens) in cases {
        B::assert_analyzes_to_graph(
            &analyzer,
            input,
            output,
            Some(starts),
            Some(ends),
            Some(types),
            Some(incs),
            Some(lens),
            true, // check that offsets are correct
        )?;
    }
    Ok(())
}

#[test]
fn test_mock_synonym_filter() {
    run_suite(
        "org.apache.lucene.tests.analysis.TestMockSynonymFilter",
        &[("test", test_mock_synonym_filter_test)],
    );
}

// ---------------------------------------------------------------------------
// TestToken
// ---------------------------------------------------------------------------

mod test_token {
    use super::*;

    pub fn test_ctor() -> Result<()> {
        let t = Token::with_offsets("hello", 0, 0);
        Assert::assert_equals(0, t.start_offset());
        Assert::assert_equals(0, t.end_offset());
        Assert::assert_equals(1, t.get_position_increment());
        Assert::assert_equals(1, t.get_position_length());
        Assert::assert_equals_str(None, Some("hello"), Some(&t.term()));
        Assert::assert_equals_str(None, Some("word"), Some(t.type_value()));
        Assert::assert_equals(0, t.get_flags());
        Assert::assert_true(t.get_payload().is_none());
        Ok(())
    }

    /* the CharTermAttributeStuff is tested by TestCharTermAttributeImpl */

    pub fn test_clone() -> Result<()> {
        let mut t = Token::new();
        t.set_offset(0, 5);
        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, 5);
        let buf = t.buffer().as_ptr();
        let copy = assert_clone_is_equal(&t);
        Assert::assert_equals_str(None, Some(&t.term()), Some(&copy.term()));
        Assert::assert_true(buf != copy.buffer().as_ptr());

        let pl = BytesRef::new(vec![1, 2, 3, 4]);
        t.set_payload(Some(pl.clone()));
        let copy = assert_clone_is_equal(&t);
        Assert::assert_true(Some(&pl) == copy.get_payload());
        Assert::assert_true(!std::ptr::eq(
            t.get_payload().unwrap(),
            copy.get_payload().unwrap(),
        ));
        Ok(())
    }

    pub fn test_copy_to() -> Result<()> {
        let t = Token::new();
        let copy = assert_copy_is_equal(&t);
        Assert::assert_equals_str(None, Some(""), Some(&t.term()));
        Assert::assert_equals_str(None, Some(""), Some(&copy.term()));

        let mut t = Token::new();
        t.set_offset(0, 5);
        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, 5);
        let buf = t.buffer().as_ptr();
        let copy = assert_copy_is_equal(&t);
        Assert::assert_equals_str(None, Some(&t.term()), Some(&copy.term()));
        Assert::assert_true(buf != copy.buffer().as_ptr());

        let pl = BytesRef::new(vec![1, 2, 3, 4]);
        t.set_payload(Some(pl.clone()));
        let copy = assert_copy_is_equal(&t);
        Assert::assert_true(Some(&pl) == copy.get_payload());
        Assert::assert_true(!std::ptr::eq(
            t.get_payload().unwrap(),
            copy.get_payload().unwrap(),
        ));
        Ok(())
    }

    pub trait SenselessAttribute: Attribute {}

    #[derive(Clone, Debug, Default)]
    pub struct SenselessAttributeImpl;

    impl Attribute for SenselessAttributeImpl {}
    impl SenselessAttribute for SenselessAttributeImpl {}

    impl AttributeImpl for SenselessAttributeImpl {
        fn copy_to(&self, _target: &mut dyn AttributeImpl) {}
        fn clear(&mut self) {}
        fn reflect_with(&self, _reflector: &mut dyn AttributeReflector) {}
        fn clone_box(&self) -> Box<dyn AttributeImpl> {
            Box::new(self.clone())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn attribute_interfaces(&self) -> &'static [std::any::TypeId] {
            static IDS: std::sync::OnceLock<Vec<std::any::TypeId>> = std::sync::OnceLock::new();
            IDS.get_or_init(|| {
                vec![
                    std::any::TypeId::of::<SenselessAttributeImpl>(),
                    std::any::TypeId::of::<dyn SenselessAttribute>(),
                ]
            })
        }
        fn equals(&self, other: &dyn AttributeImpl) -> bool {
            other.as_any().is::<SenselessAttributeImpl>()
        }
        fn hash_code(&self) -> i32 {
            0
        }
    }

    fn is_token(att: &Att) -> bool {
        att.implementation().as_any().is::<Token>()
    }

    /// Java's `DEFAULT_ATTRIBUTE_FACTORY` finds `SenselessAttributeImpl` by
    /// reflection; Rust registers it in the delegate of the `Token` factory.
    pub fn test_token_attribute_factory() -> Result<()> {
        let mut delegate = DefaultAttributeFactory::new();
        delegate.register(
            std::any::TypeId::of::<dyn SenselessAttribute>(),
            SenselessAttributeImpl::default,
        );
        let mut ts = MockTokenizer::with_factory(
            Token::token_attribute_factory_over(Arc::new(delegate)),
            Arc::clone(&WHITESPACE),
            false,
            DEFAULT_MAX_TOKEN_LENGTH,
        )?;
        ts.set_reader(Box::new(StringCharReader::new("foo bar")))?;
        let source: &mut AttributeSource =
            rucene::analysis::TokenStream::attribute_source_mut(&mut ts);

        let senseless = Att::add(source, std::any::TypeId::of::<dyn SenselessAttribute>())?;
        Assert::assert_true_msg(
            "SenselessAttribute is not implemented by SenselessAttributeImpl",
            senseless
                .implementation()
                .as_any()
                .is::<SenselessAttributeImpl>(),
        );

        Assert::assert_true_msg(
            "CharTermAttribute is not implemented by Token",
            is_token(&Att::add(source, char_term_id())?),
        );
        Assert::assert_true_msg(
            "OffsetAttribute is not implemented by Token",
            is_token(&Att::add(source, offset_id())?),
        );
        Assert::assert_true_msg(
            "FlagsAttribute is not implemented by Token",
            is_token(&Att::add(source, flags_id())?),
        );
        Assert::assert_true_msg(
            "PayloadAttribute is not implemented by Token",
            is_token(&Att::add(source, payload_id())?),
        );
        Assert::assert_true_msg(
            "PositionIncrementAttribute is not implemented by Token",
            is_token(&Att::add(source, pos_inc_id())?),
        );
        Assert::assert_true_msg(
            "TypeAttribute is not implemented by Token",
            is_token(&Att::add(source, type_id())?),
        );
        Ok(())
    }

    pub fn test_attribute_reflection() -> Result<()> {
        let mut t = Token::with_offsets("foobar", 6, 22);
        t.set_flags(8);
        t.set_position_increment(3);
        t.set_position_length(11);
        t.set_term_frequency(42);
        let packed = std::any::type_name::<rucene::analysis::PackedTokenAttributeImpl>();
        let term = std::any::type_name::<rucene::analysis::CharTermAttributeImpl>();
        let token = std::any::type_name::<Token>();
        let mut expected: HashMap<String, String> = HashMap::new();
        expected.insert(format!("{term}#term"), format!("{:?}", "foobar"));
        expected.insert(
            format!("{term}#bytes"),
            format!("{:?}", BytesRef::new(b"foobar".to_vec())),
        );
        expected.insert(format!("{packed}#startOffset"), "6".to_string());
        expected.insert(format!("{packed}#endOffset"), "22".to_string());
        expected.insert(format!("{packed}#positionIncrement"), "3".to_string());
        expected.insert(format!("{packed}#positionLength"), "11".to_string());
        expected.insert(format!("{token}#payload"), "None".to_string());
        expected.insert(format!("{packed}#type"), format!("{:?}", "word"));
        expected.insert(format!("{token}#flags"), "8".to_string());
        expected.insert(format!("{packed}#termFrequency"), "42".to_string());
        TestUtil::assert_attribute_reflection(&t, &expected);
        Ok(())
    }

    pub fn assert_clone_is_equal(att: &Token) -> Token {
        let clone = att.clone();
        Assert::assert_true_msg("Clone must be equal", att.equals(&clone));
        Assert::assert_equals_msg(
            Some("Clone's hashcode must be equal"),
            AttributeImpl::hash_code(att),
            AttributeImpl::hash_code(&clone),
        );
        clone
    }

    pub fn assert_copy_is_equal(att: &Token) -> Token {
        let mut copy = Token::default();
        att.copy_to(&mut copy);
        Assert::assert_true_msg("Copied instance must be equal", att.equals(&copy));
        Assert::assert_equals_msg(
            Some("Copied instance's hashcode must be equal"),
            AttributeImpl::hash_code(att),
            AttributeImpl::hash_code(&copy),
        );
        copy
    }
}

#[test]
fn test_token() {
    use test_token::*;
    run_suite(
        "org.apache.lucene.tests.analysis.TestToken",
        &[
            ("testCtor", test_ctor),
            ("testClone", test_clone),
            ("testCopyTo", test_copy_to),
            ("testTokenAttributeFactory", test_token_attribute_factory),
            ("testAttributeReflection", test_attribute_reflection),
        ],
    );
}

// ---------------------------------------------------------------------------
// TrivialLookaheadFilter
// ---------------------------------------------------------------------------

/// The position of [`TrivialLookaheadFilterLogic`]: Java's
/// `TrivialLookaheadFilter.TestPosition`.
#[derive(Debug, Default)]
struct TestPosition {
    position: Position,
    fact: Option<String>,
}

impl rucene::util::Resettable for TestPosition {
    fn reset(&mut self) {
        rucene::util::Resettable::reset(&mut self.position);
    }
}

impl LookaheadPosition for TestPosition {
    fn position(&self) -> &Position {
        &self.position
    }
    fn position_mut(&mut self) -> &mut Position {
        &mut self.position
    }
}

/// Simple example of a token filter that injects a "fact" after each token.
#[derive(Debug)]
struct TrivialLookaheadFilterLogic {
    term_att: Att,
    pos_inc_att: Att,
    offset_att: Att,
    insert_upto: i32,
}

impl TrivialLookaheadFilterLogic {
    fn peek_sentence(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn rucene::analysis::TokenStream,
    ) -> Result<()> {
        let mut facts: Vec<String> = Vec::new();
        let mut have_sentence = false;
        loop {
            if f.peek_token(source, input)? {
                let term = f.logic.term_att.term_string();
                facts.push(format!("{}-huh?", term));
                if term == "." {
                    have_sentence = true;
                }
            } else {
                have_sentence = true;
            }
            if have_sentence {
                break;
            }
        }

        // attach the (now disambiguated) analyzed tokens to the positions.
        for (x, fact) in facts.into_iter().enumerate() {
            // sentenceTokens is just relative to sentence, positions is absolute.
            let output_pos = f.output_pos;
            f.positions.get(output_pos + x as i32).fact = Some(fact);
        }
        Ok(())
    }
}

impl LookaheadLogic for TrivialLookaheadFilterLogic {
    type Pos = TestPosition;

    fn increment_token(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        input: &mut dyn rucene::analysis::TokenStream,
    ) -> Result<bool> {
        // At the outset, getMaxPos is -1. So we'll peek. When we reach the end of the sentence and go
        // to the
        // first token of the next sentence, maxPos will be the prev sentence's end token, and we'll go
        // again.
        if f.positions.get_max_pos() < f.output_pos {
            Self::peek_sentence(f, source, input)?;
        }

        f.next_token(source, input)
    }

    fn after_reset(f: &mut LookaheadTokenFilter<Self>) -> Result<()> {
        f.logic.insert_upto = -1;
        Ok(())
    }

    fn after_position(
        f: &mut LookaheadTokenFilter<Self>,
        source: &mut AttributeSource,
        _input: &mut dyn rucene::analysis::TokenStream,
    ) -> Result<()> {
        if f.logic.insert_upto < f.output_pos {
            f.insert_token(source);
            // replace term with 'improved' term.
            source.clear_attributes();
            f.logic.term_att.char_term_mut().set_empty();
            f.logic.pos_inc_att.pos_inc_mut().set_position_increment(0);
            let output_pos = f.output_pos;
            let fact = f
                .positions
                .get(output_pos)
                .fact
                .clone()
                .unwrap_or_else(|| "null".to_string());
            f.logic.term_att.char_term_mut().append_string(Some(&fact));
            let start = f.positions.get(output_pos).position.start_offset;
            let end = f.positions.get(output_pos + 1).position.end_offset;
            f.logic.offset_att.offset_mut().set_offset(start, end);
            f.logic.insert_upto = f.output_pos;
        }
        Ok(())
    }
}

fn trivial_lookahead_filter(
    input: Box<dyn rucene::analysis::TokenStream>,
) -> Result<rucene::analysis::TokenFilter<LookaheadTokenFilter<TrivialLookaheadFilterLogic>>> {
    new_lookahead_filter(input, |source| {
        Ok(TrivialLookaheadFilterLogic {
            term_att: Att::add(source, char_term_id())?,
            pos_inc_att: Att::add(source, pos_inc_id())?,
            offset_att: Att::add(source, offset_id())?,
            insert_upto: 0,
        })
    })
}

// ---------------------------------------------------------------------------
// TestLookaheadTokenFilter
// ---------------------------------------------------------------------------

mod test_lookahead_token_filter {
    use super::*;

    /// `IndexWriter.MAX_TERM_LENGTH`.
    const MAX_TERM_LENGTH: i32 = rucene::index::index_writer::MAX_TERM_LENGTH as i32;

    pub fn test_random_strings() -> Result<()> {
        let a = arc(FnAnalyzer::new(|_| {
            let random = random();
            let tokenizer = Rc::new(RefCell::new(MockTokenizer::with_max_token_length(
                Arc::clone(&WHITESPACE),
                random.next_boolean(),
                MAX_TERM_LENGTH / 2,
            )?));
            let output = MockRandomLookaheadTokenFilter::new(&*random, shared(&tokenizer))?;
            Ok(components(tokenizer, sink(output)))
        }));
        let max_length = if *TEST_NIGHTLY { 8192 } else { 1024 };
        B::check_random_data_max_word_length(&*random(), &a, 50 * *RANDOM_MULTIPLIER, max_length)
    }

    #[derive(Debug)]
    struct NeverPeeksLookaheadTokenFilter;

    impl LookaheadLogic for NeverPeeksLookaheadTokenFilter {
        type Pos = Position;

        fn increment_token(
            f: &mut LookaheadTokenFilter<Self>,
            source: &mut AttributeSource,
            input: &mut dyn rucene::analysis::TokenStream,
        ) -> Result<bool> {
            f.next_token(source, input)
        }
    }

    pub fn test_never_calling_peek() -> Result<()> {
        let a = arc(FnAnalyzer::new(|_| {
            let tokenizer = Rc::new(RefCell::new(MockTokenizer::with_max_token_length(
                Arc::clone(&WHITESPACE),
                random().next_boolean(),
                MAX_TERM_LENGTH / 2,
            )?));
            let output =
                new_lookahead_filter(shared(&tokenizer), |_| Ok(NeverPeeksLookaheadTokenFilter))?;
            Ok(components(tokenizer, sink(output)))
        }));
        let max_length = if *TEST_NIGHTLY { 8192 } else { 1024 };
        B::check_random_data_max_word_length(&*random(), &a, 50 * *RANDOM_MULTIPLIER, max_length)
    }

    pub fn test_missed_first_token() -> Result<()> {
        let analyzer = FnAnalyzer::new(|_| {
            let source = Rc::new(RefCell::new(MockTokenizer::new(
                Arc::clone(&WHITESPACE),
                false,
            )?));
            let filter = trivial_lookahead_filter(shared(&source))?;
            Ok(components(source, sink(filter)))
        });

        B::assert_analyzes_to(
            &analyzer,
            "Only he who is running knows .",
            &[
                "Only",
                "Only-huh?",
                "he",
                "he-huh?",
                "who",
                "who-huh?",
                "is",
                "is-huh?",
                "running",
                "running-huh?",
                "knows",
                "knows-huh?",
                ".",
                ".-huh?",
            ],
        )
    }
}

#[test]
fn test_lookahead_token_filter() {
    use test_lookahead_token_filter::*;
    run_suite(
        "org.apache.lucene.tests.analysis.TestLookaheadTokenFilter",
        &[
            ("testRandomStrings", test_random_strings),
            ("testNeverCallingPeek", test_never_calling_peek),
            ("testMissedFirstToken", test_missed_first_token),
        ],
    );
}

// ---------------------------------------------------------------------------
// Core regressions the indexing branch of `checkRandomData` and
// `testChangeGaps` depend on, each on one factory pick or value kind.
// ---------------------------------------------------------------------------

/// Indexes `field` in a one-document index and returns the term vector of
/// field `f` of document 0 as `(term bytes, start offsets, end offsets)` per
/// term.
fn index_and_read_vectors(
    field: rucene::document::Field,
    analyzer: Arc<dyn Analyzer>,
) -> Vec<(Vec<u8>, Vec<i32>, Vec<i32>)> {
    use rucene::document::Document;
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::index::{directory_reader, IndexReader, POSTINGS_ENUM_ALL};
    use rucene::store::{ByteBuffersDirectory, Directory};

    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer =
        IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::with_analyzer(analyzer)).unwrap();
    let mut doc = Document::new();
    doc.add(Box::new(field));
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader = directory_reader::open(dir).unwrap();
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>)
        .leaves()
        .unwrap();
    let leaf = leaves[0].leaf_reader();
    let fields = leaf.term_vectors().unwrap().get(0).unwrap().unwrap();
    let terms = fields.terms("f").unwrap().unwrap();
    let mut te = terms.iterator().unwrap();
    let mut out = Vec::new();
    while let Some(term) = te.next().unwrap() {
        let mut postings = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
        postings.next_doc().unwrap();
        let (mut starts, mut ends) = (Vec::new(), Vec::new());
        for _ in 0..postings.freq().unwrap() {
            postings.next_position().unwrap();
            starts.push(postings.start_offset());
            ends.push(postings.end_offset());
        }
        out.push((term.slice().to_vec(), starts, ends));
    }
    reader.close().unwrap();
    out
}

fn vectors_type() -> rucene::document::FieldType {
    let mut ft =
        rucene::document::FieldType::new_from(rucene::document::TextField::type_not_stored());
    ft.set_store_term_vectors(true).unwrap();
    ft.set_store_term_vector_positions(true).unwrap();
    ft.set_store_term_vector_offsets(true).unwrap();
    ft
}

/// A stream whose attributes are one `Token` (`Token.TOKEN_ATTRIBUTE_FACTORY`)
/// is indexed through the attribute interfaces, as Java's
/// `FieldInvertState` reads `TermToBytesRefAttribute`, `OffsetAttribute` and
/// `PositionIncrementAttribute` whatever their implementation.
#[test]
fn token_factory_stream_is_indexed_by_interface() {
    let tokens = vec![
        Token::with_offsets("ab", 0, 2),
        Token::with_offsets("cd", 3, 5),
    ];
    let stream = Rc::new(RefCell::new(
        rucene_test_framework::analysis::CannedTokenStream::new(tokens),
    ));
    let field =
        rucene::document::Field::new_with_token_stream("f", stream, vectors_type()).unwrap();
    let vectors =
        index_and_read_vectors(field, Arc::new(rucene::analysis::StandardAnalyzer::new()));
    assert_eq!(
        vectors,
        vec![
            (b"ab".to_vec(), vec![0], vec![2]),
            (b"cd".to_vec(), vec![3], vec![5]),
        ]
    );
}

/// A field whose value is a `Reader` is analyzed from that reader, as
/// `Field.tokenStream` does with `analyzer.tokenStream(name(),
/// readerValue())`.
#[test]
fn reader_valued_field_is_analyzed_from_its_reader() {
    let field = rucene::document::Field::new_with_reader(
        "f",
        Box::new(StringCharReader::new("ab cd")),
        vectors_type(),
    )
    .unwrap();
    let vectors =
        index_and_read_vectors(field, Arc::new(rucene::analysis::StandardAnalyzer::new()));
    assert_eq!(
        vectors,
        vec![
            (b"ab".to_vec(), vec![0], vec![2]),
            (b"cd".to_vec(), vec![3], vec![5]),
        ]
    );
}

/// `Analyzer.normalize` builds its stream over `attributeFactory(fieldName)`:
/// `MockBytesAnalyzer`'s `MockUTF16TermAttributeImpl` turns the term into its
/// UTF-16LE bytes.
#[test]
fn normalize_uses_the_field_attribute_factory() {
    let a = rucene_test_framework::analysis::MockBytesAnalyzer::new();
    let bytes = a.normalize("f", "ab").unwrap();
    assert_eq!(bytes.slice(), &[0x61, 0x00, 0x62, 0x00]);
}

/// `RandomIndexWriter.mockIndexWriter`'s writer has its test points enabled
/// (`isEnableTestPoints()` overridden to `true`): the `TP` messages reach the
/// `TestPoint` through the `TestPointInfoStream`.
#[test]
fn mock_index_writer_enables_test_points() {
    use rucene::document::{Document, Store, StringField};
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::store::{ByteBuffersDirectory, Directory};
    use rucene_test_framework::index::random_index_writer::TestPoint;
    use rucene_test_framework::index::RandomIndexWriter;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(AtomicUsize);
    impl TestPoint for Counting {
        fn apply(&self, _message: &str) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let counting = Arc::new(Counting(AtomicUsize::new(0)));
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = RandomIndexWriter::mock_index_writer_with_test_point(
        &randomizedtesting::jdk::random::JdkRandom::new(7),
        dir,
        IndexWriterConfig::new(),
        Arc::clone(&counting) as Arc<dyn TestPoint>,
    )
    .unwrap();
    Assert::assert_true(writer.is_enable_test_points());
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", "1".to_string(), Store::NO).unwrap(),
    ));
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    Assert::assert_true_msg(
        "no test point reached the TestPoint",
        counting.0.load(Ordering::Relaxed) > 0,
    );
}

/// `MockBytesAnalyzer` indexes the UTF-16LE bytes its
/// `MockUTF16TermAttributeImpl` gives for the term (`TermToBytesRefAttribute`
/// read by interface), as in Java. Its `MockTokenizer` draws from the
/// randomized context, so the check runs as a suite.
fn mock_bytes_analyzer_indexes_utf16_bytes() -> Result<()> {
    let mut ft = vectors_type();
    ft.set_store_term_vector_offsets(false)?;
    let field = rucene::document::Field::new("f", "ab".to_string(), ft)?;
    let vectors = index_and_read_vectors(
        field,
        Arc::new(rucene_test_framework::analysis::MockBytesAnalyzer::new()),
    );
    Assert::assert_equals(1, vectors.len() as i32);
    Assert::assert_true(vectors[0].0 == [0x61, 0x00, 0x62, 0x00]);
    Ok(())
}

#[test]
fn test_mock_bytes_analyzer_indexing() {
    run_suite(
        "org.apache.lucene.tests.analysis.TestMockBytesAnalyzerIndexing",
        &[(
            "testIndexesUtf16Bytes",
            mock_bytes_analyzer_indexes_utf16_bytes,
        )],
    );
}
