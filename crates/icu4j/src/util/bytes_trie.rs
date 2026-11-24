// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.BytesTrie`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result as JResult};

use crate::jdk::ByteBuffer;

/// Return values for `BytesTrie.next()`, `CharsTrie.next()` and similar
/// methods.
///
/// Equivalent to the enum `com.ibm.icu.util.BytesTrie.Result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result {
    /// `NO_MATCH`: the input unit(s) did not continue a matching string.
    NoMatch,
    /// `NO_VALUE`: the input unit(s) continued a matching string but there
    /// is no value for the string so far.
    NoValue,
    /// `FINAL_VALUE`: the input unit(s) continued a matching string and
    /// there is a value for the string so far; no further input continues a
    /// matching string.
    FinalValue,
    /// `INTERMEDIATE_VALUE`: there is a value for the string so far, and
    /// another input unit may continue a matching string.
    IntermediateValue,
}

impl Result {
    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            Result::NoMatch => 0,
            Result::NoValue => 1,
            Result::FinalValue => 2,
            Result::IntermediateValue => 3,
        }
    }

    /// `matches()`: same as `!= NO_MATCH`.
    pub fn matches(self) -> bool {
        self != Result::NoMatch
    }

    /// `hasValue()`: `FINAL_VALUE` or `INTERMEDIATE_VALUE`.
    pub fn has_value(self) -> bool {
        self.ordinal() >= 2
    }

    /// `hasNext()`: `NO_VALUE` or `INTERMEDIATE_VALUE`.
    pub fn has_next(self) -> bool {
        (self.ordinal() & 1) != 0
    }
}

/// `valueResults_`.
const VALUE_RESULTS: [Result; 2] = [Result::IntermediateValue, Result::FinalValue];

/// `kMaxBranchLinearSubNodeLength` (package-private).
pub const K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH: i32 = 5;
/// `kMinLinearMatch` (package-private): 10..1f: linear-match node.
pub const K_MIN_LINEAR_MATCH: i32 = 0x10;
/// `kMaxLinearMatchLength` (package-private).
pub const K_MAX_LINEAR_MATCH_LENGTH: i32 = 0x10;
/// `kMinValueLead` (package-private): 20..ff: value node.
pub const K_MIN_VALUE_LEAD: i32 = K_MIN_LINEAR_MATCH + K_MAX_LINEAR_MATCH_LENGTH;
/// `kValueIsFinal`.
const K_VALUE_IS_FINAL: i32 = 1;
/// `kMinOneByteValueLead` (package-private).
pub const K_MIN_ONE_BYTE_VALUE_LEAD: i32 = K_MIN_VALUE_LEAD / 2;
/// `kMaxOneByteValue` (package-private).
pub const K_MAX_ONE_BYTE_VALUE: i32 = 0x40;
/// `kMinTwoByteValueLead` (package-private).
pub const K_MIN_TWO_BYTE_VALUE_LEAD: i32 = K_MIN_ONE_BYTE_VALUE_LEAD + K_MAX_ONE_BYTE_VALUE + 1;
/// `kMaxTwoByteValue` (package-private).
pub const K_MAX_TWO_BYTE_VALUE: i32 = 0x1aff;
/// `kMinThreeByteValueLead` (package-private).
pub const K_MIN_THREE_BYTE_VALUE_LEAD: i32 =
    K_MIN_TWO_BYTE_VALUE_LEAD + (K_MAX_TWO_BYTE_VALUE >> 8) + 1;
/// `kFourByteValueLead` (package-private).
pub const K_FOUR_BYTE_VALUE_LEAD: i32 = 0x7e;
/// `kMaxThreeByteValue` (package-private).
pub const K_MAX_THREE_BYTE_VALUE: i32 =
    ((K_FOUR_BYTE_VALUE_LEAD - K_MIN_THREE_BYTE_VALUE_LEAD) << 16) - 1;
/// `kFiveByteValueLead` (package-private).
pub const K_FIVE_BYTE_VALUE_LEAD: i32 = 0x7f;
/// `kMaxOneByteDelta` (package-private).
pub const K_MAX_ONE_BYTE_DELTA: i32 = 0xbf;
/// `kMinTwoByteDeltaLead` (package-private).
pub const K_MIN_TWO_BYTE_DELTA_LEAD: i32 = K_MAX_ONE_BYTE_DELTA + 1;
/// `kMinThreeByteDeltaLead` (package-private).
pub const K_MIN_THREE_BYTE_DELTA_LEAD: i32 = 0xf0;
/// `kFourByteDeltaLead` (package-private).
pub const K_FOUR_BYTE_DELTA_LEAD: i32 = 0xfe;
/// `kFiveByteDeltaLead` (package-private).
pub const K_FIVE_BYTE_DELTA_LEAD: i32 = 0xff;
/// `kMaxTwoByteDelta` (package-private).
pub const K_MAX_TWO_BYTE_DELTA: i32 =
    ((K_MIN_THREE_BYTE_DELTA_LEAD - K_MIN_TWO_BYTE_DELTA_LEAD) << 8) - 1;
