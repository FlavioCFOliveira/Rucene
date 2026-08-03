//! Analysis pipeline ported from `org.apache.lucene.analysis`.
//!
//! Tokenizers, token filters, analyzers, attribute sources, and the
//! character-level readers used by the pipeline live here. The API is designed
//! to mirror Lucene's `TokenStream` lifecycle while remaining safe and idiomatic
//! in Rust.

#![deny(unsafe_code)]

pub mod char_array_map;
pub mod char_array_set;
pub mod character_utils;
pub mod filtering_token_filter;
pub mod lower_case_filter;
pub mod reusable_string_reader;
pub mod stop_filter;
pub mod tokenattributes;
pub mod wordlist_loader;

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

pub use char_array_map::CharArrayMap;
pub use char_array_set::CharArraySet;
pub use character_utils::{
    fill, fill_buffer, new_character_buffer, to_chars, to_code_points, to_lower_case,
    to_upper_case, CharacterBuffer,
};
pub use filtering_token_filter::{
    new_filtering_token_filter, FilteringTokenFilter, FilteringTokenFilterAdapter,
    FilteringTokenFilterLogic,
};
pub use lower_case_filter::{new_lower_case_filter, LowerCaseFilter, LowerCaseFilterLogic};
pub use reusable_string_reader::ReusableStringReader;
pub use stop_filter::{make_stop_set, new_stop_filter, StopFilter, StopFilterLogic};
pub use tokenattributes::{
    BytesTermAttribute, BytesTermAttributeImpl, CharTermAttribute, CharTermAttributeImpl,
    FlagsAttribute, FlagsAttributeImpl, KeywordAttribute, KeywordAttributeImpl, OffsetAttribute,
    OffsetAttributeImpl, PackedTokenAttributeImpl, PayloadAttribute, PayloadAttributeImpl,
    PositionIncrementAttribute, PositionIncrementAttributeImpl, PositionLengthAttribute,
    PositionLengthAttributeImpl, SentenceAttribute, SentenceAttributeImpl, TermFrequencyAttribute,
    TermFrequencyAttributeImpl, TermToBytesRefAttribute, TypeAttribute, TypeAttributeImpl,
};
pub use wordlist_loader::{
    get_snowball_word_set, get_word_set, get_word_set_ignoring_comments, get_word_set_into,
    get_word_set_with_comment,
};

use crate::error::{LuceneError, Result};
use crate::util::attribute::{
    AsUnwrappable, AttributeFactory, AttributeReflector, AttributeSource, DefaultAttributeFactory,
    Unwrappable,
};

// -----------------------------------------------------------------------------
// CharReader
// -----------------------------------------------------------------------------

/// Character-oriented input source for tokenizers.
///
/// Equivalent to `java.io.Reader`. Rust's `std::io::Read` is byte-oriented, so
/// this trait exposes the character-level operations that tokenizers need.
pub trait CharReader: Debug {
    /// Reads characters into `buf`, returning the number read.
    ///
    /// A return value of `0` indicates end-of-stream.
    fn read(&mut self, buf: &mut [char]) -> Result<usize>;

    /// Closes the reader and releases any resources.
    fn close(&mut self) -> Result<()>;

    /// Returns a view of this reader as a [`CharFilter`], if it is one.
    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        None
    }
}

/// A [`CharReader`] backed by an in-memory string.
#[derive(Debug)]
pub struct StringCharReader {
    chars: Vec<char>,
    position: usize,
}

impl StringCharReader {
    /// Creates a reader over the characters of `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            chars: text.into().chars().collect(),
            position: 0,
        }
    }
}

