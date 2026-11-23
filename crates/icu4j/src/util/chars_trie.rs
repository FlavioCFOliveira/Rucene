// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CharsTrie`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result as JResult};

use crate::text::utf16::UTF16;
use crate::util::bytes_trie::Result;

/// `valueResults_`.
const VALUE_RESULTS: [Result; 2] = [Result::IntermediateValue, Result::FinalValue];

/// `kMaxBranchLinearSubNodeLength` (package-private).
pub const K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH: i32 = 5;
/// `kMinLinearMatch` (package-private): 0030..003f: linear-match node.
pub const K_MIN_LINEAR_MATCH: i32 = 0x30;
/// `kMaxLinearMatchLength` (package-private).
pub const K_MAX_LINEAR_MATCH_LENGTH: i32 = 0x10;
/// `kMinValueLead` (package-private): 0040..ffff: value node.
pub const K_MIN_VALUE_LEAD: i32 = K_MIN_LINEAR_MATCH + K_MAX_LINEAR_MATCH_LENGTH;
/// `kNodeTypeMask` (package-private).
pub const K_NODE_TYPE_MASK: i32 = K_MIN_VALUE_LEAD - 1;
/// `kValueIsFinal` (package-private).
pub const K_VALUE_IS_FINAL: i32 = 0x8000;
/// `kMaxOneUnitValue` (package-private).
pub const K_MAX_ONE_UNIT_VALUE: i32 = 0x3fff;
/// `kMinTwoUnitValueLead` (package-private).
pub const K_MIN_TWO_UNIT_VALUE_LEAD: i32 = K_MAX_ONE_UNIT_VALUE + 1;
/// `kThreeUnitValueLead` (package-private).
pub const K_THREE_UNIT_VALUE_LEAD: i32 = 0x7fff;
/// `kMaxTwoUnitValue` (package-private).
pub const K_MAX_TWO_UNIT_VALUE: i32 = ((K_THREE_UNIT_VALUE_LEAD - K_MIN_TWO_UNIT_VALUE_LEAD) << 16) - 1;
/// `kMaxOneUnitNodeValue` (package-private).
pub const K_MAX_ONE_UNIT_NODE_VALUE: i32 = 0xff;
/// `kMinTwoUnitNodeValueLead` (package-private).
pub const K_MIN_TWO_UNIT_NODE_VALUE_LEAD: i32 = K_MIN_VALUE_LEAD + ((K_MAX_ONE_UNIT_NODE_VALUE + 1) << 6);
/// `kThreeUnitNodeValueLead` (package-private).
pub const K_THREE_UNIT_NODE_VALUE_LEAD: i32 = 0x7fc0;
/// `kMaxTwoUnitNodeValue` (package-private).
pub const K_MAX_TWO_UNIT_NODE_VALUE: i32 = ((K_THREE_UNIT_NODE_VALUE_LEAD - K_MIN_TWO_UNIT_NODE_VALUE_LEAD) << 10) - 1;
/// `kMaxOneUnitDelta` (package-private).
pub const K_MAX_ONE_UNIT_DELTA: i32 = 0xfbff;
/// `kMinTwoUnitDeltaLead` (package-private).
pub const K_MIN_TWO_UNIT_DELTA_LEAD: i32 = K_MAX_ONE_UNIT_DELTA + 1;
/// `kThreeUnitDeltaLead` (package-private).
pub const K_THREE_UNIT_DELTA_LEAD: i32 = 0xffff;
/// `kMaxTwoUnitDelta` (package-private).
pub const K_MAX_TWO_UNIT_DELTA: i32 = ((K_THREE_UNIT_DELTA_LEAD - K_MIN_TWO_UNIT_DELTA_LEAD) << 16) - 1;

/// Light-weight, non-const reader class for a `CharsTrie`: a trie of
/// UTF-16 strings mapped to `int` values, traversed one code unit at a
/// time.
///
/// Equivalent to `com.ibm.icu.util.CharsTrie` (`Cloneable` is [`Clone`];
/// `Iterable<Entry>` is [`iterator`](Self::iterator)). The serialized trie
/// is shared, as Java shares the `CharSequence` it is given.
#[derive(Debug, Clone)]
pub struct CharsTrie {
    chars: Arc<[u16]>,
    root: i32,
    /// Current position in the trie. -1 means the trie has stopped.
    pos: i32,
    /// Remaining length of a linear-match node, minus 1. Negative if not in such a node.
    remaining_match_length: i32,
}