/// `kMaxThreeByteDelta` (package-private).
pub const K_MAX_THREE_BYTE_DELTA: i32 =
    ((K_FOUR_BYTE_DELTA_LEAD - K_MIN_THREE_BYTE_DELTA_LEAD) << 16) - 1;

/// Light-weight, non-const reader class for a `BytesTrie`: a trie of byte
/// sequences mapped to `int` values, traversed one byte at a time.
///
/// Equivalent to `com.ibm.icu.util.BytesTrie` (`Cloneable` is [`Clone`];
/// `Iterable<Entry>` is [`iterator`](Self::iterator)). The serialized trie
/// is shared, as Java shares the array it is given.
#[derive(Debug, Clone)]
pub struct BytesTrie {
    bytes: Arc<[i8]>,
    root: i32,
    /// Current position in the trie. -1 means the trie has stopped.
    pos: i32,
    /// Remaining length of a linear-match node, minus 1. Negative if not in such a node.
    remaining_match_length: i32,
}

/// `BytesTrie.State`: a saved position, for
/// [`reset_to_state`](BytesTrie::reset_to_state).
///
/// Equivalent to `com.ibm.icu.util.BytesTrie.State`.
#[derive(Debug, Clone, Default)]
pub struct State {
    bytes: Option<Arc<[i8]>>,
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

impl BytesTrie {
    /// `BytesTrie(byte[] trieBytes, int offset)`: a reader over the trie
    /// serialized in `trie_bytes` at `offset`.
    pub fn new(trie_bytes: Arc<[i8]>, offset: i32) -> Self {
        Self {
            bytes: trie_bytes,
            root: offset,
            pos: offset,
            remaining_match_length: -1,
        }
    }

    /// `BytesTrie(BytesTrie other)`: a copy with the same state.
    pub fn from_other(other: &BytesTrie) -> Self {
        other.clone()
    }

    #[inline]
    fn b(&self, i: i32) -> i32 {
        self.bytes[i as usize] as i32 & 0xff
    }

    /// `reset()`: back to the initial state.
    pub fn reset(&mut self) -> &mut Self {
        self.pos = self.root;
        self.remaining_match_length = -1;
        self
    }

    /// `getState64()`: the state as a `long`.
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
        state.bytes = Some(self.bytes.clone());
        state.root = self.root;
        state.pos = self.pos;
        state.remaining_match_length = self.remaining_match_length;
        self
    }

    /// `resetToState(State state)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("incompatible trie state")` when the state
    /// was saved from another trie or root.
    pub fn reset_to_state(&mut self, state: &State) -> JResult<&mut Self> {
        if state
            .bytes
            .as_ref()
            .is_some_and(|b| Arc::ptr_eq(b, &self.bytes))
            && self.root == state.root
        {
            self.pos = state.pos;
            self.remaining_match_length = state.remaining_match_length;
        } else {
            return Err(LuceneError::illegal_argument("incompatible trie state"));
        }
        Ok(self)
    }

    /// `current()`: the state of the last `first`/`next` call.
    pub fn current(&self) -> Result {
        let pos = self.pos;
        if pos < 0 {
            Result::NoMatch
        } else {
            let node;
            if self.remaining_match_length < 0 && {
                node = self.b(pos);
                node >= K_MIN_VALUE_LEAD
            } {
                VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
            } else {
                Result::NoValue
            }
        }
    }

    /// `first(int inByte)`: resets and traverses with one input byte
    /// (negative values are taken as unsigned bytes).
    pub fn first(&mut self, mut in_byte: i32) -> Result {
        self.remaining_match_length = -1;
        if in_byte < 0 {
            in_byte += 0x100;
        }
        self.next_impl(self.root, in_byte)
    }

    /// `next(int inByte)`: traverses one input byte.
    pub fn next(&mut self, mut in_byte: i32) -> Result {
        let mut pos = self.pos;
        if pos < 0 {
            return Result::NoMatch;
        }
        if in_byte < 0 {
            in_byte += 0x100;
        }
        let mut length = self.remaining_match_length; // Actual remaining match length minus 1.
        if length >= 0 {
            // Remaining part of a linear-match node.
            let b = self.b(pos);
            pos += 1;
            if in_byte == b {
                length -= 1;
                self.remaining_match_length = length;
                self.pos = pos;
                let node;
                if length < 0 && {
                    node = self.b(pos);
                    node >= K_MIN_VALUE_LEAD
                } {
                    VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
                } else {
                    Result::NoValue
                }
            } else {
                self.stop();
                Result::NoMatch
            }
        } else {
            self.next_impl(pos, in_byte)
        }
    }

