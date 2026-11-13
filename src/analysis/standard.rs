//! Standard analyzer and tokenizer ported from
//! `org.apache.lucene.analysis.standard`.
//!
//! [`StandardTokenizer`] runs the JFlex scanner of Lucene 10.5.0,
//! [`StandardTokenizerImpl`], whose DFA tables are generated from the Java
//! source by `tools/gen/standard_tokenizer_tables.py`.

#![deny(unsafe_code)]

mod standard_tokenizer_impl;
mod standard_tokenizer_impl_tables;

pub use standard_tokenizer_impl::{
    StandardTokenizerImpl, EMOJI_TYPE, HANGUL_TYPE, HIRAGANA_TYPE, IDEOGRAPHIC_TYPE, KATAKANA_TYPE,
    NUMERIC_TYPE, SOUTH_EAST_ASIAN_TYPE, WORD_TYPE, YYEOF, YYINITIAL,
};

use crate::util::atomic_ref_cell::AtomicRefCell;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::analysis::{
    get_buffered_reader, get_word_set, new_lower_case_filter, new_stop_filter, Analyzer,
    AnalyzerState, CharArraySet, CharReader, CharTermAttribute, GlobalReuseStrategy,
    OffsetAttribute, PositionIncrementAttribute, SharedTokenStream, StringCharReader, TokenStream,
    TokenStreamComponents, Tokenizer, TokenizerLogic, TypeAttribute,
};
use crate::error::{LuceneError, Result};
use crate::util::attribute::AttributeSource;
use crate::util::IOUtils;

/// Alpha/numeric token type.
pub const ALPHANUM: usize = 0;
/// Numeric token type.
pub const NUM: usize = 1;
/// Southeast Asian token type.
pub const SOUTHEAST_ASIAN: usize = 2;
/// Ideographic token type.
pub const IDEOGRAPHIC: usize = 3;
/// Hiragana token type.
pub const HIRAGANA: usize = 4;
/// Katakana token type.
pub const KATAKANA: usize = 5;
/// Hangul token type.
pub const HANGUL: usize = 6;
/// Emoji token type.
pub const EMOJI: usize = 7;

/// String token types that correspond to the integer constants above.
pub const TOKEN_TYPES: [&str; 8] = [
    "<ALPHANUM>",
    "<NUM>",
    "<SOUTHEAST_ASIAN>",
    "<IDEOGRAPHIC>",
    "<HIRAGANA>",
    "<KATAKANA>",
    "<HANGUL>",
    "<EMOJI>",
];

/// Absolute maximum token length supported by Lucene.
pub const MAX_TOKEN_LENGTH_LIMIT: usize = 1024 * 1024;

/// Default maximum token length, matching `StandardAnalyzer`.
pub const DEFAULT_MAX_TOKEN_LENGTH: usize = 255;

/// The body of [`StandardTokenizer`]: what Lucene's `StandardTokenizer` adds to
/// `Tokenizer`.
///
/// Equivalent to the members of `org.apache.lucene.analysis.standard.StandardTokenizer`;
/// the `Tokenizer` half is [`Tokenizer`](crate::analysis::Tokenizer).
#[derive(Debug)]
pub struct StandardTokenizerLogic {
    /// A private instance of the JFlex-constructed scanner.
    scanner: StandardTokenizerImpl,
    skipped_positions: i32,
    max_token_length: i32,
}

impl StandardTokenizerLogic {
    /// Creates the tokenizer logic with the default maximum token length.
    ///
    /// Equivalent to `StandardTokenizer.init()`, `new
    /// StandardTokenizerImpl(input)`.
    pub fn new() -> Self {
        Self {
            scanner: StandardTokenizerImpl::new(),
            skipped_positions: 0,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH as i32,
        }
    }

    /// Set the max allowed token length. Tokens larger than this will be
    /// chopped up at this token length and emitted as multiple tokens. If you
    /// need to skip such large tokens, you could increase this max length,
    /// and then use `LengthFilter` to remove long tokens. The default is
    /// [`DEFAULT_MAX_TOKEN_LENGTH`].
    ///
    /// Equivalent to `StandardTokenizer.setMaxTokenLength(int)`
    /// (`StandardTokenizer.java:95-105`): a new length resizes the scanner's
    /// buffer (`scanner.setBufferSize(length)`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) with Lucene's messages,
    /// `maxTokenLength must be greater than zero` and
    /// `maxTokenLength may not exceed 1048576`.
    pub fn set_max_token_length(&mut self, length: i32) -> Result<()> {
        if length < 1 {
            return Err(LuceneError::illegal_argument(
                "maxTokenLength must be greater than zero".to_string(),
            ));
        } else if length as usize > MAX_TOKEN_LENGTH_LIMIT {
            return Err(LuceneError::illegal_argument(format!(
                "maxTokenLength may not exceed {MAX_TOKEN_LENGTH_LIMIT}"
            )));
        }
        if length != self.max_token_length {
            self.max_token_length = length;
            self.scanner.set_buffer_size(length as usize);
        }
        Ok(())
    }

