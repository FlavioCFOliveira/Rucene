// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationIterator`.

use std::sync::Arc;

use crate::jdk::{char_count, code_point_at, is_low_surrogate, offset_by_code_points, to_code_point};
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fcd::CollationFCD;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::r#impl::normalizer2_impl::Hangul;
use crate::r#impl::trie2_32::Trie2_32;
use crate::util::bytes_trie::Result as TrieResult;
use crate::util::chars_trie::{CharsTrie, State};

const INITIAL_CAPACITY: usize = 40;

/// `CollationIterator.CEBuffer` (private).
#[derive(Debug, Clone)]
struct CEBuffer {
    length: i32,
    buffer: Vec<i64>,
}

impl CEBuffer {
    fn new() -> Self {
        Self { length: 0, buffer: vec![0; INITIAL_CAPACITY] }
    }

    fn append(&mut self, ce: i64) {
        if self.length as usize >= INITIAL_CAPACITY {
            self.ensure_append_capacity(1);
        }
        self.buffer[self.length as usize] = ce;
        self.length += 1;
    }

    fn append_unsafe(&mut self, ce: i64) {
        self.buffer[self.length as usize] = ce;
        self.length += 1;
    }

    fn ensure_append_capacity(&mut self, app_cap: i32) {
        let mut capacity = self.buffer.len() as i32;
        if (self.length + app_cap) <= capacity {
            return;
        }
        loop {
            if capacity < 1000 {
                capacity *= 4;
            } else {
                capacity *= 2;
            }
            if capacity >= (self.length + app_cap) {
                break;
            }
        }
        let mut new_buffer = vec![0i64; capacity as usize];
        new_buffer[..self.length as usize].copy_from_slice(&self.buffer[..self.length as usize]);
        self.buffer = new_buffer;
    }

    fn inc_length(&mut self) {
        // Use INITIAL_CAPACITY for a very simple fastpath.
        // (Rather than buffer.getCapacity().)
        if self.length as usize >= INITIAL_CAPACITY {
            self.ensure_append_capacity(1);
        }
        self.length += 1;
    }

    fn set(&mut self, i: i32, ce: i64) -> i64 {
        self.buffer[i as usize] = ce;
        ce
    }

    fn get(&self, i: i32) -> i64 {
        self.buffer[i as usize]
    }
}

/// State of combining marks skipped in discontiguous contraction.
/// We create a state object on first use and keep it around deactivated
/// between uses.
///
/// Equivalent to `CollationIterator.SkippedState` (private).
#[derive(Debug, Clone, Default)]
pub struct SkippedState {
    /// Combining marks skipped in previous discontiguous-contraction matching.
    old_buffer: Vec<u16>,
    /// Combining marks newly skipped in current discontiguous-contraction matching.
    new_buffer: Vec<u16>,
    /// Reading index in oldBuffer, or counter for how many code points have
    /// been read beyond oldBuffer (pos-oldBuffer.length()).
    pos: i32,
    /// newBuffer.length() at the time of the last matching character.
    skip_length_at_match: i32,
    /// We save the trie state before we attempt to match a character.
    state: State,
}

fn append_code_point(sb: &mut Vec<u16>, c: i32) {
    crate::jdk::append_code_point(sb, c).unwrap_or_else(|e| panic!("{e}"));
}

impl SkippedState {
    fn clear(&mut self) {
        self.old_buffer.clear();
        self.pos = 0;
        // The newBuffer is reset by setFirstSkipped().
    }

    fn is_empty(&self) -> bool {
        self.old_buffer.is_empty()
    }

    fn has_next(&self) -> bool {
        self.pos < self.old_buffer.len() as i32
    }

    // Requires hasNext().
    fn next(&mut self) -> i32 {
        let c = code_point_at(&self.old_buffer, self.pos);
        self.pos += char_count(c);
        c
    }

    // Accounts for one more input code point read beyond the end of the marks buffer.
    fn inc_beyond(&mut self) {
        debug_assert!(!self.has_next());
        self.pos += 1;
    }

    // Goes backward through the skipped-marks buffer.
    // Returns the number of code points read beyond the skipped marks
    // that need to be backtracked through normal input.
    fn backward_num_code_points(&mut self, n: i32) -> i32 {
        let length = self.old_buffer.len() as i32;
        let beyond = self.pos - length;
        if beyond > 0 {
            if beyond >= n {
                // Not back far enough to re-enter the oldBuffer.
                self.pos -= n;
                n
            } else {
                // Back out all beyond-oldBuffer code points and re-enter the buffer.
                self.pos = offset_by_code_points(&self.old_buffer, length, beyond - n);
                beyond
            }
        } else {
            // Go backwards from inside the oldBuffer.
            self.pos = offset_by_code_points(&self.old_buffer, self.pos, -n);
            0
        }
    }

    fn set_first_skipped(&mut self, c: i32) {
        self.skip_length_at_match = 0;
        self.new_buffer.clear();
        append_code_point(&mut self.new_buffer, c);
    }

    fn skip(&mut self, c: i32) {
        append_code_point(&mut self.new_buffer, c);
    }

    fn record_match(&mut self) {
        self.skip_length_at_match = self.new_buffer.len() as i32;
    }

    // Replaces the characters we consumed with the newly skipped ones.
    fn replace_match(&mut self) {
        // Note: UnicodeString.replace() pins pos to at most length().
        let old_length = self.old_buffer.len() as i32;
        if self.pos > old_length {
            self.pos = old_length;
        }
        self.old_buffer.drain(0..self.pos as usize);
        let inserted: Vec<u16> = self.new_buffer[..self.skip_length_at_match as usize].to_vec();
        self.old_buffer.splice(0..0, inserted);
        self.pos = 0;
    }

    fn save_trie_state(&mut self, trie: &CharsTrie) {
        trie.save_state(&mut self.state);
    }

    fn reset_to_trie_state(&self, trie: &mut CharsTrie) {
        if let Err(e) = trie.reset_to_state(&self.state) {
            panic!("{e}");
        }
    }
}

