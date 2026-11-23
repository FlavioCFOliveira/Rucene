// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.FCDUTF16CollationIterator`.

use std::sync::Arc;

use crate::jdk::{char_count, code_point_at, code_point_before, is_high_surrogate, is_low_surrogate, to_code_point};
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fcd::CollationFCD;
use crate::r#impl::coll::collation_iterator::{CollationIterator, CollationIteratorBase, NO_CP_AND_CE32};
use crate::r#impl::normalizer2_impl::Normalizer2Impl;

/// `rawStart`: the input text is rawSeq\[rawStart, rawLimit\[.
const RAW_START: i32 = 0;

/// Incrementally checks the input text for FCD and normalizes where
/// necessary.
///
/// Equivalent to `com.ibm.icu.impl.coll.FCDUTF16CollationIterator` (a
/// subclass of `UTF16CollationIterator`, whose fields it carries).
/// Java's `seq` is either `rawSeq` or `normalized`; here `seq_is_raw`
/// tells which buffer `seq` stands for, and Java's identity test
/// `seq == rawSeq` reads it.
///
/// **Divergence.** As for [`UTF16CollationIterator`]: the raw text is
/// copied into a reused buffer instead of referenced
/// (`FCDUTF16CollationIterator.java:517-522`).
///
/// [`UTF16CollationIterator`]: crate::r#impl::coll::utf16_collation_iterator::UTF16CollationIterator
#[derive(Debug, Clone)]
pub struct FCDUTF16CollationIterator {
    base: CollationIteratorBase,
    seq_is_raw: bool,
    start: i32,
    pos: i32,
    limit: i32,
    raw_seq: Vec<u16>,
    segment_start: i32,
    segment_limit: i32,
    raw_limit: i32,
    nfc_impl: Arc<Normalizer2Impl>,
    normalized: Option<Vec<u16>>,
    /// Direction of incremental FCD check.
    check_dir: i32,
}

impl FCDUTF16CollationIterator {
    /// Partial constructor, see `CollationIterator(CollationData)`.
    pub fn new(d: Arc<CollationData>) -> Self {
        let nfc_impl = d.nfc_impl.clone();
        Self {
            base: CollationIteratorBase::new(d),
            seq_is_raw: true,
            start: 0,
            pos: 0,
            limit: 0,
            raw_seq: Vec::new(),
            segment_start: 0,
            segment_limit: 0,
            raw_limit: 0,
            nfc_impl,
            normalized: None,
            check_dir: 0,
        }
    }

    /// `FCDUTF16CollationIterator(CollationData, boolean, CharSequence, int)`.
    pub fn with_text(data: Arc<CollationData>, numeric: bool, s: &[u16], p: i32) -> Self {
        let nfc_impl = data.nfc_impl.clone();
        Self {
            base: CollationIteratorBase::with_numeric(data, numeric),
            seq_is_raw: true,
            start: 0,
            pos: p,
            limit: s.len() as i32,
            raw_seq: s.to_vec(),
            segment_start: p,
            segment_limit: 0,
            raw_limit: s.len() as i32,
            nfc_impl,
            normalized: None,
            check_dir: 1,
        }
    }

    /// `equals(Object)`.
    pub fn equals(&self, o: &FCDUTF16CollationIterator) -> bool {
        // Skip the UTF16CollationIterator and call its parent.
        if !self.base.equals(&o.base) {
            return false;
        }
        // Compare the iterator state but not the text: Assume that the caller does that.
        if self.check_dir != o.check_dir {
            return false;
        }
        if self.check_dir == 0 && self.seq_is_raw != o.seq_is_raw {
            return false;
        }
        if self.check_dir != 0 || self.seq_is_raw {
            (self.pos - RAW_START) == (o.pos - RAW_START)
        } else {
            (self.segment_start - RAW_START) == (o.segment_start - RAW_START) && (self.pos - self.start) == (o.pos - o.start)
        }
    }

