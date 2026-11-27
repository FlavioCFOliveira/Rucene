// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.UTF16CollationIterator`.

use std::sync::Arc;

use crate::jdk::{is_high_surrogate, is_low_surrogate, to_code_point};
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_iterator::{
    CollationIterator, CollationIteratorBase, NO_CP_AND_CE32,
};

/// UTF-16 collation element and character iterator. Handles normalized
/// UTF-16 text, with length or NUL-terminated. Unnormalized text is
/// handled by a subclass.
///
/// Equivalent to `com.ibm.icu.impl.coll.UTF16CollationIterator`.
///
/// **Divergence.** Java keeps a reference to the caller's `CharSequence`
/// (`UTF16CollationIterator.java:185-188`); the iterator here copies the
/// UTF-16 units of the text into a buffer it reuses across `setText`
/// calls, because the collator caches its iterators beyond the lifetime of
/// any one text. The text is never modified during iteration, so the
/// iteration is the same.
#[derive(Debug, Clone)]
pub struct UTF16CollationIterator {
    base: CollationIteratorBase,
    /// `seq`.
    pub(crate) seq: Vec<u16>,
    /// `start`.
    pub(crate) start: i32,
    /// `pos`.
    pub(crate) pos: i32,
    /// `limit`.
    pub(crate) limit: i32,
}

impl UTF16CollationIterator {
    /// Partial constructor, see `CollationIterator(CollationData)`.
    pub fn new(d: Arc<CollationData>) -> Self {
        Self {
            base: CollationIteratorBase::new(d),
            seq: Vec::new(),
            start: 0,
            pos: 0,
            limit: 0,
        }
    }

    /// `UTF16CollationIterator(CollationData, boolean, CharSequence, int)`.
    pub fn with_text(d: Arc<CollationData>, numeric: bool, s: &[u16], p: i32) -> Self {
        Self {
            base: CollationIteratorBase::with_numeric(d, numeric),
            seq: s.to_vec(),
            start: 0,
            pos: p,
            limit: s.len() as i32,
        }
    }

    /// `equals(Object)`: same class, same base state and same offset.
    pub fn equals(&self, o: &UTF16CollationIterator) -> bool {
        if !self.base.equals(&o.base) {
            return false;
        }
        // Compare the iterator state but not the text: Assume that the caller does that.
        (self.pos - self.start) == (o.pos - o.start)
    }

    /// `hashCode()`: not designed (`assert false`, then 42).
    pub fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42 // any arbitrary constant will do
    }

    /// `setText(boolean, CharSequence, int)`.
    pub fn set_text(&mut self, numeric: bool, s: &[u16], p: i32) {
        self.base.reset_numeric(numeric);
        self.seq.clear();
        self.seq.extend_from_slice(s);
        self.start = 0;
        self.pos = p;
        self.limit = s.len() as i32;
    }

    fn char_at(&self, i: i32) -> u16 {
        self.seq[i as usize]
    }
}

impl CollationIterator for UTF16CollationIterator {
    fn base(&self) -> &CollationIteratorBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut CollationIteratorBase {
        &mut self.base
    }

    fn reset_to_offset(&mut self, new_offset: i32) {
        self.base.reset();
        self.pos = self.start + new_offset;
    }

    fn get_offset(&self) -> i32 {
        self.pos - self.start
    }

    fn next_code_point(&mut self) -> i32 {
        if self.pos == self.limit {
            return Collation::SENTINEL_CP;
        }
        let c = self.char_at(self.pos);
        self.pos += 1;
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
        if self.pos == self.start {
            return Collation::SENTINEL_CP;
        }
        self.pos -= 1;
        let c = self.char_at(self.pos);
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
        if self.pos == self.limit {
            return NO_CP_AND_CE32;
        }
        let c = self.char_at(self.pos);
        self.pos += 1;
        let trie = self
            .base
            .trie
            .as_deref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        CollationIteratorBase::make_code_point_and_ce32_pair(
            c as i32,
            trie.get_from_u16_single_lead(c),
        )
    }

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
        while num > 0 && self.pos != self.limit {
            let c = self.char_at(self.pos);
            self.pos += 1;
            num -= 1;
            if is_high_surrogate(c)
                && self.pos != self.limit
                && is_low_surrogate(self.char_at(self.pos))
            {
                self.pos += 1;
            }
        }
    }

    fn backward_num_code_points(&mut self, mut num: i32) {
        while num > 0 && self.pos != self.start {
            self.pos -= 1;
            let c = self.char_at(self.pos);
            num -= 1;
            if is_low_surrogate(c)
                && self.pos != self.start
                && is_high_surrogate(self.char_at(self.pos - 1))
            {
                self.pos -= 1;
            }
        }
    }
}