    /// Returns the current maximum token length.
    ///
    /// Equivalent to `StandardTokenizer.getMaxTokenLength()`.
    pub fn max_token_length(&self) -> i32 {
        self.max_token_length
    }
}

impl Default for StandardTokenizerLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenizerLogic for StandardTokenizerLogic {
    /// Equivalent to `StandardTokenizer.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        self.skipped_positions = 0;

        loop {
            let token_type = self.scanner.get_next_token(reader)?;

            if token_type == YYEOF {
                return Ok(false);
            }

            if self.scanner.yylength() <= self.max_token_length {
                interface_mut::<dyn PositionIncrementAttribute>(source)?
                    .set_position_increment(self.skipped_positions + 1);
                self.scanner
                    .get_text(&mut *interface_mut::<dyn CharTermAttribute>(source)?);
                let start = self.scanner.yychar();
                // `correctOffset(start + termAtt.length())`: the term's length
                // in UTF-16 code units, `yylength()`, as Java's `char` length
                // is; the attribute here counts characters.
                let input: &dyn CharReader = reader;
                let correct =
                    |offset: i32| Tokenizer::<Self>::correct_offset_of(Some(input), offset);
                interface_mut::<dyn OffsetAttribute>(source)?
                    .set_offset(correct(start), correct(start + self.scanner.yylength()));
                interface_mut::<dyn TypeAttribute>(source)?
                    .set_type_static(TOKEN_TYPES[token_type as usize]);
                return Ok(true);
            } else {
                // When we skip a too-long term, we still increment the
                // position increment
                self.skipped_positions += 1;
            }
        }
    }

    /// Equivalent to `StandardTokenizer.end()`; `Tokenizer::end` has run
    /// `super.end()` (`endAttributes()`).
    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        // set final offset
        let final_offset = Tokenizer::<Self>::correct_offset_of(
            input,
            self.scanner.yychar() + self.scanner.yylength(),
        );
        interface_mut::<dyn OffsetAttribute>(source)?.set_offset(final_offset, final_offset);
        // adjust any skipped tokens
        let mut pos_incr_att = interface_mut::<dyn PositionIncrementAttribute>(source)?;
        let current = pos_incr_att.get_position_increment();
        pos_incr_att.set_position_increment(current + self.skipped_positions);
        Ok(())
    }

    /// Equivalent to `StandardTokenizer.reset()`; `Tokenizer::reset` has run
    /// `super.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        self.skipped_positions = 0;
        Ok(())
    }

    /// Equivalent to `StandardTokenizer.close()`; `Tokenizer::close` has run
    /// `super.close()`.
    fn close(&mut self, _reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.yyreset();
        Ok(())
    }
}

/// A grammar-based tokenizer constructed with JFlex.
///
/// This class implements the Word Break rules from the Unicode Text
/// Segmentation algorithm, as specified in
/// [Unicode Standard Annex #29](http://unicode.org/reports/tr29/).
///
/// Equivalent to `org.apache.lucene.analysis.standard.StandardTokenizer`.
pub type StandardTokenizer = Tokenizer<StandardTokenizerLogic>;

/// The attribute interface `I` of a `StandardTokenizer`'s source, which
/// [`new_standard_tokenizer_with_factory`] added.
fn interface_mut<I: crate::util::attribute::AttributeInterface + ?Sized>(
    source: &AttributeSource,
) -> Result<crate::util::atomic_ref_cell::AtomicRefMut<'_, I>> {
    Ok(source
        .get_attribute_interface_mut::<I>()
        .expect("INVARIANT: new_standard_tokenizer_with_factory added the attribute"))
}

/// Creates a new `StandardTokenizer` with default settings.
///
/// Equivalent to `StandardTokenizer()`, whose attributes come from
/// `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY`.
pub fn new_standard_tokenizer() -> StandardTokenizer {
    new_standard_tokenizer_with_factory(crate::analysis::default_token_attribute_factory())
        .expect("INVARIANT: the default token attribute factory implements the core attributes")
}

/// Creates a new `StandardTokenizer` whose attributes come from `factory`.
///
/// Equivalent to `StandardTokenizer(AttributeFactory)`: the fields add the
/// `CharTermAttribute`, the `OffsetAttribute`, the
/// `PositionIncrementAttribute` and the `TypeAttribute`, in that order
/// (`StandardTokenizer.java:140-144`).
///
/// # Errors
///
/// What `factory` raises for an interface it cannot implement, as Java's
/// `addAttribute` throws.
pub fn new_standard_tokenizer_with_factory(
    factory: Arc<dyn crate::util::attribute::AttributeFactory>,
) -> Result<StandardTokenizer> {
    Ok(StandardTokenizer::with_source(
        standard_tokenizer_source(factory)?,
        StandardTokenizerLogic::new(),
    ))
}