    /// `hashCode()`: not designed (`assert false`, then 42).
    pub fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42 // any arbitrary constant will do
    }

    /// `setText(boolean, CharSequence, int)`.
    pub fn set_text(&mut self, numeric: bool, s: &[u16], p: i32) {
        // super.setText(numeric, s, p)
        self.base.reset_numeric(numeric);
        self.raw_seq.clear();
        self.raw_seq.extend_from_slice(s);
        self.seq_is_raw = true;
        self.start = 0;
        self.pos = p;
        self.limit = s.len() as i32;
        self.segment_start = p;
        self.raw_limit = s.len() as i32;
        self.limit = self.raw_limit;
        self.check_dir = 1;
    }

    fn char_at(&self, i: i32) -> u16 {
        if self.seq_is_raw {
            self.raw_seq[i as usize]
        } else {
            self.normalized.as_ref().expect("normalized segment")[i as usize]
        }
    }

    fn switch_to_forward(&mut self) {
        debug_assert!((self.check_dir < 0 && self.seq_is_raw) || (self.check_dir == 0 && self.pos == self.limit));
        if self.check_dir < 0 {
            // Turn around from backward checking.
            self.start = self.pos;
            self.segment_start = self.pos;
            if self.pos == self.segment_limit {
                self.limit = self.raw_limit;
                self.check_dir = 1; // Check forward.
            } else {
                // pos < segmentLimit
                self.check_dir = 0; // Stay in FCD segment.
            }
        } else {
            // Reached the end of the FCD segment.
            if self.seq_is_raw {
                // The input text segment is FCD, extend it forward.
            } else {
                // The input text segment needed to be normalized.
                // Switch to checking forward from it.
                self.seq_is_raw = true;
                self.pos = self.segment_limit;
                self.start = self.segment_limit;
                self.segment_start = self.segment_limit;
                // Note: If this segment is at the end of the input text,
                // then it might help to return false to indicate that, so that
                // we do not have to re-check and normalize when we turn around and go backwards.
                // However, that would complicate the call sites for an optimization of an unusual case.
            }
            self.limit = self.raw_limit;
            self.check_dir = 1;
        }
    }

    fn next_segment(&mut self) {
        debug_assert!(self.check_dir > 0 && self.seq_is_raw && self.pos != self.limit);
        // The input text [segmentStart..pos[ passes the FCD check.
        let mut p = self.pos;
        let mut prev_cc = 0;
        loop {
            // Fetch the next character's fcd16 value.
            let mut q = p;
            let mut c = code_point_at(&self.raw_seq, p);
            p += char_count(c);
            let fcd16 = self.nfc_impl.get_fcd16(c);
            let lead_cc = fcd16 >> 8;
            if lead_cc == 0 && q != self.pos {
                // FCD boundary before the [q, p[ character.
                self.limit = q;
                self.segment_limit = q;
                break;
            }
            if lead_cc != 0 && (prev_cc > lead_cc || CollationFCD::is_fcd16_of_tibetan_composite_vowel(fcd16)) {
                // Fails FCD check. Find the next FCD boundary and normalize.
                loop {
                    q = p;
                    if p == self.raw_limit {
                        break;
                    }
                    c = code_point_at(&self.raw_seq, p);
                    p += char_count(c);
                    if self.nfc_impl.get_fcd16(c) <= 0xff {
                        break;
                    }
                }
                self.normalize(self.pos, q);
                self.pos = self.start;
                break;
            }
            prev_cc = fcd16 & 0xff;
            if p == self.raw_limit || prev_cc == 0 {
                // FCD boundary after the last character.
                self.limit = p;
                self.segment_limit = p;
                break;
            }
        }
        debug_assert!(self.pos != self.limit);
        self.check_dir = 0;
    }

    fn switch_to_backward(&mut self) {
        debug_assert!((self.check_dir > 0 && self.seq_is_raw) || (self.check_dir == 0 && self.pos == self.start));
        if self.check_dir > 0 {
            // Turn around from forward checking.
            self.limit = self.pos;
            self.segment_limit = self.pos;
            if self.pos == self.segment_start {
                self.start = RAW_START;
                self.check_dir = -1; // Check backward.
            } else {
                // pos > segmentStart
                self.check_dir = 0; // Stay in FCD segment.
            }
        } else {
            // Reached the start of the FCD segment.
            if self.seq_is_raw {
                // The input text segment is FCD, extend it backward.
            } else {
                // The input text segment needed to be normalized.
                // Switch to checking backward from it.
                self.seq_is_raw = true;
                self.pos = self.segment_start;
                self.limit = self.segment_start;
                self.segment_limit = self.segment_start;
            }
            self.start = RAW_START;
            self.check_dir = -1;
        }
    }

    fn previous_segment(&mut self) {
        debug_assert!(self.check_dir < 0 && self.seq_is_raw && self.pos != self.start);
        // The input text [pos..segmentLimit[ passes the FCD check.
        let mut p = self.pos;
        let mut next_cc = 0;
        loop {
            // Fetch the previous character's fcd16 value.
            let mut q = p;
            let mut c = code_point_before(&self.raw_seq, p);
            p -= char_count(c);
            let mut fcd16 = self.nfc_impl.get_fcd16(c);
            let trail_cc = fcd16 & 0xff;
            if trail_cc == 0 && q != self.pos {
                // FCD boundary after the [p, q[ character.
                self.start = q;
                self.segment_start = q;
                break;
            }
            if trail_cc != 0
                && ((next_cc != 0 && trail_cc > next_cc) || CollationFCD::is_fcd16_of_tibetan_composite_vowel(fcd16))
            {
                // Fails FCD check. Find the previous FCD boundary and normalize.
                loop {
                    q = p;
                    if fcd16 <= 0xff || p == RAW_START {
                        break;
                    }
                    c = code_point_before(&self.raw_seq, p);
                    p -= char_count(c);
                    fcd16 = self.nfc_impl.get_fcd16(c);
                    if fcd16 == 0 {
                        break;
                    }
                }
                self.normalize(q, self.pos);
                self.pos = self.limit;
                break;
            }
            next_cc = fcd16 >> 8;
            if p == RAW_START || next_cc == 0 {
                // FCD boundary before the following character.
                self.start = p;
                self.segment_start = p;
                break;
            }
        }
        debug_assert!(self.pos != self.start);
        self.check_dir = 0;
    }

    fn normalize(&mut self, from: i32, to: i32) {
        let normalized = self.normalized.get_or_insert_with(Vec::new);
        // NFD without argument checking.
        self.nfc_impl.decompose_range_into(&self.raw_seq, from, to, normalized, to - from);
        // Switch collation processing into the FCD buffer
        // with the result of normalizing [segmentStart, segmentLimit[.
        self.segment_start = from;
        self.segment_limit = to;
        self.seq_is_raw = false;
        self.start = 0;
        self.limit = self.start + normalized.len() as i32;
    }
}

