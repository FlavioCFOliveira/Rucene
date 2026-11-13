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

//! `java.text.BreakIterator`, the boundary analysis `SegmentingTokenizerBase`
//! segments its text with.
//!
//! This module declares the emulation point of `java.text.BreakIterator`:
//! the [`BreakIterator`] trait, holding exactly the methods the module's
//! ported classes call (`setText(CharacterIterator)`, `current()`,
//! `next()`, and the constant `DONE`). The JDK's implementations and its
//! factory methods (`getSentenceInstance`, `getWordInstance`) are delivered
//! by the `BreakIterator` emulation task (rmp task #447), which completes this
//! module; until then a caller supplies its own implementation.

use std::fmt::Debug;

use super::character_iterator::CharacterIterator;

/// The value [`BreakIterator::next`] answers when the last boundary has been
/// passed, and [`BreakIterator::current`] when there is none:
/// `BreakIterator.DONE`, `-1`.
pub const DONE: i32 = -1;

/// The location of boundaries in text.
///
/// Equivalent to the abstract class `java.text.BreakIterator` of JDK 25,
/// restricted to the members `lucene/analysis/common`'s ported classes call:
/// an implementation maintains a current position in its text and moves it
/// from boundary to boundary.
///
/// **Divergence.** Java's `BreakIterator` is an abstract class with further
/// members (`first`, `last`, `previous`, `following`, `preceding`,
/// `isBoundary`, `getText`, `setText(String)`, `clone`, and the static
/// factories). They are declared, with their implementations, by the task
/// that emulates the JDK's break iterators (rmp task #447); this trait holds
/// the members the classes ported so far call, so that they compile against
/// the emulation point that task completes.
pub trait BreakIterator: Debug + Send + Sync {
    /// Returns the boundary most recently returned by `next()` (or by any
    /// other movement); the first boundary of the text after `setText`:
    /// `BreakIterator.current()`.
    fn current(&self) -> i32;

    /// Advances to the boundary following the current one and returns it, or
    /// [`DONE`] when the current boundary is the last:
    /// `BreakIterator.next()`.
    fn next(&mut self) -> i32;

    /// Sets a new text to scan, resetting the current position to its first
    /// boundary: `BreakIterator.setText(CharacterIterator)`.
    fn set_text(&mut self, new_text: Box<dyn CharacterIterator>);
}
