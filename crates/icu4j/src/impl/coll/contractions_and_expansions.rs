// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.ContractionsAndExpansions`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::append_code_point;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::utf16_collation_iterator::UTF16CollationIterator;
use crate::text::unicode_set::UnicodeSet;
use crate::util::chars_trie::CharsTrie;

/// Receives the CEs of the enumerated mappings.
///
/// Equivalent to `ContractionsAndExpansions.CESink`.
pub trait CESink {
    /// `handleCE(long)`.
    fn handle_ce(&mut self, ce: i64);
    /// `handleExpansion(long[], int, int)`.
    fn handle_expansion(&mut self, ces: &[i64], start: i32, length: i32);
}

/// Java's `StringBuilder.reverse()`.
fn string_builder_reverse(s: &mut [u16]) {
    s.reverse();
    let mut i = 0;
    while i + 1 < s.len() {
        if crate::jdk::is_low_surrogate(s[i]) && crate::jdk::is_high_surrogate(s[i + 1]) {
            s.swap(i, i + 1);
            i += 1;
        }
        i += 1;
    }
}

/// Enumerates the contractions and expansions of collation data.
///
/// Equivalent to `com.ibm.icu.impl.coll.ContractionsAndExpansions`; the
/// sets and the sink it is given are borrowed for its lifetime.
pub struct ContractionsAndExpansions<'a> {
    data: Option<Arc<CollationData>>,
    contractions: Option<&'a mut UnicodeSet>,
    expansions: Option<&'a mut UnicodeSet>,
    sink: Option<&'a mut dyn CESink>,
    add_prefixes: bool,
    check_tailored: i32, // -1: collected tailored  +1: exclude tailored
    tailored: UnicodeSet,
    ranges: Option<UnicodeSet>,
    unreversed_prefix: Vec<u16>,
    suffix: Option<Vec<u16>>,
    ces: [i64; Collation::MAX_EXPANSION_LENGTH as usize],
}

impl std::fmt::Debug for ContractionsAndExpansions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractionsAndExpansions")
            .field("add_prefixes", &self.add_prefixes)
            .field("check_tailored", &self.check_tailored)
            .finish_non_exhaustive()
    }
}

impl<'a> ContractionsAndExpansions<'a> {
    /// `ContractionsAndExpansions(UnicodeSet, UnicodeSet, CESink, boolean)`.
    pub fn new(
        con: Option<&'a mut UnicodeSet>,
        exp: Option<&'a mut UnicodeSet>,
        s: Option<&'a mut dyn CESink>,
        prefixes: bool,
    ) -> Self {
        Self {
            data: None,
            contractions: con,
            expansions: exp,
            sink: s,
            add_prefixes: prefixes,
            check_tailored: 0,
            tailored: UnicodeSet::new(),
            ranges: None,
            unreversed_prefix: Vec::new(),
            suffix: None,
            ces: [0; Collation::MAX_EXPANSION_LENGTH as usize],
        }
    }

