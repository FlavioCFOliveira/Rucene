//! Token attribute interfaces and implementations ported from
//! `org.apache.lucene.analysis.tokenattributes`.
//!
//! These attributes carry per-token metadata through the analysis pipeline.
//! Every implementation satisfies [`AttributeImpl`](crate::util::attribute::AttributeImpl)
//! so it can be stored in an [`AttributeSource`](crate::util::attribute::AttributeSource).

#![deny(unsafe_code)]

use std::any::{Any, TypeId};
use std::fmt::{self, Debug, Formatter, Write};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use crate::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use crate::util::{ArrayUtil, BytesRef, BytesRefBuilder};

/// Produces a lazily-initialized `&'static [TypeId]` that respects the crate's
/// 1.80 MSRV by computing `TypeId`s at runtime rather than in a `const`
/// context.
macro_rules! static_type_ids {
    ($($ty:ty),* $(,)?) => {{
        static IDS: OnceLock<&'static [TypeId]> = OnceLock::new();
        IDS.get_or_init(|| {
            let ids = vec![$(TypeId::of::<$ty>()),*];
            Box::leak(ids.into_boxed_slice())
        })
    }};
}

// -----------------------------------------------------------------------------
// CharTermAttribute
// -----------------------------------------------------------------------------

/// The term text of a token, equivalent to `org.apache.lucene.analysis.tokenattributes.CharTermAttribute`.
///
/// This trait exposes a growable character buffer and append-style builders
/// similar to Java's `CharSequence` / `Appendable` combination.
pub trait CharTermAttribute: Attribute {
    /// Copies `length` characters from `buffer` starting at `offset` into the
    /// internal term buffer and sets the term length to `length`.
    fn copy_buffer(&mut self, buffer: &[char], offset: usize, length: usize);

    /// Returns an immutable view of the internal term buffer.
    fn buffer(&self) -> &[char];

    /// Returns a mutable view of the internal term buffer.
    fn buffer_mut(&mut self) -> &mut [char];

    /// Grows the internal buffer to at least `new_size` characters, preserving
    /// existing content, and returns a mutable view.
    fn resize_buffer(&mut self, new_size: usize) -> &mut [char];

    /// Sets the valid term length.
    ///
    /// # Panics
    ///
    /// Panics if `length` is greater than the current buffer capacity.
    fn set_length(&mut self, length: usize);

    /// Clears the term length to zero.
    fn set_empty(&mut self);

    /// Returns the number of valid characters in the term.
    fn length(&self) -> usize;

    /// Returns the character at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    fn char_at(&self, index: usize) -> char;

    /// Returns the characters between `start` (inclusive) and `end` (exclusive)
    /// as a new `String`.
    ///
    /// # Panics
    ///
    /// Panics if the range is invalid.
    fn sub_sequence(&self, start: usize, end: usize) -> String;

    /// Appends the entire string.
    fn append_string(&mut self, s: &str);

    /// Appends a substring of `s` between `start` and `end`.
    ///
    /// # Panics
    ///
    /// Panics if the range is invalid.
    fn append_string_range(&mut self, s: &str, start: usize, end: usize);

    /// Appends a single character.
    fn append_char(&mut self, c: char);

    /// Appends the contents of another `CharTermAttribute`.
    fn append_char_term_attribute(&mut self, term_att: &dyn CharTermAttribute);
}

/// Default implementation of [`CharTermAttribute`], also implementing
/// [`TermToBytesRefAttribute`].
pub struct CharTermAttributeImpl {
    term_buffer: Vec<char>,
    term_length: usize,
    builder: BytesRefBuilder,
}

const MIN_BUFFER_SIZE: usize = 10;

impl CharTermAttributeImpl {
    /// Creates an empty attribute with an oversized initial buffer.
    pub fn new() -> Self {
        let min_size = ArrayUtil::oversize(MIN_BUFFER_SIZE, std::mem::size_of::<char>());
        Self {
            term_buffer: vec!['\0'; min_size],
            term_length: 0,
            builder: BytesRefBuilder::new(),
        }
    }

    fn term(&self) -> String {
        self.term_buffer[..self.term_length].iter().collect()
    }

    fn grow_term_buffer(&mut self, new_size: usize) {
        if self.term_buffer.len() < new_size {
            self.term_buffer =
                vec!['\0'; ArrayUtil::oversize(new_size, std::mem::size_of::<char>())];
        }
    }
}

impl Default for CharTermAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl Attribute for CharTermAttributeImpl {}

impl CharTermAttribute for CharTermAttributeImpl {
    fn copy_buffer(&mut self, buffer: &[char], offset: usize, length: usize) {
        assert!(
            offset + length <= buffer.len(),
            "source buffer range out of bounds: offset={offset}, length={length}, buffer.len()={}",
            buffer.len()
        );
        self.grow_term_buffer(length);
        self.term_buffer[..length].copy_from_slice(&buffer[offset..offset + length]);
        self.term_length = length;
    }

    fn buffer(&self) -> &[char] {
        &self.term_buffer
    }

    fn buffer_mut(&mut self) -> &mut [char] {
        &mut self.term_buffer
    }

