// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CollationElementIterator`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::jdk::{code_point_at, is_high_surrogate};
use crate::r#impl::character_iterator_wrapper::CharacterIteratorWrapper;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::contractions_and_expansions::{CESink, ContractionsAndExpansions};
use crate::r#impl::coll::fcd_iter_collation_iterator::FCDIterCollationIterator;
use crate::r#impl::coll::fcd_utf16_collation_iterator::FCDUTF16CollationIterator;
use crate::r#impl::coll::iter_collation_iterator::IterCollationIterator;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::r#impl::coll::utf16_collation_iterator::UTF16CollationIterator;
use crate::text::rule_based_collator::RuleBasedCollator;
use crate::text::u_character_iterator::UCharacterIterator;

/// The owned `iter_` of a `CollationElementIterator`: one of the four
/// iterator classes it creates (Java's `CollationIterator` field), so that
/// `equals` can compare same-class iterators.
#[derive(Debug)]
enum OwnedIter {
    Utf16(UTF16CollationIterator),
    FcdUtf16(FCDUTF16CollationIterator),
    Iter(IterCollationIterator),
    FcdIter(FCDIterCollationIterator),
}

impl OwnedIter {
    fn get(&mut self) -> &mut dyn CollationIterator {
        match self {
            OwnedIter::Utf16(i) => i,
            OwnedIter::FcdUtf16(i) => i,
            OwnedIter::Iter(i) => i,
            OwnedIter::FcdIter(i) => i,
        }
    }

    fn get_ref(&self) -> &dyn CollationIterator {
        match self {
            OwnedIter::Utf16(i) => i,
            OwnedIter::FcdUtf16(i) => i,
            OwnedIter::Iter(i) => i,
            OwnedIter::FcdIter(i) => i,
        }
    }

    /// `iter_.equals(thatceiter.iter_)`: the same class and the class's
    /// `equals`.
    fn equals(&self, other: &OwnedIter) -> bool {
        match (self, other) {
            (OwnedIter::Utf16(a), OwnedIter::Utf16(b)) => a.equals(b),
            (OwnedIter::FcdUtf16(a), OwnedIter::FcdUtf16(b)) => a.equals(b),
            (OwnedIter::Iter(a), OwnedIter::Iter(b)) => a.base().equals(b.base()),
            (OwnedIter::FcdIter(a), OwnedIter::FcdIter(b)) => a.base().equals(b.base()),
            _ => false,
        }
    }
}

/// This unchanging value is returned when the end of the string is
/// reached.
pub const NULLORDER: i32 = 0xffffffffu32 as i32;

/// Ignorable collation element order.
pub const IGNORABLE: i32 = 0;

fn get_first_half(p: i64, lower32: i32) -> i32 {
    (p as i32 & 0xffff0000u32 as i32) | ((lower32 >> 16) & 0xff00) | ((lower32 >> 8) & 0xff)
}

fn get_second_half(p: i64, lower32: i32) -> i32 {
    ((p as i32) << 16) | ((lower32 >> 8) & 0xff00) | (lower32 & 0x3f)
}

fn ce_needs_two_parts(ce: i64) -> bool {
    (ce & 0xffff00ff003f) != 0
}

/// `CollationElementIterator.MaxExpSink` (private).
struct MaxExpSink<'m> {
    max_expansions: &'m mut HashMap<i32, i32>,
}

impl CESink for MaxExpSink<'_> {
    fn handle_ce(&mut self, _ce: i64) {}

    fn handle_expansion(&mut self, ces: &[i64], start: i32, length: i32) {
        if length <= 1 {
            // We do not need to add single CEs into the map.
            return;
        }
        let mut count = 0; // number of CE "halves"
        for i in 0..length {
            count += if ce_needs_two_parts(ces[(start + i) as usize]) {
                2
            } else {
                1
            };
        }
        // last "half" of the last CE
        let ce = ces[(start + length - 1) as usize];
        let p = ((ce as u64) >> 32) as i64;
        let lower32 = ce as i32;
        let mut last_half = get_second_half(p, lower32);
        if last_half == 0 {
            last_half = get_first_half(p, lower32);
            debug_assert!(last_half != 0);
        } else {
            last_half |= 0xc0; // old-style continuation CE
        }
        let old_count = self.max_expansions.get(&last_half).copied();
        if old_count.is_none_or(|o| count > o) {
            self.max_expansions.insert(last_half, count);
        }
    }
}