/// `CharsTrie.State`: a saved position.
///
/// Equivalent to `com.ibm.icu.util.CharsTrie.State`.
#[derive(Debug, Clone, Default)]
pub struct State {
    chars: Option<Arc<[u16]>>,
    root: i32,
    pos: i32,
    remaining_match_length: i32,
}

impl State {
    /// `State()`: an empty state.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CharsTrie {
    /// `CharsTrie(CharSequence trieChars, int offset)`.
    pub fn new(trie_chars: Arc<[u16]>, offset: i32) -> Self {
        Self {
            chars: trie_chars,
            root: offset,
            pos: offset,
            remaining_match_length: -1,
        }
    }

    /// `CharsTrie(CharsTrie other)`: a copy with the same state.
    pub fn from_other(other: &CharsTrie) -> Self {
        other.clone()
    }

    #[inline]
    fn c(&self, i: i32) -> i32 {
        self.chars[i as usize] as i32
    }

    /// `reset()`.
    pub fn reset(&mut self) -> &mut Self {
        self.pos = self.root;
        self.remaining_match_length = -1;
        self
    }

    /// `getState64()`.
    pub fn get_state64(&self) -> i64 {
        ((self.remaining_match_length as i64) << 32) | (self.pos as i64)
    }

    /// `resetToState64(long state)`.
    pub fn reset_to_state64(&mut self, state: i64) -> &mut Self {
        self.remaining_match_length = (state >> 32) as i32;
        self.pos = state as i32;
        self
    }

    /// `saveState(State state)`.
    pub fn save_state(&self, state: &mut State) -> &Self {
        state.chars = Some(self.chars.clone());
        state.root = self.root;
        state.pos = self.pos;
        state.remaining_match_length = self.remaining_match_length;
        self
    }

    /// `resetToState(State state)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("incompatible trie state")`.
    pub fn reset_to_state(&mut self, state: &State) -> JResult<&mut Self> {
        if state.chars.as_ref().is_some_and(|c| Arc::ptr_eq(c, &self.chars)) && self.root == state.root {
            self.pos = state.pos;
            self.remaining_match_length = state.remaining_match_length;
        } else {
            return Err(LuceneError::illegal_argument("incompatible trie state"));
        }
        Ok(self)
    }

    /// The result for the node at `pos` after a completed match.
    fn result_at(&self, length: i32, pos: i32) -> Result {
        let node;
        if length < 0 && {
            node = self.c(pos);
            node >= K_MIN_VALUE_LEAD
        } {
            VALUE_RESULTS[(node >> 15) as usize]
        } else {
            Result::NoValue
        }
    }

    /// `current()`.
    pub fn current(&self) -> Result {
        let pos = self.pos;
        if pos < 0 {
            Result::NoMatch
        } else {
            self.result_at(self.remaining_match_length, pos)
        }
    }

    /// `first(int inUnit)`: resets and traverses one code unit.
    pub fn first(&mut self, in_unit: i32) -> Result {
        self.remaining_match_length = -1;
        self.next_impl(self.root, in_unit)
    }

    /// `firstForCodePoint(int cp)`: resets and traverses one code point
    /// (two units for a supplementary one).
    pub fn first_for_code_point(&mut self, cp: i32) -> Result {
        if cp <= 0xffff {
            self.first(cp)
        } else if self.first(UTF16::get_lead_surrogate(cp) as i32).has_next() {
            self.next(UTF16::get_trail_surrogate(cp) as i32)
        } else {
            Result::NoMatch
        }
    }

    /// `next(int inUnit)`: traverses one code unit.
    pub fn next(&mut self, in_unit: i32) -> Result {
        let mut pos = self.pos;
        if pos < 0 {
            return Result::NoMatch;
        }
        let mut length = self.remaining_match_length; // Actual remaining match length minus 1.
        if length >= 0 {
            // Remaining part of a linear-match node.
            let c = self.c(pos);
            pos += 1;
            if in_unit == c {
                length -= 1;
                self.remaining_match_length = length;
                self.pos = pos;
                self.result_at(length, pos)
            } else {
                self.stop();
                Result::NoMatch
            }
        } else {
            self.next_impl(pos, in_unit)
        }
    }

