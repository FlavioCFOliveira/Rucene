// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.FormattedValue`.

use rucene::error::Result;

use crate::jdk::text::AttributedCharacterIterator;
use crate::text::constrained_field_position::ConstrainedFieldPosition;

/// An abstract formatted value: a string with associated field attributes.
///
/// Equivalent to the interface `com.ibm.icu.text.FormattedValue`, which
/// extends `CharSequence` ([`length`](Self::length),
/// [`char_at`](Self::char_at), [`sub_sequence`](Self::sub_sequence)). An
/// `Appendable` is a `&mut Vec<u16>` (crate convention: appending cannot
/// fail, so Java's `ICUUncheckedIOException` wrapping has no counterpart).
pub trait FormattedValue: std::fmt::Debug {
    /// `length()`.
    fn length(&self) -> i32;

    /// `charAt(int index)`.
    fn char_at(&self, index: i32) -> u16;

    /// `subSequence(int start, int end)`.
    fn sub_sequence(&self, start: i32, end: i32) -> Vec<u16>;

    /// `toString()`: the formatted string, not including field information.
    fn to_java_string(&self) -> Vec<u16>;

    /// `appendTo(A appendable)`: appends the formatted string.
    fn append_to<'a>(&self, appendable: &'a mut Vec<u16>) -> &'a mut Vec<u16>;

    /// `nextPosition(ConstrainedFieldPosition cfpos)`: advances `cfpos` to
    /// the next field span matching its constraint; `false` when there is
    /// none.
    fn next_position(&self, cfpos: &mut ConstrainedFieldPosition) -> bool;

    /// `toCharacterIterator()`: the string with its field attributes.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn to_character_iterator(&self) -> Result<Box<dyn AttributedCharacterIterator>>;
}
