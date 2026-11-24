// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UForwardCharacterIterator`.

/// `DONE`: the value [`next`](UForwardCharacterIterator::next) and
/// [`next_code_point`](UForwardCharacterIterator::next_code_point) return at
/// the end of the text.
pub const DONE: i32 = -1;

/// A forward iterator over UTF-16 code units and code points.
///
/// Equivalent to the interface `com.ibm.icu.text.UForwardCharacterIterator`.
pub trait UForwardCharacterIterator {
    /// `next()`: the code unit at the index, then increments it; [`DONE`]
    /// at the end.
    fn next(&mut self) -> i32;

    /// `nextCodePoint()`: the code point at the index, then moves past it;
    /// [`DONE`] at the end. An unpaired surrogate is returned as is.
    fn next_code_point(&mut self) -> i32;
}