/// `NO_CP_AND_CE32`.
pub const NO_CP_AND_CE32: i64 = (-1i64 << 32) | (Collation::FALLBACK_CE32 as i64 & 0xffffffff);

/// The state of the abstract class `CollationIterator`: the fields every
/// subclass inherits.
///
/// Equivalent to the fields of `com.ibm.icu.impl.coll.CollationIterator`.
#[derive(Debug, Clone)]
pub struct CollationIteratorBase {
    /// Main lookup trie of the data object.
    pub trie: Option<Arc<Trie2_32>>,
    /// `data`.
    pub data: Arc<CollationData>,
    ce_buffer: Option<CEBuffer>,
    ces_index: i32,
    skipped: Option<SkippedState>,
    /// Number of code points to read forward, or -1.
    /// Used as a forward iteration limit in previousCEUnsafe().
    num_cp_fwd: i32,
    /// Numeric collation (CollationSettings.NUMERIC).
    is_numeric: bool,
}

impl CollationIteratorBase {
    /// `CollationIterator(CollationData d)`.
    pub fn new(d: Arc<CollationData>) -> Self {
        Self {
            trie: d.trie.clone(),
            data: d,
            ce_buffer: None,
            ces_index: 0,
            skipped: None,
            num_cp_fwd: -1,
            is_numeric: false,
        }
    }

    /// `CollationIterator(CollationData d, boolean numeric)`.
    pub fn with_numeric(d: Arc<CollationData>, numeric: bool) -> Self {
        Self {
            trie: d.trie.clone(),
            data: d,
            ce_buffer: Some(CEBuffer::new()),
            ces_index: 0,
            skipped: None,
            num_cp_fwd: -1,
            is_numeric: numeric,
        }
    }