    /// `nextForCodePoint(int cp)`.
    pub fn next_for_code_point(&mut self, cp: i32) -> Result {
        if cp <= 0xffff {
            self.next(cp)
        } else if self.next(UTF16::get_lead_surrogate(cp) as i32).has_next() {
            self.next(UTF16::get_trail_surrogate(cp) as i32)
        } else {
            Result::NoMatch
        }
    }

    /// `next(CharSequence s, int sIndex, int sLimit)`: traverses the units
    /// `s[s_index..s_limit]`.
    pub fn next_chars(&mut self, s: &[u16], mut s_index: i32, s_limit: i32) -> Result {
        if s_index >= s_limit {
            // Empty input.
            return self.current();
        }
        let mut pos = self.pos;
        if pos < 0 {
            return Result::NoMatch;
        }
        let mut length = self.remaining_match_length; // Actual remaining match length minus 1.
        loop {
            // Fetch the next input unit, if there is one.
            // Continue a linear-match node.
            let mut in_unit: u16;
            loop {
                if s_index == s_limit {
                    self.remaining_match_length = length;
                    self.pos = pos;
                    return self.result_at(length, pos);
                }
                in_unit = s[s_index as usize];
                s_index += 1;
                if length < 0 {
                    self.remaining_match_length = length;
                    break;
                }
                if in_unit as i32 != self.c(pos) {
                    self.stop();
                    return Result::NoMatch;
                }
                pos += 1;
                length -= 1;
            }
            let mut node = self.c(pos);
            pos += 1;
            loop {
                if node < K_MIN_LINEAR_MATCH {
                    let result = self.branch_next(pos, node, in_unit as i32);
                    if result == Result::NoMatch {
                        return Result::NoMatch;
                    }
                    // Fetch the next input unit, if there is one.
                    if s_index == s_limit {
                        return result;
                    }
                    if result == Result::FinalValue {
                        // No further matching units.
                        self.stop();
                        return Result::NoMatch;
                    }
                    in_unit = s[s_index as usize];
                    s_index += 1;
                    pos = self.pos; // branchNext() advanced pos and wrote it to pos_ .
                    node = self.c(pos);
                    pos += 1;
                } else if node < K_MIN_VALUE_LEAD {
                    // Match length+1 units.
                    length = node - K_MIN_LINEAR_MATCH; // Actual match length minus 1.
                    if in_unit as i32 != self.c(pos) {
                        self.stop();
                        return Result::NoMatch;
                    }
                    pos += 1;
                    length -= 1;
                    break;
                } else if (node & K_VALUE_IS_FINAL) != 0 {
                    // No further matching units.
                    self.stop();
                    return Result::NoMatch;
                } else {
                    // Skip intermediate value.
                    pos = skip_node_value(pos, node);
                    node &= K_NODE_TYPE_MASK;
                }
            }
        }
    }

    /// `getValue()`: the value of the current state, which must have one.
    pub fn get_value(&self) -> i32 {
        let mut pos = self.pos;
        let lead_unit = self.c(pos);
        pos += 1;
        debug_assert!(lead_unit >= K_MIN_VALUE_LEAD);
        if (lead_unit & K_VALUE_IS_FINAL) != 0 {
            read_value(&self.chars, pos, lead_unit & 0x7fff)
        } else {
            read_node_value(&self.chars, pos, lead_unit)
        }
    }

    /// `getUniqueValue()`.
    pub fn get_unique_value(&self) -> i64 {
        let pos = self.pos;
        if pos < 0 {
            return 0;
        }
        // Skip the rest of a pending linear-match node.
        let unique_value = find_unique_value(&self.chars, pos + self.remaining_match_length + 1, 0);
        // Ignore internally used bits 63..33; extend the actual value's sign bit from bit 32.
        (unique_value << 31) >> 31
    }

