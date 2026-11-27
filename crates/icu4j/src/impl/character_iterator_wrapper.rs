// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CharacterIteratorWrapper`.

use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE as CI_DONE};

use crate::text::u_character_iterator::{UCharacterIterator, DONE};

/// A [`UCharacterIterator`] over a `java.text.CharacterIterator`.
///
/// Equivalent to `com.ibm.icu.impl.CharacterIteratorWrapper`. Java's
/// constructor throws `IllegalArgumentException` for a `null` iterator,
/// which cannot be passed here.
#[derive(Debug)]
pub struct CharacterIteratorWrapper {
    iterator: Box<dyn CharacterIterator>,
}

impl CharacterIteratorWrapper {
    /// `CharacterIteratorWrapper(CharacterIterator iter)`.
    pub fn new(iter: Box<dyn CharacterIterator>) -> Self {
        Self { iterator: iter }
    }
}

impl UCharacterIterator for CharacterIteratorWrapper {
    /// `clone()`: a copy holding a clone of the wrapped iterator.
    fn clone_box(&self) -> Box<dyn UCharacterIterator> {
        Box::new(CharacterIteratorWrapper {
            iterator: self.iterator.clone_box(),
        })
    }

    fn current(&self) -> i32 {
        let c = self.iterator.current();
        if c == CI_DONE {
            return DONE;
        }
        c as i32
    }

    fn get_length(&self) -> i32 {
        self.iterator.get_end_index() - self.iterator.get_begin_index()
    }

    fn get_index(&self) -> i32 {
        self.iterator.get_index()
    }

    fn next(&mut self) -> i32 {
        let i = self.iterator.current();
        self.iterator.next();
        if i == CI_DONE {
            return DONE;
        }
        i as i32
    }

    fn previous(&mut self) -> i32 {
        let i = self.iterator.previous();
        if i == CI_DONE {
            return DONE;
        }
        i as i32
    }

    /// `setIndex(int index)`: the wrapped iterator's
    /// `IllegalArgumentException` becomes an `IndexOutOfBoundsException`.
    fn set_index(&mut self, index: i32) {
        if self.iterator.set_index(index).is_err() {
            crate::jdk::index_out_of_bounds(None);
        }
    }

    fn set_to_limit(&mut self) {
        let end = self.iterator.get_end_index();
        self.iterator
            .set_index(end)
            .unwrap_or_else(|e| panic!("{e}"));
    }

    fn get_text_into(&mut self, fill_in: &mut [u16], mut offset: i32) -> i32 {
        let length = self.iterator.get_end_index() - self.iterator.get_begin_index();
        let current_index = self.iterator.get_index();
        if offset < 0 || offset.wrapping_add(length) > fill_in.len() as i32 {
            crate::jdk::index_out_of_bounds(Some(length.to_string()));
        }

        let mut ch = self.iterator.first();
        while ch != CI_DONE {
            fill_in[offset as usize] = ch;
            offset += 1;
            ch = self.iterator.next();
        }
        self.iterator
            .set_index(current_index)
            .unwrap_or_else(|e| panic!("{e}"));

        length
    }

    /// `moveIndex(int delta)`: pinned to `0..=length`, with the value the
    /// wrapped iterator's `setIndex` returns (the unit there).
    fn move_index(&mut self, delta: i32) -> i32 {
        let length = self.iterator.get_end_index() - self.iterator.get_begin_index();
        let mut idx = self.iterator.get_index().wrapping_add(delta);

        if idx < 0 {
            idx = 0;
        } else if idx > length {
            idx = length;
        }
        self.iterator
            .set_index(idx)
            .unwrap_or_else(|e| panic!("{e}")) as i32
    }

    /// `getCharacterIterator()`: a clone of the wrapped iterator.
    fn get_character_iterator(&self) -> Box<dyn CharacterIterator> {
        self.iterator.clone_box()
    }
}
