// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.DictionaryBreakEngine`, with its nested
//! classes `PossibleWord` and `DequeI`.

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::r#impl::breakiter::dictionary_matcher::DictionaryMatcher;
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::character_iteration::{self, set_index};
use crate::text::unicode_set::UnicodeSet;

/// The candidate dictionary words at one position of the text.
///
/// Equivalent to the package-private nested class
/// `DictionaryBreakEngine.PossibleWord`.
#[derive(Debug, Clone)]
pub struct PossibleWord {
    /// List of word candidate lengths, in increasing length order.
    lengths: [i32; Self::POSSIBLE_WORD_LIST_MAX],
    /// Count of candidates.
    count: [i32; 1],
    /// The longest match length.
    prefix: i32,
    /// Offset in the text of these candidates.
    offset: i32,
    /// The preferred candidate's offset.
    mark: i32,
    /// The candidate we're currently looking at.
    current: i32,
}

impl Default for PossibleWord {
    fn default() -> Self {
        Self::new()
    }
}

impl PossibleWord {
    /// `POSSIBLE_WORD_LIST_MAX`.
    const POSSIBLE_WORD_LIST_MAX: usize = 20;

    /// `PossibleWord()`.
    pub fn new() -> Self {
        Self {
            lengths: [0; Self::POSSIBLE_WORD_LIST_MAX],
            count: [0],
            prefix: 0,
            offset: -1,
            mark: 0,
            current: 0,
        }
    }

    /// `candidates(CharacterIterator fIter, DictionaryMatcher dict, int
    /// rangeEnd)`: fills the list of candidates if needed, selects the
    /// longest, and returns the number found.
    pub fn candidates(
        &mut self,
        f_iter: &mut dyn CharacterIterator,
        dict: &dyn DictionaryMatcher,
        range_end: i32,
    ) -> i32 {
        let start = f_iter.get_index();
        if start != self.offset {
            self.offset = start;
            let limit = self.lengths.len() as i32;
            self.prefix = dict.matches_no_values(
                f_iter,
                range_end - start,
                &mut self.lengths,
                &mut self.count,
                limit,
            );
            // Dictionary leaves text after longest prefix, not longest word. Back up.
            if self.count[0] <= 0 {
                set_index(f_iter, start);
            }
        }
        if self.count[0] > 0 {
            set_index(f_iter, start + self.lengths[(self.count[0] - 1) as usize]);
        }
        self.current = self.count[0] - 1;
        self.mark = self.current;
        self.count[0]
    }

    /// `acceptMarked(CharacterIterator fIter)`: selects the currently marked
    /// candidate, points the text after it, and returns its length.
    pub fn accept_marked(&mut self, f_iter: &mut dyn CharacterIterator) -> i32 {
        set_index(f_iter, self.offset + self.lengths[self.mark as usize]);
        self.lengths[self.mark as usize]
    }

    /// `backUp(CharacterIterator fIter)`: backs up from the current
    /// candidate to the next shorter one; returns `false` when there is
    /// none.
    pub fn back_up(&mut self, f_iter: &mut dyn CharacterIterator) -> bool {
        if self.current > 0 {
            self.current -= 1;
            set_index(f_iter, self.offset + self.lengths[self.current as usize]);
            return true;
        }
        false
    }

    /// `longestPrefix()`: the longest prefix this candidate location
    /// shares with a dictionary word.
    pub fn longest_prefix(&self) -> i32 {
        self.prefix
    }

    /// `markCurrent()`: marks the current candidate as the one to accept.
    pub fn mark_current(&mut self) {
        self.mark = self.current;
    }
}

/// A deque of `int` positions, used as a stack of found breaks.
///
/// Equivalent to the public static nested class `DictionaryBreakEngine.DequeI`
/// (`Cloneable`: [`Clone`]).
#[derive(Debug, Clone)]
pub struct DequeI {
    data: Vec<i32>,
    last_idx: i32,
    first_idx: i32,
}

impl Default for DequeI {
    fn default() -> Self {
        Self::new()
    }
}

impl DequeI {
    /// `new DequeI()`: capacity 50, both ends at 4.
    pub fn new() -> Self {
        Self {
            data: vec![0; 50],
            last_idx: 4,
            first_idx: 4,
        }
    }

