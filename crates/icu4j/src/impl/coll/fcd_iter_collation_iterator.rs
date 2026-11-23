// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.FCDIterCollationIterator`.

use std::sync::Arc;

use crate::jdk::{append_code_point, char_count, code_point_at, code_point_before, is_high_surrogate, is_low_surrogate, to_code_point};
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fcd::CollationFCD;
use crate::r#impl::coll::collation_iterator::{
    is_lead_surrogate, is_trail_surrogate, CollationIterator, CollationIteratorBase, NO_CP_AND_CE32,
};
use crate::r#impl::normalizer2_impl::Normalizer2Impl;
use crate::text::u_character_iterator::UCharacterIterator;

/// `FCDIterCollationIterator.State` (private).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    /// The input text \[start..(iter index)\[ passes the FCD check.
    IterCheckFwd,
    /// The input text \[(iter index)..limit\[ passes the FCD check.
    IterCheckBwd,
    /// The input text \[start..limit\[ passes the FCD check.
    IterInFcdSegment,
    /// The input text \[start..limit\[ has been normalized, the iterator is at limit.
    InNormIterAtLimit,
    /// The input text \[start..limit\[ has been normalized, the iterator is at start.
    InNormIterAtStart,
}

/// Java's `StringBuilder.reverse()`: reverses the code units, keeping each
/// surrogate pair of the original in order.
fn string_builder_reverse(s: &mut [u16]) {
    s.reverse();
    let mut i = 0;
    while i + 1 < s.len() {
        if is_low_surrogate(s[i]) && is_high_surrogate(s[i + 1]) {
            s.swap(i, i + 1);
            i += 1;
        }
        i += 1;
    }
}

fn append_cp(sb: &mut Vec<u16>, c: i32) {
    append_code_point(sb, c).unwrap_or_else(|e| panic!("{e}"));
}

/// Incrementally checks the input text for FCD and normalizes where
/// necessary.
///
/// Equivalent to `com.ibm.icu.impl.coll.FCDIterCollationIterator` (a
/// subclass of `IterCollationIterator`, whose `iter` it carries).
#[derive(Debug)]
pub struct FCDIterCollationIterator {
    base: CollationIteratorBase,
    iter: Box<dyn UCharacterIterator>,
    state: State,
    start: i32,
    pos: i32,
    limit: i32,
    nfc_impl: Arc<Normalizer2Impl>,
    s: Option<Vec<u16>>,
    normalized: Option<Vec<u16>>,
}

impl FCDIterCollationIterator {
    /// `FCDIterCollationIterator(CollationData, boolean, UCharacterIterator, int)`.
    pub fn new(data: Arc<CollationData>, numeric: bool, ui: Box<dyn UCharacterIterator>, start_index: i32) -> Self {
        let nfc_impl = data.nfc_impl.clone();
        Self {
            base: CollationIteratorBase::with_numeric(data, numeric),
            iter: ui,
            state: State::IterCheckFwd,
            start: start_index,
            pos: 0,
            limit: 0,
            nfc_impl,
            s: None,
            normalized: None,
        }
    }