    /// `next(byte[] s, int sIndex, int sLimit)`: traverses the bytes
    /// `s[s_index..s_limit]`.
    pub fn next_bytes(&mut self, s: &[i8], mut s_index: i32, s_limit: i32) -> Result {
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
            // Fetch the next input byte, if there is one.
            // Continue a linear-match node.
            let mut in_byte: i8;
            loop {
                if s_index == s_limit {
                    self.remaining_match_length = length;
                    self.pos = pos;
                    let node;
                    return if length < 0 && {
                        node = self.b(pos);
                        node >= K_MIN_VALUE_LEAD
                    } {
                        VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
                    } else {
                        Result::NoValue
                    };
                }
                in_byte = s[s_index as usize];
                s_index += 1;
                if length < 0 {
                    self.remaining_match_length = length;
                    break;
                }
                if in_byte != self.bytes[pos as usize] {
                    self.stop();
                    return Result::NoMatch;
                }
                pos += 1;
                length -= 1;
            }
            loop {
                let node = self.b(pos);
                pos += 1;
                if node < K_MIN_LINEAR_MATCH {
                    let result = self.branch_next(pos, node, in_byte as i32 & 0xff);
                    if result == Result::NoMatch {
                        return Result::NoMatch;
                    }
                    // Fetch the next input byte, if there is one.
                    if s_index == s_limit {
                        return result;
                    }
                    if result == Result::FinalValue {
                        // No further matching bytes.
                        self.stop();
                        return Result::NoMatch;
                    }
                    in_byte = s[s_index as usize];
                    s_index += 1;
                    pos = self.pos; // branchNext() advanced pos and wrote it to pos_ .
                } else if node < K_MIN_VALUE_LEAD {
                    // Match length+1 bytes.
                    length = node - K_MIN_LINEAR_MATCH; // Actual match length minus 1.
                    if in_byte != self.bytes[pos as usize] {
                        self.stop();
                        return Result::NoMatch;
                    }
                    pos += 1;
                    length -= 1;
                    break;
                } else if (node & K_VALUE_IS_FINAL) != 0 {
                    // No further matching bytes.
                    self.stop();
                    return Result::NoMatch;
                } else {
                    // Skip intermediate value.
                    pos = skip_value(pos, node);
                    // The next node must not also be a value node.
                    debug_assert!(self.b(pos) < K_MIN_VALUE_LEAD);
                }
            }
        }
    }

    /// `getValue()`: the value of the current state, which must have a
    /// value.
    pub fn get_value(&self) -> i32 {
        let mut pos = self.pos;
        let lead_byte = self.b(pos);
        pos += 1;
        debug_assert!(lead_byte >= K_MIN_VALUE_LEAD);
        read_value(&self.bytes, pos, lead_byte >> 1)
    }

    /// `getUniqueValue()`: `(value << 1) | 1` when all strings from here
    /// have the same value, else 0.
    pub fn get_unique_value(&self) -> i64 {
        let pos = self.pos;
        if pos < 0 {
            return 0;
        }
        // Skip the rest of a pending linear-match node.
        let unique_value = find_unique_value(&self.bytes, pos + self.remaining_match_length + 1, 0);
        // Ignore internally used bits 63..33; extend the actual value's sign bit from bit 32.
        (unique_value << 31) >> 31
    }

    /// `getNextBytes(Appendable out)`: appends each byte (as a `char`) that
    /// continues a matching string, and returns their number.
    pub fn get_next_bytes(&self, out: &mut Vec<u16>) -> i32 {
        let mut pos = self.pos;
        if pos < 0 {
            return 0;
        }
        if self.remaining_match_length >= 0 {
            append(out, self.b(pos)); // Next byte of a pending linear-match node.
            return 1;
        }
        let mut node = self.b(pos);
        pos += 1;
        if node >= K_MIN_VALUE_LEAD {
            if (node & K_VALUE_IS_FINAL) != 0 {
                return 0;
            } else {
                pos = skip_value(pos, node);
                node = self.b(pos);
                pos += 1;
                debug_assert!(node < K_MIN_VALUE_LEAD);
            }
        }
        if node < K_MIN_LINEAR_MATCH {
            if node == 0 {
                node = self.b(pos);
                pos += 1;
            }
            node += 1;
            get_next_branch_bytes(&self.bytes, pos, node, out);
            node
        } else {
            // First byte of the linear-match node.
            append(out, self.b(pos));
            1
        }
    }

    /// `iterator()`: the (byte sequence, value) pairs from the current
    /// state.
    pub fn iterator(&self) -> Iterator {
        Iterator::new(self.bytes.clone(), self.pos, self.remaining_match_length, 0)
    }

    /// `iterator(int maxStringLength)`: as [`iterator`](Self::iterator),
    /// the sequences truncated to `max_string_length` bytes (0: no limit).
    pub fn iterator_max(&self, max_string_length: i32) -> Iterator {
        Iterator::new(
            self.bytes.clone(),
            self.pos,
            self.remaining_match_length,
            max_string_length,
        )
    }

    /// `iterator(byte[] trieBytes, int offset, int maxStringLength)`: the
    /// pairs of a serialized trie.
    pub fn iterator_of(trie_bytes: Arc<[i8]>, offset: i32, max_string_length: i32) -> Iterator {
        Iterator::new(trie_bytes, offset, -1, max_string_length)
    }

    /// `stop()` (private).
    fn stop(&mut self) {
        self.pos = -1;
    }

    /// `branchNext(int pos, int length, int inByte)` (private).
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn branch_next(&mut self, mut pos: i32, mut length: i32, in_byte: i32) -> Result {
        // Branch according to the current byte.
        if length == 0 {
            length = self.b(pos);
            pos += 1;
        }
        length += 1;
        // The length of the branch is the number of bytes to select from.
        // The data structure encodes a binary search.
        while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
            let b = self.b(pos);
            pos += 1;
            if in_byte < b {
                length >>= 1;
                pos = jump_by_delta(&self.bytes, pos);
            } else {
                length -= length >> 1;
                pos = skip_delta(&self.bytes, pos);
            }
        }
        // Drop down to linear search for the last few bytes.
        // length>=2 because the loop body above sees length>kMaxBranchLinearSubNodeLength>=3
        // and divides length by 2.
        loop {
            let b = self.b(pos);
            pos += 1;
            if in_byte == b {
                let result;
                let mut node = self.b(pos);
                debug_assert!(node >= K_MIN_VALUE_LEAD);
                if (node & K_VALUE_IS_FINAL) != 0 {
                    // Leave the final value for getValue() to read.
                    result = Result::FinalValue;
                } else {
                    // Use the non-final value as the jump delta.
                    pos += 1;
                    // int delta=readValue(pos, node>>1);
                    node >>= 1;
                    let delta;
                    if node < K_MIN_TWO_BYTE_VALUE_LEAD {
                        delta = node - K_MIN_ONE_BYTE_VALUE_LEAD;
                    } else if node < K_MIN_THREE_BYTE_VALUE_LEAD {
                        delta = ((node - K_MIN_TWO_BYTE_VALUE_LEAD) << 8) | self.b(pos);
                        pos += 1;
                    } else if node < K_FOUR_BYTE_VALUE_LEAD {
                        delta = ((node - K_MIN_THREE_BYTE_VALUE_LEAD) << 16)
                            | (self.b(pos) << 8)
                            | self.b(pos + 1);
                        pos += 2;
                    } else if node == K_FOUR_BYTE_VALUE_LEAD {
                        delta = (self.b(pos) << 16) | (self.b(pos + 1) << 8) | self.b(pos + 2);
                        pos += 3;
                    } else {
                        delta = ((self.bytes[pos as usize] as i32) << 24)
                            | (self.b(pos + 1) << 16)
                            | (self.b(pos + 2) << 8)
                            | self.b(pos + 3);
                        pos += 4;
                    }
                    // end readValue()
                    pos = pos.wrapping_add(delta);
                    node = self.b(pos);
                    result = if node >= K_MIN_VALUE_LEAD {
                        VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
                    } else {
                        Result::NoValue
                    };
                }
                self.pos = pos;
                return result;
            }
            length -= 1;
            pos = skip_value_at(&self.bytes, pos);
            if length <= 1 {
                break;
            }
        }
        let b = self.b(pos);
        pos += 1;
        if in_byte == b {
            self.pos = pos;
            let node = self.b(pos);
            if node >= K_MIN_VALUE_LEAD {
                VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
            } else {
                Result::NoValue
            }
        } else {
            self.stop();
            Result::NoMatch
        }
    }

    /// `nextImpl(int pos, int inByte)` (private).
    fn next_impl(&mut self, mut pos: i32, in_byte: i32) -> Result {
        loop {
            let mut node = self.b(pos);
            pos += 1;
            if node < K_MIN_LINEAR_MATCH {
                return self.branch_next(pos, node, in_byte);
            } else if node < K_MIN_VALUE_LEAD {
                // Match the first of length+1 bytes.
                let mut length = node - K_MIN_LINEAR_MATCH; // Actual match length minus 1.
                let b = self.b(pos);
                pos += 1;
                if in_byte == b {
                    length -= 1;
                    self.remaining_match_length = length;
                    self.pos = pos;
                    return if length < 0 && {
                        node = self.b(pos);
                        node >= K_MIN_VALUE_LEAD
                    } {
                        VALUE_RESULTS[(node & K_VALUE_IS_FINAL) as usize]
                    } else {
                        Result::NoValue
                    };
                } else {
                    // No match.
                    break;
                }
            } else if (node & K_VALUE_IS_FINAL) != 0 {
                // No further matching bytes.
                break;
            } else {
                // Skip intermediate value.
                pos = skip_value(pos, node);
                // The next node must not also be a value node.
                debug_assert!(self.b(pos) < K_MIN_VALUE_LEAD);
            }
        }
        self.stop();
        Result::NoMatch
    }

    /// `jumpByDelta(byte[] bytes, int pos)` (deprecated, public): the
    /// position after a jump delta at `pos`, plus the delta.
    pub fn jump_by_delta(bytes: &[i8], pos: i32) -> i32 {
        jump_by_delta(bytes, pos)
    }
}