    fn resize_buffer(&mut self, new_size: usize) -> &mut [char] {
        if self.term_buffer.len() < new_size {
            let new_len = ArrayUtil::oversize(new_size, std::mem::size_of::<char>());
            let mut new_buffer = vec!['\0'; new_len];
            new_buffer[..self.term_buffer.len()].copy_from_slice(&self.term_buffer);
            self.term_buffer = new_buffer;
        }
        &mut self.term_buffer
    }

    fn set_length(&mut self, length: usize) {
        assert!(
            length <= self.term_buffer.len(),
            "length {} out of buffer bounds {}",
            length,
            self.term_buffer.len()
        );
        self.term_length = length;
    }

    fn set_empty(&mut self) {
        self.term_length = 0;
    }

    fn length(&self) -> usize {
        self.term_length
    }

    fn char_at(&self, index: usize) -> char {
        assert!(
            index < self.term_length,
            "index {index} out of bounds (length {})",
            self.term_length
        );
        self.term_buffer[index]
    }

    fn sub_sequence(&self, start: usize, end: usize) -> String {
        assert!(
            start <= end && end <= self.term_length,
            "invalid sub-sequence: start={start}, end={end}, length={}",
            self.term_length
        );
        self.term_buffer[start..end].iter().collect()
    }

    fn append_string(&mut self, s: &str) {
        let len = s.chars().count();
        let start_len = self.term_length;
        {
            let buf = self.resize_buffer(start_len + len);
            for (i, c) in s.chars().enumerate() {
                buf[start_len + i] = c;
            }
        }
        self.term_length += len;
    }

    fn append_string_range(&mut self, s: &str, start: usize, end: usize) {
        let char_count = s.chars().count();
        assert!(
            start <= end && end <= char_count,
            "invalid append range: start={start}, end={end}, length={char_count}"
        );
        let len = end - start;
        if len == 0 {
            return;
        }
        let start_len = self.term_length;
        {
            let buf = self.resize_buffer(start_len + len);
            for (i, c) in s.chars().skip(start).take(len).enumerate() {
                buf[start_len + i] = c;
            }
        }
        self.term_length += len;
    }

    fn append_char(&mut self, c: char) {
        let start_len = self.term_length;
        {
            let buf = self.resize_buffer(start_len + 1);
            buf[start_len] = c;
        }
        self.term_length += 1;
    }

    fn append_char_term_attribute(&mut self, term_att: &dyn CharTermAttribute) {
        let len = term_att.length();
        let start_len = self.term_length;
        {
            let buf = self.resize_buffer(start_len + len);
            let src = term_att.buffer();
            buf[start_len..start_len + len].copy_from_slice(&src[..len]);
        }
        self.term_length += len;
    }
}

impl TermToBytesRefAttribute for CharTermAttributeImpl {
    fn get_bytes_ref(&self) -> BytesRef {
        let text: String = self.term_buffer[..self.term_length].iter().collect();
        let mut builder = BytesRefBuilder::new();
        builder.copy_chars(&text);
        builder.get()
    }
}

impl Clone for CharTermAttributeImpl {
    fn clone(&self) -> Self {
        let mut clone = Self::new();
        clone.copy_buffer(&self.term_buffer, 0, self.term_length);
        let bytes = self.get_bytes_ref();
        clone.builder = BytesRefBuilder::new();
        clone
            .builder
            .copy_bytes(&bytes.bytes, bytes.offset, bytes.length);
        clone
    }
}

impl PartialEq for CharTermAttributeImpl {
    fn eq(&self, other: &Self) -> bool {
        if self.term_length != other.term_length {
            return false;
        }
        self.term_buffer[..self.term_length] == other.term_buffer[..other.term_length]
    }
}

impl Eq for CharTermAttributeImpl {}

impl Hash for CharTermAttributeImpl {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Matches Java's CharTermAttributeImpl.hashCode().
        let mut code = self.term_length as i32;
        for &c in &self.term_buffer[..self.term_length] {
            code = code.wrapping_mul(31).wrapping_add(c as i32);
        }
        state.write_i32(code);
    }
}

impl Debug for CharTermAttributeImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CharTermAttributeImpl")
            .field("term", &self.term())
            .field("term_length", &self.term_length)
            .finish()
    }
}

impl Write for CharTermAttributeImpl {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.append_string(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        self.append_char(c);
        Ok(())
    }
}

impl AttributeImpl for CharTermAttributeImpl {
    fn clear(&mut self) {
        self.term_length = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<CharTermAttributeImpl>() {
            t.copy_buffer(&self.term_buffer, 0, self.term_length);
            let bytes = self.get_bytes_ref();
            t.builder = BytesRefBuilder::new();
            t.builder
                .copy_bytes(&bytes.bytes, bytes.offset, bytes.length);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn CharTermAttribute>(),
            std::any::type_name::<CharTermAttributeImpl>(),
            "term",
            &self.term(),
        );
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<CharTermAttributeImpl>(),
            "bytes",
            &self.get_bytes_ref(),
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![
            CharTermAttributeImpl,
            dyn CharTermAttribute,
            dyn TermToBytesRefAttribute
        ]
    }
}

// -----------------------------------------------------------------------------
// OffsetAttribute
// -----------------------------------------------------------------------------

/// The start and end character offset of a token.
pub trait OffsetAttribute: Attribute {
    /// Returns the token's starting offset.
    fn start_offset(&self) -> i32;