    fn normalized(&self) -> &Vec<u16> {
        self.normalized.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn switch_to_forward(&mut self) {
        debug_assert!(
            self.state == State::IterCheckBwd
                || (self.state == State::IterInFcdSegment && self.pos == self.limit)
                || (self.state >= State::InNormIterAtLimit && self.pos as usize == self.normalized().len())
        );
        if self.state == State::IterCheckBwd {
            // Turn around from backward checking.
            self.pos = self.iter.get_index();
            self.start = self.pos;
            if self.pos == self.limit {
                self.state = State::IterCheckFwd; // Check forward.
            } else {
                // pos < limit
                self.state = State::IterInFcdSegment; // Stay in FCD segment.
            }
        } else {
            // Reached the end of the FCD segment.
            if self.state == State::IterInFcdSegment {
                // The input text segment is FCD, extend it forward.
            } else {
                // The input text segment needed to be normalized.
                // Switch to checking forward from it.
                if self.state == State::InNormIterAtStart {
                    self.iter.move_index(self.limit - self.start);
                }
                self.start = self.limit;
            }
            self.state = State::IterCheckFwd;
        }
    }

    fn next_segment(&mut self) -> bool {
        debug_assert!(self.state == State::IterCheckFwd);
        // The input text [start..(iter index)[ passes the FCD check.
        self.pos = self.iter.get_index();
        // Collect the characters being checked, in case they need to be normalized.
        let mut s = self.s.take().unwrap_or_default();
        s.clear();
        let mut prev_cc = 0;
        loop {
            // Fetch the next character and its fcd16 value.
            let mut c = self.iter.next_code_point();
            if c < 0 {
                break;
            }
            let fcd16 = self.nfc_impl.get_fcd16(c);
            let lead_cc = fcd16 >> 8;
            if lead_cc == 0 && !s.is_empty() {
                // FCD boundary before this character.
                self.iter.previous_code_point();
                break;
            }
            append_cp(&mut s, c);
            if lead_cc != 0 && (prev_cc > lead_cc || CollationFCD::is_fcd16_of_tibetan_composite_vowel(fcd16)) {
                // Fails FCD check. Find the next FCD boundary and normalize.
                loop {
                    c = self.iter.next_code_point();
                    if c < 0 {
                        break;
                    }
                    if self.nfc_impl.get_fcd16(c) <= 0xff {
                        self.iter.previous_code_point();
                        break;
                    }
                    append_cp(&mut s, c);
                }
                self.normalize(&s);
                self.start = self.pos;
                self.limit = self.pos + s.len() as i32;
                self.state = State::InNormIterAtLimit;
                self.pos = 0;
                self.s = Some(s);
                return true;
            }
            prev_cc = fcd16 & 0xff;
            if prev_cc == 0 {
                // FCD boundary after the last character.
                break;
            }
        }
        self.limit = self.pos + s.len() as i32;
        debug_assert!(self.pos != self.limit);
        self.iter.move_index(-(s.len() as i32));
        self.state = State::IterInFcdSegment;
        self.s = Some(s);
        true
    }

    fn switch_to_backward(&mut self) {
        debug_assert!(
            self.state == State::IterCheckFwd
                || (self.state == State::IterInFcdSegment && self.pos == self.start)
                || (self.state >= State::InNormIterAtLimit && self.pos == 0)
        );
        if self.state == State::IterCheckFwd {
            // Turn around from forward checking.
            self.pos = self.iter.get_index();
            self.limit = self.pos;
            if self.pos == self.start {
                self.state = State::IterCheckBwd; // Check backward.
            } else {
                // pos > start
                self.state = State::IterInFcdSegment; // Stay in FCD segment.
            }
        } else {
            // Reached the start of the FCD segment.
            if self.state == State::IterInFcdSegment {
                // The input text segment is FCD, extend it backward.
            } else {
                // The input text segment needed to be normalized.
                // Switch to checking backward from it.
                if self.state == State::InNormIterAtLimit {
                    self.iter.move_index(self.start - self.limit);
                }
                self.limit = self.start;
            }
            self.state = State::IterCheckBwd;
        }
    }

    fn previous_segment(&mut self) -> bool {
        debug_assert!(self.state == State::IterCheckBwd);
        // The input text [(iter index)..limit[ passes the FCD check.
        self.pos = self.iter.get_index();
        // Collect the characters being checked, in case they need to be normalized.
        let mut s = self.s.take().unwrap_or_default();
        s.clear();
        let mut next_cc = 0;
        loop {
            // Fetch the previous character and its fcd16 value.
            let mut c = self.iter.previous_code_point();
            if c < 0 {
                break;
            }
            let mut fcd16 = self.nfc_impl.get_fcd16(c);
            let trail_cc = fcd16 & 0xff;
            if trail_cc == 0 && !s.is_empty() {
                // FCD boundary after this character.
                self.iter.next_code_point();
                break;
            }
            append_cp(&mut s, c);
            if trail_cc != 0
                && ((next_cc != 0 && trail_cc > next_cc) || CollationFCD::is_fcd16_of_tibetan_composite_vowel(fcd16))
            {
                // Fails FCD check. Find the previous FCD boundary and normalize.
                while fcd16 > 0xff {
                    c = self.iter.previous_code_point();
                    if c < 0 {
                        break;
                    }
                    fcd16 = self.nfc_impl.get_fcd16(c);
                    if fcd16 == 0 {
                        self.iter.next_code_point();
                        break;
                    }
                    append_cp(&mut s, c);
                }
                string_builder_reverse(&mut s);
                self.normalize(&s);
                self.limit = self.pos;
                self.start = self.pos - s.len() as i32;
                self.state = State::InNormIterAtStart;
                self.pos = self.normalized().len() as i32;
                self.s = Some(s);
                return true;
            }
            next_cc = fcd16 >> 8;
            if next_cc == 0 {
                // FCD boundary before the following character.
                break;
            }
        }
        self.start = self.pos - s.len() as i32;
        debug_assert!(self.pos != self.start);
        self.iter.move_index(s.len() as i32);
        self.state = State::IterInFcdSegment;
        self.s = Some(s);
        true
    }

    fn normalize(&mut self, s: &[u16]) {
        let normalized = self.normalized.get_or_insert_with(Vec::new);
        // NFD without argument checking.
        self.nfc_impl.decompose_into(s, normalized);
    }
}

impl CollationIterator for FCDIterCollationIterator {
    fn base(&self) -> &CollationIteratorBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut CollationIteratorBase {
        &mut self.base
    }

    fn reset_to_offset(&mut self, new_offset: i32) {
        // super.resetToOffset(newOffset)
        self.base.reset();
        self.iter.set_index(new_offset);
        self.start = new_offset;
        self.state = State::IterCheckFwd;
    }

    fn get_offset(&self) -> i32 {
        if self.state <= State::IterCheckBwd {
            self.iter.get_index()
        } else if self.state == State::IterInFcdSegment {
            self.pos
        } else if self.pos == 0 {
            self.start
        } else {
            self.limit
        }
    }

