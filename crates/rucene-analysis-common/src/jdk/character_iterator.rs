/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! `java.text.CharacterIterator`, the interface `CharArrayIterator`
//! implements and `BreakIterator.setText` reads.

use std::fmt::Debug;

use rucene::error::Result;

/// The value the iterator answers past either end of its text:
/// `CharacterIterator.DONE`, `'\uFFFF'`.
pub const DONE: u16 = 0xFFFF;

/// Bidirectional iteration over a text of UTF-16 units.
///
/// Equivalent to the interface `java.text.CharacterIterator` of JDK 25: a
/// text with a begin index, an end index (one past the last unit) and a
/// current index, every movement answering the unit it lands on, or [`DONE`]
/// outside the text.
///
/// **Divergence.** `clone()` is [`clone_box`](Self::clone_box), a
/// trait object being unable to return `Self`; an implementation answers
/// with a copy of itself as Java's `clone` does.
pub trait CharacterIterator: Debug + Send + Sync {
    /// Sets the position to [`get_begin_index`](Self::get_begin_index) and
    /// returns the unit there, or [`DONE`] for an empty text:
    /// `CharacterIterator.first()`.
    fn first(&mut self) -> u16;

    /// Sets the position to `getEndIndex() - 1` (`getEndIndex()` for an
    /// empty text) and returns the unit there, or [`DONE`] for an empty
    /// text: `CharacterIterator.last()`.
    fn last(&mut self) -> u16;

    /// Returns the unit at the current position, or [`DONE`] when the
    /// position is off the end of the text: `CharacterIterator.current()`.
    fn current(&self) -> u16;

    /// Increments the position by one and returns the unit there, or
    /// [`DONE`], the position then being the end index, when it reaches the
    /// end: `CharacterIterator.next()`.
    fn next(&mut self) -> u16;

    /// Decrements the position by one and returns the unit there, or
    /// [`DONE`], the position staying the begin index, when it is already
    /// there: `CharacterIterator.previous()`.
    fn previous(&mut self) -> u16;

    /// Sets the position to `position` and returns the unit there:
    /// `CharacterIterator.setIndex(int)`.
    ///
    /// # Errors
    ///
    /// Returns `IllegalArgumentException` (`LuceneErrorKind::IllegalArgument`)
    /// when `position` is outside `getBeginIndex()..=getEndIndex()`.
    fn set_index(&mut self, position: i32) -> Result<u16>;

    /// Returns the start index of the text: `CharacterIterator.getBeginIndex()`.
    fn get_begin_index(&self) -> i32;

    /// Returns the end index of the text, one past its last unit:
    /// `CharacterIterator.getEndIndex()`.
    fn get_end_index(&self) -> i32;

    /// Returns the current index: `CharacterIterator.getIndex()`.
    fn get_index(&self) -> i32;

    /// Returns a copy of this iterator: `CharacterIterator.clone()`.
    fn clone_box(&self) -> Box<dyn CharacterIterator>;
}
