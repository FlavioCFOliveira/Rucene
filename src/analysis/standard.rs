//! Standard analyzer and tokenizer ported from
//! `org.apache.lucene.analysis.standard`.
//!
//! The implementation follows the hybrid strategy decided in
//! `specs/modules/standard-tokenizer-strategy.md`: word boundaries are obtained
//! from the `unicode-segmentation` crate (UAX #29) and each segment is then
//! classified into one of the Lucene token types.

#![deny(unsafe_code)]

use std::fmt::Debug;

use unicode_segmentation::UnicodeSegmentation;

use std::cell::RefCell;
use std::rc::Rc;

use crate::analysis::{
    new_lower_case_filter, new_stop_filter, Analyzer, AnalyzerState, CharArraySet, CharReader,
    CharTermAttribute, GlobalReuseStrategy, OffsetAttribute, PositionIncrementAttribute,
    SharedTokenStream, TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
    TypeAttribute,
};
use crate::error::{LuceneError, Result};
use crate::util::attribute::{AttributeImpl, AttributeSource};

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

/// Internal scanner logic for the standard tokenizer.
///
/// Equivalent to `org.apache.lucene.analysis.standard.StandardTokenizerImpl`.
#[derive(Debug)]
pub struct StandardTokenizerImpl {
    pub(crate) text: Vec<char>,
    /// Word boundary offsets in character indices.
    word_bounds: Vec<usize>,
    current_word: usize,
    max_token_length: usize,
    /// Number of positions skipped because of over-long tokens.
    skipped_positions: i32,
}

impl StandardTokenizerImpl {
    /// Creates a new scanner with the default buffer size.
    pub fn new() -> Self {
        Self {
            text: Vec::new(),
            word_bounds: Vec::new(),
            current_word: 0,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH,
            skipped_positions: 0,
        }
    }

    /// Sets the maximum token length.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `length` is outside
    /// `[1, MAX_TOKEN_LENGTH_LIMIT]`.
    pub fn set_max_token_length(&mut self, length: usize) -> Result<()> {
        if length < 1 {
            return Err(LuceneError::IllegalArgument(
                "maxTokenLength must be greater than zero".to_string(),
            ));
        }
        if length > MAX_TOKEN_LENGTH_LIMIT {
            return Err(LuceneError::IllegalArgument(format!(
                "maxTokenLength may not exceed {MAX_TOKEN_LENGTH_LIMIT}"
            )));
        }
        self.max_token_length = length;
        Ok(())
    }

    /// Returns the current maximum token length.
    pub fn max_token_length(&self) -> usize {
        self.max_token_length
    }

    /// Resets the scanner with text read from `reader`.
    pub fn reset(&mut self, reader: &mut dyn CharReader) -> Result<()> {
        self.text.clear();
        self.word_bounds.clear();
        self.current_word = 0;
        self.skipped_positions = 0;

        let mut buf = vec!['\0'; 1024];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            self.text.extend_from_slice(&buf[..n]);
        }

        let s: String = self.text.iter().collect();
        let mut valid_bounds = Vec::new();
        for (byte_idx, segment) in s.split_word_bound_indices() {
            let seg_char_start = s[..byte_idx].chars().count();
            let seg_chars: Vec<char> = segment.chars().collect();
            let mut token_start: Option<usize> = None;
            for (i, &c) in seg_chars.iter().enumerate() {
                let global = seg_char_start + i;
                let is_word = is_word_char(c);
                if is_word && token_start.is_none() {
                    token_start = Some(global);
                } else if !is_word && token_start.is_some() {
                    valid_bounds.push(token_start.unwrap());
                    valid_bounds.push(global);
                    token_start = None;
                }
            }
            if let Some(start) = token_start {
                valid_bounds.push(start);
                valid_bounds.push(seg_char_start + seg_chars.len());
            }
        }
        self.word_bounds = valid_bounds;

