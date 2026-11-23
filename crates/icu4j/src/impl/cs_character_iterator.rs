// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CSCharacterIterator`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::jdk::illegal_argument_no_message;

/// `CharacterIterator.DONE`.
const DONE: u16 = 0xFFFF;

/// Implement the Java CharacterIterator interface on a CharSequence.
/// Intended for internal use by ICU only.
///
/// Equivalent to `com.ibm.icu.impl.CSCharacterIterator` (ported early by
/// rmp task #645 for `BreakIterator.setText(CharSequence)`; owner #647).
/// Java's `NullPointerException` for a null text cannot occur.
#[derive(Debug, Clone)]
pub struct CSCharacterIterator {
    index: i32,
    seq: Arc<[u16]>,
}

impl CSCharacterIterator {
    /// `CSCharacterIterator(CharSequence text)`.
    pub fn new(text: Arc<[u16]>) -> Self {
        Self { seq: text, index: 0 }
    }
}

impl CharacterIterator for CSCharacterIterator {
    fn first(&mut self) -> u16 {
        self.index = 0;
        self.current()
    }

    fn last(&mut self) -> u16 {
        self.index = self.seq.len() as i32;
        self.previous()
    }

    fn current(&self) -> u16 {
        if self.index == self.seq.len() as i32 {
            return DONE;
        }
        self.seq[self.index as usize]
    }

    fn next(&mut self) -> u16 {
        if self.index < self.seq.len() as i32 {
            self.index += 1;
        }
        self.current()
    }

    fn previous(&mut self) -> u16 {
        if self.index == 0 {
            return DONE;
        }
        self.index -= 1;
        self.current()
    }

    fn set_index(&mut self, position: i32) -> Result<u16> {
        if position < 0 || position > self.seq.len() as i32 {
            return Err(illegal_argument_no_message());
        }
        self.index = position;
        Ok(self.current())
    }

    fn get_begin_index(&self) -> i32 {
        0
    }

    fn get_end_index(&self) -> i32 {
        self.seq.len() as i32
    }

    fn get_index(&self) -> i32 {
        self.index
    }

    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        let mut copy = CSCharacterIterator::new(self.seq.clone());
        copy.index = self.index;
        Box::new(copy)
    }
}