/// The CollationElementIterator object is an iterator created by a
/// RuleBasedCollator to walk through a string.
///
/// Equivalent to `com.ibm.icu.text.CollationElementIterator`. Java keeps
/// an alias of the collator (`rbc_`); the iterator borrows it.
#[derive(Debug)]
pub struct CollationElementIterator<'r> {
    iter_: Option<OwnedIter>,    // owned
    rbc_: &'r RuleBasedCollator, // aliased
    other_half_: i32,
    /// <0: backwards; 0: just after reset() (previous() begins from end);
    /// 1: just after setOffset(); >1: forward
    dir_: i8,
    /// Stores offsets from expansions and from unsafe-backwards iteration,
    /// so that getOffset() returns intermediate offsets for the CEs
    /// that are consistent with forward iteration.
    offsets_: Option<UVector32>,
    string_: Vec<u16>, // TODO: needed in Java? if so, then add a UCharacterIterator field too?
}

impl<'r> CollationElementIterator<'r> {
    /// Return the primary order of the specified collation element, i.e.
    /// the first 16 bits.
    ///
    /// Equivalent to `primaryOrder(int)`.
    pub fn primary_order(ce: i32) -> i32 {
        ((ce as u32) >> 16) as i32 & 0xffff
    }

    /// Return the secondary order of the specified collation element.
    ///
    /// Equivalent to `secondaryOrder(int)`.
    pub fn secondary_order(ce: i32) -> i32 {
        ((ce as u32) >> 8) as i32 & 0xff
    }

    /// Return the tertiary order of the specified collation element.
    ///
    /// Equivalent to `tertiaryOrder(int)`.
    pub fn tertiary_order(ce: i32) -> i32 {
        ce & 0xff
    }

    fn with_collator(collator: &'r RuleBasedCollator) -> Self {
        Self {
            iter_: None,
            rbc_: collator,
            other_half_: 0,
            dir_: 0,
            offsets_: None,
            string_: Vec::new(),
        }
    }

    /// `CollationElementIterator(String, RuleBasedCollator)` (package
    /// private).
    pub(crate) fn from_string(source: &[u16], collator: &'r RuleBasedCollator) -> Self {
        let mut it = Self::with_collator(collator);
        it.set_text(source);
        it
    }

    /// `CollationElementIterator(CharacterIterator, RuleBasedCollator)`
    /// (package private).
    pub(crate) fn from_character_iterator(
        source: Box<dyn CharacterIterator>,
        collator: &'r RuleBasedCollator,
    ) -> Self {
        let mut it = Self::with_collator(collator);
        it.set_text_character_iterator(source);
        it
    }

    /// `CollationElementIterator(UCharacterIterator, RuleBasedCollator)`
    /// (package private).
    pub(crate) fn from_u_character_iterator(
        source: &dyn UCharacterIterator,
        collator: &'r RuleBasedCollator,
    ) -> Self {
        let mut it = Self::with_collator(collator);
        it.set_text_u_character_iterator(source);
        it
    }

    fn iter(&mut self) -> &mut dyn CollationIterator {
        self.iter_
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .get()
    }

    fn iter_ref(&self) -> &dyn CollationIterator {
        self.iter_
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .get_ref()
    }

    /// Returns the character offset in the source string corresponding to
    /// the next collation element.
    ///
    /// Equivalent to `getOffset()`.
    pub fn get_offset(&self) -> i32 {
        if let Some(offsets) = self.offsets_.as_ref() {
            if self.dir_ < 0 && !offsets.is_empty() {
                // CollationIterator.previousCE() decrements the CEs length
                // while it pops CEs from its internal buffer.
                let mut i = self.iter_ref().base().get_ces_length();
                if self.other_half_ != 0 {
                    // Return the trailing CE offset while we are in the middle of a 64-bit CE.
                    i += 1;
                }
                debug_assert!(i < offsets.size());
                return offsets.element_ati(i);
            }
        }
        self.iter_ref().get_offset()
    }

