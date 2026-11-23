// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UCharacterIterator`.

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::r#impl::character_iterator_wrapper::CharacterIteratorWrapper;
use crate::r#impl::replaceable_u_character_iterator::ReplaceableUCharacterIterator;
use crate::r#impl::u_char_array_iterator::UCharArrayIterator;
use crate::r#impl::u_character_iterator_wrapper::UCharacterIteratorWrapper;
use crate::text::replaceable::Replaceable;
pub use crate::text::u_forward_character_iterator::DONE;
use crate::text::u_forward_character_iterator::UForwardCharacterIterator;
use crate::text::utf16::UTF16;

/// Abstract class that defines an API for iteration on text objects: a
/// bidirectional iterator over UTF-16 code units and code points with an
/// index from 0 to the length.
///
/// Equivalent to the abstract class `com.ibm.icu.text.UCharacterIterator`,
/// which implements `UForwardCharacterIterator` (every implementation of
/// this trait is one, see the blanket implementation) and `Cloneable`
/// ([`clone_box`](Self::clone_box)). The static factories `getInstance(...)`
/// are the functions of this module.
pub trait UCharacterIterator: Debug + Send + Sync {
    /// `clone()`: a copy of this iterator.
    fn clone_box(&self) -> Box<dyn UCharacterIterator>;

    /// `getCharacterIterator()`: a `java.text.CharacterIterator` over the
    /// same text and index.
    fn get_character_iterator(&self) -> Box<dyn CharacterIterator> {
        Box::new(UCharacterIteratorWrapper::new(self.clone_box()))
    }

    /// `current()`: the code unit at the index, [`DONE`] at the end.
    fn current(&self) -> i32;

    /// `currentCodePoint()`: the code point at the index; a lead surrogate
    /// followed by a trail one gives the supplementary code point, any
    /// other surrogate is returned as is. The index does not move.
    fn current_code_point(&mut self) -> i32 {
        let ch = self.current();
        if UTF16::is_lead_surrogate(ch) {
            // advance the index to get the
            // next code point
            self.next();
            // due to post increment semantics
            // current() after next() actually
            // returns the char we want
            let ch2 = self.current();
            // current should never change
            // the current index so back off
            self.previous();

            if UTF16::is_trail_surrogate(ch2) {
                // we found a surrogate pair
                // return the codepoint
                return crate::jdk::to_code_point(ch as u16, ch2 as u16);
            }
        }
        ch
    }

    /// `getLength()`: the length of the text.
    fn get_length(&self) -> i32;

    /// `getIndex()`: the current index.
    fn get_index(&self) -> i32;

    /// `next()`: the code unit at the index, then increments it; [`DONE`]
    /// at the end.
    fn next(&mut self) -> i32;

    /// `nextCodePoint()`: the code point at the index, then moves past it.
    fn next_code_point(&mut self) -> i32 {
        let ch1 = self.next();
        if UTF16::is_lead_surrogate(ch1) {
            let ch2 = self.next();
            if UTF16::is_trail_surrogate(ch2) {
                return crate::jdk::to_code_point(ch1 as u16, ch2 as u16);
            } else if ch2 != DONE {
                // unmatched surrogate so back out
                self.previous();
            }
        }
        ch1
    }

    /// `previous()`: decrements the index and returns the code unit there;
    /// [`DONE`] at the start.
    fn previous(&mut self) -> i32;

    /// `previousCodePoint()`: moves back over one code point and returns it.
    fn previous_code_point(&mut self) -> i32 {
        let ch1 = self.previous();
        if UTF16::is_trail_surrogate(ch1) {
            let ch2 = self.previous();
            if UTF16::is_lead_surrogate(ch2) {
                return crate::jdk::to_code_point(ch2 as u16, ch1 as u16);
            } else if ch2 != DONE {
                // unmatched trail surrogate so back out
                self.next();
            }
        }
        ch1
    }

    /// `setIndex(int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `index` is outside
    /// `0..=getLength()`.
    fn set_index(&mut self, index: i32);

