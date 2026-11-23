// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharArrayIterator`.
//!
//! Not in the closure of FINDING 4694 on rmp #640, but the target of
//! `UCharacterIterator.getInstance(char[], int, int)`, which the whole-file
//! port of `UCharacterIterator` (rmp #643) includes; ported with it.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::text::u_character_iterator::{UCharacterIterator, DONE};

/// A [`UCharacterIterator`] over a range of a `char[]`.
///
/// Equivalent to `com.ibm.icu.impl.UCharArrayIterator`. The array is shared
/// with the creator, as Java keeps the caller's array.
#[derive(Debug, Clone)]
pub struct UCharArrayIterator {
    text: Arc<[u16]>,
    start: i32,
    limit: i32,
    pos: i32,
}

impl UCharArrayIterator {
    /// `UCharArrayIterator(char[] text, int start, int limit)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("start: <start> or limit: <limit> out of
    /// range [0, <length>)")` for an invalid range.
    pub fn new(text: Arc<[u16]>, start: i32, limit: i32) -> Result<Self> {
        if start < 0 || limit > text.len() as i32 || start > limit {
            return Err(LuceneError::illegal_argument(format!(
                "start: {} or limit: {} out of range [0, {})",
                start,
                limit,
                text.len()
            )));
        }
        Ok(Self {
            text,
            start,
            limit,
            pos: start,
        })
    }
}

impl UCharacterIterator for UCharArrayIterator {
    /// `clone()`: a shallow copy sharing the array.
    fn clone_box(&self) -> Box<dyn UCharacterIterator> {
        Box::new(self.clone())
    }

    fn current(&self) -> i32 {
        if self.pos < self.limit {
            self.text[self.pos as usize] as i32
        } else {
            DONE
        }
    }

    fn get_length(&self) -> i32 {
        self.limit - self.start
    }

    fn get_index(&self) -> i32 {
        self.pos - self.start
    }

    fn next(&mut self) -> i32 {
        if self.pos < self.limit {
            let c = self.text[self.pos as usize] as i32;
            self.pos += 1;
            c
        } else {
            DONE
        }
    }

    fn previous(&mut self) -> i32 {
        if self.pos > self.start {
            self.pos -= 1;
            self.text[self.pos as usize] as i32
        } else {
            DONE
        }
    }

    fn set_index(&mut self, index: i32) {
        if index < 0 || index > self.limit - self.start {
            crate::jdk::index_out_of_bounds(Some(format!(
                "index: {} out of range [0, {})",
                index,
                self.limit - self.start
            )));
        }
        self.pos = self.start + index;
    }

    fn get_text_into(&mut self, fill_in: &mut [u16], offset: i32) -> i32 {
        let len = self.limit - self.start;
        let o = offset as usize;
        fill_in[o..o + len as usize]
            .copy_from_slice(&self.text[self.start as usize..self.limit as usize]);
        len
    }
}