    /// Sets the starting and ending offsets.
    ///
    /// # Panics
    ///
    /// Panics if `start_offset` is negative or `end_offset` is less than
    /// `start_offset`.
    fn set_offset(&mut self, start_offset: i32, end_offset: i32);

    /// Returns the token's ending offset.
    fn end_offset(&self) -> i32;
}

/// Default implementation of [`OffsetAttribute`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct OffsetAttributeImpl {
    start_offset: i32,
    end_offset: i32,
}

impl OffsetAttributeImpl {
    /// Creates an attribute with both offsets set to zero.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Attribute for OffsetAttributeImpl {}

impl OffsetAttribute for OffsetAttributeImpl {
    fn start_offset(&self) -> i32 {
        self.start_offset
    }

    fn set_offset(&mut self, start_offset: i32, end_offset: i32) {
        assert!(
            start_offset >= 0 && end_offset >= start_offset,
            "startOffset must be non-negative, and endOffset must be >= startOffset; got startOffset={start_offset}, endOffset={end_offset}"
        );
        self.start_offset = start_offset;
        self.end_offset = end_offset;
    }

    fn end_offset(&self) -> i32 {
        self.end_offset
    }
}

impl AttributeImpl for OffsetAttributeImpl {
    fn clear(&mut self) {
        self.start_offset = 0;
        self.end_offset = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<OffsetAttributeImpl>() {
            t.set_offset(self.start_offset, self.end_offset);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn OffsetAttribute>(),
            std::any::type_name::<OffsetAttributeImpl>(),
            "startOffset",
            &self.start_offset,
        );
        reflector.reflect(
            TypeId::of::<dyn OffsetAttribute>(),
            std::any::type_name::<OffsetAttributeImpl>(),
            "endOffset",
            &self.end_offset,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![OffsetAttributeImpl, dyn OffsetAttribute]
    }
}

// -----------------------------------------------------------------------------
// PositionIncrementAttribute
// -----------------------------------------------------------------------------

/// Determines the position of this token relative to the previous token.
pub trait PositionIncrementAttribute: Attribute {
    /// Sets the position increment.
    ///
    /// # Panics
    ///
    /// Panics if `position_increment` is negative.
    fn set_position_increment(&mut self, position_increment: i32);

    /// Returns the position increment.
    fn get_position_increment(&self) -> i32;
}

/// Default implementation of [`PositionIncrementAttribute`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PositionIncrementAttributeImpl {
    position_increment: i32,
}

impl Default for PositionIncrementAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionIncrementAttributeImpl {
    /// Creates an attribute with position increment of 1.
    pub fn new() -> Self {
        Self {
            position_increment: 1,
        }
    }
}

impl Attribute for PositionIncrementAttributeImpl {}

impl PositionIncrementAttribute for PositionIncrementAttributeImpl {
    fn set_position_increment(&mut self, position_increment: i32) {
        assert!(
            position_increment >= 0,
            "Position increment must be zero or greater; got {position_increment}"
        );
        self.position_increment = position_increment;
    }

    fn get_position_increment(&self) -> i32 {
        self.position_increment
    }
}

impl AttributeImpl for PositionIncrementAttributeImpl {
    fn clear(&mut self) {
        self.position_increment = 1;
    }

    fn end(&mut self) {
        self.position_increment = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<PositionIncrementAttributeImpl>()
        {
            t.set_position_increment(self.position_increment);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn PositionIncrementAttribute>(),
            std::any::type_name::<PositionIncrementAttributeImpl>(),
            "positionIncrement",
            &self.position_increment,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![
            PositionIncrementAttributeImpl,
            dyn PositionIncrementAttribute
        ]
    }
}

// -----------------------------------------------------------------------------
// PositionLengthAttribute
// -----------------------------------------------------------------------------

/// Determines how many positions this token spans.
pub trait PositionLengthAttribute: Attribute {
    /// Sets the position length.
    ///
    /// # Panics
    ///
    /// Panics if `position_length` is less than 1.
    fn set_position_length(&mut self, position_length: i32);

    /// Returns the position length.
    fn get_position_length(&self) -> i32;
}

/// Default implementation of [`PositionLengthAttribute`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PositionLengthAttributeImpl {
    position_length: i32,
}

impl Default for PositionLengthAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionLengthAttributeImpl {
    /// Creates an attribute with position length of 1.
    pub fn new() -> Self {
        Self { position_length: 1 }
    }
}

impl Attribute for PositionLengthAttributeImpl {}

impl PositionLengthAttribute for PositionLengthAttributeImpl {
    fn set_position_length(&mut self, position_length: i32) {
        assert!(
            position_length >= 1,
            "Position length must be 1 or greater; got {position_length}"
        );
        self.position_length = position_length;
    }

    fn get_position_length(&self) -> i32 {
        self.position_length
    }
}

impl AttributeImpl for PositionLengthAttributeImpl {
    fn clear(&mut self) {
        self.position_length = 1;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<PositionLengthAttributeImpl>()
        {
            t.set_position_length(self.position_length);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn PositionLengthAttribute>(),
            std::any::type_name::<PositionLengthAttributeImpl>(),
            "positionLength",
            &self.position_length,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![PositionLengthAttributeImpl, dyn PositionLengthAttribute]
    }
}

// -----------------------------------------------------------------------------
// TypeAttribute
// -----------------------------------------------------------------------------

/// The default lexical type value, matching Lucene's `"word"`.
pub const DEFAULT_TYPE: &str = "word";

/// A token's lexical type.
pub trait TypeAttribute: Attribute {
    /// Returns the lexical type.
    fn type_value(&self) -> &str;