/// The source of a `StandardTokenizer` over `factory`, holding the attributes
/// its fields add, in their declaration order (`StandardTokenizer.java:140-144`).
fn standard_tokenizer_source(
    factory: Arc<dyn crate::util::attribute::AttributeFactory>,
) -> Result<AttributeSource> {
    let mut source = AttributeSource::new_with_factory(factory);
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(source)
}

// -----------------------------------------------------------------------------
// StopwordAnalyzerBase
// -----------------------------------------------------------------------------

/// Base class for analyzers that use a stop-word set.
///
/// Equivalent to `org.apache.lucene.analysis.StopwordAnalyzerBase`.
#[derive(Debug)]
pub struct StopwordAnalyzerBase {
    /// An immutable stop-word set.
    stopwords: CharArraySet,
}

impl StopwordAnalyzerBase {
    /// Creates a base analyzer using the supplied stop-word set.
    ///
    /// The set is copied and then made unmodifiable, so the analyzer's view of
    /// it can neither be written to nor changed from underneath it.
    ///
    /// Equivalent to `org.apache.lucene.analysis.StopwordAnalyzerBase#StopwordAnalyzerBase(CharArraySet)`.
    pub fn new(stopwords: CharArraySet) -> Self {
        Self {
            stopwords: CharArraySet::unmodifiable_set(Some(stopwords.copy())),
        }
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to `org.apache.lucene.analysis.StopwordAnalyzerBase#getStopwordSet`.
    pub fn stopwords(&self) -> &CharArraySet {
        &self.stopwords
    }

    /// Reads a stop-word set from `reader`, closing it afterwards.
    ///
    /// Every non-blank line becomes one entry, trimmed of leading and trailing
    /// whitespace; the returned set is unmodifiable.
    ///
    /// # Errors
    ///
    /// Returns whatever the reader returns while being read or closed. A close
    /// failure is reported in preference to a read failure: Java closes the
    /// reader in a `finally` block, `IOUtils.close(stopwords)`
    /// (`StopwordAnalyzerBase.java:83-89`), and an exception thrown from a
    /// `finally` block replaces the one in flight.
    ///
    /// Equivalent to `org.apache.lucene.analysis.StopwordAnalyzerBase#loadStopwordSet(Reader)`.
    pub fn load_stopword_set(reader: &mut dyn CharReader) -> Result<CharArraySet> {
        let lines = get_buffered_reader(reader);
        IOUtils::close([Some(|| reader.close())])?;
        Ok(CharArraySet::unmodifiable_set(Some(get_word_set(lines?))))
    }

    /// Reads a stop-word set from the UTF-8 file at `stopwords`.
    ///
    /// Equivalent to the overload `StopwordAnalyzerBase.loadStopwordSet(Path)`.
    ///
    /// Divergence: Java streams the file through
    /// `Files.newBufferedReader(path, StandardCharsets.UTF_8)`
    /// (`StopwordAnalyzerBase.java` lines 70-74). This crate's
    /// [`CharReader`](crate::analysis::CharReader) has no file-backed
    /// implementation, so the port decodes the file into memory and reads it
    /// through a [`StringCharReader`](crate::analysis::StringCharReader). The
    /// resulting set is the same; only the peak memory differs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::Io`](crate::error::LuceneErrorKind::Io) when the file cannot be read or is not valid
    /// UTF-8.
    ///
    /// Equivalent to `org.apache.lucene.analysis.StopwordAnalyzerBase#loadStopwordSet(Path)`.
    pub fn load_stopword_set_from_path(stopwords: &Path) -> Result<CharArraySet> {
        let text = std::fs::read_to_string(stopwords)?;
        let mut reader = StringCharReader::new(text);
        Self::load_stopword_set(&mut reader)
    }
}

// -----------------------------------------------------------------------------
// StandardAnalyzer
// -----------------------------------------------------------------------------

/// Analyzer that tokenizes with the standard tokenizer, lowercases, and removes
/// stop words.
///
/// Equivalent to `org.apache.lucene.analysis.standard.StandardAnalyzer`, which
/// extends `StopwordAnalyzerBase`: the stop words live in the
/// [`StopwordAnalyzerBase`] this analyzer holds, as Java's live in the
/// inherited field.
#[derive(Debug)]
pub struct StandardAnalyzer {
    state: AnalyzerState,
    base: StopwordAnalyzerBase,
    /// Java's `maxTokenLength` field. The components' source reads it again on
    /// every reader, as Java's lambda reads `StandardAnalyzer.this.maxTokenLength`,
    /// so it is shared with them.
    max_token_length: Arc<AtomicI32>,
}

impl StandardAnalyzer {
    /// Creates a `StandardAnalyzer` with the default, empty stop-word set.
    ///
    /// Equivalent to `StandardAnalyzer()`, `this(CharArraySet.EMPTY_SET)`.
    pub fn new() -> Self {
        Self::with_stopwords(CharArraySet::new(0, false))
    }