#[inline]
fn ub(bytes: &[i8], i: i32) -> i32 {
    bytes[i as usize] as i32 & 0xff
}

/// `readValue(byte[] bytes, int pos, int leadByte)` (private).
fn read_value(bytes: &[i8], pos: i32, lead_byte: i32) -> i32 {
    if lead_byte < K_MIN_TWO_BYTE_VALUE_LEAD {
        lead_byte - K_MIN_ONE_BYTE_VALUE_LEAD
    } else if lead_byte < K_MIN_THREE_BYTE_VALUE_LEAD {
        ((lead_byte - K_MIN_TWO_BYTE_VALUE_LEAD) << 8) | ub(bytes, pos)
    } else if lead_byte < K_FOUR_BYTE_VALUE_LEAD {
        ((lead_byte - K_MIN_THREE_BYTE_VALUE_LEAD) << 16)
            | (ub(bytes, pos) << 8)
            | ub(bytes, pos + 1)
    } else if lead_byte == K_FOUR_BYTE_VALUE_LEAD {
        (ub(bytes, pos) << 16) | (ub(bytes, pos + 1) << 8) | ub(bytes, pos + 2)
    } else {
        ((bytes[pos as usize] as i32) << 24)
            | (ub(bytes, pos + 1) << 16)
            | (ub(bytes, pos + 2) << 8)
            | ub(bytes, pos + 3)
    }
}