    /// `getNextChars(Appendable out)`: appends each code unit that
    /// continues a matching string, and returns their number.
    pub fn get_next_chars(&self, out: &mut Vec<u16>) -> i32 {
        let mut pos = self.pos;
        if pos < 0 {
            return 0;
        }
        if self.remaining_match_length >= 0 {
            append(out, self.c(pos)); // Next unit of a pending linear-match node.
            return 1;
        }
        let mut node = self.c(pos);
        pos += 1;
        if node >= K_MIN_VALUE_LEAD {
            if (node & K_VALUE_IS_FINAL) != 0 {
                return 0;
            } else {
                pos = skip_node_value(pos, node);
                node &= K_NODE_TYPE_MASK;
            }
        }
        if node < K_MIN_LINEAR_MATCH {
            if node == 0 {
                node = self.c(pos);
                pos += 1;
            }
            node += 1;
            get_next_branch_chars(&self.chars, pos, node, out);
            node
        } else {
            // First unit of the linear-match node.
            append(out, self.c(pos));
            1
        }
    }

    /// `iterator()`.
    pub fn iterator(&self) -> Iterator {
        Iterator::new(self.chars.clone(), self.pos, self.remaining_match_length, 0)
    }

    /// `iterator(int maxStringLength)`.
    pub fn iterator_max(&self, max_string_length: i32) -> Iterator {
        Iterator::new(self.chars.clone(), self.pos, self.remaining_match_length, max_string_length)
    }

    /// `iterator(CharSequence trieChars, int offset, int maxStringLength)`.
    pub fn iterator_of(trie_chars: Arc<[u16]>, offset: i32, max_string_length: i32) -> Iterator {
        Iterator::new(trie_chars, offset, -1, max_string_length)
    }

    /// `stop()` (private).
    fn stop(&mut self) {
        self.pos = -1;
    }

    /// `branchNext(int pos, int length, int inUnit)` (private).
    fn branch_next(&mut self, mut pos: i32, mut length: i32, in_unit: i32) -> Result {
        // Branch according to the current unit.
        if length == 0 {
            length = self.c(pos);
            pos += 1;
        }
        length += 1;
        // The length of the branch is the number of units to select from.
        // The data structure encodes a binary search.
        while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
            let c = self.c(pos);
            pos += 1;
            if in_unit < c {
                length >>= 1;
                pos = jump_by_delta(&self.chars, pos);
            } else {
                length -= length >> 1;
                pos = skip_delta(&self.chars, pos);
            }
        }
        // Drop down to linear search for the last few units.
        // length>=2 because the loop body above sees length>kMaxBranchLinearSubNodeLength>=3
        // and divides length by 2.
        loop {
            let c = self.c(pos);
            pos += 1;
            if in_unit == c {
                let result;
                let mut node = self.c(pos);
                if (node & K_VALUE_IS_FINAL) != 0 {
                    // Leave the final value for getValue() to read.
                    result = Result::FinalValue;
                } else {
                    // Use the non-final value as the jump delta.
                    pos += 1;
                    // int delta=readValue(pos, node);
                    let delta;
                    if node < K_MIN_TWO_UNIT_VALUE_LEAD {
                        delta = node;
                    } else if node < K_THREE_UNIT_VALUE_LEAD {
                        delta = ((node - K_MIN_TWO_UNIT_VALUE_LEAD) << 16) | self.c(pos);
                        pos += 1;
                    } else {
                        delta = (self.c(pos) << 16) | self.c(pos + 1);
                        pos += 2;
                    }
                    // end readValue()
                    pos = pos.wrapping_add(delta);
                    node = self.c(pos);
                    result = if node >= K_MIN_VALUE_LEAD {
                        VALUE_RESULTS[(node >> 15) as usize]
                    } else {
                        Result::NoValue
                    };
                }
                self.pos = pos;
                return result;
            }
            length -= 1;
            pos = skip_value_at(&self.chars, pos);
            if length <= 1 {
                break;
            }
        }
        let c = self.c(pos);
        pos += 1;
        if in_unit == c {
            self.pos = pos;
            let node = self.c(pos);
            if node >= K_MIN_VALUE_LEAD {
                VALUE_RESULTS[(node >> 15) as usize]
            } else {
                Result::NoValue
            }
        } else {
            self.stop();
            Result::NoMatch
        }
    }

    /// `nextImpl(int pos, int inUnit)` (private).
    fn next_impl(&mut self, mut pos: i32, in_unit: i32) -> Result {
        let mut node = self.c(pos);
        pos += 1;
        loop {
            if node < K_MIN_LINEAR_MATCH {
                return self.branch_next(pos, node, in_unit);
            } else if node < K_MIN_VALUE_LEAD {
                // Match the first of length+1 units.
                let mut length = node - K_MIN_LINEAR_MATCH; // Actual match length minus 1.
                let c = self.c(pos);
                pos += 1;
                if in_unit == c {
                    length -= 1;
                    self.remaining_match_length = length;
                    self.pos = pos;
                    return self.result_at(length, pos);
                } else {
                    // No match.
                    break;
                }
            } else if (node & K_VALUE_IS_FINAL) != 0 {
                // No further matching units.
                break;
            } else {
                // Skip intermediate value.
                pos = skip_node_value(pos, node);
                node &= K_NODE_TYPE_MASK;
            }
        }
        self.stop();
        Result::NoMatch
    }
}