        Ok(())
    }

    /// Returns the number of characters processed so far.
    pub fn yychar(&self) -> i32 {
        if self.current_word == 0 || self.current_word < 2 {
            0
        } else {
            self.word_bounds[self.current_word - 2] as i32
        }
    }

    /// Returns the length of the current token in characters.
    pub fn yylength(&self) -> usize {
        if self.current_word < 2 {
            0
        } else {
            self.word_bounds[self.current_word - 1] - self.word_bounds[self.current_word - 2]
        }
    }

    /// Advances to the next token and returns its type index, or `None` at
    /// end-of-stream.
    pub fn get_next_token(&mut self) -> Option<usize> {
        while self.current_word + 1 < self.word_bounds.len() {
            let start = self.word_bounds[self.current_word];
            let end = self.word_bounds[self.current_word + 1];
            self.current_word += 2;
            if start == end {
                continue;
            }
            let token_type = classify_token(&self.text[start..end]);
            return Some(token_type);
        }
        None
    }

    /// Copies the current token text into `term_att`.
    pub fn get_text(&self, term_att: &mut dyn CharTermAttribute) {
        if self.current_word < 2 {
            term_att.set_empty();
            return;
        }
        let start = self.word_bounds[self.current_word - 2];
        let end = self.word_bounds[self.current_word - 1];
        term_att.copy_buffer(&self.text, start, end - start);
    }

    /// Returns the current skipped-position count.
    pub fn skipped_positions(&self) -> i32 {
        self.skipped_positions
    }

    /// Resets the skipped-position counter.
    pub fn clear_skipped_positions(&mut self) {
        self.skipped_positions = 0;
    }
}

impl Default for StandardTokenizerImpl {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true if the character slice contains at least one word-forming
/// character (letter, digit, ideograph, hangul, kana, emoji, or complex-script
/// character).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || is_cjk_ideograph(c)
        || is_hangul(c)
        || is_hiragana(c)
        || is_katakana(c)
        || is_southeast_asian(c)
        || is_emoji_start(c)
}

/// Classifies a character slice into a Lucene standard token type.
fn classify_token(chars: &[char]) -> usize {
    if chars.is_empty() {
        return ALPHANUM;
    }

    let first = chars[0];

    // Emoji heuristic: starts with an emoji/extended pictographic character.
    if is_emoji_start(first) {
        return EMOJI;
    }

    let mut has_letter = false;
    let mut has_digit = false;
    let mut all_hiragana = !chars.is_empty();
    let mut all_katakana = !chars.is_empty();
    let mut all_hangul = !chars.is_empty();

    for &c in chars {
        if c.is_ascii_alphabetic() {
            has_letter = true;
        }
        if c.is_ascii_digit() || c.is_numeric() {
            has_digit = true;
        }
        if !is_hiragana(c) {
            all_hiragana = false;
        }
        if !is_katakana(c) {
            all_katakana = false;
        }
        if !is_hangul(c) {
            all_hangul = false;
        }
    }

    if all_hiragana {
        return HIRAGANA;
    }
    if all_katakana {
        return KATAKANA;
    }
    if all_hangul {
        return HANGUL;
    }

    // Single CJK ideograph.
    if chars.len() == 1 && is_cjk_ideograph(first) {
        return IDEOGRAPHIC;
    }

    // Complex-context scripts (Thai, Lao, Myanmar, Khmer).
    if chars.iter().all(|&c| is_southeast_asian(c)) {
        return SOUTHEAST_ASIAN;
    }

    if has_digit && !has_letter {
        return NUM;
    }

    ALPHANUM
}

fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{309F}')
}

fn is_katakana(c: char) -> bool {
    matches!(c, '\u{30A0}'..='\u{30FF}')
}