    fn next_code_point(&mut self) -> i32 {
        let mut c;
        loop {
            if self.state == State::IterCheckFwd {
                c = self.iter.next();
                if c < 0 {
                    return c;
                }
                if CollationFCD::has_tccc(c)
                    && (CollationFCD::maybe_tibetan_composite_vowel(c) || CollationFCD::has_lccc(self.iter.current()))
                {
                    self.iter.previous();
                    if !self.next_segment() {
                        return Collation::SENTINEL_CP;
                    }
                    continue;
                }
                if is_lead_surrogate(c) {
                    let trail = self.iter.next();
                    if is_trail_surrogate(trail) {
                        return to_code_point(c as u16, trail as u16);
                    } else if trail >= 0 {
                        self.iter.previous();
                    }
                }
                return c;
            } else if self.state == State::IterInFcdSegment && self.pos != self.limit {
                c = self.iter.next_code_point();
                self.pos += char_count(c);
                debug_assert!(c >= 0);
                return c;
            } else if self.state >= State::InNormIterAtLimit && self.pos as usize != self.normalized().len() {
                c = code_point_at(self.normalized(), self.pos);
                self.pos += char_count(c);
                return c;
            } else {
                self.switch_to_forward();
            }
        }
    }

    fn previous_code_point(&mut self) -> i32 {
        let mut c;
        loop {
            if self.state == State::IterCheckBwd {
                c = self.iter.previous();
                if c < 0 {
                    self.start = 0;
                    self.pos = 0;
                    self.state = State::IterInFcdSegment;
                    return Collation::SENTINEL_CP;
                }
                if CollationFCD::has_lccc(c) {
                    let mut prev = Collation::SENTINEL_CP;
                    if CollationFCD::maybe_tibetan_composite_vowel(c) || {
                        prev = self.iter.previous();
                        CollationFCD::has_tccc(prev)
                    } {
                        self.iter.next();
                        if prev >= 0 {
                            self.iter.next();
                        }
                        if !self.previous_segment() {
                            return Collation::SENTINEL_CP;
                        }
                        continue;
                    }
                    // hasLccc(trail)=true for all trail surrogates
                    if is_trail_surrogate(c) {
                        if prev < 0 {
                            prev = self.iter.previous();
                        }
                        if is_lead_surrogate(prev) {
                            return to_code_point(prev as u16, c as u16);
                        }
                    }
                    if prev >= 0 {
                        self.iter.next();
                    }
                }
                return c;
            } else if self.state == State::IterInFcdSegment && self.pos != self.start {
                c = self.iter.previous_code_point();
                self.pos -= char_count(c);
                debug_assert!(c >= 0);
                return c;
            } else if self.state >= State::InNormIterAtLimit && self.pos != 0 {
                c = code_point_before(self.normalized(), self.pos);
                self.pos -= char_count(c);
                return c;
            } else {
                self.switch_to_backward();
            }
        }
    }

    fn handle_next_ce32(&mut self) -> i64 {
        let c;
        loop {
            if self.state == State::IterCheckFwd {
                let ch = self.iter.next();
                if ch < 0 {
                    return NO_CP_AND_CE32;
                }
                if CollationFCD::has_tccc(ch)
                    && (CollationFCD::maybe_tibetan_composite_vowel(ch) || CollationFCD::has_lccc(self.iter.current()))
                {
                    self.iter.previous();
                    if !self.next_segment() {
                        return Collation::FALLBACK_CE32 as i64;
                    }
                    continue;
                }
                c = ch;
                break;
            } else if self.state == State::IterInFcdSegment && self.pos != self.limit {
                c = self.iter.next();
                self.pos += 1;
                debug_assert!(c >= 0);
                break;
            } else if self.state >= State::InNormIterAtLimit && self.pos as usize != self.normalized().len() {
                c = self.normalized()[self.pos as usize] as i32;
                self.pos += 1;
                break;
            } else {
                self.switch_to_forward();
            }
        }
        let trie = self.base.trie.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        CollationIteratorBase::make_code_point_and_ce32_pair(c, trie.get_from_u16_single_lead(c as u16))
    }

    fn handle_get_trail_surrogate(&mut self) -> u16 {
        if self.state <= State::IterInFcdSegment {
            let trail = self.iter.next();
            if is_trail_surrogate(trail) {
                if self.state == State::IterInFcdSegment {
                    self.pos += 1;
                }
            } else if trail >= 0 {
                self.iter.previous();
            }
            trail as u16
        } else {
            debug_assert!((self.pos as usize) < self.normalized().len());
            let trail = self.normalized()[self.pos as usize];
            if is_low_surrogate(trail) {
                self.pos += 1;
            }
            trail
        }
    }

    fn forward_num_code_points(&mut self, mut num: i32) {
        // Specify the class to avoid a virtual-function indirection.
        // In Java, we would declare this class final.
        while num > 0 && self.next_code_point() >= 0 {
            num -= 1;
        }
    }

    fn backward_num_code_points(&mut self, mut num: i32) {
        // Specify the class to avoid a virtual-function indirection.
        // In Java, we would declare this class final.
        while num > 0 && self.previous_code_point() >= 0 {
            num -= 1;
        }
    }
}