    /// `setToLimit()`: sets the index to the length.
    fn set_to_limit(&mut self) {
        let len = self.get_length();
        self.set_index(len);
    }

    /// `setToStart()`: sets the index to 0.
    fn set_to_start(&mut self) {
        self.set_index(0);
    }

    /// `getText(char[] fillIn, int offset)`: copies the whole text into
    /// `fill_in` at `offset` and returns its length.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when the text does not fit.
    fn get_text_into(&mut self, fill_in: &mut [u16], offset: i32) -> i32;

    /// `getText(char[] fillIn)` (final): [`get_text_into`](Self::get_text_into)
    /// at offset 0.
    fn get_text_fill(&mut self, fill_in: &mut [u16]) -> i32 {
        self.get_text_into(fill_in, 0)
    }

    /// `getText()`: the whole text.
    fn get_text(&mut self) -> Vec<u16> {
        let mut text = vec![0u16; self.get_length() as usize];
        self.get_text_fill(&mut text);
        text
    }

    /// `moveIndex(int delta)`: moves the index by `delta` code units,
    /// pinned to `0..=getLength()`, and returns the new index.
    fn move_index(&mut self, delta: i32) -> i32 {
        let x = 0.max((self.get_index().wrapping_add(delta)).min(self.get_length()));
        self.set_index(x);
        x
    }

    /// `moveCodePointIndex(int delta)`: moves the index by `delta` code
    /// points and returns the new index.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when the text ends before `delta` code
    /// points.
    fn move_code_point_index(&mut self, mut delta: i32) -> i32 {
        if delta > 0 {
            while delta > 0 && self.next_code_point() != DONE {
                delta -= 1;
            }
        } else {
            while delta < 0 && self.previous_code_point() != DONE {
                delta += 1;
            }
        }
        if delta != 0 {
            crate::jdk::index_out_of_bounds(None);
        }

        self.get_index()
    }
}

impl<T: UCharacterIterator + ?Sized> UForwardCharacterIterator for T {
    fn next(&mut self) -> i32 {
        UCharacterIterator::next(self)
    }

    fn next_code_point(&mut self) -> i32 {
        UCharacterIterator::next_code_point(self)
    }
}

impl Clone for Box<dyn UCharacterIterator> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// `getInstance(Replaceable source)`: an iterator over `source`, which it
/// shares.
pub fn get_instance_replaceable(source: Arc<dyn Replaceable>) -> Box<dyn UCharacterIterator> {
    Box::new(ReplaceableUCharacterIterator::from_replaceable(source))
}

/// `getInstance(String source)`.
pub fn get_instance_string(source: &[u16]) -> Box<dyn UCharacterIterator> {
    Box::new(ReplaceableUCharacterIterator::from_string(source))
}

/// `getInstance(char[] source)`.
pub fn get_instance_chars(source: Arc<[u16]>) -> Box<dyn UCharacterIterator> {
    let len = source.len() as i32;
    get_instance_chars_range(source, 0, len).expect("the whole array is a valid range")
}

/// `getInstance(char[] source, int start, int limit)`.
///
/// # Errors
///
/// `IllegalArgumentException` when `start..limit` is not a range of the
/// array.
pub fn get_instance_chars_range(
    source: Arc<[u16]>,
    start: i32,
    limit: i32,
) -> Result<Box<dyn UCharacterIterator>> {
    Ok(Box::new(UCharArrayIterator::new(source, start, limit)?))
}

/// `getInstance(StringBuffer source)`: an iterator over the shared buffer.
pub fn get_instance_string_buffer(source: Arc<Mutex<Vec<u16>>>) -> Box<dyn UCharacterIterator> {
    Box::new(ReplaceableUCharacterIterator::from_string_buffer(source))
}

/// `getInstance(CharacterIterator source)`.
pub fn get_instance_character_iterator(source: Box<dyn CharacterIterator>) -> Box<dyn UCharacterIterator> {
    Box::new(CharacterIteratorWrapper::new(source))
}
