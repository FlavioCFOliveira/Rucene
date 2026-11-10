//! Port of `org.apache.lucene.analysis.TestDelegatingAnalyzerWrapper`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharFilter, CharReader, DelegatingAnalyzerWrapper,
    DelegatingAnalyzerWrapperLogic, GlobalReuseStrategy, TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_bytes_analyzer::MockBytesAnalyzer;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::tokenizer_components;
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::{lucene_class, t};

/// The anonymous `DelegatingAnalyzerWrapper`s of this class, whose
/// `getWrappedAnalyzer` returns `analyzer`.
#[derive(Debug)]
struct Delegating<A: Analyzer + Send + Sync> {
    analyzer: A,
}

impl<A: Analyzer + Send + Sync + std::fmt::Debug> DelegatingAnalyzerWrapperLogic for Delegating<A> {
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        &self.analyzer
    }
}

/// `new DelegatingAnalyzerWrapper(Analyzer.GLOBAL_REUSE_STRATEGY) {
/// getWrappedAnalyzer(fieldName) { return analyzer; } }`.
fn delegating<A: Analyzer + Send + Sync + std::fmt::Debug + 'static>(
    analyzer: A,
) -> DelegatingAnalyzerWrapper {
    DelegatingAnalyzerWrapper::new(
        Box::new(GlobalReuseStrategy),
        Box::new(Delegating { analyzer }),
    )
}

fn test_delegates_normalization() -> Result<()> {
    let analyzer1 =
        MockAnalyzer::with_automaton(&*LuceneTestCase::random(), Arc::clone(&WHITESPACE), false);
    let w1 = delegating(analyzer1);
    Assert::assert_equals(
        BytesRef::from_char_sequence("Ab C"),
        w1.normalize("foo", "Ab C")?,
    );

    let analyzer2 =
        MockAnalyzer::with_automaton(&*LuceneTestCase::random(), Arc::clone(&WHITESPACE), true);
    let w2 = delegating(analyzer2);
    Assert::assert_equals(
        BytesRef::from_char_sequence("ab c"),
        w2.normalize("foo", "Ab C")?,
    );
    Ok(())
}

fn test_delegates_attribute_factory() -> Result<()> {
    let analyzer1 = MockBytesAnalyzer::new();
    let w1 = delegating(analyzer1);
    // `"Ab C".getBytes(StandardCharsets.UTF_16LE)`
    let utf16le: Vec<u8> = "Ab C"
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    Assert::assert_equals(BytesRef::new(utf16le), w1.normalize("foo", "Ab C")?);
    Ok(())
}

/// The anonymous `Analyzer` of `testDelegatesCharFilter`.
#[derive(Debug)]
struct CharFilteringAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CharFilteringAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.TestDelegatingAnalyzerWrapper$4"
    }

    fn init_reader_for_normalization(
        &self,
        _field_name: &str,
        reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        Box::new(DummyCharFilter::new(reader, 'b', 'z'))
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::with_factory_only(self.attribute_factory(field_name))?;
        Ok(tokenizer_components(std::rc::Rc::new(
            std::cell::RefCell::new(tokenizer),
        )))
    }
}

fn test_delegates_char_filter() -> Result<()> {
    let analyzer1 = CharFilteringAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    };
    let w1 = delegating(analyzer1);
    Assert::assert_equals(
        BytesRef::from_char_sequence("az c"),
        w1.normalize("foo", "ab c")?,
    );
    Ok(())
}

/// `DummyCharFilter` (`private static class … extends CharFilter`): replaces
/// each `match` char it reads by `repl`.
#[derive(Debug)]
struct DummyCharFilter {
    input: Box<dyn CharReader>,
    r#match: char,
    repl: char,
}

impl DummyCharFilter {
    fn new(input: Box<dyn CharReader>, r#match: char, repl: char) -> DummyCharFilter {
        DummyCharFilter {
            input,
            r#match,
            repl,
        }
    }
}

impl CharReader for DummyCharFilter {
    /// `read(char[] cbuf, int off, int len)` over the slice `cbuf[off..off +
    /// len]`.
    fn read(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        let read = self.input.read(cbuf)?;
        for c in cbuf.iter_mut().take(read) {
            // `match` and `repl` are BMP chars, one UTF-16 unit each.
            if u32::from(*c) == u32::from(self.r#match) {
                *c = self.repl as u16;
            }
        }
        Ok(read)
    }

    /// `CharFilter.close()`: closes the input.
    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for DummyCharFilter {
    fn correct(&self, current_off: i32) -> i32 {
        current_off
    }

    fn input(&self) -> &dyn CharReader {
        self.input.as_ref()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.input.as_mut()
    }
}

/// The class `org.apache.lucene.analysis.TestDelegatingAnalyzerWrapper`.
pub fn class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.TestDelegatingAnalyzerWrapper")
        .method(t(
            "testDelegatesNormalization",
            test_delegates_normalization,
        ))
        .method(t(
            "testDelegatesAttributeFactory",
            test_delegates_attribute_factory,
        ))
        .method(t("testDelegatesCharFilter", test_delegates_char_filter))
        .build()
}