    /// Sets the lexical type.
    fn set_type(&mut self, type_value: String);
}

/// Default implementation of [`TypeAttribute`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeAttributeImpl {
    type_value: String,
}

impl Default for TypeAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeAttributeImpl {
    /// Creates an attribute with the default type.
    pub fn new() -> Self {
        Self {
            type_value: DEFAULT_TYPE.to_string(),
        }
    }

    /// Creates an attribute with the given type.
    pub fn with_type(type_value: String) -> Self {
        Self { type_value }
    }
}

impl Attribute for TypeAttributeImpl {}

impl TypeAttribute for TypeAttributeImpl {
    fn type_value(&self) -> &str {
        &self.type_value
    }

    fn set_type(&mut self, type_value: String) {
        self.type_value = type_value;
    }
}

impl AttributeImpl for TypeAttributeImpl {
    fn clear(&mut self) {
        self.type_value = DEFAULT_TYPE.to_string();
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<TypeAttributeImpl>() {
            t.type_value.clone_from(&self.type_value);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn TypeAttribute>(),
            std::any::type_name::<TypeAttributeImpl>(),
            "type",
            &self.type_value,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![TypeAttributeImpl, dyn TypeAttribute]
    }
}

// -----------------------------------------------------------------------------
// TermFrequencyAttribute
// -----------------------------------------------------------------------------

/// Custom term frequency within one document.
pub trait TermFrequencyAttribute: Attribute {
    /// Sets the custom term frequency.
    ///
    /// # Panics
    ///
    /// Panics if `term_frequency` is less than 1.
    fn set_term_frequency(&mut self, term_frequency: i32);

    /// Returns the custom term frequency.
    fn get_term_frequency(&self) -> i32;
}

/// Default implementation of [`TermFrequencyAttribute`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TermFrequencyAttributeImpl {
    term_frequency: i32,
}

impl Default for TermFrequencyAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TermFrequencyAttributeImpl {
    /// Creates an attribute with term frequency of 1.
    pub fn new() -> Self {
        Self { term_frequency: 1 }
    }
}

impl Attribute for TermFrequencyAttributeImpl {}

impl TermFrequencyAttribute for TermFrequencyAttributeImpl {
    fn set_term_frequency(&mut self, term_frequency: i32) {
        assert!(
            term_frequency >= 1,
            "Term frequency must be 1 or greater; got {term_frequency}"
        );
        self.term_frequency = term_frequency;
    }

    fn get_term_frequency(&self) -> i32 {
        self.term_frequency
    }
}

impl AttributeImpl for TermFrequencyAttributeImpl {
    fn clear(&mut self) {
        self.term_frequency = 1;
    }

    fn end(&mut self) {
        self.term_frequency = 1;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<TermFrequencyAttributeImpl>()
        {
            t.set_term_frequency(self.term_frequency);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn TermFrequencyAttribute>(),
            std::any::type_name::<TermFrequencyAttributeImpl>(),
            "termFrequency",
            &self.term_frequency,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![TermFrequencyAttributeImpl, dyn TermFrequencyAttribute]
    }
}

// -----------------------------------------------------------------------------
// KeywordAttribute
// -----------------------------------------------------------------------------

/// Marks a token as a keyword.
pub trait KeywordAttribute: Attribute {
    /// Returns `true` if the current token is a keyword.
    fn is_keyword(&self) -> bool;

    /// Marks the current token as a keyword.
    fn set_keyword(&mut self, is_keyword: bool);
}

/// Default implementation of [`KeywordAttribute`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeywordAttributeImpl {
    keyword: bool,
}

impl KeywordAttributeImpl {
    /// Creates an attribute with keyword set to `false`.
    pub fn new() -> Self {
        Self { keyword: false }
    }
}

impl Attribute for KeywordAttributeImpl {}

impl KeywordAttribute for KeywordAttributeImpl {
    fn is_keyword(&self) -> bool {
        self.keyword
    }

    fn set_keyword(&mut self, is_keyword: bool) {
        self.keyword = is_keyword;
    }
}

impl Hash for KeywordAttributeImpl {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Matches Java's KeywordAttributeImpl.hashCode().
        state.write_i32(if self.keyword { 31 } else { 37 });
    }
}

impl AttributeImpl for KeywordAttributeImpl {
    fn clear(&mut self) {
        self.keyword = false;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<KeywordAttributeImpl>() {
            t.set_keyword(self.keyword);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn KeywordAttribute>(),
            std::any::type_name::<KeywordAttributeImpl>(),
            "keyword",
            &self.keyword,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![KeywordAttributeImpl, dyn KeywordAttribute]
    }
}

// -----------------------------------------------------------------------------
// FlagsAttribute
// -----------------------------------------------------------------------------

/// Passes bit flags down the tokenizer chain.
pub trait FlagsAttribute: Attribute {
    /// Returns the current flags.
    fn get_flags(&self) -> i32;

    /// Sets the flags.
    fn set_flags(&mut self, flags: i32);
}

/// Default implementation of [`FlagsAttribute`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FlagsAttributeImpl {
    flags: i32,
}