#[inline]
fn uc(chars: &[u16], i: i32) -> i32 {
    chars[i as usize] as i32
}

/// `readValue(CharSequence chars, int pos, int leadUnit)` (private).
fn read_value(chars: &[u16], pos: i32, lead_unit: i32) -> i32 {
    if lead_unit < K_MIN_TWO_UNIT_VALUE_LEAD {
        lead_unit
    } else if lead_unit < K_THREE_UNIT_VALUE_LEAD {
        ((lead_unit - K_MIN_TWO_UNIT_VALUE_LEAD) << 16) | uc(chars, pos)
    } else {
        (uc(chars, pos) << 16) | uc(chars, pos + 1)
    }
}

/// `skipValue(int pos, int leadUnit)` (private).
fn skip_value(mut pos: i32, lead_unit: i32) -> i32 {
    if lead_unit >= K_MIN_TWO_UNIT_VALUE_LEAD {
        if lead_unit < K_THREE_UNIT_VALUE_LEAD {
            pos += 1;
        } else {
            pos += 2;
        }
    }
    pos
}

/// `skipValue(CharSequence chars, int pos)` (private).
fn skip_value_at(chars: &[u16], mut pos: i32) -> i32 {
    let lead_unit = uc(chars, pos);
    pos += 1;
    skip_value(pos, lead_unit & 0x7fff)
}

/// `readNodeValue(CharSequence chars, int pos, int leadUnit)` (private).
fn read_node_value(chars: &[u16], pos: i32, lead_unit: i32) -> i32 {
    debug_assert!(K_MIN_VALUE_LEAD <= lead_unit && lead_unit < K_VALUE_IS_FINAL);
    if lead_unit < K_MIN_TWO_UNIT_NODE_VALUE_LEAD {
        (lead_unit >> 6) - 1
    } else if lead_unit < K_THREE_UNIT_NODE_VALUE_LEAD {
        (((lead_unit & 0x7fc0) - K_MIN_TWO_UNIT_NODE_VALUE_LEAD) << 10) | uc(chars, pos)
    } else {
        (uc(chars, pos) << 16) | uc(chars, pos + 1)
    }
}

/// `skipNodeValue(int pos, int leadUnit)` (private).
fn skip_node_value(mut pos: i32, lead_unit: i32) -> i32 {
    debug_assert!(K_MIN_VALUE_LEAD <= lead_unit && lead_unit < K_VALUE_IS_FINAL);
    if lead_unit >= K_MIN_TWO_UNIT_NODE_VALUE_LEAD {
        if lead_unit < K_THREE_UNIT_NODE_VALUE_LEAD {
            pos += 1;
        } else {
            pos += 2;
        }
    }
    pos
}

/// `jumpByDelta(CharSequence chars, int pos)` (private).
fn jump_by_delta(chars: &[u16], mut pos: i32) -> i32 {
    let mut delta = uc(chars, pos);
    pos += 1;
    if delta >= K_MIN_TWO_UNIT_DELTA_LEAD {
        if delta == K_THREE_UNIT_DELTA_LEAD {
            delta = (uc(chars, pos) << 16) | uc(chars, pos + 1);
            pos += 2;
        } else {
            delta = ((delta - K_MIN_TWO_UNIT_DELTA_LEAD) << 16) | uc(chars, pos);
            pos += 1;
        }
    }
    pos.wrapping_add(delta)
}