    /// Creates a `StandardAnalyzer` with the supplied stop-word set.
    ///
    /// Equivalent to `StandardAnalyzer(CharArraySet)`, `super(stopWords)`:
    /// [`StopwordAnalyzerBase::new`] keeps an unmodifiable copy of the set.
    pub fn with_stopwords(stopwords: CharArraySet) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            base: StopwordAnalyzerBase::new(stopwords),
            max_token_length: Arc::new(AtomicI32::new(DEFAULT_MAX_TOKEN_LENGTH as i32)),
        }
    }

    /// Sets the maximum token length.
    ///
    /// Equivalent to `StandardAnalyzer.setMaxTokenLength(int)`, which only
    /// stores the value: it is checked when the tokenizer receives it, so an
    /// out-of-range length surfaces as the error of the next
    /// [`token_stream`](Analyzer::token_stream) — from `create_components`
    /// when no components exist yet, from their source otherwise — and
    /// components created earlier pick the new length up on their next
    /// reader.
    pub fn set_max_token_length(&mut self, length: i32) {
        self.max_token_length.store(length, Ordering::Relaxed);
    }

    /// Returns the current maximum token length.
    ///
    /// Equivalent to `StandardAnalyzer.getMaxTokenLength()`.
    pub fn max_token_length(&self) -> i32 {
        self.max_token_length.load(Ordering::Relaxed)
    }

    /// Returns the stop-word set used by this analyzer.
    ///
    /// Equivalent to `StopwordAnalyzerBase.getStopwordSet()`, which returns
    /// the inherited `stopwords` field.
    pub fn stopwords(&self) -> &CharArraySet {
        self.base.stopwords()
    }
}