fn is_hangul(c: char) -> bool {
    matches!(c,
        '\u{AC00}'..='\u{D7AF}' | // Hangul Syllables
        '\u{1100}'..='\u{11FF}' | // Hangul Jamo
        '\u{3130}'..='\u{318F}' | // Hangul Compatibility Jamo
        '\u{A960}'..='\u{A97F}' | // Hangul Jamo Extended-A
        '\u{D7B0}'..='\u{D7FF}'    // Hangul Jamo Extended-B
    )
}

fn is_cjk_ideograph(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}' |   // CJK Extension A
        '\u{20000}'..='\u{2A6DF}' | // CJK Extension B
        '\u{2A700}'..='\u{2B73F}' | // CJK Extension C
        '\u{2B740}'..='\u{2B81F}' | // CJK Extension D
        '\u{2B820}'..='\u{2CEAF}' | // CJK Extension E
        '\u{2CEB0}'..='\u{2EBEF}' | // CJK Extension F
        '\u{30000}'..='\u{3134F}' | // CJK Extension G
        '\u{31350}'..='\u{323AF}'    // CJK Extension H
    )
}

fn is_southeast_asian(c: char) -> bool {
    matches!(c,
        '\u{0E00}'..='\u{0E7F}' | // Thai
        '\u{0E80}'..='\u{0EFF}' | // Lao
        '\u{1000}'..='\u{109F}' | // Myanmar
        '\u{1780}'..='\u{17FF}' | // Khmer
        '\u{19E0}'..='\u{19FF}'    // Khmer Symbols
    )
}

fn is_emoji_start(c: char) -> bool {
    // Emoji ranges are intentionally conservative. This matches many common
    // emoji and pictographic characters.
    matches!(c,
        '\u{1F600}'..='\u{1F64F}' | // Emoticons
        '\u{1F300}'..='\u{1F5FF}' | // Misc symbols and pictographs
        '\u{1F680}'..='\u{1F6FF}' | // Transport and map symbols
        '\u{1F1E0}'..='\u{1F1FF}' | // Regional indicator symbols
        '\u{1F900}'..='\u{1F9FF}' | // Supplemental symbols and pictographs
        '\u{2600}'..='\u{26FF}' |   // Misc symbols
        '\u{2700}'..='\u{27BF}' |   // Dingbats
        '\u{1F018}'..='\u{1F270}' | // Chess/checker pieces, etc.
        '\u{2B50}'..='\u{2B55}'
    )
}

/// State for a token that is being chopped because it exceeds the maximum
/// length. `(token_start, token_end, token_type, next_piece_start)`.
type LongTokenState = (usize, usize, usize, usize);

/// User-supplied logic for [`StandardTokenizer`].
#[derive(Debug)]
pub struct StandardTokenizerLogic {
    scanner: StandardTokenizerImpl,
    skipped_positions: i32,
    long_token: Option<LongTokenState>,
}

impl StandardTokenizerLogic {
    /// Creates the tokenizer logic with the default maximum token length.
    pub fn new() -> Self {
        Self {
            scanner: StandardTokenizerImpl::new(),
            skipped_positions: 0,
            long_token: None,
        }
    }

    /// Sets the maximum token length.
    pub fn set_max_token_length(&mut self, length: usize) -> Result<()> {
        self.scanner.set_max_token_length(length)
    }

    /// Returns the current maximum token length.
    pub fn max_token_length(&self) -> usize {
        self.scanner.max_token_length()
    }
}

impl Default for StandardTokenizerLogic {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenizerLogic for StandardTokenizerLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        let mut att = source
            .get_attribute_mut::<crate::analysis::PackedTokenAttributeImpl>()
            .ok_or_else(|| LuceneError::IllegalState("missing PackedTokenAttribute".to_string()))?;

        att.clear();