impl CharReader for StringCharReader {
    fn read(&mut self, buf: &mut [char]) -> Result<usize> {
        let remaining = self.chars.len().saturating_sub(self.position);
        let n = remaining.min(buf.len());
        for (i, dst) in buf.iter_mut().enumerate().take(n) {
            *dst = self.chars[self.position + i];
        }
        self.position += n;
        Ok(n)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// CharFilter
// -----------------------------------------------------------------------------

/// A character-level filter that wraps another reader and can correct offsets.
///
/// Equivalent to `org.apache.lucene.analysis.CharFilter`.
pub trait CharFilter: CharReader {
    /// Corrects the current offset.
    fn correct(&self, current_off: i32) -> i32;

    /// Returns the underlying reader.
    fn input(&self) -> &dyn CharReader;

    /// Returns a mutable reference to the underlying reader.
    fn input_mut(&mut self) -> &mut dyn CharReader;

    /// Chains offset correction through nested `CharFilter`s.
    fn correct_offset(&self, current_off: i32) -> i32 {
        let corrected = self.correct(current_off);
        if let Some(filter) = self.input().as_char_filter() {
            filter.correct_offset(corrected)
        } else {
            corrected
        }
    }
}

/// A concrete [`CharFilter`] implemented by a correction function.
pub struct CharFilterFn<F> {
    input: Box<dyn CharReader>,
    correct_fn: F,
}

impl<F> Debug for CharFilterFn<F>
where
    F: Fn(i32) -> i32,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CharFilterFn")
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}

impl<F> CharFilterFn<F>
where
    F: Fn(i32) -> i32 + Send + Sync + 'static,
{
    /// Creates a filter over `input` using `correct_fn` to adjust offsets.
    pub fn new(input: Box<dyn CharReader>, correct_fn: F) -> Self {
        Self { input, correct_fn }
    }
}

impl<F> CharReader for CharFilterFn<F>
where
    F: Fn(i32) -> i32 + Send + Sync + 'static,
{
    fn read(&mut self, buf: &mut [char]) -> Result<usize> {
        self.input.read(buf)
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl<F> CharFilter for CharFilterFn<F>
where
    F: Fn(i32) -> i32 + Send + Sync + 'static,
{
    fn correct(&self, current_off: i32) -> i32 {
        (self.correct_fn)(current_off)
    }

    fn input(&self) -> &dyn CharReader {
        self.input.as_ref()
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        self.input.as_mut()
    }
}

// -----------------------------------------------------------------------------
// TokenStream
// -----------------------------------------------------------------------------

/// A stream of tokens produced by an analyzer, equivalent to
/// `org.apache.lucene.analysis.TokenStream`.
///
/// A `TokenStream` owns an [`AttributeSource`] and exposes a lifecycle:
/// `reset`, `increment_token`, `end`, `close`.
pub trait TokenStream: Debug {
    /// Advances to the next token and updates the attributes.
    ///
    /// Returns `true` while tokens remain; `false` at end-of-stream.
    fn increment_token(&mut self) -> Result<bool>;

    /// Performs end-of-stream work after `increment_token` returns `false`.
    fn end(&mut self) -> Result<()> {
        self.attribute_source_mut().end_attributes();
        Ok(())
    }

    /// Resets the stream to a clean state before consumption.
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    /// Releases resources associated with this stream.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    /// Returns the shared attribute source.
    fn attribute_source(&self) -> &AttributeSource;

    /// Returns a mutable reference to the shared attribute source.
    fn attribute_source_mut(&mut self) -> &mut AttributeSource;

    /// Reflects every attribute through `reflector`.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        self.attribute_source().reflect_with(reflector);
    }

    /// Returns an unwrappable view if this stream is a wrapper.
    ///
    /// The default is `None`; [`TokenFilter`] implementations override this.
    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        None
    }
}

impl AsUnwrappable<dyn TokenStream> for dyn TokenStream {
    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        TokenStream::as_unwrappable(self)
    }
}

/// Returns Lucene's default token attribute factory.
///
/// The factory uses [`PackedTokenAttributeImpl`] as the implementation for all
/// of the standard token attribute interfaces it supports.
pub fn default_token_attribute_factory() -> Arc<dyn AttributeFactory> {
    let mut factory = DefaultAttributeFactory::new();
    factory.register_attribute_impl::<PackedTokenAttributeImpl>();
    Arc::new(factory)
}

fn illegal_state_message() -> String {
    "TokenStream contract violation: reset()/close() call missing, reset() called multiple times, or subclass does not call super.reset().".to_string()
}

// -----------------------------------------------------------------------------
// Tokenizer
// -----------------------------------------------------------------------------