impl FlagsAttributeImpl {
    /// Creates an attribute with no bits set.
    pub fn new() -> Self {
        Self { flags: 0 }
    }
}

impl Attribute for FlagsAttributeImpl {}

impl FlagsAttribute for FlagsAttributeImpl {
    fn get_flags(&self) -> i32 {
        self.flags
    }

    fn set_flags(&mut self, flags: i32) {
        self.flags = flags;
    }
}

impl AttributeImpl for FlagsAttributeImpl {
    fn clear(&mut self) {
        self.flags = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<FlagsAttributeImpl>() {
            t.set_flags(self.flags);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn FlagsAttribute>(),
            std::any::type_name::<FlagsAttributeImpl>(),
            "flags",
            &self.flags,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![FlagsAttributeImpl, dyn FlagsAttribute]
    }
}

// -----------------------------------------------------------------------------
// PayloadAttribute
// -----------------------------------------------------------------------------

/// The payload of a token.
pub trait PayloadAttribute: Attribute {
    /// Returns this token's payload, if any.
    fn get_payload(&self) -> Option<&BytesRef>;

    /// Sets this token's payload.
    fn set_payload(&mut self, payload: Option<BytesRef>);
}

/// Default implementation of [`PayloadAttribute`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PayloadAttributeImpl {
    payload: Option<BytesRef>,
}

impl PayloadAttributeImpl {
    /// Creates an attribute with no payload.
    pub fn new() -> Self {
        Self { payload: None }
    }

    /// Creates an attribute with the given payload.
    pub fn with_payload(payload: BytesRef) -> Self {
        Self {
            payload: Some(payload),
        }
    }
}

impl Attribute for PayloadAttributeImpl {}

impl PayloadAttribute for PayloadAttributeImpl {
    fn get_payload(&self) -> Option<&BytesRef> {
        self.payload.as_ref()
    }

    fn set_payload(&mut self, payload: Option<BytesRef>) {
        self.payload = payload;
    }
}

impl Clone for PayloadAttributeImpl {
    fn clone(&self) -> Self {
        Self {
            payload: self.payload.as_ref().map(BytesRef::deep_copy_of),
        }
    }
}

impl AttributeImpl for PayloadAttributeImpl {
    fn clear(&mut self) {
        self.payload = None;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<PayloadAttributeImpl>() {
            t.set_payload(self.payload.as_ref().map(BytesRef::deep_copy_of));
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn PayloadAttribute>(),
            std::any::type_name::<PayloadAttributeImpl>(),
            "payload",
            &self.payload,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![PayloadAttributeImpl, dyn PayloadAttribute]
    }
}

// -----------------------------------------------------------------------------
// TermToBytesRefAttribute & BytesTermAttribute
// -----------------------------------------------------------------------------

/// Provides the raw bytes used for indexing a term.
pub trait TermToBytesRefAttribute: Attribute {
    /// Returns a `BytesRef` for the current term.
    fn get_bytes_ref(&self) -> BytesRef;
}

/// Attribute for raw binary term bytes.
pub trait BytesTermAttribute: TermToBytesRefAttribute {
    /// Sets the term bytes.
    fn set_bytes_ref(&mut self, bytes: BytesRef);
}

/// Default implementation of [`BytesTermAttribute`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BytesTermAttributeImpl {
    bytes: Option<BytesRef>,
}

impl BytesTermAttributeImpl {
    /// Creates an attribute with no bytes.
    pub fn new() -> Self {
        Self { bytes: None }
    }

    /// Creates an attribute with the given bytes.
    pub fn with_bytes(bytes: BytesRef) -> Self {
        Self { bytes: Some(bytes) }
    }
}

impl Attribute for BytesTermAttributeImpl {}

impl TermToBytesRefAttribute for BytesTermAttributeImpl {
    fn get_bytes_ref(&self) -> BytesRef {
        match &self.bytes {
            Some(b) => BytesRef::deep_copy_of(b),
            None => BytesRef::default(),
        }
    }
}

impl BytesTermAttribute for BytesTermAttributeImpl {
    fn set_bytes_ref(&mut self, bytes: BytesRef) {
        self.bytes = Some(bytes);
    }
}

impl Clone for BytesTermAttributeImpl {
    fn clone(&self) -> Self {
        Self {
            bytes: self.bytes.as_ref().map(BytesRef::deep_copy_of),
        }
    }
}

impl AttributeImpl for BytesTermAttributeImpl {
    fn clear(&mut self) {
        self.bytes = None;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<BytesTermAttributeImpl>() {
            t.bytes = self.bytes.as_ref().map(BytesRef::deep_copy_of);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            std::any::type_name::<BytesTermAttributeImpl>(),
            "bytes",
            &self.bytes,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![
            BytesTermAttributeImpl,
            dyn BytesTermAttribute,
            dyn TermToBytesRefAttribute
        ]
    }
}

// -----------------------------------------------------------------------------
// SentenceAttribute
// -----------------------------------------------------------------------------

/// Tracks which sentence a token belongs to.
pub trait SentenceAttribute: Attribute {
    /// Returns the sentence index for the current token.
    fn get_sentence_index(&self) -> i32;

    /// Sets the sentence index.
    fn set_sentence_index(&mut self, sentence_index: i32);
}

/// Default implementation of [`SentenceAttribute`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SentenceAttributeImpl {
    index: i32,
}