    /// `size()`.
    pub fn size(&self) -> i32 {
        self.first_idx - self.last_idx
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// `grow()`: doubles the capacity.
    fn grow(&mut self) {
        let len = self.data.len();
        self.data.resize(len * 2, 0);
    }

    /// `offer(int v)`: adds `v` at the bottom of the stack.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when the bottom reserve is used up
    /// (the Java `assert lastIdx > 0` runs as a `debug_assert!`).
    pub fn offer(&mut self, v: i32) {
        // Note that the actual use cases of offer() add at most one element.
        //   We make no attempt to handle more than a few.
        debug_assert!(self.last_idx > 0);
        self.last_idx -= 1;
        self.data[self.last_idx as usize] = v;
    }

    /// `push(int v)`.
    pub fn push(&mut self, v: i32) {
        if self.first_idx as usize >= self.data.len() {
            self.grow();
        }
        self.data[self.first_idx as usize] = v;
        self.first_idx += 1;
    }

    /// `pop()`.
    pub fn pop(&mut self) -> i32 {
        debug_assert!(self.size() > 0);
        self.first_idx -= 1;
        self.data[self.first_idx as usize]
    }

    /// `peek()`.
    pub fn peek(&self) -> i32 {
        debug_assert!(self.size() > 0);
        self.data[(self.first_idx - 1) as usize]
    }

    /// `peekLast()`.
    pub fn peek_last(&self) -> i32 {
        debug_assert!(self.size() > 0);
        self.data[self.last_idx as usize]
    }

    /// `pollLast()`.
    pub fn poll_last(&mut self) -> i32 {
        debug_assert!(self.size() > 0);
        let v = self.data[self.last_idx as usize];
        self.last_idx += 1;
        v
    }

    /// `contains(int v)`.
    pub fn contains(&self, v: i32) -> bool {
        (self.last_idx..self.first_idx).any(|i| self.data[i as usize] == v)
    }

    /// `elementAt(int i)`.
    pub fn element_at(&self, i: i32) -> i32 {
        debug_assert!(i < self.size());
        self.data[(self.last_idx + i) as usize]
    }

    /// `removeAllElements()`.
    pub fn remove_all_elements(&mut self) {
        self.last_idx = 4;
        self.first_idx = 4;
    }
}

/// The state shared by every dictionary-based engine: the set of
/// characters it handles.
///
/// Equivalent to the field `UnicodeSet fSet` of the abstract class
/// `DictionaryBreakEngine`, with `setCharacters`.
#[derive(Debug, Clone)]
pub struct DictionaryBreakEngineBase {
    /// `fSet`: the set of characters handled by this engine.
    pub f_set: UnicodeSet,
}

impl Default for DictionaryBreakEngineBase {
    fn default() -> Self {
        Self::new()
    }
}

impl DictionaryBreakEngineBase {
    /// `DictionaryBreakEngine()`: `fSet = new UnicodeSet()`.
    pub fn new() -> Self {
        Self {
            f_set: UnicodeSet::new(),
        }
    }

    /// `setCharacters(UnicodeSet set)`: an unfrozen, compacted copy of `set`.
    ///
    /// # Errors
    ///
    /// As `UnicodeSet.compact()`.
    pub fn set_characters(&mut self, set: &UnicodeSet) -> Result<()> {
        self.f_set = UnicodeSet::from_set(set);
        self.f_set.compact()?;
        Ok(())
    }
}

/// The abstract part of a dictionary-based engine.
///
/// Equivalent to the abstract class
/// `com.ibm.icu.impl.breakiter.DictionaryBreakEngine`: an implementation
/// supplies its [`DictionaryBreakEngineBase`] and
/// `divideUpDictionaryRange`; its `LanguageBreakEngine` implementation
/// calls [`dictionary_handles`] and [`dictionary_find_breaks`], the bodies
/// of `DictionaryBreakEngine.handles` and `findBreaks`, unless it overrides
/// them as Java's subclasses do.
pub trait DictionaryBreakEngine: LanguageBreakEngine {
    /// The `DictionaryBreakEngine` state of this engine.
    fn base(&self) -> &DictionaryBreakEngineBase;

    /// `divideUpDictionaryRange(CharacterIterator text, int rangeStart, int
    /// rangeEnd, DequeI foundBreaks, boolean isPhraseBreaking)`: divides up
    /// a range of dictionary characters, pushing the breaks found onto
    /// `found_breaks` and returning their number.
    fn divide_up_dictionary_range(
        &self,
        text: &mut dyn CharacterIterator,
        range_start: i32,
        range_end: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32;
}

/// `UnicodeSet.contains(int c)` on the infallible iteration paths of the
/// engines.
///
/// # Panics
///
/// With the `IllegalArgumentException` text for an invalid code point (for
/// example `DONE32`): Java's exception propagates out of the iteration
/// methods, whose signatures are infallible.
pub(crate) fn uset_contains(set: &UnicodeSet, c: i32) -> bool {
    set.contains(c).unwrap_or_else(|e| panic!("{e}"))
}

/// `DictionaryBreakEngine.handles(int c)`: `fSet.contains(c)`.
pub fn dictionary_handles<E: DictionaryBreakEngine + ?Sized>(engine: &E, c: i32) -> bool {
    uset_contains(&engine.base().f_set, c)
}

/// `DictionaryBreakEngine.findBreaks(CharacterIterator text, int startPos,
/// int endPos, DequeI foundBreaks, boolean isPhraseBreaking)`.
pub fn dictionary_find_breaks<E: DictionaryBreakEngine + ?Sized>(
    engine: &E,
    text: &mut dyn CharacterIterator,
    _start_pos: i32,
    end_pos: i32,
    found_breaks: &mut DequeI,
    is_phrase_breaking: bool,
) -> i32 {
    // Find the span of characters included in the set.
    //   The span to break begins at the current position int the text, and
    //   extends towards the start or end of the text, depending on 'reverse'.

    let start = text.get_index();
    let mut current;
    let mut c = character_iteration::current32(text);
    loop {
        current = text.get_index();
        if !(current < end_pos && uset_contains(&engine.base().f_set, c)) {
            break;
        }
        character_iteration::next32(text);
        c = character_iteration::current32(text);
    }
    let range_start = start;
    let range_end = current;

    // if (break type is not one we handle) return 0;
    let result = engine.divide_up_dictionary_range(
        text,
        range_start,
        range_end,
        found_breaks,
        is_phrase_breaking,
    );
    set_index(text, current);

    result
}