    /// Get the next collation element in the source string.
    ///
    /// Equivalent to `next()`.
    ///
    /// # Panics
    ///
    /// Java's `IllegalStateException("Illegal change of direction")`, which
    /// ICU throws (an unchecked exception no caller catches).
    #[allow(clippy::should_implement_trait)] // Java method name
    pub fn next(&mut self) -> i32 {
        if self.dir_ > 1 {
            // Continue forward iteration. Test this first.
            if self.other_half_ != 0 {
                let oh = self.other_half_;
                self.other_half_ = 0;
                return oh;
            }
        } else if self.dir_ == 1 {
            // next() after setOffset()
            self.dir_ = 2;
        } else if self.dir_ == 0 {
            // The iter_ is already reset to the start of the text.
            self.dir_ = 2;
        } else {
            // illegal change of direction
            panic!("java.lang.IllegalStateException: Illegal change of direction");
            // Java porting note: ICU4C sets U_INVALID_STATE_ERROR to the return status.
        }
        // No need to keep all CEs in the buffer when we iterate.
        self.iter().base_mut().clear_ces_if_none_remaining();
        let ce = self.iter().next_ce();
        if ce == Collation::NO_CE {
            return NULLORDER;
        }
        // Turn the 64-bit CE into two old-style 32-bit CEs, without quaternary bits.
        let p = ((ce as u64) >> 32) as i64;
        let lower32 = ce as i32;
        let first_half = get_first_half(p, lower32);
        let second_half = get_second_half(p, lower32);
        if second_half != 0 {
            self.other_half_ = second_half | 0xc0; // continuation CE
        }
        first_half
    }

    /// Get the previous collation element in the source string.
    ///
    /// Equivalent to `previous()`.
    ///
    /// # Panics
    ///
    /// Java's `IllegalStateException("Illegal change of direction")`.
    pub fn previous(&mut self) -> i32 {
        if self.dir_ < 0 {
            // Continue backwards iteration. Test this first.
            if self.other_half_ != 0 {
                let oh = self.other_half_;
                self.other_half_ = 0;
                return oh;
            }
        } else if self.dir_ == 0 {
            let len = self.string_.len() as i32;
            self.iter().reset_to_offset(len);
            self.dir_ = -1;
        } else if self.dir_ == 1 {
            // previous() after setOffset()
            self.dir_ = -1;
        } else {
            // illegal change of direction
            panic!("java.lang.IllegalStateException: Illegal change of direction");
            // Java porting note: ICU4C sets U_INVALID_STATE_ERROR to the return status.
        }
        let mut offsets = self.offsets_.take().unwrap_or_default();
        // If we already have expansion CEs, then we also have offsets.
        // Otherwise remember the trailing offset in case we need to
        // write offsets for an artificial expansion.
        let limit_offset = if self.iter().base().get_ces_length() == 0 {
            self.iter().get_offset()
        } else {
            0
        };
        let ce = self.iter().previous_ce(&mut offsets);
        if ce == Collation::NO_CE {
            self.offsets_ = Some(offsets);
            return NULLORDER;
        }
        // Turn the 64-bit CE into two old-style 32-bit CEs, without quaternary bits.
        let p = ((ce as u64) >> 32) as i64;
        let lower32 = ce as i32;
        let first_half = get_first_half(p, lower32);
        let second_half = get_second_half(p, lower32);
        if second_half != 0 {
            if offsets.is_empty() {
                // When we convert a single 64-bit CE into two 32-bit CEs,
                // we need to make this artificial expansion behave like a normal expansion.
                // See CollationIterator.previousCE().
                offsets.add_element(self.iter().get_offset());
                offsets.add_element(limit_offset);
            }
            self.offsets_ = Some(offsets);
            self.other_half_ = first_half;
            return second_half | 0xc0; // continuation CE
        }
        self.offsets_ = Some(offsets);
        first_half
    }

    /// Resets the cursor to the beginning of the string.
    ///
    /// Equivalent to `reset()`.
    pub fn reset(&mut self) {
        self.iter().reset_to_offset(0);
        self.other_half_ = 0;
        self.dir_ = 0;
    }