impl SentenceAttributeImpl {
    /// Creates an attribute with sentence index 0.
    pub fn new() -> Self {
        Self { index: 0 }
    }
}

impl Attribute for SentenceAttributeImpl {}

impl SentenceAttribute for SentenceAttributeImpl {
    fn get_sentence_index(&self) -> i32 {
        self.index
    }

    fn set_sentence_index(&mut self, sentence_index: i32) {
        self.index = sentence_index;
    }
}

impl AttributeImpl for SentenceAttributeImpl {
    fn clear(&mut self) {
        self.index = 0;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<SentenceAttributeImpl>() {
            t.set_sentence_index(self.index);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn SentenceAttribute>(),
            std::any::type_name::<SentenceAttributeImpl>(),
            "sentences",
            &self.index,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static_type_ids![SentenceAttributeImpl, dyn SentenceAttribute]
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::attribute::{AttributeSource, DefaultAttributeFactory};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    fn hash_one<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn char_term_resize_preserves_content() {
        let mut t = CharTermAttributeImpl::new();
        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, content.len());
        for i in 0..2000 {
            t.resize_buffer(i);
            assert!(t.buffer().len() >= i);
            assert_eq!(t.term(), "hello");
        }
    }

    #[test]
    fn char_term_set_length_rejects_overflow() {
        let mut t = CharTermAttributeImpl::new();
        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, content.len());
        t.set_length(5);
        // Setting length beyond buffer capacity should panic.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut t2 = CharTermAttributeImpl::new();
            let content: Vec<char> = "hello".chars().collect();
            t2.copy_buffer(&content, 0, content.len());
            t2.set_length(t2.buffer().len() + 1);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn char_term_grow_by_copy_and_append() {
        let mut t = CharTermAttributeImpl::new();
        let mut buf = "ab".to_string();
        for _ in 0..20 {
            let content: Vec<char> = buf.chars().collect();
            t.copy_buffer(&content, 0, content.len());
            assert_eq!(t.length(), buf.chars().count());
            assert_eq!(t.term(), buf);
            buf += &buf.clone();
        }
        assert_eq!(t.length(), 1_048_576);

        let mut t = CharTermAttributeImpl::new();
        let mut buf = "ab".to_string();
        for _ in 0..20 {
            t.set_empty();
            t.append_string(&buf);
            assert_eq!(t.length(), buf.chars().count());
            assert_eq!(t.term(), buf);
            buf += &t.term();
        }
        assert_eq!(t.length(), 1_048_576);

        let mut t = CharTermAttributeImpl::new();
        let mut buf = "a".to_string();
        for _ in 0..20000 {
            t.set_empty();
            t.append_string(&buf);
            assert_eq!(t.length(), buf.chars().count());
            assert_eq!(t.term(), buf);
            buf.push('a');
        }
        assert_eq!(t.length(), 20000);
    }

    #[test]
    fn char_term_to_string_and_copy_buffer() {
        let chars = ['a', 'l', 'o', 'h', 'a'];
        let mut t = CharTermAttributeImpl::new();
        t.copy_buffer(&chars, 0, 5);
        assert_eq!(t.term(), "aloha");
        t.set_empty();
        t.append_string("hi there");
        assert_eq!(t.term(), "hi there");
    }

    #[test]
    fn char_term_clone_is_independent() {
        let mut t = CharTermAttributeImpl::new();
        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, 5);
        let buf_ptr = t.buffer().as_ptr();
        let clone = t.clone();
        assert_eq!(t.term(), clone.term());
        assert_ne!(buf_ptr, clone.buffer().as_ptr());
    }

    #[test]
    fn char_term_equals_and_hash() {
        let mut t1a = CharTermAttributeImpl::new();
        let mut t1b = CharTermAttributeImpl::new();
        let mut t2 = CharTermAttributeImpl::new();
        let c1: Vec<char> = "hello".chars().collect();
        let c2: Vec<char> = "hello2".chars().collect();
        t1a.copy_buffer(&c1, 0, 5);
        t1b.copy_buffer(&c1, 0, 5);
        t2.copy_buffer(&c2, 0, 6);
        assert_eq!(t1a, t1b);
        assert_ne!(t1a, t2);
        assert_ne!(t2, t1b);
        assert_eq!(hash_one(&t1a), hash_one(&t1b));
    }

    #[test]
    fn char_term_copy_to() {
        let mut t = CharTermAttributeImpl::new();
        let mut copy = CharTermAttributeImpl::new();
        t.copy_to(&mut copy);
        assert_eq!(t.term(), "");
        assert_eq!(copy.term(), "");

        let content: Vec<char> = "hello".chars().collect();
        t.copy_buffer(&content, 0, 5);
        t.copy_to(&mut copy);
        assert_eq!(t.term(), "hello");
        assert_eq!(copy.term(), "hello");
        assert_ne!(t.buffer().as_ptr(), copy.buffer().as_ptr());
    }

