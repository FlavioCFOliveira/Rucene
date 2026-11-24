// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CodePointMap`.

use rucene::error::Result;

use crate::util::code_point_trie::CodePointTrie;

/// Selectors for how `getRange` should report value ranges overlapping
/// with surrogates.
///
/// Equivalent to the enum `com.ibm.icu.util.CodePointMap.RangeOption`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOption {
    /// `NORMAL`: `getRange` enumerates all same-value ranges as stored in
    /// the map.
    Normal,
    /// `FIXED_LEAD_SURROGATES`: lead surrogates (U+D800..U+DBFF) are
    /// treated as having the surrogate value.
    FixedLeadSurrogates,
    /// `FIXED_ALL_SURROGATES`: all surrogates (U+D800..U+DFFF) are treated
    /// as having the surrogate value.
    FixedAllSurrogates,
}

/// `ValueFilter`: modifies map values while enumerating ranges.
///
/// Equivalent to the interface `com.ibm.icu.util.CodePointMap.ValueFilter`.
pub type ValueFilter<'f> = &'f dyn Fn(i32) -> i32;

/// Range iteration result data: the code points `start..=end` all have
/// `value`.
///
/// Equivalent to `com.ibm.icu.util.CodePointMap.Range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub(crate) start: i32,
    pub(crate) end: i32,
    pub(crate) value: i32,
}

impl Default for Range {
    fn default() -> Self {
        Self::new()
    }
}

impl Range {
    /// `Range()`: start and end -1, value 0.
    pub fn new() -> Self {
        Self {
            start: -1,
            end: -1,
            value: 0,
        }
    }

    /// `getStart()`.
    pub fn get_start(&self) -> i32 {
        self.start
    }

    /// `getEnd()`: inclusive.
    pub fn get_end(&self) -> i32 {
        self.end
    }

    /// `getValue()`.
    pub fn get_value(&self) -> i32 {
        self.value
    }

    /// `set(int start, int end, int value)`.
    pub fn set(&mut self, start: i32, end: i32, value: i32) {
        self.start = start;
        self.end = end;
        self.value = value;
    }
}

/// An iterator over the same-value ranges of a map.
///
/// Equivalent to `CodePointMap.RangeIterator` (private inner class). Each
/// step returns a copy of the range the Java iterator overwrites.
pub struct RangeIterator<'a> {
    map: &'a dyn CodePointMap,
    range: Range,
}

impl<'a> RangeIterator<'a> {
    /// A new iterator over `map`, from code point 0.
    pub fn new(map: &'a dyn CodePointMap) -> Self {
        Self {
            map,
            range: Range::new(),
        }
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        -1 <= self.range.end && self.range.end < 0x10ffff
    }

    /// `remove()` (final): unsupported.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException`.
    pub fn remove(&mut self) -> Result<()> {
        Err(crate::jdk::unchecked_no_message(
            "java.lang.UnsupportedOperationException",
        ))
    }
}

impl Iterator for RangeIterator<'_> {
    type Item = Range;

    /// `next()`; `None` where Java throws `NoSuchElementException`.
    fn next(&mut self) -> Option<Range> {
        if !self.has_next() {
            return None;
        }
        let start = self.range.end + 1;
        if self.map.get_range(start, None, &mut self.range) {
            Some(self.range)
        } else {
            None
        }
    }
}

/// How a [`StringIterator`] looks values up: the map's `get`, or the
/// overrides of `CodePointTrie.Fast` and `CodePointTrie.Small`.
#[derive(Clone, Copy)]
pub(crate) enum StringIteratorKind<'a> {
    /// `CodePointMap.StringIterator` itself.
    Map(&'a dyn CodePointMap),
    /// `CodePointTrie.Fast.FastStringIterator`.
    Fast(&'a CodePointTrie),
    /// `CodePointTrie.Small.SmallStringIterator`.
    Small(&'a CodePointTrie),
}

/// Iterates over code points of a string and fetches map values.
///
/// Equivalent to `com.ibm.icu.util.CodePointMap.StringIterator` (an inner
/// class of the map) and its subclasses `FastStringIterator` and
/// `SmallStringIterator` in `CodePointTrie`, which [`StringIteratorKind`]
/// selects.
pub struct StringIterator<'a> {
    kind: StringIteratorKind<'a>,
    /// `s` (deprecated, protected).
    pub(crate) s: &'a [u16],
    /// `sIndex` (deprecated, protected).
    pub(crate) s_index: i32,
    /// `c` (deprecated, protected).
    pub(crate) c: i32,
    /// `value` (deprecated, protected).
    pub(crate) value: i32,
}

impl<'a> StringIterator<'a> {
    /// `StringIterator(CharSequence s, int sIndex)` (protected).
    pub(crate) fn new(kind: StringIteratorKind<'a>, s: &'a [u16], s_index: i32) -> Self {
        Self {
            kind,
            s,
            s_index,
            c: -1,
            value: 0,
        }
    }

    /// `reset(CharSequence s, int sIndex)`.
    pub fn reset(&mut self, s: &'a [u16], s_index: i32) {
        self.s = s;
        self.s_index = s_index;
        self.c = -1;
        self.value = 0;
    }

    /// `next()`: reads the code point at the string index and its value,
    /// and moves past it; false at the end of the string.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        match self.kind {
            StringIteratorKind::Map(map) => {
                if self.s_index >= self.s.len() as i32 {
                    return false;
                }
                self.c = crate::jdk::code_point_at(self.s, self.s_index);
                self.s_index += crate::jdk::char_count(self.c);
                self.value = map.get(self.c);
                true
            }
            StringIteratorKind::Fast(trie) => trie.fast_string_iterator_next(self),
            StringIteratorKind::Small(trie) => trie.small_string_iterator_next(self),
        }
    }

    /// `previous()`: reads the code point before the string index and its
    /// value, and moves before it; false at the start of the string.
    pub fn previous(&mut self) -> bool {
        match self.kind {
            StringIteratorKind::Map(map) => {
                if self.s_index <= 0 {
                    return false;
                }
                self.c = code_point_before(self.s, self.s_index);
                self.s_index -= crate::jdk::char_count(self.c);
                self.value = map.get(self.c);
                true
            }
            StringIteratorKind::Fast(trie) => trie.fast_string_iterator_previous(self),
            StringIteratorKind::Small(trie) => trie.small_string_iterator_previous(self),
        }
    }

    /// `getIndex()` (final): the string index.
    pub fn get_index(&self) -> i32 {
        self.s_index
    }

    /// `getCodePoint()` (final): the code point of the last step.
    pub fn get_code_point(&self) -> i32 {
        self.c
    }

    /// `getValue()` (final): the map value of the last step.
    pub fn get_value(&self) -> i32 {
        self.value
    }
}

