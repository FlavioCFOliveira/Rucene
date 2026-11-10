//! Port of `org.apache.lucene.analysis.TestCachingTokenFilter`
//! (`extends BaseTokenStreamTestCase`).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, CachingTokenFilter, TokenStream,
};
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::index::{IndexReader, MultiTerms};
use rucene::search::NO_MORE_DOCS;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, Att};
use rucene_test_framework::analysis::{BaseTokenStreamTestCase, MockAnalyzer};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::t;

/// The `tokens` field.
const TOKENS: [&str; 4] = ["term1", "term2", "term3", "term2"];

/// The anonymous `TokenStream` of `testCaching`.
#[derive(Debug)]
struct CountingStream {
    source: AttributeSource,
    index: usize,
    term_att: Att,
    offset_att: Att,
    reset_count: Arc<AtomicI32>,
}

impl CountingStream {
    fn new(reset_count: Arc<AtomicI32>) -> Result<Self> {
        // `new TokenStream()`: `DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        Ok(Self {
            source,
            index: 0,
            term_att,
            offset_att,
            reset_count,
        })
    }
}

impl TokenStream for CountingStream {
    fn reset(&mut self) -> Result<()> {
        // `super.reset()` does nothing.
        self.reset_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn increment_token(&mut self) -> Result<bool> {
        if self.index == TOKENS.len() {
            Ok(false)
        } else {
            self.source.clear_attributes();
            self.term_att
                .char_term_mut()
                .append_string(Some(TOKENS[self.index]));
            self.index += 1;
            self.offset_att.offset_mut().set_offset(0, 0);
            Ok(true)
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

fn test_caching() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer =
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?;
    let mut doc = Document::new();
    let reset_count = Arc::new(AtomicI32::new(0));
    let stream = CountingStream::new(Arc::clone(&reset_count))?;

    // `stream = new CachingTokenFilter(stream)`: shared by the field and the
    // test, as Java's reference is.
    let stream = Arc::new(AtomicRefCell::new(CachingTokenFilter::new(Box::new(
        stream,
    ))));

    let field_stream: Arc<AtomicRefCell<dyn TokenStream>> = stream.clone();
    doc.add(Box::new(TextField::new_with_token_stream(
        "preanalyzed",
        field_stream,
    )?));

    // 1) we consume all tokens twice before we add the doc to the index
    Assert::assert_false(stream.borrow().is_cached());
    stream.borrow_mut().reset()?;
    Assert::assert_false(stream.borrow().is_cached());
    check_tokens(&mut *stream.borrow_mut())?;
    stream.borrow_mut().reset()?;
    check_tokens(&mut *stream.borrow_mut())?;
    Assert::assert_true(stream.borrow().is_cached());

    // 2) now add the document to the index and verify if all tokens are indexed
    //    don't reset the stream here, the DocumentWriter should do that implicitly
    writer.add_document(&doc)?;

    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    let mut term_positions = MultiTerms::get_term_postings_enum_all(
        &reader,
        "preanalyzed",
        &BytesRef::new(b"term1".to_vec()),
    )?
    .expect("postings of term1");
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    Assert::assert_equals(1, term_positions.freq()?);
    Assert::assert_equals(0, term_positions.next_position()?);

    term_positions = MultiTerms::get_term_postings_enum_all(
        &reader,
        "preanalyzed",
        &BytesRef::new(b"term2".to_vec()),
    )?
    .expect("postings of term2");
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    Assert::assert_equals(2, term_positions.freq()?);
    Assert::assert_equals(1, term_positions.next_position()?);
    Assert::assert_equals(3, term_positions.next_position()?);

    term_positions = MultiTerms::get_term_postings_enum_all(
        &reader,
        "preanalyzed",
        &BytesRef::new(b"term3".to_vec()),
    )?
    .expect("postings of term3");
    Assert::assert_true(term_positions.next_doc()? != NO_MORE_DOCS);
    Assert::assert_equals(1, term_positions.freq()?);
    Assert::assert_equals(2, term_positions.next_position()?);
    drop(term_positions);
    reader.close()?;
    writer.close()?;
    // 3) reset stream and consume tokens again
    stream.borrow_mut().reset()?;
    check_tokens(&mut *stream.borrow_mut())?;

    Assert::assert_equals(1, reset_count.load(Ordering::SeqCst));

    drop(writer);
    dir.close()
}

fn test_double_reset_fails() -> Result<()> {
    let analyzer = MockAnalyzer::new(&*LuceneTestCase::random());
    let input = analyzer.token_stream_from_str("field", "abc")?;
    let mut buffer =
        CachingTokenFilter::new(Box::new(rucene::analysis::SharedTokenStream::new(input)));
    buffer.reset()?; // ok
    let e = expect_throws("java.lang.IllegalStateException", || {
        buffer.reset() // bad (this used to work which we don't want)
    });
    Assert::assert_equals_str(None, Some("double reset()"), e.message());
    Ok(())
}

/// `checkTokens(TokenStream stream)` (`private`).
fn check_tokens(stream: &mut dyn TokenStream) -> Result<()> {
    let mut count = 0;

    let term_att = Att::get(stream.attribute_source(), char_term_id())
        .expect("getAttribute(CharTermAttribute.class)");
    while stream.increment_token()? {
        Assert::assert_true(count < TOKENS.len());
        Assert::assert_equals_str(None, Some(TOKENS[count]), Some(&term_att.term_string()));
        count += 1;
    }

    Assert::assert_equals(TOKENS.len(), count);
    Ok(())
}

/// The class `org.apache.lucene.analysis.TestCachingTokenFilter`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.analysis.TestCachingTokenFilter")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testCaching", test_caching))
        .method(t("testDoubleResetFails", test_double_reset_fails))
        .build()
}