/// `skipDelta(CharSequence chars, int pos)` (private).
fn skip_delta(chars: &[u16], mut pos: i32) -> i32 {
    let delta = uc(chars, pos);
    pos += 1;
    if delta >= K_MIN_TWO_UNIT_DELTA_LEAD {
        if delta == K_THREE_UNIT_DELTA_LEAD {
            pos += 2;
        } else {
            pos += 1;
        }
    }
    pos
}

/// `findUniqueValueFromBranch(CharSequence chars, int pos, int length, long
/// uniqueValue)` (private).
fn find_unique_value_from_branch(chars: &[u16], mut pos: i32, mut length: i32, mut unique_value: i64) -> i64 {
    while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
        pos += 1; // ignore the comparison unit
        unique_value = find_unique_value_from_branch(chars, jump_by_delta(chars, pos), length >> 1, unique_value);
        if unique_value == 0 {
            return 0;
        }
        length -= length >> 1;
        pos = skip_delta(chars, pos);
    }
    loop {
        pos += 1; // ignore a comparison unit
        // handle its value
        let mut node = uc(chars, pos);
        pos += 1;
        let is_final = (node & K_VALUE_IS_FINAL) != 0;
        node &= 0x7fff;
        let value = read_value(chars, pos, node);
        pos = skip_value(pos, node);
        if is_final {
            if unique_value != 0 {
                if value != (unique_value >> 1) as i32 {
                    return 0;
                }
            } else {
                unique_value = ((value as i64) << 1) | 1;
            }
        } else {
            unique_value = find_unique_value(chars, pos.wrapping_add(value), unique_value);
            if unique_value == 0 {
                return 0;
            }
        }
        length -= 1;
        if length <= 1 {
            break;
        }
    }
    // ignore the last comparison unit
    (((pos + 1) as i64) << 33) | (unique_value & 0x1ffffffff)
}

/// `findUniqueValue(CharSequence chars, int pos, long uniqueValue)`
/// (private).
fn find_unique_value(chars: &[u16], mut pos: i32, mut unique_value: i64) -> i64 {
    let mut node = uc(chars, pos);
    pos += 1;
    loop {
        if node < K_MIN_LINEAR_MATCH {
            if node == 0 {
                node = uc(chars, pos);
                pos += 1;
            }
            unique_value = find_unique_value_from_branch(chars, pos, node + 1, unique_value);
            if unique_value == 0 {
                return 0;
            }
            pos = ((unique_value as u64) >> 33) as i32;
            node = uc(chars, pos);
            pos += 1;
        } else if node < K_MIN_VALUE_LEAD {
            // linear-match node
            pos += node - K_MIN_LINEAR_MATCH + 1; // Ignore the match units.
            node = uc(chars, pos);
            pos += 1;
        } else {
            let is_final = (node & K_VALUE_IS_FINAL) != 0;
            let value = if is_final {
                read_value(chars, pos, node & 0x7fff)
            } else {
                read_node_value(chars, pos, node)
            };
            if unique_value != 0 {
                if value != (unique_value >> 1) as i32 {
                    return 0;
                }
            } else {
                unique_value = ((value as i64) << 1) | 1;
            }
            if is_final {
                return unique_value;
            }
            pos = skip_node_value(pos, node);
            node &= K_NODE_TYPE_MASK;
        }
    }
}

/// `getNextBranchChars(CharSequence chars, int pos, int length, Appendable
/// out)` (private).
fn get_next_branch_chars(chars: &[u16], mut pos: i32, mut length: i32, out: &mut Vec<u16>) {
    while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
        pos += 1; // ignore the comparison unit
        get_next_branch_chars(chars, jump_by_delta(chars, pos), length >> 1, out);
        length -= length >> 1;
        pos = skip_delta(chars, pos);
    }
    loop {
        append(out, uc(chars, pos));
        pos += 1;
        pos = skip_value_at(chars, pos);
        length -= 1;
        if length <= 1 {
            break;
        }
    }
    append(out, uc(chars, pos));
}

/// `append(Appendable out, int c)` (private).
fn append(out: &mut Vec<u16>, c: i32) {
    out.push(c as u16);
}