/// `skipValue(int pos, int leadByte)` (private).
fn skip_value(mut pos: i32, lead_byte: i32) -> i32 {
    debug_assert!(lead_byte >= K_MIN_VALUE_LEAD);
    if lead_byte >= (K_MIN_TWO_BYTE_VALUE_LEAD << 1) {
        if lead_byte < (K_MIN_THREE_BYTE_VALUE_LEAD << 1) {
            pos += 1;
        } else if lead_byte < (K_FOUR_BYTE_VALUE_LEAD << 1) {
            pos += 2;
        } else {
            pos += 3 + ((lead_byte >> 1) & 1);
        }
    }
    pos
}

/// `skipValue(byte[] bytes, int pos)` (private).
fn skip_value_at(bytes: &[i8], mut pos: i32) -> i32 {
    let lead_byte = ub(bytes, pos);
    pos += 1;
    skip_value(pos, lead_byte)
}

/// `jumpByDelta(byte[] bytes, int pos)`: reads a jump delta and jumps.
fn jump_by_delta(bytes: &[i8], mut pos: i32) -> i32 {
    let mut delta = ub(bytes, pos);
    pos += 1;
    if delta < K_MIN_TWO_BYTE_DELTA_LEAD {
        // nothing to do
    } else if delta < K_MIN_THREE_BYTE_DELTA_LEAD {
        delta = ((delta - K_MIN_TWO_BYTE_DELTA_LEAD) << 8) | ub(bytes, pos);
        pos += 1;
    } else if delta < K_FOUR_BYTE_DELTA_LEAD {
        delta = ((delta - K_MIN_THREE_BYTE_DELTA_LEAD) << 16)
            | (ub(bytes, pos) << 8)
            | ub(bytes, pos + 1);
        pos += 2;
    } else if delta == K_FOUR_BYTE_DELTA_LEAD {
        delta = (ub(bytes, pos) << 16) | (ub(bytes, pos + 1) << 8) | ub(bytes, pos + 2);
        pos += 3;
    } else {
        delta = ((bytes[pos as usize] as i32) << 24)
            | (ub(bytes, pos + 1) << 16)
            | (ub(bytes, pos + 2) << 8)
            | ub(bytes, pos + 3);
        pos += 4;
    }
    pos.wrapping_add(delta)
}