impl Default for StandardAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for StandardAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.standard.StandardAnalyzer"
    }

    /// Equivalent to `StandardAnalyzer.createComponents(String)`
    /// (`StandardAnalyzer.java:84-96`): the tokenizer receives the analyzer's
    /// maximum token length, which fails for an out-of-range length before
    /// the components exist, and the source applies the analyzer's current
    /// length again before every reader, as Java's
    /// `src.setMaxTokenLength(StandardAnalyzer.this.maxTokenLength)` lambda
    /// does.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let max_token_length = Arc::clone(&self.max_token_length);
        tokenizer
            .borrow_mut()
            .logic_mut()
            .set_max_token_length(max_token_length.load(Ordering::Relaxed))?;
        let tokenizer_for_source = Arc::clone(&tokenizer);
        let source = Box::new(move |reader: Box<dyn CharReader>| {
            let mut tokenizer = tokenizer_for_source.borrow_mut();
            tokenizer
                .logic_mut()
                .set_max_token_length(max_token_length.load(Ordering::Relaxed))?;
            tokenizer.set_reader(reader)
        });
        let shared = SharedTokenStream::new(tokenizer);
        let lower = new_lower_case_filter(Box::new(shared))?;
        let stop = new_stop_filter(Box::new(lower), self.base.stopwords().clone())?;
        let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(stop));
        Ok(TokenStreamComponents::new(source, sink))
    }

    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `Analyzer.normalize` hands this a `StringTokenStream`, which holds
        // the `CharTermAttribute` already, so the filter's `addAttribute`
        // finds it and cannot fail.
        Box::new(
            new_lower_case_filter(input)
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    /// `CharTermAttribute.toString()`: the term text.
    trait TermText {
        fn term(&self) -> String;
    }

    impl<T: CharTermAttribute + ?Sized> TermText for T {
        fn term(&self) -> String {
            self.buffer()[..self.length()].iter().collect()
        }
    }
    use crate::analysis::{
        OffsetAttribute, PositionIncrementAttribute, ReusableStringReader, TokenStream,
        TypeAttribute,
    };

    /// `loadStopwordSet(Reader)` closes the reader in a `finally` block,
    /// `IOUtils.close(stopwords)` (`StopwordAnalyzerBase.java:83-89`), whose
    /// failure replaces the read failure — Java oracle
    /// (`scratchpad/oracle/e2/StopwordOracle.java`): `threw close failed`.
    #[test]
    fn load_stopword_set_reports_the_close_failure_over_the_read_failure() {
        #[derive(Debug)]
        struct FailingReader {
            closes: usize,
        }
        impl crate::analysis::CharReader for FailingReader {
            fn read(&mut self, _buf: &mut [u16]) -> Result<usize> {
                Err(LuceneError::from(LuceneErrorKind::Io(
                    std::io::Error::other("read failed"),
                )))
            }
            fn close(&mut self) -> Result<()> {
                self.closes += 1;
                Err(LuceneError::from(LuceneErrorKind::Io(
                    std::io::Error::other("close failed"),
                )))
            }
        }
        let mut reader = FailingReader { closes: 0 };
        let error = StopwordAnalyzerBase::load_stopword_set(&mut reader).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Io(e), .. } if e.to_string() == "close failed"),
            "{error:?}"
        );
        assert_eq!(reader.closes, 1);
    }

    fn tokenize(text: &str) -> Vec<(String, i32, i32, String)> {
        let mut tokenizer = new_standard_tokenizer();
        let mut reader = ReusableStringReader::new();
        reader.set_value(text);
        tokenizer.set_reader(Box::new(reader)).unwrap();
        tokenizer.reset().unwrap();

        let mut tokens = Vec::new();
        while tokenizer.increment_token().unwrap() {
            let source = tokenizer.attribute_source();
            let term = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap()
                .term();
            let start = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .unwrap()
                .start_offset();
            let end = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .unwrap()
                .end_offset();
            let type_value = source
                .get_attribute_interface::<dyn TypeAttribute>()
                .unwrap()
                .type_value()
                .to_string();
            tokens.push((term, start, end, type_value));
        }
        tokens
    }

    #[test]
    fn tokenizes_ascii_text() {
        let tokens = tokenize("Hello, world! This is Rucene.");
        assert_eq!(
            tokens,
            vec![
                ("Hello".to_string(), 0, 5, "<ALPHANUM>".to_string()),
                ("world".to_string(), 7, 12, "<ALPHANUM>".to_string()),
                ("This".to_string(), 14, 18, "<ALPHANUM>".to_string()),
                ("is".to_string(), 19, 21, "<ALPHANUM>".to_string()),
                ("Rucene".to_string(), 22, 28, "<ALPHANUM>".to_string()),
            ]
        );
    }

    /// UAX #29 rule WB12 keeps `123.45` whole (`MidNumLet` between digits),
    /// as Lucene 10.5.0's `StandardTokenizer` does: `Price 0 5 <ALPHANUM>`,
    /// `123.45 8 14 <NUM>`.
    #[test]
    fn tokenizes_numbers() {
        let tokens = tokenize("Price: $123.45");
        assert_eq!(
            tokens,
            vec![
                ("Price".to_string(), 0, 5, "<ALPHANUM>".to_string()),
                ("123.45".to_string(), 8, 14, "<NUM>".to_string()),
            ]
        );
    }

    #[test]
    fn tokenizes_cjk() {
        let tokens = tokenize("日本語テスト");
        assert_eq!(
            tokens,
            vec![
                ("日".to_string(), 0, 1, "<IDEOGRAPHIC>".to_string()),
                ("本".to_string(), 1, 2, "<IDEOGRAPHIC>".to_string()),
                ("語".to_string(), 2, 3, "<IDEOGRAPHIC>".to_string()),
                ("テスト".to_string(), 3, 6, "<KATAKANA>".to_string()),
            ]
        );
    }

    #[test]
    fn tokenizes_hangul() {
        let tokens = tokenize("한글테스트");
        assert_eq!(
            tokens,
            vec![("한글테스트".to_string(), 0, 5, "<HANGUL>".to_string()),]
        );
    }

    #[test]
    fn max_token_length_chops_token() {
        let mut tokenizer = new_standard_tokenizer();
        tokenizer.logic_mut().set_max_token_length(3).unwrap();
        let mut reader = ReusableStringReader::new();
        reader.set_value("abcdefg");
        tokenizer.set_reader(Box::new(reader)).unwrap();
        tokenizer.reset().unwrap();

        let mut terms = Vec::new();
        while tokenizer.increment_token().unwrap() {
            let source = tokenizer.attribute_source();
            let term = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap()
                .term();
            let pos = source
                .get_attribute_interface::<dyn PositionIncrementAttribute>()
                .unwrap()
                .get_position_increment();
            terms.push((term, pos));
        }
        assert_eq!(
            terms,
            vec![
                ("abc".to_string(), 1),
                ("def".to_string(), 1),
                ("g".to_string(), 1),
            ]
        );
    }

    #[test]
    fn end_sets_final_offset() {
        let mut tokenizer = new_standard_tokenizer();
        let mut reader = ReusableStringReader::new();
        reader.set_value("hello world");
        tokenizer.set_reader(Box::new(reader)).unwrap();
        tokenizer.reset().unwrap();
        while tokenizer.increment_token().unwrap() {}
        tokenizer.end().unwrap();

        let source = tokenizer.attribute_source();
        let att = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap();
        assert_eq!(att.start_offset(), 11);
        assert_eq!(att.end_offset(), 11);
    }

    /// After `end()` every attribute holds its `end()` value: Java's
    /// `PackedTokenAttributeImpl.end()` clears the term and the type and
    /// sets the position increment to `0`, then `StandardTokenizer.end()`
    /// sets the final offset and adds the skipped positions (oracle row
    /// `core 0 standard.end` of `analysis_mocks.tsv`: `end:-,11,11,0,1,word`).
    #[test]
    fn end_leaves_the_attributes_of_javas_end() {
        let mut tokenizer = new_standard_tokenizer();
        tokenizer
            .set_reader(Box::new(crate::analysis::StringCharReader::new(
                "hello world",
            )))
            .unwrap();
        tokenizer.reset().unwrap();
        while tokenizer.increment_token().unwrap() {}
        {
            // a consumer's leftovers, which end() must clear as Java's does
            let source = tokenizer.attribute_source();
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .unwrap()
                .append_string(Some("leftover"));
            source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .unwrap()
                .set_type("<ALPHANUM>".to_string());
            source
                .get_attribute_interface_mut::<dyn crate::analysis::PositionLengthAttribute>()
                .unwrap()
                .set_position_length(3);
        }
        tokenizer.end().unwrap();
        let source = tokenizer.attribute_source();
        let term = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        let type_att = source
            .get_attribute_interface::<dyn TypeAttribute>()
            .unwrap();
        let offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap();
        let pos_inc = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap();
        let pos_len = source
            .get_attribute_interface::<dyn crate::analysis::PositionLengthAttribute>()
            .unwrap();
        assert_eq!(term.term(), "");
        assert_eq!(type_att.type_value(), "word");
        assert_eq!((offset.start_offset(), offset.end_offset()), (11, 11));
        assert_eq!(pos_inc.get_position_increment(), 0);
        assert_eq!(pos_len.get_position_length(), 1);
    }

    fn analyze(analyzer: &dyn Analyzer, text: &str) -> Vec<String> {
        let stream = analyzer.token_stream_from_str("field", text).unwrap();
        let mut stream = stream.borrow_mut();
        stream.reset().unwrap();
        let mut terms = Vec::new();
        while stream.increment_token().unwrap() {
            let term = stream
                .attribute_source()
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap()
                .term();
            terms.push(term);
        }
        stream.end().unwrap();
        terms
    }

    #[test]
    fn standard_analyzer_lowercases_text() {
        let analyzer = StandardAnalyzer::new();
        let terms = analyze(&analyzer, "Hello, World!");
        assert_eq!(terms, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn standard_analyzer_default_has_no_stopwords() {
        let analyzer = StandardAnalyzer::new();
        let terms = analyze(&analyzer, "The quick brown fox");
        assert_eq!(
            terms,
            vec![
                "the".to_string(),
                "quick".to_string(),
                "brown".to_string(),
                "fox".to_string(),
            ]
        );
    }

    #[test]
    fn standard_analyzer_removes_configured_stopwords() {
        let stopwords = CharArraySet::from_iter(["the", "a", "is"], false);
        let analyzer = StandardAnalyzer::with_stopwords(stopwords);
        let terms = analyze(&analyzer, "The quick brown fox is running");
        assert_eq!(
            terms,
            vec![
                "quick".to_string(),
                "brown".to_string(),
                "fox".to_string(),
                "running".to_string(),
            ]
        );
    }

    #[test]
    fn standard_analyzer_respects_max_token_length() {
        let mut analyzer = StandardAnalyzer::new();
        analyzer.set_max_token_length(3);
        let terms = analyze(&analyzer, "abcdefg");
        assert_eq!(
            terms,
            vec!["abc".to_string(), "def".to_string(), "g".to_string()]
        );
    }

    /// Consumes one token stream of `analyzer` over `text` to the end and
    /// closes it, as Java's try-with-resources does, rendering each token as
    /// `term(start,end)` and the end state as `end(start,end)`.
    fn tokens(analyzer: &dyn Analyzer, text: &str) -> Result<Vec<String>> {
        let stream = analyzer.token_stream_from_str("f", text)?;
        let mut stream = stream.borrow_mut();
        let render = |stream: &dyn TokenStream, term: Option<String>| {
            let source = stream.attribute_source();
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap();
            let att = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .unwrap();
            format!(
                "{}({},{})",
                term.unwrap_or_else(|| term_att.term()),
                att.start_offset(),
                att.end_offset()
            )
        };
        stream.reset()?;
        let mut out = Vec::new();
        while stream.increment_token()? {
            out.push(render(&*stream, None));
        }
        stream.end()?;
        out.push(render(&*stream, Some("end".to_string())));
        stream.close()?;
        Ok(out)
    }

    /// `MiscOracle` O4 (`oracle/h2/MiscOracle.java`): `setMaxTokenLength`
    /// only stores the length; the reused components apply it on their next
    /// reader, and an out-of-range length fails the next `tokenStream` with
    /// the tokenizer's message, until a valid one is set again.
    #[test]
    fn the_analyzer_applies_its_max_token_length_on_every_reader() {
        let mut analyzer = StandardAnalyzer::new();
        assert_eq!(
            tokens(&analyzer, "abcdefgh ij").unwrap(),
            ["abcdefgh(0,8)", "ij(9,11)", "end(11,11)"]
        );
        analyzer.set_max_token_length(3);
        assert_eq!(
            tokens(&analyzer, "abcdefgh ij").unwrap(),
            ["abc(0,3)", "def(3,6)", "gh(6,8)", "ij(9,11)", "end(11,11)"]
        );
        analyzer.set_max_token_length(0);
        assert_eq!(analyzer.max_token_length(), 0);
        match tokens(&analyzer, "abcdefgh ij") {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "maxTokenLength must be greater than zero")
            }
            other => panic!("expected the tokenizer's refusal, got {other:?}"),
        }
        analyzer.set_max_token_length(4);
        assert_eq!(
            tokens(&analyzer, "abcdefgh ij").unwrap(),
            ["abcd(0,4)", "efgh(4,8)", "ij(9,11)", "end(11,11)"]
        );
    }

    /// `StandardAnalyzer.createComponents` gives the tokenizer the analyzer's
    /// length before the components exist, so an out-of-range length fails
    /// `tokenStream` with no components stored for reuse. Java oracle
    /// (`scratchpad/auditH/oracle/AuditHExtra.java`, `extra_out.txt`):
    /// `tokenStream java.lang.IllegalArgumentException: maxTokenLength must be
    /// greater than zero`, `components after failure=null`, then `after fix
    /// tokens=[ab, cd, e] components=true`.
    #[test]
    fn an_out_of_range_length_fails_before_the_components_are_stored() {
        let mut analyzer = StandardAnalyzer::new();
        analyzer.set_max_token_length(0);
        match tokens(&analyzer, "abc") {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "maxTokenLength must be greater than zero")
            }
            other => panic!("expected the tokenizer's refusal, got {other:?}"),
        }
        let state = analyzer.analyzer_state();
        assert!(state
            .reuse_strategy()
            .get_reusable_components(state, "f")
            .is_none());
        analyzer.set_max_token_length(2);
        assert_eq!(
            tokens(&analyzer, "abcde").unwrap(),
            ["ab(0,2)", "cd(2,4)", "e(4,5)", "end(5,5)"]
        );
        let state = analyzer.analyzer_state();
        assert!(state
            .reuse_strategy()
            .get_reusable_components(state, "f")
            .is_some());
    }

    /// `MiscOracle` O4: the analyzer's stop words are the
    /// `StopwordAnalyzerBase`'s unmodifiable copy — Java's
    /// `getStopwordSet().add("x")` threw `UnsupportedOperationException` —
    /// and they remove their words from the stream.
    #[test]
    fn the_analyzer_keeps_an_unmodifiable_copy_of_its_stop_words() {
        let mut stop = CharArraySet::new(4, false);
        stop.add("the");
        let analyzer = StandardAnalyzer::with_stopwords(stop);
        assert_eq!(analyzer.stopwords().to_string(), "[the]");
        assert_eq!(
            tokens(&analyzer, "the quick fox").unwrap(),
            ["quick(4,9)", "fox(10,13)", "end(13,13)"]
        );
        let mut stopwords = analyzer.stopwords().clone();
        let added = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            stopwords.add("x");
        }));
        assert!(added.is_err(), "the stop-word set refuses writes");
        assert_eq!(StandardAnalyzer::new().stopwords().to_string(), "[]");
    }

    /// `MiscOracle` O5: the tokenizer reports offsets in the text the user
    /// supplied, corrected through every char filter below it
    /// (`correctOffset` in `incrementToken` and `end`); two doubling filters
    /// gave `[hello(0,20), world(24,44), end(44,44)]`.
    #[test]
    fn offsets_are_corrected_through_the_char_filters() {
        let doubling = |inner: Box<dyn CharReader>| -> Box<dyn CharReader> {
            Box::new(crate::analysis::CharFilterFn::new(inner, |off: i32| {
                off * 2
            }))
        };
        let mut tokenizer = new_standard_tokenizer();
        tokenizer
            .set_reader(doubling(doubling(Box::new(StringCharReader::new(
                "hello world",
            )))))
            .unwrap();
        tokenizer.reset().unwrap();
        let render = |tokenizer: &StandardTokenizer, term: Option<&str>| {
            let source = tokenizer.attribute_source();
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap();
            let att = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .unwrap();
            format!(
                "{}({},{})",
                term.map_or_else(|| term_att.term(), str::to_string),
                att.start_offset(),
                att.end_offset()
            )
        };
        let mut out = Vec::new();
        while tokenizer.increment_token().unwrap() {
            out.push(render(&tokenizer, None));
        }
        tokenizer.end().unwrap();
        out.push(render(&tokenizer, Some("end")));
        assert_eq!(out, ["hello(0,20)", "world(24,44)", "end(44,44)"]);
    }
}