/// Return value type for the [`Iterator`]: a string and its value.
///
/// Equivalent to `com.ibm.icu.util.CharsTrie.Entry`; `chars` is a copy of
/// the iterator's string builder, which Java's entry refers to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// `chars`: the string.
    pub chars: Vec<u16>,
    /// `value`: the value associated with the string, -1 when the string
    /// was truncated by the maximum length.
    pub value: i32,
}

/// Iterator for all of the (string, value) pairs in a `CharsTrie`.
///
/// Equivalent to `com.ibm.icu.util.CharsTrie.Iterator`. Each step returns a
/// copy of the entry.
#[derive(Debug, Clone)]
pub struct Iterator {
    chars: Arc<[u16]>,
    pos: i32,
    initial_pos: i32,
    remaining_match_length: i32,
    initial_remaining_match_length: i32,
    skip_value: bool, // Skip intermediate value which was already delivered.
    str_: Vec<u16>,
    max_length: i32,
    entry: Entry,
    /// The stack stores longs for backtracking to another
    /// outbound edge of a branch node.
    /// Each long has the offset in chars_ in bits 62..32,
    /// the str_.length() from before the node in bits 15..0,
    /// and the remaining branch length in bits 31..16.
    /// (We could store the remaining branch length minus 1 in bits 30..16 and not use bit 31,
    /// but the code looks more confusing that way.)
    stack: Vec<i64>,
}

impl Iterator {
    /// `Iterator(CharSequence trieChars, int offset, int remainingMatchLength,
    /// int maxStringLength)` (private).
    fn new(trie_chars: Arc<[u16]>, offset: i32, remaining_match_length: i32, max_string_length: i32) -> Self {
        let mut it = Self {
            chars: trie_chars,
            pos: offset,
            initial_pos: offset,
            remaining_match_length,
            initial_remaining_match_length: remaining_match_length,
            skip_value: false,
            str_: Vec::new(),
            max_length: max_string_length,
            entry: Entry::default(),
            stack: Vec::new(),
        };
        let mut length = it.remaining_match_length; // Actual remaining match length minus 1.
        if length >= 0 {
            // Pending linear-match node, append remaining bytes to str_.
            length += 1;
            if it.max_length > 0 && length > it.max_length {
                length = it.max_length; // This will leave remainingMatchLength>=0 as a signal.
            }
            let p = it.pos as usize;
            it.str_.extend_from_slice(&it.chars[p..p + length as usize]);
            it.pos += length;
            it.remaining_match_length -= length;
        }
        it
    }

    /// `reset()`.
    pub fn reset(&mut self) -> &mut Self {
        self.pos = self.initial_pos;
        self.remaining_match_length = self.initial_remaining_match_length;
        self.skip_value = false;
        let mut length = self.remaining_match_length + 1; // Remaining match length.
        if self.max_length > 0 && length > self.max_length {
            length = self.max_length;
        }
        set_length(&mut self.str_, length);
        self.pos += length;
        self.remaining_match_length -= length;
        self.stack.clear();
        self
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        self.pos >= 0 || !self.stack.is_empty()
    }

    /// `remove()`: unsupported.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException`.
    pub fn remove(&mut self) -> JResult<()> {
        Err(crate::jdk::unchecked_no_message("java.lang.UnsupportedOperationException"))
    }