        // Continue chopping a previously discovered long token.
        if let Some((token_start, token_end, token_type, mut next_start)) = self.long_token {
            let max_len = self.scanner.max_token_length();
            let piece_end = (next_start + max_len).min(token_end);
            let length = (piece_end - next_start) as i32;
            att.copy_buffer(&self.scanner.text, next_start, piece_end - next_start);
            att.set_position_increment(1);
            att.set_offset(next_start as i32, next_start as i32 + length);
            att.set_type(TOKEN_TYPES[token_type].to_string());
            next_start = piece_end;
            if next_start >= token_end {
                self.long_token = None;
            } else {
                self.long_token = Some((token_start, token_end, token_type, next_start));
            }
            return Ok(true);
        }

        let token_type = match self.scanner.get_next_token() {
            Some(t) => t,
            None => return Ok(false),
        };

        let start = self.scanner.yychar() as usize;
        let end = start + self.scanner.yylength();
        let max_len = self.scanner.max_token_length();

        if end - start <= max_len {
            att.set_position_increment(self.skipped_positions + 1);
            self.scanner.get_text(&mut *att);
            let length = att.length() as i32;
            att.set_offset(start as i32, start as i32 + length);
            att.set_type(TOKEN_TYPES[token_type].to_string());
            self.skipped_positions = 0;
            return Ok(true);
        }

        // Token exceeds max length: emit the first chunk and remember the
        // rest for subsequent calls.
        let piece_end = start + max_len;
        att.copy_buffer(&self.scanner.text, start, max_len);
        att.set_position_increment(self.skipped_positions + 1);
        att.set_offset(start as i32, piece_end as i32);
        att.set_type(TOKEN_TYPES[token_type].to_string());
        self.long_token = Some((start, end, token_type, piece_end));
        self.skipped_positions = 0;
        Ok(true)
    }

    fn end(&mut self, source: &mut AttributeSource) -> Result<()> {
        let mut att = source
            .get_attribute_mut::<crate::analysis::PackedTokenAttributeImpl>()
            .ok_or_else(|| LuceneError::IllegalState("missing PackedTokenAttribute".to_string()))?;

        let final_offset = self.scanner.yychar() + self.scanner.yylength() as i32;
        att.set_offset(final_offset, final_offset);
        let current = att.get_position_increment();
        att.set_position_increment(current + self.skipped_positions);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, reader: &mut dyn CharReader) -> Result<()> {
        self.scanner.reset(reader)?;
        self.skipped_positions = 0;
        self.long_token = None;
        Ok(())
    }
}

/// A grammar-based tokenizer constructed with UAX #29 word boundaries.
///
/// Equivalent to `org.apache.lucene.analysis.standard.StandardTokenizer`.
pub type StandardTokenizer = Tokenizer<StandardTokenizerLogic>;

/// Creates a new `StandardTokenizer` with default settings.
pub fn new_standard_tokenizer() -> StandardTokenizer {
    StandardTokenizer::new(StandardTokenizerLogic::new())
}

// -----------------------------------------------------------------------------
// StopwordAnalyzerBase
// -----------------------------------------------------------------------------

/// Base class for analyzers that use a stop-word set.
///
/// Equivalent to `org.apache.lucene.analysis.core.StopwordAnalyzerBase`.
#[derive(Debug)]
pub struct StopwordAnalyzerBase {
    stopwords: CharArraySet,
}

impl StopwordAnalyzerBase {
    /// Creates a base analyzer using the supplied stop-word set.
    pub fn new(stopwords: CharArraySet) -> Self {
        Self { stopwords }
    }

    /// Returns the stop-word set used by this analyzer.
    pub fn stopwords(&self) -> &CharArraySet {
        &self.stopwords
    }
}

// -----------------------------------------------------------------------------
// StandardAnalyzer
// -----------------------------------------------------------------------------

/// Analyzer that tokenizes with the standard tokenizer, lowercases, and removes
/// stop words.
///
/// Equivalent to `org.apache.lucene.analysis.standard.StandardAnalyzer`.
#[derive(Debug)]
pub struct StandardAnalyzer {
    state: AnalyzerState,
    stopwords: CharArraySet,
    max_token_length: usize,
}

