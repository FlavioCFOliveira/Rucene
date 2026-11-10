//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocInverterPerFieldErrorInfo.java`.
//!
//! Test adding to the info stream when there's an exception thrown during
//! field analysis.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::util::attribute::AttributeSource;
use rucene::util::PrintStreamInfoStream;
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{components, shared, sink, tokenizer_components};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, test};

/// `TestDocInverterPerFieldErrorInfo`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocInverterPerFieldErrorInfo",
        vec![
            test(
                "testInfoStreamGetsFieldName",
                test_info_stream_gets_field_name,
            ),
            test("testNoExtraNoise", test_no_extra_noise),
        ],
    )
}

/// `private static final FieldType storedTextType = new
/// FieldType(TextField.TYPE_NOT_STORED)`.
fn stored_text_type() -> FieldType {
    FieldType::new_from(TextField::type_not_stored())
}

/// The Java class of `private static class BadNews extends RuntimeException`.
const BAD_NEWS: &str = "org.apache.lucene.index.TestDocInverterPerFieldErrorInfo$BadNews";

/// `new BadNews(message)`.
fn bad_news(message: &str) -> LuceneError {
    LuceneError::Unchecked {
        class: BAD_NEWS,
        message: Some(message.to_string()),
        cause: None,
    }
}

/// The anonymous `TokenFilter` of `ThrowingAnalyzer`, whose
/// `incrementToken()` throws `BadNews`.
#[derive(Debug)]
struct Tosser;

impl TokenFilterLogic for Tosser {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        _input: &mut dyn TokenStream,
    ) -> Result<bool> {
        Err(bad_news("Something is icky."))
    }
}

/// `private static class ThrowingAnalyzer extends Analyzer`.
#[derive(Debug)]
struct ThrowingAnalyzer {
    state: AnalyzerState,
}

impl ThrowingAnalyzer {
    fn new() -> Self {
        ThrowingAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for ThrowingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDocInverterPerFieldErrorInfo$ThrowingAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Rc::new(RefCell::new(MockTokenizer::whitespace_lower_case()?));
        if field_name == "distinctiveFieldName" {
            let tosser = TokenFilter::new(shared(&tokenizer), Tosser);
            Ok(components(tokenizer, sink(tosser)))
        } else {
            Ok(tokenizer_components(tokenizer))
        }
    }
}

/// A `ByteArrayOutputStream` a `PrintStream` writes to.
#[derive(Clone, Default)]
struct ByteArrayOutputStream(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for ByteArrayOutputStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ByteArrayOutputStream {
    /// `infoBytes.toString(UTF_8)`.
    fn to_string_utf8(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

/// `new IndexWriterConfig(new ThrowingAnalyzer())` logging to a
/// `PrintStreamInfoStream` over `infoBytes`.
fn throwing_config(info_bytes: &ByteArrayOutputStream) -> Result<IndexWriterConfig> {
    let mut c = IndexWriterConfig::with_analyzer(Some(Arc::new(ThrowingAnalyzer::new())));
    let print_stream_info_stream = PrintStreamInfoStream::new(Box::new(info_bytes.clone()));
    c.set_info_stream(Arc::new(print_stream_info_stream))?;
    Ok(c)
}

/// Port of `TestDocInverterPerFieldErrorInfo.testInfoStreamGetsFieldName`.
fn test_info_stream_gets_field_name() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let info_bytes = ByteArrayOutputStream::default();
    let c = throwing_config(&info_bytes)?;
    let writer = IndexWriter::new(dir.as_directory(), c)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "distinctiveFieldName",
        "aaa ",
        &stored_text_type(),
    )));
    expect_throws(BAD_NEWS, || writer.add_document(&doc));
    let info_stream = info_bytes.to_string_utf8();
    Assert::assert_true(info_stream.contains("distinctiveFieldName"));

    writer.close()?;
    dir.close()
}

/// Port of `TestDocInverterPerFieldErrorInfo.testNoExtraNoise`.
fn test_no_extra_noise() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let info_bytes = ByteArrayOutputStream::default();
    let c = throwing_config(&info_bytes)?;
    let writer = IndexWriter::new(dir.as_directory(), c)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_field(
        "boringFieldName",
        "aaa ",
        &stored_text_type(),
    )));
    // should not throw BadNews
    writer.add_document(&doc)?;
    let info_stream = info_bytes.to_string_utf8();
    Assert::assert_false(info_stream.contains("boringFieldName"));

    writer.close()?;
    dir.close()
}