// -----------------------------------------------------------------------------
// StandardTokenizerFactory
// -----------------------------------------------------------------------------

/// Builds a [`StandardTokenizer`] from a configuration.
///
/// Equivalent to
/// `org.apache.lucene.analysis.standard.StandardTokenizerFactory`.
#[derive(Clone, Debug)]
pub struct StandardTokenizerFactory {
    /// Java's private `maxTokenLength`, read by [`create`](crate::analysis::factories::TokenizerFactory::create).
    max_token_length: i32,
}

/// SPI name this factory is registered under.
///
/// Equivalent to `StandardTokenizerFactory.NAME`.
pub const STANDARD_TOKENIZER_FACTORY_NAME: &str = "standard";

impl StandardTokenizerFactory {
    /// Builds the factory from `args`, consuming every argument it recognises.
    ///
    /// Equivalent to `StandardTokenizerFactory(Map<String, String>)`, including
    /// its refusal of any argument left over, `Unknown parameters: ` followed
    /// by the map's `toString()`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for a leftover argument.
    ///
    /// **Divergence from Lucene 10.5.0.** Java prints the caller's `HashMap`,
    /// in that map's iteration order. The leftovers are rendered here in the
    /// order of a `java.util.HashMap` holding them alone, filled in key order:
    /// the same order whenever the caller's map had the default capacity and
    /// no two leftover keys share a bucket.
    ///
    /// [`AbstractAnalysisFactory::new`]: crate::analysis::factories::AbstractAnalysisFactory::new
    /// [`AbstractAnalysisFactory::get_int`]: crate::analysis::factories::AbstractAnalysisFactory::get_int
    pub fn new(args: &mut std::collections::HashMap<String, String>) -> crate::error::Result<Self> {
        use crate::analysis::factories::AbstractAnalysisFactory;
        AbstractAnalysisFactory::new(args)?;
        let max_token_length = AbstractAnalysisFactory::get_int(
            args,
            "maxTokenLength",
            DEFAULT_MAX_TOKEN_LENGTH as i32,
        )?;
        if !args.is_empty() {
            let mut leftovers: Vec<(&String, &String)> = args.iter().collect();
            leftovers.sort();
            let mut map = crate::util::java_collections::JavaHashMap::new();
            for (key, value) in leftovers {
                map.insert(key.clone(), value.clone());
            }
            let rendered = map
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(crate::error::LuceneError::illegal_argument(format!(
                "Unknown parameters: {{{rendered}}}"
            )));
        }
        Ok(Self { max_token_length })
    }