impl CollationIterator for FCDUTF16CollationIterator {
    fn base(&self) -> &CollationIteratorBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut CollationIteratorBase {
        &mut self.base
    }

    fn reset_to_offset(&mut self, new_offset: i32) {
        self.base.reset();
        self.seq_is_raw = true;
        let p = RAW_START + new_offset;
        self.start = p;
        self.segment_start = p;
        self.pos = p;
        self.limit = self.raw_limit;
        self.check_dir = 1;
    }

    fn get_offset(&self) -> i32 {
        if self.check_dir != 0 || self.seq_is_raw {
            self.pos - RAW_START
        } else if self.pos == self.start {
            self.segment_start - RAW_START
        } else {
            self.segment_limit - RAW_START
        }
    }

    fn next_code_point(&mut self) -> i32 {
        let mut c;
        loop {
            if self.check_dir > 0 {
                if self.pos == self.limit {
                    return Collation::SENTINEL_CP;
                }
                c = self.char_at(self.pos);
                self.pos += 1;
                if CollationFCD::has_tccc(c as i32)
                    && (CollationFCD::maybe_tibetan_composite_vowel(c as i32)
                        || (self.pos != self.limit && CollationFCD::has_lccc(self.char_at(self.pos) as i32)))
                {
                    self.pos -= 1;
                    self.next_segment();
                    c = self.char_at(self.pos);
                    self.pos += 1;
                }
                break;
            } else if self.check_dir == 0 && self.pos != self.limit {
                c = self.char_at(self.pos);
                self.pos += 1;
                break;
            } else {
                self.switch_to_forward();
            }
        }
        if is_high_surrogate(c) && self.pos != self.limit {
            let trail = self.char_at(self.pos);
            if is_low_surrogate(trail) {
                self.pos += 1;
                return to_code_point(c, trail);
            }
        }
        c as i32
    }

    fn previous_code_point(&mut self) -> i32 {
        let mut c;
        loop {
            if self.check_dir < 0 {
                if self.pos == self.start {
                    return Collation::SENTINEL_CP;
                }
                self.pos -= 1;
                c = self.char_at(self.pos);
                if CollationFCD::has_lccc(c as i32)
                    && (CollationFCD::maybe_tibetan_composite_vowel(c as i32)
                        || (self.pos != self.start && CollationFCD::has_tccc(self.char_at(self.pos - 1) as i32)))
                {
                    self.pos += 1;
                    self.previous_segment();
                    self.pos -= 1;
                    c = self.char_at(self.pos);
                }
                break;
            } else if self.check_dir == 0 && self.pos != self.start {
                self.pos -= 1;
                c = self.char_at(self.pos);
                break;
            } else {
                self.switch_to_backward();
            }
        }
        if is_low_surrogate(c) && self.pos != self.start {
            let lead = self.char_at(self.pos - 1);
            if is_high_surrogate(lead) {
                self.pos -= 1;
                return to_code_point(lead, c);
            }
        }
        c as i32
    }

    fn handle_next_ce32(&mut self) -> i64 {
        let mut c;
        loop {
            if self.check_dir > 0 {
                if self.pos == self.limit {
                    return NO_CP_AND_CE32;
                }
                c = self.char_at(self.pos);
                self.pos += 1;
                if CollationFCD::has_tccc(c as i32)
                    && (CollationFCD::maybe_tibetan_composite_vowel(c as i32)
                        || (self.pos != self.limit && CollationFCD::has_lccc(self.char_at(self.pos) as i32)))
                {
                    self.pos -= 1;
                    self.next_segment();
                    c = self.char_at(self.pos);
                    self.pos += 1;
                }
                break;
            } else if self.check_dir == 0 && self.pos != self.limit {
                c = self.char_at(self.pos);
                self.pos += 1;
                break;
            } else {
                self.switch_to_forward();
            }
        }
        let trie = self.base.trie.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        CollationIteratorBase::make_code_point_and_ce32_pair(c as i32, trie.get_from_u16_single_lead(c))
    }

    /// Inherited from `UTF16CollationIterator.handleGetTrailSurrogate()`.
    fn handle_get_trail_surrogate(&mut self) -> u16 {
        if self.pos == self.limit {
            return 0;
        }
        let trail = self.char_at(self.pos);
        if is_low_surrogate(trail) {
            self.pos += 1;
        }
        trail
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