/// User-supplied tokenization logic for a [`Tokenizer`].
pub trait TokenizerLogic: Debug {
    /// Produces the next token, updating `source`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool>;

    /// Called at end-of-stream.
    fn end(&mut self, _source: &mut AttributeSource) -> Result<()> {
        Ok(())
    }

    /// Called when the tokenizer is reset.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        Ok(())
    }

    /// Called when the tokenizer is closed.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        Ok(())
    }
}

/// A `TokenStream` whose input is a [`CharReader`], equivalent to
/// `org.apache.lucene.analysis.Tokenizer`.
#[derive(Debug)]
pub struct Tokenizer<T: TokenizerLogic> {
    source: AttributeSource,
    input: Option<Box<dyn CharReader>>,
    input_pending: Option<Box<dyn CharReader>>,
    logic: T,
}

impl<T: TokenizerLogic> Tokenizer<T> {
    /// Creates a tokenizer with no input, awaiting a call to [`Self::set_reader`].
    pub fn new(logic: T) -> Self {
        let factory = default_token_attribute_factory();
        let mut source = AttributeSource::new_with_factory(factory);
        source.add_attribute::<PackedTokenAttributeImpl>().unwrap();
        Self {
            source,
            input: None,
            input_pending: None,
            logic,
        }
    }

    /// Sets a new reader on the tokenizer.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if the previous input has not been
    /// closed.
    pub fn set_reader(&mut self, reader: Box<dyn CharReader>) -> Result<()> {
        if self.input.is_some() {
            return Err(LuceneError::IllegalState(
                "TokenStream contract violation: close() call missing".to_string(),
            ));
        }
        self.input_pending = Some(reader);
        Ok(())
    }
}

impl<T: TokenizerLogic> TokenStream for Tokenizer<T> {
    fn increment_token(&mut self) -> Result<bool> {
        let reader = self
            .input
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState(illegal_state_message()))?;
        self.logic
            .increment_token(&mut self.source, reader.as_mut())
    }

    fn end(&mut self) -> Result<()> {
        self.logic.end(&mut self.source)?;
        self.source.end_attributes();
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // The base TokenStream::reset default is a no-op; the Java Tokenizer
        // contracts the reader move before delegating to subclass state reset.
        let reader = self
            .input_pending
            .take()
            .ok_or_else(|| LuceneError::IllegalState(illegal_state_message()))?;
        self.input = Some(reader);
        let reader = self.input.as_mut().unwrap();
        self.logic.reset(&mut self.source, reader.as_mut())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(reader) = self.input.as_mut() {
            self.logic.close(reader.as_mut())?;
            reader.close()?;
        }
        self.input = None;
        self.input_pending = None;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

// -----------------------------------------------------------------------------
// TokenFilter
// -----------------------------------------------------------------------------

/// User-supplied filtering logic for a [`TokenFilter`].
pub trait TokenFilterLogic: Debug {
    /// Produces the next filtered token.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool>;

    /// Called at end-of-stream.
    fn end(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.end()
    }

    /// Called when the filter is reset.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()
    }

    /// Called when the filter is closed.
    fn close(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.close()
    }
}

/// A `TokenStream` whose input is another `TokenStream`, equivalent to
/// `org.apache.lucene.analysis.TokenFilter`.
#[derive(Debug)]
pub struct TokenFilter<T: TokenFilterLogic> {
    source: AttributeSource,
    input: Box<dyn TokenStream>,
    logic: T,
}

impl<T: TokenFilterLogic> TokenFilter<T> {
    /// Creates a filter over the supplied input stream.
    pub fn new(input: Box<dyn TokenStream>, logic: T) -> Self {
        Self {
            source: AttributeSource::new_from(input.attribute_source()),
            input,
            logic,
        }
    }
}

impl<T: TokenFilterLogic> TokenStream for TokenFilter<T> {
    fn increment_token(&mut self) -> Result<bool> {
        self.logic
            .increment_token(&mut self.source, self.input.as_mut())
    }

    fn end(&mut self) -> Result<()> {
        self.logic.end(&mut self.source, self.input.as_mut())
    }

    fn reset(&mut self) -> Result<()> {
        self.logic.reset(&mut self.source, self.input.as_mut())
    }

