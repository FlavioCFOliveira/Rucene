// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ReplaceableUCharacterIterator`.

use std::sync::{Arc, Mutex};

use crate::text::replaceable::Replaceable;
use crate::text::replaceable_string::ReplaceableString;
use crate::text::u_character_iterator::{UCharacterIterator, DONE};
use crate::text::utf16::UTF16;

/// A [`UCharacterIterator`] over a [`Replaceable`], which it shares with
/// its creator and its clones.
///
/// Equivalent to `com.ibm.icu.impl.ReplaceableUCharacterIterator`. Java's
/// constructors throw `IllegalArgumentException` for a `null` argument,
/// which cannot be passed here.
#[derive(Debug, Clone)]
pub struct ReplaceableUCharacterIterator {
    replaceable: Arc<dyn Replaceable>,
    current_index: i32,
}

impl ReplaceableUCharacterIterator {
    /// `ReplaceableUCharacterIterator(Replaceable replaceable)`.
    pub fn from_replaceable(replaceable: Arc<dyn Replaceable>) -> Self {
        Self {
            replaceable,
            current_index: 0,
        }
    }

    /// `ReplaceableUCharacterIterator(String str)`.
    pub fn from_string(s: &[u16]) -> Self {
        Self {
            replaceable: Arc::new(ReplaceableString::from_str_units(s)),
            current_index: 0,
        }
    }

    /// `ReplaceableUCharacterIterator(StringBuffer buf)`: over the shared
    /// buffer.
    pub fn from_string_buffer(buf: Arc<Mutex<Vec<u16>>>) -> Self {
        Self {
            replaceable: Arc::new(ReplaceableString::from_string_buffer(buf)),
            current_index: 0,
        }
    }
}

impl UCharacterIterator for ReplaceableUCharacterIterator {
    /// `clone()`: a shallow copy sharing the replaceable.
    fn clone_box(&self) -> Box<dyn UCharacterIterator> {
        Box::new(self.clone())
    }

    fn current(&self) -> i32 {
        if self.current_index < self.replaceable.length() {
            return self.replaceable.char_at(self.current_index) as i32;
        }
        DONE
    }

    fn current_code_point(&mut self) -> i32 {
        // cannot use charAt due to it different
        // behaviour when index is pointing at a
        // trail surrogate, check for surrogates
        let ch = self.current();
        if UTF16::is_lead_surrogate(ch) {
            // advance the index to get the next code point
            self.next();
            // due to post increment semantics current() after next()
            // actually returns the next char which is what we want
            let ch2 = self.current();
            // current should never change the current index so back off
            self.previous();

            if UTF16::is_trail_surrogate(ch2) {
                // we found a surrogate pair
                return crate::jdk::to_code_point(ch as u16, ch2 as u16);
            }
        }
        ch
    }

    fn get_length(&self) -> i32 {
        self.replaceable.length()
    }

    fn get_index(&self) -> i32 {
        self.current_index
    }

    fn next(&mut self) -> i32 {
        if self.current_index < self.replaceable.length() {
            let c = self.replaceable.char_at(self.current_index);
            self.current_index += 1;
            return c as i32;
        }
        DONE
    }

    fn previous(&mut self) -> i32 {
        if self.current_index > 0 {
            self.current_index -= 1;
            return self.replaceable.char_at(self.current_index) as i32;
        }
        DONE
    }

    fn set_index(&mut self, current_index: i32) {
        if current_index < 0 || current_index > self.replaceable.length() {
            crate::jdk::index_out_of_bounds(None);
        }
        self.current_index = current_index;
    }

    fn get_text_into(&mut self, fill_in: &mut [u16], offset: i32) -> i32 {
        let length = self.replaceable.length();
        if offset < 0 || offset.wrapping_add(length) > fill_in.len() as i32 {
            crate::jdk::index_out_of_bounds(Some(length.to_string()));
        }
        self.replaceable.get_chars(0, length, fill_in, offset);
        length
    }
}