/// `skipDelta(byte[] bytes, int pos)` (private).
fn skip_delta(bytes: &[i8], mut pos: i32) -> i32 {
    let delta = ub(bytes, pos);
    pos += 1;
    if delta >= K_MIN_TWO_BYTE_DELTA_LEAD {
        if delta < K_MIN_THREE_BYTE_DELTA_LEAD {
            pos += 1;
        } else if delta < K_FOUR_BYTE_DELTA_LEAD {
            pos += 2;
        } else {
            pos += 3 + (delta & 1);
        }
    }
    pos
}

/// `findUniqueValueFromBranch(byte[] bytes, int pos, int length, long
/// uniqueValue)` (private): recursively finds a unique value (or whether
/// there is not a unique one) from a branch; on success, the position after
/// the branch is in bits 63..33.
fn find_unique_value_from_branch(
    bytes: &[i8],
    mut pos: i32,
    mut length: i32,
    mut unique_value: i64,
) -> i64 {
    while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
        pos += 1; // ignore the comparison byte
        unique_value = find_unique_value_from_branch(
            bytes,
            jump_by_delta(bytes, pos),
            length >> 1,
            unique_value,
        );
        if unique_value == 0 {
            return 0;
        }
        length -= length >> 1;
        pos = skip_delta(bytes, pos);
    }
    loop {
        pos += 1; // ignore a comparison byte
                  // handle its value
        let node = ub(bytes, pos);
        pos += 1;
        let is_final = (node & K_VALUE_IS_FINAL) != 0;
        let value = read_value(bytes, pos, node >> 1);
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
            unique_value = find_unique_value(bytes, pos.wrapping_add(value), unique_value);
            if unique_value == 0 {
                return 0;
            }
        }
        length -= 1;
        if length <= 1 {
            break;
        }
    }
    // ignore the last comparison byte
    (((pos + 1) as i64) << 33) | (unique_value & 0x1ffffffff)
}

/// `findUniqueValue(byte[] bytes, int pos, long uniqueValue)` (private).
fn find_unique_value(bytes: &[i8], mut pos: i32, mut unique_value: i64) -> i64 {
    loop {
        let mut node = ub(bytes, pos);
        pos += 1;
        if node < K_MIN_LINEAR_MATCH {
            if node == 0 {
                node = ub(bytes, pos);
                pos += 1;
            }
            unique_value = find_unique_value_from_branch(bytes, pos, node + 1, unique_value);
            if unique_value == 0 {
                return 0;
            }
            pos = ((unique_value as u64) >> 33) as i32;
        } else if node < K_MIN_VALUE_LEAD {
            // linear-match node
            pos += node - K_MIN_LINEAR_MATCH + 1; // Ignore the match bytes.
        } else {
            let is_final = (node & K_VALUE_IS_FINAL) != 0;
            let value = read_value(bytes, pos, node >> 1);
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
            pos = skip_value(pos, node);
        }
    }
}

/// `getNextBranchBytes(byte[] bytes, int pos, int length, Appendable out)`
/// (private).
fn get_next_branch_bytes(bytes: &[i8], mut pos: i32, mut length: i32, out: &mut Vec<u16>) {
    while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
        pos += 1; // ignore the comparison byte
        get_next_branch_bytes(bytes, jump_by_delta(bytes, pos), length >> 1, out);
        length -= length >> 1;
        pos = skip_delta(bytes, pos);
    }
    loop {
        append(out, ub(bytes, pos));
        pos += 1;
        pos = skip_value_at(bytes, pos);
        length -= 1;
        if length <= 1 {
            break;
        }
    }
    append(out, ub(bytes, pos));
}

/// `append(Appendable out, int c)` (private): `out.append((char) c)`.
fn append(out: &mut Vec<u16>, c: i32) {
    out.push(c as u16);
}

/// Return value type for the [`Iterator`]: a byte sequence and its value.
///
/// Equivalent to `com.ibm.icu.util.BytesTrie.Entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    bytes: Vec<i8>,
    length: i32,
    /// `value`: the value associated with the byte sequence, -1 when the
    /// sequence was truncated by the maximum length.
    pub value: i32,
}