    fn ceb(&self) -> &CEBuffer {
        self.ce_buffer.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn ceb_mut(&mut self) -> &mut CEBuffer {
        self.ce_buffer.as_mut().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `equals(Object)` of the base class: compares the iterator state but
    /// not the collation data (trie & data fields). The subclasses compare
    /// their own state and the runtime class.
    pub fn equals(&self, o: &CollationIteratorBase) -> bool {
        let (a, b) = (self.ceb(), o.ceb());
        if !(a.length == b.length
            && self.ces_index == o.ces_index
            && self.num_cp_fwd == o.num_cp_fwd
            && self.is_numeric == o.is_numeric)
        {
            return false;
        }
        for i in 0..a.length {
            if a.get(i) != b.get(i) {
                return false;
            }
        }
        true
    }

    /// `hashCode()`: a dummy value (Java returns 0 "to prevent compile
    /// warnings").
    pub fn hash_code(&self) -> i32 {
        0
    }

    /// `reset()` (protected final).
    pub fn reset(&mut self) {
        let ceb = self.ceb_mut();
        ceb.length = 0;
        self.ces_index = 0;
        if let Some(skipped) = self.skipped.as_mut() {
            skipped.clear();
        }
    }

    /// `reset(boolean numeric)` (protected final).
    pub fn reset_numeric(&mut self, numeric: bool) {
        if self.ce_buffer.is_none() {
            self.ce_buffer = Some(CEBuffer::new());
        }
        self.reset();
        self.is_numeric = numeric;
    }

    /// `getCEsLength()`.
    pub fn get_ces_length(&self) -> i32 {
        self.ceb().length
    }

    /// `getCE(int)`.
    pub fn get_ce(&self, i: i32) -> i64 {
        self.ceb().get(i)
    }

    /// `getCEs()`: the whole CE buffer (its length is the capacity).
    pub fn get_ces(&self) -> &[i64] {
        &self.ceb().buffer
    }

    /// `clearCEs()`.
    pub fn clear_ces(&mut self) {
        self.ceb_mut().length = 0;
        self.ces_index = 0;
    }

    /// `clearCEsIfNoneRemaining()`.
    pub fn clear_ces_if_none_remaining(&mut self) {
        if self.ces_index == self.ceb().length {
            self.clear_ces();
        }
    }

    /// `setCurrentCE(long)`.
    pub fn set_current_ce(&mut self, ce: i64) {
        debug_assert!(self.ces_index > 0);
        let i = self.ces_index - 1;
        self.ceb_mut().set(i, ce);
    }

    /// `makeCodePointAndCE32Pair(int, int)`.
    pub fn make_code_point_and_ce32_pair(c: i32, ce32: i32) -> i64 {
        ((c as i64) << 32) | (ce32 as i64 & 0xffffffff)
    }
}

/// `isSurrogate(int)`.
fn is_surrogate(c: i32) -> bool {
    (c as u32 & 0xfffff800) == 0xd800
}

/// `isLeadSurrogate(int)`.
pub fn is_lead_surrogate(c: i32) -> bool {
    (c as u32 & 0xfffffc00) == 0xd800
}

/// `isTrailSurrogate(int)`.
pub fn is_trail_surrogate(c: i32) -> bool {
    (c as u32 & 0xfffffc00) == 0xdc00
}

fn base_of(d: &CollationData) -> Arc<CollationData> {
    d.base.clone().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
}

/// Collation element iterator and abstract character iterator.
///
/// Equivalent to the abstract class
/// `com.ibm.icu.impl.coll.CollationIterator`: the abstract and overridable
/// methods are required and provided trait methods; the `final` methods are
/// provided methods that implementors do not override. The shared state is
/// a [`CollationIteratorBase`].
///
/// The "internal program error: should be unreachable" `ICUException`s are
/// panics: ICU never catches them and they cannot occur with valid data.
pub trait CollationIterator {
    /// The inherited state.
    fn base(&self) -> &CollationIteratorBase;

    /// The inherited state, mutably.
    fn base_mut(&mut self) -> &mut CollationIteratorBase;

    /// Resets the iterator state and sets the position to the specified
    /// offset.
    fn reset_to_offset(&mut self, new_offset: i32);

    /// `getOffset()`.
    fn get_offset(&self) -> i32;

    /// Returns the next code point (with post-increment); SENTINEL_CP (<0)
    /// at the end.
    fn next_code_point(&mut self) -> i32;

    /// Returns the previous code point (with pre-decrement); SENTINEL_CP
    /// (<0) at the start.
    fn previous_code_point(&mut self) -> i32;

    /// `forwardNumCodePoints(int)`.
    fn forward_num_code_points(&mut self, num: i32);

    /// `backwardNumCodePoints(int)`.
    fn backward_num_code_points(&mut self, num: i32);

    /// Returns the next code point and its local CE32 value.
    ///
    /// Equivalent to `handleNextCE32()`.
    fn handle_next_ce32(&mut self) -> i64 {
        let c = self.next_code_point();
        if c < 0 {
            return NO_CP_AND_CE32;
        }
        CollationIteratorBase::make_code_point_and_ce32_pair(c, self.base().data.get_ce32(c))
    }

    /// Called when handleNextCE32() returns a LEAD_SURROGATE_TAG for a lead
    /// surrogate code unit.
    fn handle_get_trail_surrogate(&mut self) -> u16 {
        0
    }

    /// Returns true if surrogate code points should map to U+FFFD.
    fn forbid_surrogate_code_points(&self) -> bool {
        false
    }

    /// Returns the CE32 from the data trie. Normally the same as data.getCE32().
    fn get_data_ce32(&mut self, c: i32) -> i32 {
        self.base().data.get_ce32(c)
    }

    /// `getCE32FromBuilderData(int)`.
    fn get_ce32_from_builder_data(&mut self, _ce32: i32) -> i32 {
        panic!("com.ibm.icu.util.ICUException: internal program error: should be unreachable")
    }

    /// `nextCE()` (final).
    fn next_ce(&mut self) -> i64 {
        {
            let b = self.base_mut();
            let ces_index = b.ces_index;
            let ceb = b.ceb_mut();
            if ces_index < ceb.length {
                // Return the next buffered CE.
                b.ces_index += 1;
                return b.ceb().get(ces_index);
            }
            debug_assert!(ces_index == ceb.length);
            ceb.inc_length();
        }
        let c_and_ce32 = self.handle_next_ce32();
        let c = (c_and_ce32 >> 32) as i32;
        let mut ce32 = c_and_ce32 as i32;
        let mut t = ce32 & 0xff;
        if t < Collation::SPECIAL_CE32_LOW_BYTE {
            // Forced-inline of isSpecialCE32(ce32).
            // Normal CE from the main data.
            // Forced-inline of ceFromSimpleCE32(ce32).
            return self.set_next(
                (((ce32 as u32 & 0xffff0000) as i32 as i64) << 32) | (((ce32 & 0xff00) as i64) << 16) | (t << 8) as i64,
            );
        }
        let d: Arc<CollationData>;
        // The compiler should be able to optimize the previous and the following
        // comparisons of t with the same constant.
        if t == Collation::SPECIAL_CE32_LOW_BYTE {
            if c < 0 {
                return self.set_next(Collation::NO_CE);
            }
            d = base_of(&self.base().data);
            ce32 = d.get_ce32(c);
            t = ce32 & 0xff;
            if t < Collation::SPECIAL_CE32_LOW_BYTE {
                // Normal CE from the base data.
                return self.set_next(
                    (((ce32 as u32 & 0xffff0000) as i32 as i64) << 32)
                        | (((ce32 & 0xff00) as i64) << 16)
                        | (t << 8) as i64,
                );
            }
        } else {
            d = self.base().data.clone();
        }
        if t == Collation::LONG_PRIMARY_CE32_LOW_BYTE {
            // Forced-inline of ceFromLongPrimaryCE32(ce32).
            return self.set_next((((ce32 - t) as i64) << 32) | Collation::COMMON_SEC_AND_TER_CE as i64);
        }
        self.next_ce_from_ce32(d, c, ce32)
    }

    #[doc(hidden)]
    /// `ceBuffer.set(cesIndex++, ce)`.
    fn set_next(&mut self, ce: i64) -> i64 {
        let b = self.base_mut();
        let i = b.ces_index;
        b.ces_index += 1;
        b.ceb_mut().set(i, ce)
    }

    /// `fetchCEs()` (final).
    fn fetch_ces(&mut self) -> i32 {
        while self.next_ce() != Collation::NO_CE {
            // No need to loop for each expansion CE.
            let b = self.base_mut();
            b.ces_index = b.ceb().length;
        }
        self.base().ceb().length
    }

    /// `previousCE(UVector32)` (final).
    fn previous_ce(&mut self, offsets: &mut UVector32) -> i64 {
        if self.base().ceb().length > 0 {
            // Return the previous buffered CE.
            let ceb = self.base_mut().ceb_mut();
            ceb.length -= 1;
            return ceb.get(ceb.length);
        }
        offsets.remove_all_elements();
        let limit_offset = self.get_offset();
        let c = self.previous_code_point();
        if c < 0 {
            return Collation::NO_CE;
        }
        let is_numeric = self.base().is_numeric;
        if self.base().data.is_unsafe_backward(c, is_numeric) {
            return self.previous_ce_unsafe(c, offsets);
        }
        // Simple, safe-backwards iteration:
        // Get a CE going backwards, handle prefixes but no contractions.
        let mut ce32 = self.base().data.get_ce32(c);
        let d: Arc<CollationData>;
        if ce32 == Collation::FALLBACK_CE32 {
            d = base_of(&self.base().data);
            ce32 = d.get_ce32(c);
        } else {
            d = self.base().data.clone();
        }
        if Collation::is_simple_or_long_ce32(ce32) {
            return Collation::ce_from_ce32(ce32);
        }
        self.append_ces_from_ce32(d, c, ce32, false);
        if self.base().ceb().length > 1 {
            offsets.add_element(self.get_offset());
            // For an expansion, the offset of each non-initial CE is the limit offset,
            // consistent with forward iteration.
            while offsets.size() <= self.base().ceb().length {
                offsets.add_element(limit_offset);
            }
        }
        let ceb = self.base_mut().ceb_mut();
        ceb.length -= 1;
        ceb.get(ceb.length)
    }

    /// `appendCEsFromCE32(CollationData, int, int, boolean)` (protected final).
    fn append_ces_from_ce32(&mut self, mut d: Arc<CollationData>, mut c: i32, mut ce32: i32, forward: bool) {
        while Collation::is_special_ce32(ce32) {
            match Collation::tag_from_ce32(ce32) {
                Collation::FALLBACK_TAG | Collation::RESERVED_TAG_3 => {
                    panic!("com.ibm.icu.util.ICUException: internal program error: should be unreachable")
                }
                Collation::LONG_PRIMARY_TAG => {
                    self.base_mut().ceb_mut().append(Collation::ce_from_long_primary_ce32(ce32));
                    return;
                }
                Collation::LONG_SECONDARY_TAG => {
                    self.base_mut().ceb_mut().append(Collation::ce_from_long_secondary_ce32(ce32));
                    return;
                }
                Collation::LATIN_EXPANSION_TAG => {
                    let ceb = self.base_mut().ceb_mut();
                    ceb.ensure_append_capacity(2);
                    let l = ceb.length;
                    ceb.set(l, Collation::latin_ce0_from_ce32(ce32));
                    ceb.set(l + 1, Collation::latin_ce1_from_ce32(ce32));
                    ceb.length += 2;
                    return;
                }
                Collation::EXPANSION32_TAG => {
                    let mut index = Collation::index_from_ce32(ce32);
                    let mut length = Collation::length_from_ce32(ce32);
                    let ceb = self.base_mut().ceb_mut();
                    ceb.ensure_append_capacity(length);
                    loop {
                        ceb.append_unsafe(Collation::ce_from_ce32(d.ce32s[index as usize]));
                        index += 1;
                        length -= 1;
                        if length <= 0 {
                            break;
                        }
                    }
                    return;
                }
                Collation::EXPANSION_TAG => {
                    let mut index = Collation::index_from_ce32(ce32);
                    let mut length = Collation::length_from_ce32(ce32);
                    let ceb = self.base_mut().ceb_mut();
                    ceb.ensure_append_capacity(length);
                    loop {
                        ceb.append_unsafe(d.ces[index as usize]);
                        index += 1;
                        length -= 1;
                        if length <= 0 {
                            break;
                        }
                    }
                    return;
                }
                Collation::BUILDER_DATA_TAG => {
                    ce32 = self.get_ce32_from_builder_data(ce32);
                    if ce32 == Collation::FALLBACK_CE32 {
                        d = base_of(&self.base().data);
                        ce32 = d.get_ce32(c);
                    } else {
                        // BUILDER_DATA_TAG occurs only in the builder's data, which
                        // getCE32FromBuilderData() may have updated in place in Java.
                        d = self.base().data.clone();
                    }
                }
                Collation::PREFIX_TAG => {
                    if forward {
                        self.backward_num_code_points(1);
                    }
                    ce32 = self.get_ce32_from_prefix(&d, ce32);
                    if forward {
                        self.forward_num_code_points(1);
                    }
                }
                Collation::CONTRACTION_TAG => {
                    let index = Collation::index_from_ce32(ce32);
                    let default_ce32 = d.get_ce32_from_contexts(index); // Default if no suffix match.
                    if !forward {
                        // Backward contractions are handled by previousCEUnsafe().
                        // c has contractions but they were not found.
                        ce32 = default_ce32;
                        continue;
                    }
                    let next_cp;
                    if self.base().skipped.is_none() && self.base().num_cp_fwd < 0 {
                        // Some portion of nextCE32FromContraction() pulled out here as an ASCII fast path,
                        // avoiding the function call and the nextSkippedCodePoint() overhead.
                        next_cp = self.next_code_point();
                        if next_cp < 0 {
                            // No more text.
                            ce32 = default_ce32;
                            continue;
                        } else if (ce32 & Collation::CONTRACT_NEXT_CCC) != 0 && !CollationFCD::may_have_lccc(next_cp) {
                            // All contraction suffixes start with characters with lccc!=0
                            // but the next code point has lccc==0.
                            self.backward_num_code_points(1);
                            ce32 = default_ce32;
                            continue;
                        }
                    } else {
                        next_cp = self.next_skipped_code_point();
                        if next_cp < 0 {
                            // No more text.
                            ce32 = default_ce32;
                            continue;
                        } else if (ce32 & Collation::CONTRACT_NEXT_CCC) != 0 && !CollationFCD::may_have_lccc(next_cp) {
                            // All contraction suffixes start with characters with lccc!=0
                            // but the next code point has lccc==0.
                            self.backward_num_skipped(1);
                            ce32 = default_ce32;
                            continue;
                        }
                    }
                    let contexts = d.contexts.clone();
                    ce32 = self.next_ce32_from_contraction(d.clone(), ce32, contexts, index + 2, default_ce32, next_cp);
                    if ce32 == Collation::NO_CE32 {
                        // CEs from a discontiguous contraction plus the skipped combining marks
                        // have been appended already.
                        return;
                    }
                }
                Collation::DIGIT_TAG => {
                    if self.base().is_numeric {
                        self.append_numeric_ces(ce32, forward);
                        return;
                    } else {
                        // Fetch the non-numeric-collation CE32 and continue.
                        ce32 = d.ce32s[Collation::index_from_ce32(ce32) as usize];
                    }
                }
                Collation::U0000_TAG => {
                    debug_assert!(c == 0);
                    // NUL-terminated input not supported in Java.
                    // Fetch the normal ce32 for U+0000 and continue.
                    ce32 = d.ce32s[0];
                }
                Collation::HANGUL_TAG => {
                    let jamo_ce32s = d.jamo_ce32s.clone();
                    c -= Hangul::HANGUL_BASE;
                    let t = c % Hangul::JAMO_T_COUNT;
                    c /= Hangul::JAMO_T_COUNT;
                    let v = c % Hangul::JAMO_V_COUNT;
                    c /= Hangul::JAMO_V_COUNT;
                    if (ce32 & Collation::HANGUL_NO_SPECIAL_JAMO) != 0 {
                        // None of the Jamo CE32s are isSpecialCE32().
                        // Avoid recursive function calls and per-Jamo tests.
                        let ceb = self.base_mut().ceb_mut();
                        ceb.ensure_append_capacity(if t == 0 { 2 } else { 3 });
                        let l = ceb.length;
                        ceb.set(l, Collation::ce_from_ce32(jamo_ce32s[c as usize]));
                        ceb.set(l + 1, Collation::ce_from_ce32(jamo_ce32s[(19 + v) as usize]));
                        ceb.length += 2;
                        if t != 0 {
                            ceb.append_unsafe(Collation::ce_from_ce32(jamo_ce32s[(39 + t) as usize]));
                        }
                        return;
                    } else {
                        // We should not need to compute each Jamo code point.
                        // In particular, there should be no offset or implicit ce32.
                        self.append_ces_from_ce32(d.clone(), Collation::SENTINEL_CP, jamo_ce32s[c as usize], forward);
                        self.append_ces_from_ce32(
                            d.clone(),
                            Collation::SENTINEL_CP,
                            jamo_ce32s[(19 + v) as usize],
                            forward,
                        );
                        if t == 0 {
                            return;
                        }
                        // offset 39 = 19 + 21 - 1:
                        // 19 = JAMO_L_COUNT
                        // 21 = JAMO_T_COUNT
                        // -1 = omit t==0
                        ce32 = jamo_ce32s[(39 + t) as usize];
                        c = Collation::SENTINEL_CP;
                    }
                }
                Collation::LEAD_SURROGATE_TAG => {
                    debug_assert!(forward); // Backward iteration should never see lead surrogate code _unit_ data.
                    debug_assert!(is_lead_surrogate(c));
                    let trail = self.handle_get_trail_surrogate();
                    if is_low_surrogate(trail) {
                        c = to_code_point(c as u16, trail);
                        ce32 &= Collation::LEAD_TYPE_MASK;
                        if ce32 == Collation::LEAD_ALL_UNASSIGNED {
                            ce32 = Collation::UNASSIGNED_CE32; // unassigned-implicit
                        } else if ce32 == Collation::LEAD_ALL_FALLBACK || {
                            ce32 = d.get_ce32_from_supplementary(c);
                            ce32 == Collation::FALLBACK_CE32
                        } {
                            // fall back to the base data
                            d = base_of(&d);
                            ce32 = d.get_ce32_from_supplementary(c);
                        }
                    } else {
                        // c is an unpaired surrogate.
                        ce32 = Collation::UNASSIGNED_CE32;
                    }
                }
                Collation::OFFSET_TAG => {
                    debug_assert!(c >= 0);
                    let ce = d.get_ce_from_offset_ce32(c, ce32);
                    self.base_mut().ceb_mut().append(ce);
                    return;
                }
                Collation::IMPLICIT_TAG => {
                    debug_assert!(c >= 0);
                    if is_surrogate(c) && self.forbid_surrogate_code_points() {
                        ce32 = Collation::FFFD_CE32;
                    } else {
                        self.base_mut().ceb_mut().append(Collation::unassigned_ce_from_code_point(c));
                        return;
                    }
                }
                _ => unreachable!("tags are 4 bits"),
            }
        }
        self.base_mut().ceb_mut().append(Collation::ce_from_simple_ce32(ce32));
    }

    #[doc(hidden)]
    /// `nextCEFromCE32(CollationData, int, int)` (private final).
    fn next_ce_from_ce32(&mut self, d: Arc<CollationData>, c: i32, ce32: i32) -> i64 {
        self.base_mut().ceb_mut().length -= 1; // Undo ceBuffer.incLength().
        self.append_ces_from_ce32(d, c, ce32, true);
        let b = self.base_mut();
        let i = b.ces_index;
        b.ces_index += 1;
        b.ceb().get(i)
    }

    #[doc(hidden)]
    /// `getCE32FromPrefix(CollationData, int)` (private final).
    fn get_ce32_from_prefix(&mut self, d: &CollationData, mut ce32: i32) -> i32 {
        let mut index = Collation::index_from_ce32(ce32);
        ce32 = d.get_ce32_from_contexts(index); // Default if no prefix match.
        index += 2;
        // Number of code points read before the original code point.
        let mut look_behind = 0;
        let mut prefixes = CharsTrie::new(d.contexts.clone(), index);
        loop {
            let c = self.previous_code_point();
            if c < 0 {
                break;
            }
            look_behind += 1;
            let m = prefixes.next_for_code_point(c);
            if m.has_value() {
                ce32 = prefixes.get_value();
            }
            if !m.has_next() {
                break;
            }
        }
        self.forward_num_code_points(look_behind);
        ce32
    }

    #[doc(hidden)]
    /// `nextSkippedCodePoint()` (private final).
    fn next_skipped_code_point(&mut self) -> i32 {
        if let Some(skipped) = self.base_mut().skipped.as_mut() {
            if skipped.has_next() {
                return skipped.next();
            }
        }
        if self.base().num_cp_fwd == 0 {
            return Collation::SENTINEL_CP;
        }
        let c = self.next_code_point();
        let b = self.base_mut();
        if let Some(skipped) = b.skipped.as_mut() {
            if !skipped.is_empty() && c >= 0 {
                skipped.inc_beyond();
            }
        }
        if b.num_cp_fwd > 0 && c >= 0 {
            b.num_cp_fwd -= 1;
        }
        c
    }

    #[doc(hidden)]
    /// `backwardNumSkipped(int)` (private final).
    fn backward_num_skipped(&mut self, mut n: i32) {
        if let Some(skipped) = self.base_mut().skipped.as_mut() {
            if !skipped.is_empty() {
                n = skipped.backward_num_code_points(n);
            }
        }
        self.backward_num_code_points(n);
        let b = self.base_mut();
        if b.num_cp_fwd >= 0 {
            b.num_cp_fwd += n;
        }
    }

    #[doc(hidden)]
    /// `nextCE32FromContraction(...)` (private final).
    fn next_ce32_from_contraction(
        &mut self,
        d: Arc<CollationData>,
        contraction_ce32: i32,
        trie_chars: Arc<[u16]>,
        trie_offset: i32,
        mut ce32: i32,
        mut c: i32,
    ) -> i32 {
        // c: next code point after the original one

        // Number of code points read beyond the original code point.
        // Needed for discontiguous contraction matching.
        let mut look_ahead = 1;
        // Number of code points read since the last match (initially only c).
        let mut since_match = 1;
        // Normally we only need a contiguous match,
        // and therefore need not remember the suffixes state from before a mismatch for retrying.
        // If we are already processing skipped combining marks, then we do track the state.
        let mut suffixes = CharsTrie::new(trie_chars, trie_offset);
        self.save_skipped_trie_state_if_active(&suffixes);
        let mut m = suffixes.first_for_code_point(c);
        loop {
            let next_cp;
            if m.has_value() {
                ce32 = suffixes.get_value();
                if !m.has_next() || {
                    c = self.next_skipped_code_point();
                    c < 0
                } {
                    return ce32;
                }
                self.save_skipped_trie_state_if_active(&suffixes);
                since_match = 1;
            } else if m == TrieResult::NoMatch || {
                next_cp = self.next_skipped_code_point();
                next_cp < 0
            } {
                // No match for c, or partial match (BytesTrie.Result.NO_VALUE) and no further text.
                // Back up if necessary, and try a discontiguous contraction.
                if (contraction_ce32 & Collation::CONTRACT_TRAILING_CCC) != 0
                    // Discontiguous contraction matching extends an existing match.
                    // If there is no match yet, then there is nothing to do.
                    && ((contraction_ce32 & Collation::CONTRACT_SINGLE_CP_NO_MATCH) == 0 || since_match < look_ahead)
                {
                    // The last character of at least one suffix has lccc!=0,
                    // allowing for discontiguous contractions.
                    // UCA S2.1.1 only processes non-starters immediately following
                    // "a match in the table" (sinceMatch=1).
                    if since_match > 1 {
                        // Return to the state after the last match.
                        // (Return to sinceMatch=0 and re-fetch the first partially-matched character.)
                        self.backward_num_skipped(since_match);
                        c = self.next_skipped_code_point();
                        look_ahead -= since_match - 1;
                        since_match = 1;
                    }
                    if d.get_fcd16(c) > 0xff {
                        return self.next_ce32_from_discontiguous_contraction(d, &mut suffixes, ce32, look_ahead, c);
                    }
                }
                break;
            } else {
                // Continue after partial match (BytesTrie.Result.NO_VALUE) for c.
                // It does not have a result value, therefore it is not itself "a match in the table".
                // If a partially-matched c has ccc!=0 then
                // it might be skipped in discontiguous contraction.
                c = next_cp;
                since_match += 1;
            }
            look_ahead += 1;
            m = suffixes.next_for_code_point(c);
        }
        self.backward_num_skipped(since_match);
        ce32
    }

    #[doc(hidden)]
    /// `if(skipped != null && !skipped.isEmpty()) { skipped.saveTrieState(suffixes); }`
    fn save_skipped_trie_state_if_active(&mut self, suffixes: &CharsTrie) {
        if let Some(skipped) = self.base_mut().skipped.as_mut() {
            if !skipped.is_empty() {
                skipped.save_trie_state(suffixes);
            }
        }
    }

    #[doc(hidden)]
    /// `nextCE32FromDiscontiguousContraction(...)` (private final).
    fn next_ce32_from_discontiguous_contraction(
        &mut self,
        mut d: Arc<CollationData>,
        suffixes: &mut CharsTrie,
        mut ce32: i32,
        mut look_ahead: i32,
        mut c: i32,
    ) -> i32 {
        // UCA section 3.3.2 Contractions:
        // Contractions that end with non-starter characters
        // are known as discontiguous contractions.
        // ... discontiguous contractions must be detected in input text
        // whenever the final sequence of non-starter characters could be rearranged
        // so as to make a contiguous matching sequence that is canonically equivalent.

        // UCA: http://www.unicode.org/reports/tr10/#S2.1
        // S2.1 Find the longest initial substring S at each point that has a match in the table.
        // S2.1.1 If there are any non-starters following S, process each non-starter C.
        // S2.1.2 If C is not blocked from S, find if S + C has a match in the table.
        //     Note: A non-starter in a string is called blocked
        //     if there is another non-starter of the same canonical combining class or zero
        //     between it and the last character of canonical combining class 0.
        // S2.1.3 If there is a match, replace S by S + C, and remove C.

        // First: Is a discontiguous contraction even possible?
        let mut fcd16 = d.get_fcd16(c);
        debug_assert!(fcd16 > 0xff); // The caller checked this already, as a shortcut.
        let next_cp = self.next_skipped_code_point();
        if next_cp < 0 {
            // No further text.
            self.backward_num_skipped(1);
            return ce32;
        }
        look_ahead += 1;
        let mut prev_cc = fcd16 & 0xff;
        fcd16 = d.get_fcd16(next_cp);
        if fcd16 <= 0xff {
            // The next code point after c is a starter (S2.1.1 "process each non-starter").
            self.backward_num_skipped(2);
            return ce32;
        }

        // We have read and matched (lookAhead-2) code points,
        // read non-matching c and peeked ahead at nextCp.
        // Return to the state before the mismatch and continue matching with nextCp.
        if self.base().skipped.as_ref().is_none_or(|s| s.is_empty()) {
            if self.base().skipped.is_none() {
                self.base_mut().skipped = Some(SkippedState::default());
            }
            suffixes.reset();
            if look_ahead > 2 {
                // Replay the partial match so far.
                self.backward_num_code_points(look_ahead);
                let cp = self.next_code_point();
                suffixes.first_for_code_point(cp);
                for _ in 3..look_ahead {
                    let cp = self.next_code_point();
                    suffixes.next_for_code_point(cp);
                }
                // Skip c (which did not match) and nextCp (which we will try now).
                self.forward_num_code_points(2);
            }
            self.skipped_mut().save_trie_state(suffixes);
        } else {
            // Reset to the trie state before the failed match of c.
            self.skipped_mut().reset_to_trie_state(suffixes);
        }

        self.skipped_mut().set_first_skipped(c);
        // Number of code points read since the last match (at this point: c and nextCp).
        let mut since_match = 2;
        c = next_cp;
        loop {
            let m;
            // "If C is not blocked from S, find if S + C has a match in the table." (S2.1.2)
            if prev_cc < (fcd16 >> 8) && {
                m = suffixes.next_for_code_point(c);
                m.has_value()
            } {
                // "If there is a match, replace S by S + C, and remove C." (S2.1.3)
                // Keep prevCC unchanged.
                ce32 = suffixes.get_value();
                since_match = 0;
                self.skipped_mut().record_match();
                if !m.has_next() {
                    break;
                }
                self.skipped_mut().save_trie_state(suffixes);
            } else {
                // No match for "S + C", skip C.
                self.skipped_mut().skip(c);
                self.skipped_mut().reset_to_trie_state(suffixes);
                prev_cc = fcd16 & 0xff;
            }
            c = self.next_skipped_code_point();
            if c < 0 {
                break;
            }
            since_match += 1;
            fcd16 = d.get_fcd16(c);
            if fcd16 <= 0xff {
                // The next code point after c is a starter (S2.1.1 "process each non-starter").
                break;
            }
        }
        self.backward_num_skipped(since_match);
        let is_top_discontiguous = self.skipped_mut().is_empty();
        self.skipped_mut().replace_match();
        if is_top_discontiguous && !self.skipped_mut().is_empty() {
            // We did get a match after skipping one or more combining marks,
            // and we are not in a recursive discontiguous contraction.
            // Append CEs from the contraction ce32
            // and then from the combining marks that we skipped before the match.
            c = Collation::SENTINEL_CP;
            loop {
                self.append_ces_from_ce32(d.clone(), c, ce32, true);
                // Fetch CE32s for skipped combining marks from the normal data, with fallback,
                // rather than from the CollationData where we found the contraction.
                if !self.skipped_mut().has_next() {
                    break;
                }
                c = self.skipped_mut().next();
                ce32 = self.get_data_ce32(c);
                if ce32 == Collation::FALLBACK_CE32 {
                    d = base_of(&self.base().data);
                    ce32 = d.get_ce32(c);
                } else {
                    d = self.base().data.clone();
                }
                // Note: A nested discontiguous-contraction match
                // replaces consumed combining marks with newly skipped ones
                // and resets the reading position to the beginning.
            }
            self.skipped_mut().clear();
            ce32 = Collation::NO_CE32; // Signal to the caller that the result is in the ceBuffer.
        }
        ce32
    }

    #[doc(hidden)]
    /// The skipped-marks state, which exists once discontiguous matching started.
    fn skipped_mut(&mut self) -> &mut SkippedState {
        self.base_mut().skipped.as_mut().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    #[doc(hidden)]
    /// `previousCEUnsafe(int, UVector32)` (private final).
    #[allow(unused_assignments)] // Java reassigns the parameter before reading it
    fn previous_ce_unsafe(&mut self, mut c: i32, offsets: &mut UVector32) -> i64 {
        // We just move through the input counting safe and unsafe code points
        // without collecting the unsafe-backward substring into a buffer and
        // switching to it.
        // This is to keep the logic simple. Otherwise we would have to handle
        // prefix matching going before the backward buffer, switching
        // to iteration and back, etc.
        // In the most important case of iterating over a normal string,
        // reading from the string itself is already maximally fast.
        // The only drawback there is that after getting the CEs we always
        // skip backward to the safe character rather than switching out
        // of a backwardBuffer.
        // But this should not be the common case for previousCE(),
        // and correctness and maintainability are more important than
        // complex optimizations.
        // Find the first safe character before c.
        let mut num_backward = 1;
        loop {
            c = self.previous_code_point();
            if c < 0 {
                break;
            }
            num_backward += 1;
            let is_numeric = self.base().is_numeric;
            if !self.base().data.is_unsafe_backward(c, is_numeric) {
                break;
            }
        }
        // Set the forward iteration limit.
        // Note: This counts code points.
        // We cannot enforce a limit in the middle of a surrogate pair or similar.
        self.base_mut().num_cp_fwd = num_backward;
        // Reset the forward iterator.
        self.base_mut().ces_index = 0;
        debug_assert!(self.base().ceb().length == 0);
        // Go forward and collect the CEs.
        let mut offset = self.get_offset();
        while self.base().num_cp_fwd > 0 {
            // nextCE() normally reads one code point.
            // Contraction matching and digit specials read more and check numCpFwd.
            self.base_mut().num_cp_fwd -= 1;
            // Append one or more CEs to the ceBuffer.
            self.next_ce();
            debug_assert!(self.base().ceb().get(self.base().ceb().length - 1) != Collation::NO_CE);
            // No need to loop for getting each expansion CE from nextCE().
            let b = self.base_mut();
            b.ces_index = b.ceb().length;
            // However, we need to write an offset for each CE.
            // This is for CollationElementIterator.getOffset() to return
            // intermediate offsets from the unsafe-backwards segment.
            debug_assert!(offsets.size() < self.base().ceb().length);
            offsets.add_element(offset);
            // For an expansion, the offset of each non-initial CE is the limit offset,
            // consistent with forward iteration.
            offset = self.get_offset();
            while offsets.size() < self.base().ceb().length {
                offsets.add_element(offset);
            }
        }
        debug_assert!(offsets.size() == self.base().ceb().length);
        // End offset corresponding to just after the unsafe-backwards segment.
        offsets.add_element(offset);
        // Reset the forward iteration limit
        // and move backward to before the segment for which we fetched CEs.
        self.base_mut().num_cp_fwd = -1;
        self.backward_num_code_points(num_backward);
        // Use the collected CEs and return the last one.
        let b = self.base_mut();
        b.ces_index = 0; // Avoid cesIndex > ceBuffer.length when that gets decremented.
        let ceb = b.ceb_mut();
        ceb.length -= 1;
        ceb.get(ceb.length)
    }

    #[doc(hidden)]
    /// `appendNumericCEs(int, boolean)` (private final).
    fn append_numeric_ces(&mut self, mut ce32: i32, forward: bool) {
        // Collect digits.
        // TODO: Use some kind of a byte buffer? We only store values 0..9.
        let mut digits: Vec<u16> = Vec::new();
        if forward {
            loop {
                let digit = Collation::digit_from_ce32(ce32);
                digits.push(digit);
                if self.base().num_cp_fwd == 0 {
                    break;
                }
                let c = self.next_code_point();
                if c < 0 {
                    break;
                }
                ce32 = self.base().data.get_ce32(c);
                if ce32 == Collation::FALLBACK_CE32 {
                    ce32 = self.base().data.base().get_ce32(c);
                }
                if !Collation::has_ce32_tag(ce32, Collation::DIGIT_TAG) {
                    self.backward_num_code_points(1);
                    break;
                }
                if self.base().num_cp_fwd > 0 {
                    self.base_mut().num_cp_fwd -= 1;
                }
            }
        } else {
            loop {
                let digit = Collation::digit_from_ce32(ce32);
                digits.push(digit);
                let c = self.previous_code_point();
                if c < 0 {
                    break;
                }
                ce32 = self.base().data.get_ce32(c);
                if ce32 == Collation::FALLBACK_CE32 {
                    ce32 = self.base().data.base().get_ce32(c);
                }
                if !Collation::has_ce32_tag(ce32, Collation::DIGIT_TAG) {
                    self.forward_num_code_points(1);
                    break;
                }
            }
            // Reverse the digit string (digits are 0..9, never surrogates).
            digits.reverse();
        }
        let mut pos = 0usize;
        loop {
            // Skip leading zeros.
            while pos < (digits.len() - 1) && digits[pos] == 0 {
                pos += 1;
            }
            // Write a sequence of CEs for at most 254 digits at a time.
            let mut segment_length = digits.len() - pos;
            if segment_length > 254 {
                segment_length = 254;
            }
            self.append_numeric_segment_ces(&digits[pos..pos + segment_length]);
            pos += segment_length;
            if pos >= digits.len() {
                break;
            }
        }
    }

    #[doc(hidden)]
    /// `appendNumericSegmentCEs(CharSequence)` (private final).
    fn append_numeric_segment_ces(&mut self, digits: &[u16]) {
        let mut length = digits.len();
        debug_assert!((1..=254).contains(&length));
        debug_assert!(length == 1 || digits[0] != 0);
        let numeric_primary = self.base().data.numeric_primary;
        let ceb = self.base_mut().ceb_mut();
        // Note: We use primary byte values 2..255: digits are not compressible.
        if length <= 7 {
            // Very dense encoding for small numbers.
            let mut value = digits[0] as i32;
            for &d in &digits[1..length] {
                value = value * 10 + d as i32;
            }
            // Primary weight second byte values:
            //     74 byte values   2.. 75 for small numbers in two-byte primary weights.
            //     40 byte values  76..115 for medium numbers in three-byte primary weights.
            //     16 byte values 116..131 for large numbers in four-byte primary weights.
            //    124 byte values 132..255 for very large numbers with 4..127 digit pairs.
            let mut first_byte = 2;
            let mut num_bytes = 74;
            if value < num_bytes {
                // Two-byte primary for 0..73, good for day & month numbers etc.
                let primary = numeric_primary | ((first_byte + value) << 16) as i64;
                ceb.append(Collation::make_ce(primary));
                return;
            }
            value -= num_bytes;
            first_byte += num_bytes;
            num_bytes = 40;
            if value < num_bytes * 254 {
                // Three-byte primary for 74..10233=74+40*254-1, good for year numbers and more.
                let primary = numeric_primary | ((first_byte + value / 254) << 16) as i64 | ((2 + value % 254) << 8) as i64;
                ceb.append(Collation::make_ce(primary));
                return;
            }
            value -= num_bytes * 254;
            first_byte += num_bytes;
            num_bytes = 16;
            if value < num_bytes * 254 * 254 {
                // Four-byte primary for 10234..1042489=10234+16*254*254-1.
                let mut primary = numeric_primary | (2 + value % 254) as i64;
                value /= 254;
                primary |= ((2 + value % 254) << 8) as i64;
                value /= 254;
                primary |= ((first_byte + value % 254) << 16) as i64;
                ceb.append(Collation::make_ce(primary));
                return;
            }
            // original value > 1042489
        }
        debug_assert!(length >= 7);

        // The second primary byte value 132..255 indicates the number of digit pairs (4..127),
        // then we generate primary bytes with those pairs.
        // Omit trailing 00 pairs.
        // Decrement the value for the last pair.

        // Set the exponent. 4 pairs.132, 5 pairs.133, ..., 127 pairs.255.
        let num_pairs = (length as i32 + 1) / 2;
        let mut primary = numeric_primary | ((132 - 4 + num_pairs) << 16) as i64;
        // Find the length without trailing 00 pairs.
        while digits[length - 1] == 0 && digits[length - 2] == 0 {
            length -= 2;
        }
        // Read the first pair.
        let mut pair: i32;
        let mut pos: usize;
        if (length & 1) != 0 {
            // Only "half a pair" if we have an odd number of digits.
            pair = digits[0] as i32;
            pos = 1;
        } else {
            pair = digits[0] as i32 * 10 + digits[1] as i32;
            pos = 2;
        }
        pair = 11 + 2 * pair;
        // Add the pairs of digits between pos and length.
        let mut shift = 8;
        while pos < length {
            if shift == 0 {
                // Every three pairs/bytes we need to store a 4-byte-primary CE
                // and start with a new CE with the '0' primary lead byte.
                primary |= pair as i64;
                ceb.append(Collation::make_ce(primary));
                primary = numeric_primary;
                shift = 16;
            } else {
                primary |= (pair << shift) as i64;
                shift -= 8;
            }
            pair = 11 + 2 * (digits[pos] as i32 * 10 + digits[pos + 1] as i32);
            pos += 2;
        }
        primary |= ((pair - 1) << shift) as i64;
        ceb.append(Collation::make_ce(primary));
    }
}