    /// Refuses to build the factory without arguments.
    ///
    /// Equivalent to the public no-argument `StandardTokenizerFactory()`
    /// constructor, which exists for `ServiceLoader` and throws
    /// `defaultCtorException()`.
    ///
    /// # Errors
    ///
    /// Always returns [`AbstractAnalysisFactory::default_ctor_exception`](crate::analysis::factories::AbstractAnalysisFactory::default_ctor_exception).
    pub fn new_without_args() -> crate::error::Result<Self> {
        Err(crate::analysis::factories::AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The factory's class, as `lucene-core`'s
    /// `META-INF/services/org.apache.lucene.analysis.TokenizerFactory` names
    /// it: `StandardTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> crate::analysis::factories::AnalysisFactoryClass<
        dyn crate::analysis::factories::TokenizerFactory,
    > {
        crate::analysis::factories::AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.standard.StandardTokenizerFactory",
            crate::analysis::factories::SPINameField::Declared(STANDARD_TOKENIZER_FACTORY_NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(
        args: &mut std::collections::HashMap<String, String>,
    ) -> crate::error::Result<Arc<dyn crate::analysis::factories::TokenizerFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl crate::analysis::factories::TokenizerFactory for StandardTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.standard.StandardTokenizerFactory"
    }

    /// Equivalent to `StandardTokenizerFactory.create(AttributeFactory)`: a
    /// `new StandardTokenizer(factory)` whose maximum token length is set.
    fn create_with_factory(
        &self,
        factory: Arc<dyn crate::util::attribute::AttributeFactory>,
    ) -> crate::error::Result<crate::analysis::factories::FactoryTokenizer> {
        let mut logic = StandardTokenizerLogic::new();
        logic.set_max_token_length(self.max_token_length)?;
        Ok(Tokenizer::with_source(
            standard_tokenizer_source(factory)?,
            Box::new(logic),
        ))
    }
}
