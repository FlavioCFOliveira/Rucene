// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.DictionaryMatcher`.

use std::fmt::Debug;

use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE as CI_DONE};

use crate::text::u_character_iterator::DONE;
use crate::text::utf16::UTF16;

/// The interface of a dictionary lookup over a text.
///
/// Equivalent to the package-private abstract class
/// `com.ibm.icu.impl.breakiter.DictionaryMatcher`.
pub trait DictionaryMatcher: Debug + Send + Sync {
    /// `matches(CharacterIterator text, int maxLength, int[] lengths, int[]
    /// count, int limit, int[] values)`: finds the dictionary words that
    /// start at the text's index, at most `max_length` code points long;
    /// stores up to `limit` of their lengths (in code points) in `lengths`
    /// and their values in `values` when given, their number in `count[0]`,
    /// and returns the number of code points read. The text is advanced over
    /// the code points read.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `lengths` or `values` is
    /// shorter than `limit` words need.
    fn matches(
        &self,
        text: &mut dyn CharacterIterator,
        max_length: i32,
        lengths: &mut [i32],
        count: &mut [i32],
        limit: i32,
        values: Option<&mut [i32]>,
    ) -> i32;

    /// `matches(CharacterIterator text, int maxLength, int[] lengths, int[]
    /// count, int limit)`: [`matches`](Self::matches) without values.
    fn matches_no_values(
        &self,
        text: &mut dyn CharacterIterator,
        max_length: i32,
        lengths: &mut [i32],
        count: &mut [i32],
        limit: i32,
    ) -> i32 {
        self.matches(text, max_length, lengths, count, limit, None)
    }

    /// `getType()`: the trie type of the dictionary,
    /// `DictionaryData.TRIE_TYPE_BYTES` or `TRIE_TYPE_UCHARS`.
    fn get_type(&self) -> i32;
}

/// `UCharacterIterator.getInstance(text).nextCodePoint()` over the caller's
/// iterator: the code point at the index, moving past it (the post-increment
/// `CharacterIteratorWrapper.next()` under `UCharacterIterator.nextCodePoint()`),
/// or `UCharacterIterator.DONE` at the end.
///
/// **Divergence.** Java wraps the caller's `CharacterIterator` in a
/// `CharacterIteratorWrapper` that moves it (`BytesDictionaryMatcher.java:57`,
/// `CharsDictionaryMatcher.java:35`); the crate's wrapper owns a boxed
/// iterator and cannot borrow the caller's, so the matcher runs the same two
/// algorithms on the borrowed iterator directly. Same moves, same values.
pub(crate) fn wrapper_next_code_point(text: &mut dyn CharacterIterator) -> i32 {
    let ch1 = wrapper_next(text);
    if UTF16::is_lead_surrogate(ch1) {
        let ch2 = wrapper_next(text);
        if UTF16::is_trail_surrogate(ch2) {
            return crate::jdk::to_code_point(ch1 as u16, ch2 as u16);
        } else if ch2 != DONE {
            // unmatched surrogate so back out
            wrapper_previous(text);
        }
    }
    ch1
}

/// `CharacterIteratorWrapper.next()`: the unit at the index, then advances.
fn wrapper_next(text: &mut dyn CharacterIterator) -> i32 {
    let i = text.current();
    text.next();
    if i == CI_DONE {
        return DONE;
    }
    i as i32
}

/// `CharacterIteratorWrapper.previous()`.
fn wrapper_previous(text: &mut dyn CharacterIterator) -> i32 {
    let i = text.previous();
    if i == CI_DONE {
        return DONE;
    }
    i as i32
}