impl Entry {
    /// `Entry(int capacity)` (private).
    fn new(capacity: i32) -> Self {
        Self {
            bytes: vec![0; capacity as usize],
            length: 0,
            value: 0,
        }
    }

    /// `bytesLength()`.
    pub fn bytes_length(&self) -> i32 {
        self.length
    }

    /// `byteAt(int index)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` outside the buffer.
    pub fn byte_at(&self, index: i32) -> i8 {
        self.bytes[index as usize]
    }

    /// `copyBytesTo(byte[] dest, int destOffset)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `dest` is too short.
    pub fn copy_bytes_to(&self, dest: &mut [i8], dest_offset: i32) {
        let d = dest_offset as usize;
        let n = self.length as usize;
        dest[d..d + n].copy_from_slice(&self.bytes[..n]);
    }

    /// `bytesAsByteBuffer()`: a read-only buffer over the sequence (position
    /// 0, limit the length, capacity the buffer's).
    pub fn bytes_as_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer::wrap_range(
            self.bytes.iter().map(|&b| b as u8).collect(),
            0,
            self.length,
        )
    }

    /// `ensureCapacity(int len)` (private).
    fn ensure_capacity(&mut self, len: i32) {
        if (self.bytes.len() as i32) < len {
            let new_len = (2 * self.bytes.len() as i32).min(2 * len);
            let mut new_bytes = vec![0i8; new_len as usize];
            new_bytes[..self.length as usize].copy_from_slice(&self.bytes[..self.length as usize]);
            self.bytes = new_bytes;
        }
    }

    /// `append(byte b)` (private).
    fn append_byte(&mut self, b: i8) {
        self.ensure_capacity(self.length + 1);
        self.bytes[self.length as usize] = b;
        self.length += 1;
    }

    /// `append(byte[] b, int off, int len)` (private).
    fn append_bytes(&mut self, b: &[i8], off: i32, len: i32) {
        self.ensure_capacity(self.length + len);
        let l = self.length as usize;
        self.bytes[l..l + len as usize].copy_from_slice(&b[off as usize..(off + len) as usize]);
        self.length += len;
    }

    /// `truncateString(int newLength)` (private).
    fn truncate_string(&mut self, new_length: i32) {
        self.length = new_length;
    }
}

/// Iterator for all of the (byte sequence, value) pairs in a `BytesTrie`.
///
/// Equivalent to `com.ibm.icu.util.BytesTrie.Iterator`. Each step returns a
/// copy of the entry the Java iterator overwrites in place.
#[derive(Debug, Clone)]
pub struct Iterator {
    bytes: Arc<[i8]>,
    pos: i32,
    initial_pos: i32,
    remaining_match_length: i32,
    initial_remaining_match_length: i32,
    max_length: i32,
    entry: Entry,
    /// The stack stores longs for backtracking to another
    /// outbound edge of a branch node.
    /// Each long has the offset from bytes_ in bits 62..32,
    /// the entry_.length from before the node in bits 15..0,
    /// and the remaining branch length in bits 24..16. (Bits 31..25 are unused.)
    /// (We could store the remaining branch length minus 1 in bits 23..16 and not use bits 31..24,
    /// but the code looks more confusing that way.)
    stack: Vec<i64>,
}

impl Iterator {
    /// `Iterator(byte[] trieBytes, int offset, int remainingMatchLength, int
    /// maxStringLength)` (private).
    fn new(
        trie_bytes: Arc<[i8]>,
        offset: i32,
        remaining_match_length: i32,
        max_string_length: i32,
    ) -> Self {
        let mut it = Self {
            bytes: trie_bytes,
            pos: offset,
            initial_pos: offset,
            remaining_match_length,
            initial_remaining_match_length: remaining_match_length,
            max_length: max_string_length,
            entry: Entry::new(if max_string_length != 0 {
                max_string_length
            } else {
                32
            }),
            stack: Vec::new(),
        };
        let mut length = it.remaining_match_length; // Actual remaining match length minus 1.
        if length >= 0 {
            // Pending linear-match node, append remaining bytes to entry_.
            length += 1;
            if it.max_length > 0 && length > it.max_length {
                length = it.max_length; // This will leave remainingMatchLength>=0 as a signal.
            }
            let bytes = it.bytes.clone();
            it.entry.append_bytes(&bytes, it.pos, length);
            it.pos += length;
            it.remaining_match_length -= length;
        }
        it
    }