    /// Sets the iterator to point to the collation element corresponding
    /// to the character at the specified offset.
    ///
    /// Equivalent to `setOffset(int)`.
    pub fn set_offset(&mut self, mut new_offset: i32) {
        if 0 < new_offset && new_offset < self.string_.len() as i32 {
            let mut offset = new_offset;
            loop {
                let c = self.string_[offset as usize];
                if !self.rbc_.is_unsafe(c as i32)
                    || (is_high_surrogate(c)
                        && !self.rbc_.is_unsafe(code_point_at(&self.string_, offset)))
                {
                    break;
                }
                // Back up to before this unsafe character.
                offset -= 1;
                if offset <= 0 {
                    break;
                }
            }
            if offset < new_offset {
                // We might have backed up more than necessary.
                // For example, contractions "ch" and "cu" make both 'h' and 'u' unsafe,
                // but for text "chu" setOffset(2) should remain at 2
                // although we initially back up to offset 0.
                // Find the last safe offset no greater than newOffset by iterating forward.
                let mut last_safe_offset = offset;
                loop {
                    self.iter().reset_to_offset(last_safe_offset);
                    loop {
                        self.iter().next_ce();
                        offset = self.iter().get_offset();
                        if offset != last_safe_offset {
                            break;
                        }
                    }
                    if offset <= new_offset {
                        last_safe_offset = offset;
                    }
                    if offset >= new_offset {
                        break;
                    }
                }
                new_offset = last_safe_offset;
            }
        }
        self.iter().reset_to_offset(new_offset);
        self.other_half_ = 0;
        self.dir_ = 1;
    }

    /// Set a new source string for iteration, and reset the offset to the
    /// beginning of the text.
    ///
    /// Equivalent to `setText(String)`.
    pub fn set_text(&mut self, source: &[u16]) {
        self.string_ = source.to_vec(); // TODO: do we need to remember the source string in a field?
        let ro = self
            .rbc_
            .settings
            .read_only()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let numeric = ro.is_numeric();
        let new_iter = if ro.dont_check_fcd() {
            OwnedIter::Utf16(UTF16CollationIterator::with_text(
                self.rbc_.data.clone(),
                numeric,
                &self.string_,
                0,
            ))
        } else {
            OwnedIter::FcdUtf16(FCDUTF16CollationIterator::with_text(
                self.rbc_.data.clone(),
                numeric,
                &self.string_,
                0,
            ))
        };
        self.iter_ = Some(new_iter);
        self.other_half_ = 0;
        self.dir_ = 0;
    }

    /// Set a new source string iterator for iteration, and reset the offset
    /// to the beginning of the text. The source iterator's integrity will
    /// be preserved since a new copy will be created for use.
    ///
    /// Equivalent to `setText(UCharacterIterator)`. Java's `clone()` of a
    /// `UCharacterIterator` never fails in ICU, so the fallback to the
    /// text contents (`CloneNotSupportedException`) is never taken.
    pub fn set_text_u_character_iterator(&mut self, source: &dyn UCharacterIterator) {
        let mut text_copy = source.clone_box();
        self.string_ = text_copy.get_text(); // TODO: do we need to remember the source string in a field?
                                             // Note: In C++, we just setText(source.getText()).
                                             // In Java, we actually operate on a character iterator.
                                             // (The old code apparently did so only for a CharacterIterator;
                                             // for a UCharacterIterator it also just used source.getText()).
                                             // TODO: do we need to remember the cloned iterator in a field?
        let mut src = source.clone_box();
        src.set_to_start();
        let ro = self
            .rbc_
            .settings
            .read_only()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let numeric = ro.is_numeric();
        let new_iter = if ro.dont_check_fcd() {
            OwnedIter::Iter(IterCollationIterator::new(
                self.rbc_.data.clone(),
                numeric,
                src,
            ))
        } else {
            OwnedIter::FcdIter(FCDIterCollationIterator::new(
                self.rbc_.data.clone(),
                numeric,
                src,
                0,
            ))
        };
        self.iter_ = Some(new_iter);
        self.other_half_ = 0;
        self.dir_ = 0;
    }