    /// `next()` itself.
    fn next_entry(&mut self) -> Option<&Entry> {
        let mut pos = self.pos;
        if pos < 0 {
            let top = self.stack.pop()?;
            // Pop the state off the stack and continue with the next outbound edge of
            // the branch node.
            let mut length = top as i32;
            pos = (top >> 32) as i32;
            set_length(&mut self.str_, length & 0xffff);
            length = ((length as u32) >> 16) as i32;
            if length > 1 {
                pos = self.branch_next(pos, length);
                if pos < 0 {
                    return Some(&self.entry); // Reached a final value.
                }
            } else {
                self.str_.push(self.chars[pos as usize]);
                pos += 1;
            }
        }
        if self.remaining_match_length >= 0 {
            // We only get here if we started in a pending linear-match node
            // with more than maxLength remaining units.
            return Some(self.truncate_and_stop());
        }
        loop {
            let mut node = uc(&self.chars, pos);
            pos += 1;
            if node >= K_MIN_VALUE_LEAD {
                if self.skip_value {
                    pos = skip_node_value(pos, node);
                    node &= K_NODE_TYPE_MASK;
                    self.skip_value = false;
                } else {
                    // Deliver value for the string so far.
                    let is_final = (node & K_VALUE_IS_FINAL) != 0;
                    if is_final {
                        self.entry.value = read_value(&self.chars, pos, node & 0x7fff);
                    } else {
                        self.entry.value = read_node_value(&self.chars, pos, node);
                    }
                    if is_final || (self.max_length > 0 && self.str_.len() as i32 == self.max_length) {
                        self.pos = -1;
                    } else {
                        // We cannot skip the value right here because it shares its
                        // lead unit with a match node which we have to evaluate
                        // next time.
                        // Instead, keep pos_ on the node lead unit itself.
                        self.pos = pos - 1;
                        self.skip_value = true;
                    }
                    self.entry.chars = self.str_.clone();
                    return Some(&self.entry);
                }
            }
            if self.max_length > 0 && self.str_.len() as i32 == self.max_length {
                return Some(self.truncate_and_stop());
            }
            if node < K_MIN_LINEAR_MATCH {
                if node == 0 {
                    node = uc(&self.chars, pos);
                    pos += 1;
                }
                pos = self.branch_next(pos, node + 1);
                if pos < 0 {
                    return Some(&self.entry); // Reached a final value.
                }
            } else {
                // Linear-match node, append length units to str_.
                let length = node - K_MIN_LINEAR_MATCH + 1;
                if self.max_length > 0 && self.str_.len() as i32 + length > self.max_length {
                    let n = self.max_length - self.str_.len() as i32;
                    let p = pos as usize;
                    self.str_.extend_from_slice(&self.chars[p..p + n as usize]);
                    return Some(self.truncate_and_stop());
                }
                let p = pos as usize;
                self.str_.extend_from_slice(&self.chars[p..p + length as usize]);
                pos += length;
            }
        }
    }

    /// `truncateAndStop()` (private).
    fn truncate_and_stop(&mut self) -> &Entry {
        self.pos = -1;
        // We reset entry_.chars every time we return entry_
        // just because the caller might have modified the Entry.
        self.entry.chars = self.str_.clone();
        self.entry.value = -1; // no real value for str
        &self.entry
    }

    /// `branchNext(int pos, int length)` (private).
    fn branch_next(&mut self, mut pos: i32, mut length: i32) -> i32 {
        while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
            pos += 1; // ignore the comparison unit
            // Push state for the greater-or-equal edge.
            let low = ((length - (length >> 1)) << 16) | self.str_.len() as i32;
            self.stack.push(((skip_delta(&self.chars, pos) as i64) << 32) | (low as i64));
            // Follow the less-than edge.
            length >>= 1;
            pos = jump_by_delta(&self.chars, pos);
        }
        // List of key-value pairs where values are either final values or jump deltas.
        // Read the first (key, value) pair.
        let trie_unit = self.chars[pos as usize];
        pos += 1;
        let mut node = uc(&self.chars, pos);
        pos += 1;
        let is_final = (node & K_VALUE_IS_FINAL) != 0;
        node &= 0x7fff;
        let value = read_value(&self.chars, pos, node);
        pos = skip_value(pos, node);
        let low = ((length - 1) << 16) | self.str_.len() as i32;
        self.stack.push(((pos as i64) << 32) | (low as i64));
        self.str_.push(trie_unit);
        if is_final {
            self.pos = -1;
            self.entry.chars = self.str_.clone();
            self.entry.value = value;
            -1
        } else {
            pos.wrapping_add(value)
        }
    }
}

impl std::iter::Iterator for Iterator {
    type Item = Entry;

    /// `next()`: a copy of the next entry; `None` where Java throws
    /// `NoSuchElementException`.
    fn next(&mut self) -> Option<Entry> {
        self.next_entry().cloned()
    }
}

/// `StringBuilder.setLength(int)`: truncates, or pads with `\0`.
fn set_length(sb: &mut Vec<u16>, length: i32) {
    sb.resize(length as usize, 0);
}
