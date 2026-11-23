// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharacterIteratorWrapper`.
//!
//! Not in the closure of FINDING 4694 on rmp #640, but the result of
//! `UCharacterIterator.getCharacterIterator()`, which the whole-file port
//! of `UCharacterIterator` (rmp #643) includes; ported with it.

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::text::u_character_iterator::UCharacterIterator;

/// A `java.text.CharacterIterator` over a [`UCharacterIterator`].
///
/// Equivalent to `com.ibm.icu.impl.UCharacterIteratorWrapper`. Each method
/// narrows the iterator's `int` result to `char`, so its `DONE` (-1)
/// becomes `CharacterIterator.DONE` (`￿`).
#[derive(Debug)]
pub struct UCharacterIteratorWrapper {
    iterator: Box<dyn UCharacterIterator>,
}

impl UCharacterIteratorWrapper {
    /// `UCharacterIteratorWrapper(UCharacterIterator iter)`.
    pub fn new(iter: Box<dyn UCharacterIterator>) -> Self {
        Self { iterator: iter }
    }
}

impl CharacterIterator for UCharacterIteratorWrapper {
    fn first(&mut self) -> u16 {
        //UCharacterIterator always iterates from 0 to length
        self.iterator.set_to_start();
        self.iterator.current() as u16
    }

    fn last(&mut self) -> u16 {
        self.iterator.set_to_limit();
        self.iterator.previous() as u16
    }

    fn current(&self) -> u16 {
        self.iterator.current() as u16
    }

    fn next(&mut self) -> u16 {
        //pre-increment
        self.iterator.next();
        self.iterator.current() as u16
    }

    fn previous(&mut self) -> u16 {
        //pre-decrement
        self.iterator.previous() as u16
    }

    /// `setIndex(int position)`: the iterator's `IndexOutOfBoundsException`
    /// propagates (a panic), as in Java, where `CharacterIterator`'s
    /// contract would have an `IllegalArgumentException`.
    fn set_index(&mut self, position: i32) -> Result<u16> {
        self.iterator.set_index(position);
        Ok(self.iterator.current() as u16)
    }

    fn get_begin_index(&self) -> i32 {
        //UCharacterIterator always starts from 0
        0
    }

    fn get_end_index(&self) -> i32 {
        self.iterator.get_length()
    }

    fn get_index(&self) -> i32 {
        self.iterator.get_index()
    }

    /// `clone()`: a copy holding a clone of the wrapped iterator.
    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(UCharacterIteratorWrapper {
            iterator: self.iterator.clone_box(),
        })
    }
}