    fn data(&self) -> &Arc<CollationData> {
        self.data
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `forData(CollationData)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when a given set is frozen.
    pub fn for_data(&mut self, d: &Arc<CollationData>) -> Result<()> {
        // Add all from the data, can be tailoring or base.
        if d.base.is_some() {
            self.check_tailored = -1;
        }
        self.data = Some(d.clone());
        let ranges: Vec<_> = {
            let trie = d
                .trie
                .as_deref()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            trie.iterator().take_while(|r| !r.lead_surrogate).collect()
        };
        for range in ranges {
            self.enum_cn_e_range(range.start_code_point, range.end_code_point, range.value)?;
        }
        let Some(base) = d.base.clone() else {
            return Ok(());
        };
        // Add all from the base data but only for un-tailored code points.
        self.tailored.freeze();
        self.check_tailored = 1;
        self.data = Some(base.clone());
        let ranges: Vec<_> = {
            let trie = base
                .trie
                .as_deref()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            trie.iterator().take_while(|r| !r.lead_surrogate).collect()
        };
        for range in ranges {
            self.enum_cn_e_range(range.start_code_point, range.end_code_point, range.value)?;
        }
        Ok(())
    }

    fn enum_cn_e_range(&mut self, start: i32, end: i32, ce32: i32) -> Result<()> {
        if self.check_tailored == 0 {
            // There is no tailoring.
            // No need to collect nor check the tailored set.
        } else if self.check_tailored < 0 {
            // Collect the set of code points with mappings in the tailoring data.
            if ce32 == Collation::FALLBACK_CE32 {
                return Ok(()); // fallback to base, not tailored
            } else {
                self.tailored.add_range(start, end)?;
            }
            // checkTailored > 0: Exclude tailored ranges from the base data enumeration.
        } else if start == end {
            if self.tailored.contains(start)? {
                return Ok(());
            }
        } else if self.tailored.contains_some_range(start, end)? {
            let ranges = self.ranges.get_or_insert_with(UnicodeSet::new);
            ranges.set_range(start, end)?.remove_all(&self.tailored)?;
            let ranges = ranges.clone();
            let count = ranges.get_range_count();
            for i in 0..count {
                self.handle_ce32(ranges.get_range_start(i), ranges.get_range_end(i), ce32)?;
            }
        }
        self.handle_ce32(start, end, ce32)
    }

    /// `forCodePoint(CollationData, int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when a given set is frozen.
    pub fn for_code_point(&mut self, d: &Arc<CollationData>, c: i32) -> Result<()> {
        let mut d = d.clone();
        let mut ce32 = d.get_ce32(c);
        if ce32 == Collation::FALLBACK_CE32 {
            d = d
                .base
                .clone()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            ce32 = d.get_ce32(c);
        }
        self.data = Some(d);
        self.handle_ce32(c, c, ce32)
    }

    #[allow(clippy::unnecessary_unwrap)] // Java's null check; the guarded block also mutates other fields
    fn handle_ce32(&mut self, start: i32, end: i32, mut ce32: i32) -> Result<()> {
        loop {
            if (ce32 & 0xff) < Collation::SPECIAL_CE32_LOW_BYTE {
                // !isSpecialCE32()
                if let Some(sink) = self.sink.as_mut() {
                    sink.handle_ce(Collation::ce_from_simple_ce32(ce32));
                }
                return Ok(());
            }
            match Collation::tag_from_ce32(ce32) {
                Collation::FALLBACK_TAG => return Ok(()),
                Collation::RESERVED_TAG_3
                | Collation::BUILDER_DATA_TAG
                | Collation::LEAD_SURROGATE_TAG => {
                    // Java porting note: U_INTERNAL_PROGRAM_ERROR is set to errorCode in ICU4C.
                    panic!(
                        "java.lang.AssertionError: Unexpected CE32 tag type {} for ce32=0x{:08x}",
                        Collation::tag_from_ce32(ce32),
                        ce32 as u32
                    );
                }
                Collation::LONG_PRIMARY_TAG => {
                    if let Some(sink) = self.sink.as_mut() {
                        sink.handle_ce(Collation::ce_from_long_primary_ce32(ce32));
                    }
                    return Ok(());
                }
                Collation::LONG_SECONDARY_TAG => {
                    if let Some(sink) = self.sink.as_mut() {
                        sink.handle_ce(Collation::ce_from_long_secondary_ce32(ce32));
                    }
                    return Ok(());
                }
                Collation::LATIN_EXPANSION_TAG => {
                    if self.sink.is_some() {
                        self.ces[0] = Collation::latin_ce0_from_ce32(ce32);
                        self.ces[1] = Collation::latin_ce1_from_ce32(ce32);
                        let ces = self.ces;
                        self.sink
                            .as_mut()
                            .expect("checked")
                            .handle_expansion(&ces, 0, 2);
                    }
                    // Optimization: If we have a prefix,
                    // then the relevant strings have been added already.
                    if self.unreversed_prefix.is_empty() {
                        self.add_expansions(start, end)?;
                    }
                    return Ok(());
                }
                Collation::EXPANSION32_TAG => {
                    if self.sink.is_some() {
                        let idx = Collation::index_from_ce32(ce32);
                        let length = Collation::length_from_ce32(ce32);
                        let data = self.data().clone();
                        for i in 0..length {
                            self.ces[i as usize] =
                                Collation::ce_from_ce32(data.ce32s[(idx + i) as usize]);
                        }
                        let ces = self.ces;
                        self.sink
                            .as_mut()
                            .expect("checked")
                            .handle_expansion(&ces, 0, length);
                    }
                    // Optimization: If we have a prefix,
                    // then the relevant strings have been added already.
                    if self.unreversed_prefix.is_empty() {
                        self.add_expansions(start, end)?;
                    }
                    return Ok(());
                }
                Collation::EXPANSION_TAG => {
                    if self.sink.is_some() {
                        let idx = Collation::index_from_ce32(ce32);
                        let length = Collation::length_from_ce32(ce32);
                        let data = self.data().clone();
                        self.sink
                            .as_mut()
                            .expect("checked")
                            .handle_expansion(&data.ces, idx, length);
                    }
                    // Optimization: If we have a prefix,
                    // then the relevant strings have been added already.
                    if self.unreversed_prefix.is_empty() {
                        self.add_expansions(start, end)?;
                    }
                    return Ok(());
                }
                Collation::PREFIX_TAG => {
                    return self.handle_prefixes(start, end, ce32);
                }
                Collation::CONTRACTION_TAG => {
                    return self.handle_contractions(start, end, ce32);
                }
                Collation::DIGIT_TAG => {
                    // Fetch the non-numeric-collation CE32 and continue.
                    ce32 = self.data().ce32s[Collation::index_from_ce32(ce32) as usize];
                }
                Collation::U0000_TAG => {
                    debug_assert!(start == 0 && end == 0);
                    // Fetch the normal ce32 for U+0000 and continue.
                    ce32 = self.data().ce32s[0];
                }
                Collation::HANGUL_TAG => {
                    if self.sink.is_some() {
                        // TODO: This should be optimized,
                        // especially if [start..end] is the complete Hangul range. (assert that)
                        let mut iter = UTF16CollationIterator::new(self.data().clone());
                        let mut hangul: Vec<u16> = Vec::with_capacity(1);
                        for c in start..=end {
                            hangul.clear();
                            append_code_point(&mut hangul, c)?;
                            iter.set_text(false, &hangul, 0);
                            let length = iter.fetch_ces();
                            // Ignore the terminating non-CE.
                            debug_assert!(
                                length >= 2 && iter.base().get_ce(length - 1) == Collation::NO_CE
                            );
                            self.sink.as_mut().expect("checked").handle_expansion(
                                iter.base().get_ces(),
                                0,
                                length - 1,
                            );
                        }
                    }
                    // Optimization: If we have a prefix,
                    // then the relevant strings have been added already.
                    if self.unreversed_prefix.is_empty() {
                        self.add_expansions(start, end)?;
                    }
                    return Ok(());
                }
                Collation::OFFSET_TAG => {
                    // Currently no need to send offset CEs to the sink.
                    return Ok(());
                }
                Collation::IMPLICIT_TAG => {
                    // Currently no need to send implicit CEs to the sink.
                    return Ok(());
                }
                _ => unreachable!("tags are 4 bits"),
            }
        }
    }

    fn handle_prefixes(&mut self, start: i32, end: i32, mut ce32: i32) -> Result<()> {
        let index = Collation::index_from_ce32(ce32);
        ce32 = self.data().get_ce32_from_contexts(index); // Default if no prefix match.
        self.handle_ce32(start, end, ce32)?;
        if !self.add_prefixes {
            return Ok(());
        }
        let prefixes = CharsTrie::new(self.data().contexts.clone(), index + 2).iterator();
        for e in prefixes {
            self.set_prefix(&e.chars);
            // Prefix/pre-context mappings are special kinds of contractions
            // that always yield expansions.
            let mut contractions = self.contractions.take();
            let r = self.add_strings(start, end, contractions.as_deref_mut());
            self.contractions = contractions;
            r?;
            let mut expansions = self.expansions.take();
            let r = self.add_strings(start, end, expansions.as_deref_mut());
            self.expansions = expansions;
            r?;
            self.handle_ce32(start, end, e.value)?;
        }
        self.reset_prefix();
        Ok(())
    }

    /// `handleContractions(int, int, int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when a given set is frozen.
    pub fn handle_contractions(&mut self, start: i32, end: i32, mut ce32: i32) -> Result<()> {
        let index = Collation::index_from_ce32(ce32);
        if (ce32 & Collation::CONTRACT_SINGLE_CP_NO_MATCH) != 0 {
            // No match on the single code point.
            // We are underneath a prefix, and the default mapping is just
            // a fallback to the mappings for a shorter prefix.
            debug_assert!(!self.unreversed_prefix.is_empty());
        } else {
            ce32 = self.data().get_ce32_from_contexts(index); // Default if no suffix match.
            debug_assert!(!Collation::is_contraction_ce32(ce32));
            self.handle_ce32(start, end, ce32)?;
        }
        let suffixes = CharsTrie::new(self.data().contexts.clone(), index + 2).iterator();
        for e in suffixes {
            self.suffix = Some(e.chars.clone());
            let mut contractions = self.contractions.take();
            let r = self.add_strings(start, end, contractions.as_deref_mut());
            self.contractions = contractions;
            r?;
            if !self.unreversed_prefix.is_empty() {
                let mut expansions = self.expansions.take();
                let r = self.add_strings(start, end, expansions.as_deref_mut());
                self.expansions = expansions;
                r?;
            }
            self.handle_ce32(start, end, e.value)?;
        }
        self.suffix = None;
        Ok(())
    }

    /// `addExpansions(int, int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the expansions set is frozen.
    pub fn add_expansions(&mut self, start: i32, end: i32) -> Result<()> {
        if self.unreversed_prefix.is_empty() && self.suffix.is_none() {
            if let Some(expansions) = self.expansions.as_mut() {
                expansions.add_range(start, end)?;
            }
            Ok(())
        } else {
            let mut expansions = self.expansions.take();
            let r = self.add_strings(start, end, expansions.as_deref_mut());
            self.expansions = expansions;
            r
        }
    }

    /// `addStrings(int, int, UnicodeSet)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the set is frozen.
    pub fn add_strings(
        &self,
        mut start: i32,
        end: i32,
        set: Option<&mut UnicodeSet>,
    ) -> Result<()> {
        let Some(set) = set else {
            return Ok(());
        };
        let mut s: Vec<u16> = self.unreversed_prefix.clone();
        loop {
            append_code_point(&mut s, start)?;
            if let Some(suffix) = &self.suffix {
                s.extend_from_slice(suffix);
            }
            set.add_str(&s)?;
            s.truncate(self.unreversed_prefix.len());
            start += 1;
            if start > end {
                break;
            }
        }
        Ok(())
    }

    // Prefixes are reversed in the data structure.
    fn set_prefix(&mut self, pfx: &[u16]) {
        self.unreversed_prefix.clear();
        self.unreversed_prefix.extend_from_slice(pfx);
        string_builder_reverse(&mut self.unreversed_prefix);
    }

    fn reset_prefix(&mut self) {
        self.unreversed_prefix.clear();
    }
}