    /// `reset()`: back to the initial state.
    pub fn reset(&mut self) -> &mut Self {
        self.pos = self.initial_pos;
        self.remaining_match_length = self.initial_remaining_match_length;
        let mut length = self.remaining_match_length + 1; // Remaining match length.
        if self.max_length > 0 && length > self.max_length {
            length = self.max_length;
        }
        self.entry.truncate_string(length);
        self.pos += length;
        self.remaining_match_length -= length;
        self.stack.clear();
        self
    }

    /// `hasNext()`: true if there are more elements.
    pub fn has_next(&self) -> bool {
        self.pos >= 0 || !self.stack.is_empty()
    }

    /// `remove()`: unsupported.
    ///
    /// # Errors
    ///
    /// Always `UnsupportedOperationException`.
    pub fn remove(&mut self) -> JResult<()> {
        Err(crate::jdk::unchecked_no_message(
            "java.lang.UnsupportedOperationException",
        ))
    }

    /// `next()` itself: the next entry, `None` where Java throws
    /// `NoSuchElementException`.
    fn next_entry(&mut self) -> Option<&Entry> {
        let mut pos = self.pos;
        if pos < 0 {
            let top = self.stack.pop()?;
            // Pop the state off the stack and continue with the next outbound edge of
            // the branch node.
            let mut length = top as i32;
            pos = (top >> 32) as i32;
            self.entry.truncate_string(length & 0xffff);
            length = ((length as u32) >> 16) as i32;
            if length > 1 {
                pos = self.branch_next(pos, length);
                if pos < 0 {
                    return Some(&self.entry); // Reached a final value.
                }
            } else {
                let b = self.bytes[pos as usize];
                pos += 1;
                self.entry.append_byte(b);
            }
        }
        if self.remaining_match_length >= 0 {
            // We only get here if we started in a pending linear-match node
            // with more than maxLength remaining bytes.
            return Some(self.truncate_and_stop());
        }
        loop {
            let mut node = ub(&self.bytes, pos);
            pos += 1;
            if node >= K_MIN_VALUE_LEAD {
                // Deliver value for the byte sequence so far.
                let is_final = (node & K_VALUE_IS_FINAL) != 0;
                self.entry.value = read_value(&self.bytes, pos, node >> 1);
                if is_final || (self.max_length > 0 && self.entry.length == self.max_length) {
                    self.pos = -1;
                } else {
                    self.pos = skip_value(pos, node);
                }
                return Some(&self.entry);
            }
            if self.max_length > 0 && self.entry.length == self.max_length {
                return Some(self.truncate_and_stop());
            }
            if node < K_MIN_LINEAR_MATCH {
                if node == 0 {
                    node = ub(&self.bytes, pos);
                    pos += 1;
                }
                pos = self.branch_next(pos, node + 1);
                if pos < 0 {
                    return Some(&self.entry); // Reached a final value.
                }
            } else {
                // Linear-match node, append length bytes to entry_.
                let length = node - K_MIN_LINEAR_MATCH + 1;
                let bytes = self.bytes.clone();
                if self.max_length > 0 && self.entry.length + length > self.max_length {
                    let n = self.max_length - self.entry.length;
                    self.entry.append_bytes(&bytes, pos, n);
                    return Some(self.truncate_and_stop());
                }
                self.entry.append_bytes(&bytes, pos, length);
                pos += length;
            }
        }
    }

    /// `truncateAndStop()` (private).
    fn truncate_and_stop(&mut self) -> &Entry {
        self.pos = -1;
        self.entry.value = -1; // no real value for str
        &self.entry
    }

    /// `branchNext(int pos, int length)` (private).
    fn branch_next(&mut self, mut pos: i32, mut length: i32) -> i32 {
        while length > K_MAX_BRANCH_LINEAR_SUB_NODE_LENGTH {
            pos += 1; // ignore the comparison byte
                      // Push state for the greater-or-equal edge.
            let low = ((length - (length >> 1)) << 16) | self.entry.length;
            self.stack
                .push(((skip_delta(&self.bytes, pos) as i64) << 32) | (low as i64));
            // Follow the less-than edge.
            length >>= 1;
            pos = jump_by_delta(&self.bytes, pos);
        }
        // List of key-value pairs where values are either final values or jump deltas.
        // Read the first (key, value) pair.
        let trie_byte = self.bytes[pos as usize];
        pos += 1;
        let node = ub(&self.bytes, pos);
        pos += 1;
        let is_final = (node & K_VALUE_IS_FINAL) != 0;
        let value = read_value(&self.bytes, pos, node >> 1);
        pos = skip_value(pos, node);
        let low = ((length - 1) << 16) | self.entry.length;
        self.stack.push(((pos as i64) << 32) | (low as i64));
        self.entry.append_byte(trie_byte);
        if is_final {
            self.pos = -1;
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