impl StandardAnalyzer {
    /// Creates a `StandardAnalyzer` with the default, empty stop-word set.
    pub fn new() -> Self {
        Self::with_stopwords(CharArraySet::new(0, false))
    }

    /// Creates a `StandardAnalyzer` with the supplied stop-word set.
    pub fn with_stopwords(stopwords: CharArraySet) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            stopwords,
            max_token_length: DEFAULT_MAX_TOKEN_LENGTH,
        }
    }

    /// Sets the maximum token length.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `length` is outside
    /// `[1, MAX_TOKEN_LENGTH_LIMIT]`.
    pub fn set_max_token_length(&mut self, length: usize) -> Result<()> {
        if length < 1 {
            return Err(LuceneError::IllegalArgument(
                "maxTokenLength must be greater than zero".to_string(),
            ));
        }
        if length > MAX_TOKEN_LENGTH_LIMIT {
            return Err(LuceneError::IllegalArgument(format!(
                "maxTokenLength may not exceed {MAX_TOKEN_LENGTH_LIMIT}"
            )));
        }
        self.max_token_length = length;
        Ok(())
    }

    /// Returns the current maximum token length.
    pub fn max_token_length(&self) -> usize {
        self.max_token_length
    }

    /// Returns the stop-word set used by this analyzer.
    pub fn stopwords(&self) -> &CharArraySet {
        &self.stopwords
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

    fn create_components(&self, _field_name: &str) -> TokenStreamComponents {
        let tokenizer = Rc::new(RefCell::new(new_standard_tokenizer()));
        tokenizer
            .borrow_mut()
            .logic_mut()
            .set_max_token_length(self.max_token_length)
            .unwrap();
        let tokenizer_for_source = Rc::clone(&tokenizer);
        let source = Box::new(move |reader: Box<dyn CharReader>| {
            tokenizer_for_source.borrow_mut().set_reader(reader)
        });
        let shared = SharedTokenStream::new(tokenizer);
        let lower = new_lower_case_filter(Box::new(shared));
        let stop = new_stop_filter(Box::new(lower), self.stopwords.clone());
        let sink: Rc<RefCell<dyn TokenStream>> = Rc::new(RefCell::new(stop));
        TokenStreamComponents::new(source, sink)
    }

    fn normalize_token_stream(
        &self,
        _field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(new_lower_case_filter(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        OffsetAttribute, PositionIncrementAttribute, ReusableStringReader, TokenStream,
        TypeAttribute,
    };

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
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
                .unwrap()
                .term();
            let start = source
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
                .unwrap()
                .start_offset();
            let end = source
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
                .unwrap()
                .end_offset();
            let type_value = source
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
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

    #[test]
    fn tokenizes_numbers() {
        let tokens = tokenize("Price: $123.45");
        assert_eq!(
            tokens,
            vec![
                ("Price".to_string(), 0, 5, "<ALPHANUM>".to_string()),
                ("123".to_string(), 8, 11, "<NUM>".to_string()),
                ("45".to_string(), 12, 14, "<NUM>".to_string()),
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
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
                .unwrap()
                .term();
            let pos = source
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
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
            .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
            .unwrap();
        assert_eq!(att.start_offset(), 11);
        assert_eq!(att.end_offset(), 11);
    }

    fn analyze(analyzer: &dyn Analyzer, text: &str) -> Vec<String> {
        let stream = analyzer.token_stream_from_str("field", text).unwrap();
        let mut stream = stream.borrow_mut();
        stream.reset().unwrap();
        let mut terms = Vec::new();
        while stream.increment_token().unwrap() {
            let term = stream
                .attribute_source()
                .get_attribute::<crate::analysis::PackedTokenAttributeImpl>()
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
        analyzer.set_max_token_length(3).unwrap();
        let terms = analyze(&analyzer, "abcdefg");
        assert_eq!(
            terms,
            vec!["abc".to_string(), "def".to_string(), "g".to_string()]
        );
    }
}