    fn close(&mut self) -> Result<()> {
        self.logic.close(&mut self.source, self.input.as_mut())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        Some(self)
    }
}

impl<T: TokenFilterLogic> Unwrappable<dyn TokenStream> for TokenFilter<T> {
    fn unwrap(&self) -> &(dyn TokenStream + 'static) {
        self.input.as_ref()
    }
}

// -----------------------------------------------------------------------------
// Analyzer
// -----------------------------------------------------------------------------

/// Components that transform input text into a `TokenStream`, equivalent to
/// `org.apache.lucene.analysis.Analyzer`.
pub trait Analyzer {}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;
    use crate::util::attribute::unwrap_all;

    #[derive(Debug, Default)]
    struct WhitespaceTokenizer {
        words: Vec<String>,
        index: usize,
    }

    impl TokenizerLogic for WhitespaceTokenizer {
        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            _reader: &mut dyn CharReader,
        ) -> Result<bool> {
            source.clear_attributes();
            if self.index >= self.words.len() {
                return Ok(false);
            }
            let word = &self.words[self.index];
            self.index += 1;
            source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .unwrap()
                .append_string(word);
            Ok(true)
        }

        fn reset(
            &mut self,
            source: &mut AttributeSource,
            reader: &mut dyn CharReader,
        ) -> Result<()> {
            let mut buffer = Vec::new();
            let mut buf = ['\0'; 256];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                buffer.extend_from_slice(&buf[..n]);
            }
            self.words = buffer
                .iter()
                .collect::<String>()
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            self.index = 0;
            source.clear_attributes();
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct PassThroughFilter;

    impl TokenFilterLogic for PassThroughFilter {
        fn increment_token(
            &mut self,
            _source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            input.increment_token()
        }
    }

    #[derive(Debug, Default)]
    struct UpperCaseFilter;

    impl TokenFilterLogic for UpperCaseFilter {
        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            if !input.increment_token()? {
                return Ok(false);
            }
            let term = source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .unwrap()
                .term()
                .to_uppercase();
            source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .unwrap()
                .set_empty();
            source
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .unwrap()
                .append_string(&term);
            Ok(true)
        }
    }

    fn term_of(stream: &dyn TokenStream) -> String {
        stream
            .attribute_source()
            .get_attribute::<PackedTokenAttributeImpl>()
            .unwrap()
            .term()
    }

    #[test]
    fn token_stream_lifecycle() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("hello world")))?;
        tokenizer.reset()?;

        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "hello");
        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "world");
        assert!(!tokenizer.increment_token()?);

        tokenizer.end()?;
        tokenizer.close()?;
        Ok(())
    }

    #[test]
    fn token_filter_unwrap_and_unwrap_all() -> Result<()> {
        let tokenizer = Box::new(Tokenizer::new(WhitespaceTokenizer::default()));
        let filter1 = TokenFilter::new(tokenizer, PassThroughFilter);
        let filter2 = TokenFilter::new(Box::new(filter1), PassThroughFilter);

        let stream: &dyn TokenStream = &filter2;
        let wrapper = stream.as_unwrappable().unwrap();
        let unwrapped_once = wrapper.unwrap();
        assert!(unwrapped_once.as_unwrappable().is_some());
        assert!(unwrapped_once
            .as_unwrappable()
            .unwrap()
            .unwrap()
            .as_unwrappable()
            .is_none());

        let base = unwrap_all(stream);
        assert!(base.as_unwrappable().is_none());
        Ok(())
    }

    #[test]
    fn token_filter_shares_attributes_with_input() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("hello world")))?;

        let base = tokenizer.attribute_source() as *const AttributeSource;
        let mut filter = TokenFilter::new(Box::new(tokenizer), UpperCaseFilter);
        let wrapper = filter.attribute_source() as *const AttributeSource;

        // Different AttributeSource containers, but they share the same attribute impls.
        assert_ne!(base, wrapper);
        filter.reset()?;
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "HELLO");
        filter.end()?;
        filter.close()?;
        Ok(())
    }

    #[test]
    fn char_filter_correct_offset_chains() {
        let reader = Box::new(StringCharReader::new("text"));
        let filter1: Box<dyn CharReader> = Box::new(CharFilterFn::new(reader, |off| off + 2));
        let filter2: Box<dyn CharReader> = Box::new(CharFilterFn::new(filter1, |off| off + 3));

        let chained = filter2.as_char_filter().unwrap();
        assert_eq!(chained.correct_offset(5), 10);
    }

    #[test]
    fn tokenizer_set_reader_and_reset_with_new_reader() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());

        tokenizer.set_reader(Box::new(StringCharReader::new("hello world")))?;
        tokenizer.reset()?;
        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "hello");
        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "world");
        assert!(!tokenizer.increment_token()?);
        tokenizer.end()?;
        tokenizer.close()?;

        tokenizer.set_reader(Box::new(StringCharReader::new("foo bar")))?;
        tokenizer.reset()?;
        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "foo");
        assert!(tokenizer.increment_token()?);
        assert_eq!(term_of(&tokenizer), "bar");
        assert!(!tokenizer.increment_token()?);
        tokenizer.end()?;
        tokenizer.close()?;
        Ok(())
    }

    #[test]
    fn tokenizer_set_reader_without_close_fails() {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer
            .set_reader(Box::new(StringCharReader::new("a")))
            .unwrap();
        tokenizer.reset().unwrap();
        // Without close, set_reader must be rejected.
        let result = tokenizer.set_reader(Box::new(StringCharReader::new("b")));
        assert!(matches!(result, Err(LuceneError::IllegalState(_))));
    }

    #[test]
    fn token_stream_reflect_with_includes_attributes() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("lucene")))?;
        tokenizer.reset()?;
        assert!(tokenizer.increment_token()?);

        let mut entries: Vec<(String, String)> = Vec::new();
        tokenizer.reflect_with(
            &mut |_type_id: TypeId, _name: &'static str, key: &str, value: &dyn std::fmt::Debug| {
                entries.push((key.to_string(), format!("{:?}", value)));
            },
        );

        assert!(entries.iter().any(|(k, _)| k == "term"));
        assert!(entries.iter().any(|(k, _)| k == "positionIncrement"));
        Ok(())
    }

    #[test]
    fn default_factory_creates_packed_token_attribute() {
        let factory = default_token_attribute_factory();
        let mut source = AttributeSource::new_with_factory(factory);
        source.add_attribute::<PackedTokenAttributeImpl>().unwrap();

        assert!(source.has_attribute::<PackedTokenAttributeImpl>());
        assert!(source.has_attribute_by_id(TypeId::of::<dyn OffsetAttribute>()));
        assert!(source.has_attribute_by_id(TypeId::of::<dyn PositionIncrementAttribute>()));
        assert!(source.has_attribute_by_id(TypeId::of::<dyn PositionLengthAttribute>()));
        assert!(source.has_attribute_by_id(TypeId::of::<dyn TypeAttribute>()));

        let impl_count = source.attribute_impls_iter().count();
        assert_eq!(impl_count, 1);
    }

    // -------------------------------------------------------------------------
    // CharacterUtils and ReusableStringReader
    // -------------------------------------------------------------------------

    #[test]
    fn character_utils_to_lower_case_ascii_and_unicode() {
        let mut buf: Vec<char> = "HELLO Ω É".chars().collect();
        let len = buf.len();
        to_lower_case(&mut buf, 0, len);
        assert_eq!(buf.iter().collect::<String>(), "hello ω é");
    }

    #[test]
    fn character_utils_to_lower_case_preserves_tail() {
        let mut buf: Vec<char> = "AbCdefG".chars().collect();
        to_lower_case(&mut buf, 1, 4);
        assert_eq!(buf.iter().collect::<String>(), "AbcdefG");
    }

    #[test]
    fn reusable_string_reader_reset_with_new_value() {
        let mut reader = ReusableStringReader::new();
        reader.set_value("hello world");

        let mut buf = ['\0'; 5];
        assert_eq!(reader.read(&mut buf).unwrap(), 5);
        assert_eq!(buf.iter().collect::<String>(), "hello");

        reader.set_value("foo");
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        assert_eq!(buf[..3].iter().collect::<String>(), "foo");
    }

    // -------------------------------------------------------------------------
    // LowerCaseFilter
    // -------------------------------------------------------------------------

    #[test]
    fn lower_case_filter_ascii_and_unicode() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("HELLO Ω É ß")))?;
        let mut filter = new_lower_case_filter(Box::new(tokenizer));
        filter.reset()?;

        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "hello");
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "ω");
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "é");
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "ß");
        assert!(!filter.increment_token()?);

        filter.end()?;
        filter.close()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // FilteringTokenFilter
    // -------------------------------------------------------------------------

    #[derive(Debug)]
    struct RejectShortLogic {
        min_len: usize,
    }

    impl FilteringTokenFilterLogic for RejectShortLogic {
        fn accept(&mut self, source: &AttributeSource) -> Result<bool> {
            let len = source
                .get_attribute::<PackedTokenAttributeImpl>()
                .unwrap()
                .length();
            Ok(len >= self.min_len)
        }
    }

    #[test]
    fn filtering_token_filter_skips_rejected_tokens_and_adjusts_positions() -> Result<()> {
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("a big cat")))?;
        let mut filter =
            new_filtering_token_filter(Box::new(tokenizer), RejectShortLogic { min_len: 3 });
        filter.reset()?;

        // 'a' is rejected, 'big' is accepted with position increment 2.
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "big");
        assert_eq!(
            filter
                .attribute_source()
                .get_attribute::<PackedTokenAttributeImpl>()
                .unwrap()
                .get_position_increment(),
            2
        );

        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "cat");
        assert_eq!(
            filter
                .attribute_source()
                .get_attribute::<PackedTokenAttributeImpl>()
                .unwrap()
                .get_position_increment(),
            1
        );

        assert!(!filter.increment_token()?);
        filter.end()?;
        assert_eq!(
            filter
                .attribute_source()
                .get_attribute::<PackedTokenAttributeImpl>()
                .unwrap()
                .get_position_increment(),
            0
        );
        filter.close()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // StopFilter
    // -------------------------------------------------------------------------

    #[test]
    fn stop_filter_removes_configured_words() -> Result<()> {
        let stop_words = make_stop_set(["is", "a", "the"], false);
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("the quick is a fox")))?;
        let mut filter = new_stop_filter(Box::new(tokenizer), stop_words);
        filter.reset()?;

        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "quick");
        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "fox");
        assert!(!filter.increment_token()?);

        filter.end()?;
        filter.close()?;
        Ok(())
    }

    #[test]
    fn stop_filter_case_insensitive_stop_set() -> Result<()> {
        let stop_words = make_stop_set(["The"], true);
        let mut tokenizer = Tokenizer::new(WhitespaceTokenizer::default());
        tokenizer.set_reader(Box::new(StringCharReader::new("the cat")))?;
        let mut filter = new_stop_filter(Box::new(tokenizer), stop_words);
        filter.reset()?;

        assert!(filter.increment_token()?);
        assert_eq!(term_of(&filter), "cat");
        assert!(!filter.increment_token()?);

        filter.end()?;
        filter.close()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // WordlistLoader
    // -------------------------------------------------------------------------

    #[test]
    fn wordlist_loader_get_word_set_parses_lines() {
        let text = "  hello  \n\nworld\n  \n";
        let set = get_word_set(text.lines());
        assert_eq!(set.size(), 2);
        assert!(set.contains("hello"));
        assert!(set.contains("world"));
    }

    #[test]
    fn wordlist_loader_get_word_set_ignoring_comments() {
        let text = "alpha\n# comment\nbeta\n# another\ngamma";
        let set = get_word_set_ignoring_comments(text.lines(), "#");
        assert_eq!(set.size(), 3);
        assert!(set.contains("alpha"));
        assert!(set.contains("beta"));
        assert!(set.contains("gamma"));
    }

    #[test]
    fn wordlist_loader_get_snowball_word_set_parses_snowball_format() {
        let text = "a an the | English articles\nis are was were | verbs";
        let set = get_snowball_word_set(text.lines());
        assert_eq!(set.size(), 7);
        for word in ["a", "an", "the", "is", "are", "was", "were"] {
            assert!(set.contains(word));
        }
    }
}