    /// Set a new source string iterator for iteration, and reset the offset
    /// to the beginning of the text.
    ///
    /// Equivalent to `setText(CharacterIterator)`.
    pub fn set_text_character_iterator(&mut self, source: Box<dyn CharacterIterator>) {
        // Note: In C++, we just setText(source.getText()).
        // In Java, we actually operate on a character iterator.
        // TODO: do we need to remember the iterator in a field?
        // TODO: apparently we don't clone a CharacterIterator in Java,
        // we only clone the text for a UCharacterIterator?? see the old code in the constructors
        let mut src: Box<dyn UCharacterIterator> = Box::new(CharacterIteratorWrapper::new(source));
        src.set_to_start();
        self.string_ = src.get_text(); // TODO: do we need to remember the source string in a field?
        let ro = self
            .rbc_
            .settings
            .read_only()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let numeric = ro.is_numeric();
        let new_iter = if ro.dont_check_fcd() {
            OwnedIter::Iter(IterCollationIterator::new(
                self.rbc_.data.clone(),
                numeric,
                src,
            ))
        } else {
            OwnedIter::FcdIter(FCDIterCollationIterator::new(
                self.rbc_.data.clone(),
                numeric,
                src,
                0,
            ))
        };
        self.iter_ = Some(new_iter);
        self.other_half_ = 0;
        self.dir_ = 0;
    }

    /// `computeMaxExpansions(CollationData)` (package private).
    pub(crate) fn compute_max_expansions(data: &Arc<CollationData>) -> HashMap<i32, i32> {
        let mut max_expansions: HashMap<i32, i32> = HashMap::new();
        {
            let mut sink = MaxExpSink {
                max_expansions: &mut max_expansions,
            };
            ContractionsAndExpansions::new(None, None, Some(&mut sink), true)
                .for_data(data)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        max_expansions
    }

    /// Returns the maximum length of any expansion sequence that ends with
    /// the specified collation element.
    ///
    /// Equivalent to `getMaxExpansion(int)`.
    pub fn get_max_expansion(&self, ce: i32) -> i32 {
        Self::get_max_expansion_in(self.rbc_.tailoring.max_expansions.get(), ce)
    }

    /// `getMaxExpansion(Map, int)` (package private).
    pub(crate) fn get_max_expansion_in(
        max_expansions: Option<&HashMap<i32, i32>>,
        order: i32,
    ) -> i32 {
        if order == 0 {
            return 1;
        }
        if let Some(max) = max_expansions.and_then(|m| m.get(&order)) {
            return *max;
        }
        if (order & 0xc0) == 0xc0 {
            // old-style continuation CE
            2
        } else {
            1
        }
    }

    fn normalize_dir(&self) -> i8 {
        if self.dir_ == 1 {
            0
        } else {
            self.dir_
        }
    }

    /// Tests that argument object is equals to this
    /// CollationElementIterator.
    ///
    /// Equivalent to `equals(Object)` with a `CollationElementIterator`.
    pub fn equals(&self, thatceiter: &CollationElementIterator<'_>) -> bool {
        if std::ptr::eq(
            self as *const _ as *const u8,
            thatceiter as *const _ as *const u8,
        ) {
            return true;
        }
        use crate::text::collator::Collator;
        self.rbc_.equals(thatceiter.rbc_)
            && self.other_half_ == thatceiter.other_half_
            && self.normalize_dir() == thatceiter.normalize_dir()
            && self.string_ == thatceiter.string_
            && self
                .iter_
                .as_ref()
                .zip(thatceiter.iter_.as_ref())
                .is_some_and(|(a, b)| a.equals(b))
    }

    /// `hashCode()`: not designed (`assert false`, then 42).
    pub fn hash_code(&self) -> i32 {
        debug_assert!(false, "hashCode not designed");
        42 // any arbitrary constant will do
    }

    /// Return the RuleBasedCollator used by this iterator (deprecated).
    ///
    /// Equivalent to `getRuleBasedCollator()`.
    pub fn get_rule_based_collator(&self) -> &RuleBasedCollator {
        self.rbc_
    }
}