/// `Character.codePointBefore(CharSequence, int)` for `index >= 1`.
pub(crate) fn code_point_before(seq: &[u16], index: i32) -> i32 {
    let mut i = index as usize - 1;
    let c2 = seq[i];
    if crate::jdk::is_low_surrogate(c2) && i > 0 {
        i -= 1;
        let c1 = seq[i];
        if crate::jdk::is_high_surrogate(c1) {
            return crate::jdk::to_code_point(c1, c2);
        }
    }
    c2 as i32
}

/// Abstract map from Unicode code points (U+0000..U+10FFFF) to integer
/// values.
///
/// Equivalent to the abstract class `com.ibm.icu.util.CodePointMap`, which
/// implements `Iterable<Range>` ([`iterator`](Self::iterator)).
/// `iterator()` and `stringIterator(...)` are implemented in Java by the
/// base class; here each implementation provides them (a default method
/// cannot hand out `self` as a trait object), with Java's behaviour:
/// [`RangeIterator::new`] and a map-kind [`StringIterator`].
pub trait CodePointMap: Send + Sync {
    /// `get(int c)`: the value for a code point; the error value for an
    /// out-of-range code point.
    fn get(&self, c: i32) -> i32;

    /// `getRange(int start, ValueFilter filter, Range range)`: sets `range`
    /// to the range of equal (filtered) values from `start`; false when
    /// `start` is not a code point.
    fn get_range(&self, start: i32, filter: Option<ValueFilter<'_>>, range: &mut Range) -> bool;

    /// `getRange(int start, RangeOption option, int surrogateValue,
    /// ValueFilter filter, Range range)`: as
    /// [`get_range`](Self::get_range), with the surrogates treated as having
    /// `surrogate_value` per `option`.
    fn get_range_with_option(
        &self,
        start: i32,
        option: RangeOption,
        surrogate_value: i32,
        filter: Option<ValueFilter<'_>>,
        range: &mut Range,
    ) -> bool {
        if !self.get_range(start, filter, range) {
            return false;
        }
        if option == RangeOption::Normal {
            return true;
        }
        let surr_end = if option == RangeOption::FixedAllSurrogates {
            0xdfff
        } else {
            0xdbff
        };
        let end = range.end;
        if end < 0xd7ff || start > surr_end {
            return true;
        }
        // The range overlaps with surrogates, or ends just before the first one.
        if range.value == surrogate_value {
            if end >= surr_end {
                // Surrogates followed by a non-surrogateValue range,
                // or surrogates are part of a larger surrogateValue range.
                return true;
            }
        } else {
            if start <= 0xd7ff {
                range.end = 0xd7ff; // Non-surrogateValue range ends before surrogateValue surrogates.
                return true;
            }
            // Start is a surrogate with a non-surrogateValue code *unit* value.
            // Return a surrogateValue code *point* range.
            range.value = surrogate_value;
            if end > surr_end {
                range.end = surr_end; // Surrogate range ends before non-surrogateValue rest of range.
                return true;
            }
        }
        // See if the surrogateValue surrogate range can be merged with
        // an immediately following range.
        if self.get_range(surr_end + 1, filter, range) && range.value == surrogate_value {
            range.start = start;
            return true;
        }
        range.start = start;
        range.end = surr_end;
        range.value = surrogate_value;
        true
    }

    /// `iterator()`: the same-value ranges from code point 0.
    fn iterator(&self) -> RangeIterator<'_>;

    /// `stringIterator(CharSequence s, int sIndex)`: an iterator over the
    /// code points of `s` from `s_index` with their values.
    fn string_iterator<'a>(&'a self, s: &'a [u16], s_index: i32) -> StringIterator<'a>;
}