    #[test]
    fn char_term_reflection() {
        let mut t = CharTermAttributeImpl::new();
        t.append_string("foobar");
        let mut entries: Vec<(TypeId, &'static str, String, String)> = Vec::new();
        t.reflect_with(
            &mut |type_id: TypeId, name: &'static str, key: &str, value: &dyn Debug| {
                entries.push((type_id, name, key.to_string(), format!("{:?}", value)));
            },
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].2, "term");
        assert!(entries[0].3.contains("foobar"));
        assert_eq!(entries[1].2, "bytes");
    }

    #[test]
    fn char_term_char_sequence_interface() {
        let s = "0123456789";
        let mut t = CharTermAttributeImpl::new();
        t.append_string(s);
        assert_eq!(t.length(), s.chars().count());
        assert_eq!(t.sub_sequence(1, 3), "12");
        assert_eq!(t.sub_sequence(0, s.chars().count()), s);
        for (i, c) in s.chars().enumerate() {
            assert_eq!(t.char_at(i), c);
        }
    }

    #[test]
    fn char_term_appendable_interface() {
        let mut t = CharTermAttributeImpl::new();
        write!(t, "{}{}", 1234, 5678).unwrap();
        t.append_char('9');
        t.append_string("0");
        t.append_string_range("0123456789", 1, 3);
        assert_eq!(t.term(), "123456789012");

        let mut t2 = CharTermAttributeImpl::new();
        t2.append_string("test");
        t.append_char_term_attribute(&t2);
        assert_eq!(t.term(), "123456789012test");
    }

    #[test]
    fn char_term_exceptions() {
        let mut t = CharTermAttributeImpl::new();
        t.append_string("test");
        assert_eq!(t.term(), "test");
        assert!(std::panic::catch_unwind(|| t.char_at(4)).is_err());
        assert!(std::panic::catch_unwind(|| t.sub_sequence(0, 5)).is_err());
        assert!(std::panic::catch_unwind(|| t.sub_sequence(5, 0)).is_err());
    }

    #[test]
    fn char_term_get_bytes_ref() {
        let mut t = CharTermAttributeImpl::new();
        t.append_string("lucene");
        let bytes = t.get_bytes_ref();
        assert_eq!(bytes.slice(), b"lucene");
    }

    #[test]
    fn offset_set_get_clear() {
        let mut t = OffsetAttributeImpl::new();
        assert_eq!(t.start_offset(), 0);
        assert_eq!(t.end_offset(), 0);
        t.set_offset(3, 7);
        assert_eq!(t.start_offset(), 3);
        assert_eq!(t.end_offset(), 7);
        t.clear();
        assert_eq!(t.start_offset(), 0);
        assert_eq!(t.end_offset(), 0);
    }

    #[test]
    fn offset_invalid_arguments_panic() {
        assert!(std::panic::catch_unwind(|| {
            let mut t = OffsetAttributeImpl::new();
            t.set_offset(-1, 5);
        })
        .is_err());
        assert!(std::panic::catch_unwind(|| {
            let mut t = OffsetAttributeImpl::new();
            t.set_offset(5, 3);
        })
        .is_err());
    }

    #[test]
    fn offset_copy_to_and_reflection() {
        let mut t = OffsetAttributeImpl::new();
        t.set_offset(1, 4);
        let mut copy = OffsetAttributeImpl::new();
        t.copy_to(&mut copy);
        assert_eq!(copy.start_offset(), 1);
        assert_eq!(copy.end_offset(), 4);

        let mut entries = Vec::new();
        t.reflect_with(&mut |_type_id: TypeId,
                             _name: &'static str,
                             key: &str,
                             value: &dyn Debug| {
            entries.push((key.to_string(), format!("{:?}", value)));
        });
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "startOffset");
        assert_eq!(entries[1].0, "endOffset");
    }

    #[test]
    fn position_increment_defaults_and_end() {
        let mut t = PositionIncrementAttributeImpl::new();
        assert_eq!(t.get_position_increment(), 1);
        t.set_position_increment(5);
        assert_eq!(t.get_position_increment(), 5);
        t.clear();
        assert_eq!(t.get_position_increment(), 1);
        t.set_position_increment(3);
        t.end();
        assert_eq!(t.get_position_increment(), 0);
    }

    #[test]
    fn position_increment_negative_panics() {
        assert!(std::panic::catch_unwind(|| {
            let mut t = PositionIncrementAttributeImpl::new();
            t.set_position_increment(-1);
        })
        .is_err());
    }

    #[test]
    fn position_length_defaults() {
        let mut t = PositionLengthAttributeImpl::new();
        assert_eq!(t.get_position_length(), 1);
        t.set_position_length(3);
        assert_eq!(t.get_position_length(), 3);
        t.clear();
        assert_eq!(t.get_position_length(), 1);
    }

    #[test]
    fn position_length_invalid_panics() {
        assert!(std::panic::catch_unwind(|| {
            let mut t = PositionLengthAttributeImpl::new();
            t.set_position_length(0);
        })
        .is_err());
    }

    #[test]
    fn type_attribute_defaults_and_clear() {
        let mut t = TypeAttributeImpl::new();
        assert_eq!(t.type_value(), "word");
        t.set_type("number".to_string());
        assert_eq!(t.type_value(), "number");
        t.clear();
        assert_eq!(t.type_value(), "word");
    }

    #[test]
    fn term_frequency_defaults_and_end() {
        let mut t = TermFrequencyAttributeImpl::new();
        assert_eq!(t.get_term_frequency(), 1);
        t.set_term_frequency(7);
        assert_eq!(t.get_term_frequency(), 7);
        t.clear();
        assert_eq!(t.get_term_frequency(), 1);
        t.set_term_frequency(5);
        t.end();
        assert_eq!(t.get_term_frequency(), 1);
    }

    #[test]
    fn term_frequency_invalid_panics() {
        assert!(std::panic::catch_unwind(|| {
            let mut t = TermFrequencyAttributeImpl::new();
            t.set_term_frequency(0);
        })
        .is_err());
    }

    #[test]
    fn keyword_attribute_defaults() {
        let mut t = KeywordAttributeImpl::new();
        assert!(!t.is_keyword());
        t.set_keyword(true);
        assert!(t.is_keyword());
        t.clear();
        assert!(!t.is_keyword());
    }

    #[test]
    fn flags_attribute_defaults() {
        let mut t = FlagsAttributeImpl::new();
        assert_eq!(t.get_flags(), 0);
        t.set_flags(0b1010);
        assert_eq!(t.get_flags(), 0b1010);
        t.clear();
        assert_eq!(t.get_flags(), 0);
    }

    #[test]
    fn payload_attribute_set_get_clear_copy() {
        let mut t = PayloadAttributeImpl::with_payload(BytesRef::new(vec![1, 2, 3]));
        assert_eq!(t.get_payload().unwrap().slice(), &[1, 2, 3]);
        let mut copy = PayloadAttributeImpl::new();
        t.copy_to(&mut copy);
        assert_eq!(copy.get_payload().unwrap().slice(), &[1, 2, 3]);
        let cloned = t.clone();
        assert_eq!(cloned.get_payload().unwrap().slice(), &[1, 2, 3]);
        t.clear();
        assert!(t.get_payload().is_none());
    }

    #[test]
    fn bytes_term_attribute_set_get_clear() {
        let mut t = BytesTermAttributeImpl::with_bytes(BytesRef::new(vec![4, 5, 6]));
        assert_eq!(t.get_bytes_ref().slice(), &[4, 5, 6]);
        t.set_bytes_ref(BytesRef::new(vec![7, 8]));
        assert_eq!(t.get_bytes_ref().slice(), &[7, 8]);
        t.clear();
        assert_eq!(t.get_bytes_ref().slice(), &[]);
    }

    #[test]
    fn sentence_attribute_defaults() {
        let mut t = SentenceAttributeImpl::new();
        assert_eq!(t.get_sentence_index(), 0);
        t.set_sentence_index(3);
        assert_eq!(t.get_sentence_index(), 3);
        t.clear();
        assert_eq!(t.get_sentence_index(), 0);
    }

    #[test]
    fn all_defaults_reflection() {
        let mut factory = DefaultAttributeFactory::new();
        factory.register_self::<PositionIncrementAttributeImpl>();
        factory.register_self::<PositionLengthAttributeImpl>();
        factory.register_self::<FlagsAttributeImpl>();
        factory.register_self::<TypeAttributeImpl>();
        factory.register_self::<PayloadAttributeImpl>();
        factory.register_self::<KeywordAttributeImpl>();
        factory.register_self::<OffsetAttributeImpl>();
        factory.register_self::<SentenceAttributeImpl>();

        let mut source = AttributeSource::new_with_factory(Arc::new(factory));
        source
            .add_attribute::<PositionIncrementAttributeImpl>()
            .unwrap();
        source
            .add_attribute::<PositionLengthAttributeImpl>()
            .unwrap();
        source.add_attribute::<FlagsAttributeImpl>().unwrap();
        source.add_attribute::<TypeAttributeImpl>().unwrap();
        source.add_attribute::<PayloadAttributeImpl>().unwrap();
        source.add_attribute::<KeywordAttributeImpl>().unwrap();
        source.add_attribute::<OffsetAttributeImpl>().unwrap();
        source.add_attribute::<SentenceAttributeImpl>().unwrap();

        let mut entries: Vec<(String, String, String)> = Vec::new();
        source.reflect_with(&mut |_type_id: TypeId,
                                  name: &'static str,
                                  key: &str,
                                  value: &dyn Debug| {
            entries.push((name.to_string(), key.to_string(), format!("{:?}", value)));
        });

        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "positionIncrement" && v == "1"));
        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "positionLength" && v == "1"));
        assert!(entries.iter().any(|(_, k, v)| k == "flags" && v == "0"));
        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "type" && v == "\"word\""));
        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "payload" && v == "None"));
        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "keyword" && v == "false"));
        assert!(entries
            .iter()
            .any(|(_, k, v)| k == "startOffset" && v == "0"));
        assert!(entries.iter().any(|(_, k, v)| k == "endOffset" && v == "0"));
        assert!(entries.iter().any(|(_, k, v)| k == "sentences" && v == "0"));
    }

    #[test]
    fn attribute_source_with_char_term_attribute() {
        let mut factory = DefaultAttributeFactory::new();
        factory.register_self::<CharTermAttributeImpl>();
        let mut source = AttributeSource::new_with_factory(Arc::new(factory));
        source.add_attribute::<CharTermAttributeImpl>().unwrap();
        source
            .get_attribute_mut::<CharTermAttributeImpl>()
            .unwrap()
            .append_string("token");
        let att = source.get_attribute::<CharTermAttributeImpl>().unwrap();
        assert_eq!(att.term(), "token");
    }
}
