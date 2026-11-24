// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationFastLatinBuilder`.

use rucene::error::{LuceneError, Result};

use crate::lang::u_script::UScript;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fast_latin::CollationFastLatin;
use crate::r#impl::coll::u_vector64::UVector64;
use crate::text::collator::ReorderCodes;
use crate::util::chars_trie::CharsTrie;

fn compare_int64_as_unsigned(a: i64, b: i64) -> i32 {
    let a = a.wrapping_add(i64::MIN);
    let b = b.wrapping_add(i64::MIN);
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}

// TODO: Merge this with the near-identical version in CollationBaseDataBuilder.
fn binary_search(list: &[i64], mut limit: i32, ce: i64) -> i32 {
    if limit == 0 {
        return !0;
    }
    let mut start = 0;
    loop {
        let i = ((start as i64 + limit as i64) / 2) as i32;
        let cmp = compare_int64_as_unsigned(ce, list[i as usize]);
        if cmp == 0 {
            return i;
        } else if cmp < 0 {
            if i == start {
                return !start; // insert ce before i
            }
            limit = i;
        } else {
            if i == start {
                return !(start + 1); // insert ce after i
            }
            start = i;
        }
    }
}

/// space, punct, symbol, currency (not digit)
const NUM_SPECIAL_GROUPS: i32 = ReorderCodes::CURRENCY - ReorderCodes::FIRST + 1;

const CONTRACTION_FLAG: i64 = 0x80000000;

fn is_contraction_char_ce(ce: i64) -> bool {
    ((ce as u64) >> 32) as i64 == Collation::NO_CE_PRIMARY && ce != Collation::NO_CE
}

/// Builds the fast Latin table of a tailoring.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationFastLatinBuilder`.
#[derive(Debug, Clone)]
pub struct CollationFastLatinBuilder {
    // temporary "buffer"
    ce0: i64,
    ce1: i64,
    char_ces: Vec<[i64; 2]>,
    contraction_ces: UVector64,
    unique_ces: UVector64,
    mini_ces: Option<Vec<u16>>,
    // These are constant for a given root collator.
    /// `lastSpecialPrimaries`.
    pub last_special_primaries: [i64; NUM_SPECIAL_GROUPS as usize],
    first_digit_primary: i64,
    first_latin_primary: i64,
    last_latin_primary: i64,
    /// This determines the first normal primary weight which is mapped to
    /// a short mini primary. It must be >=firstDigitPrimary.
    first_short_primary: i64,
    short_primary_overflow: bool,
    result: Vec<u16>,
    header_length: i32,
}

impl Default for CollationFastLatinBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CollationFastLatinBuilder {
    /// `CollationFastLatinBuilder()`.
    pub fn new() -> Self {
        Self {
            ce0: 0,
            ce1: 0,
            char_ces: vec![[0; 2]; CollationFastLatin::NUM_FAST_CHARS as usize],
            contraction_ces: UVector64::new(),
            unique_ces: UVector64::new(),
            mini_ces: None,
            last_special_primaries: [0; NUM_SPECIAL_GROUPS as usize],
            first_digit_primary: 0,
            first_latin_primary: 0,
            last_latin_primary: 0,
            first_short_primary: 0,
            short_primary_overflow: false,
            result: Vec::new(),
            header_length: 0,
        }
    }

    /// `forData(CollationData)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the builder is reused.
    pub fn for_data(&mut self, data: &CollationData) -> Result<bool> {
        if !self.result.is_empty() {
            // This builder is not reusable.
            return Err(LuceneError::illegal_state("attempt to reuse a CollationFastLatinBuilder"));
        }
        if !self.load_groups(data) {
            return Ok(false);
        }

        // Fast handling of digits.
        self.first_short_primary = self.first_digit_primary;
        self.get_ces(data);
        self.encode_unique_ces();
        if self.short_primary_overflow {
            // Give digits long mini primaries,
            // so that there are more short primaries for letters.
            self.first_short_primary = self.first_latin_primary;
            self.reset_ces();
            self.get_ces(data);
            self.encode_unique_ces();
        }
        // Note: If we still have a short-primary overflow but not a long-primary overflow,
        // then we could calculate how many more long primaries would fit,
        // and set the firstShortPrimary to that many after the current firstShortPrimary,
        // and try again.
        // However, this might only benefit the en_US_POSIX tailoring,
        // and it is simpler to suppress building fast Latin data for it in genrb,
        // or by returning false here if shortPrimaryOverflow.

        let ok = !self.short_primary_overflow;
        if ok {
            self.encode_char_ces();
            self.encode_contractions();
        }
        self.contraction_ces.remove_all_elements(); // might reduce heap memory usage
        self.unique_ces.remove_all_elements();
        Ok(ok)
    }

    /// `getHeader()`.
    pub fn get_header(&self) -> Vec<u16> {
        self.result[..self.header_length as usize].to_vec()
    }

    /// `getTable()`.
    pub fn get_table(&self) -> Vec<u16> {
        self.result[self.header_length as usize..].to_vec()
    }

    fn load_groups(&mut self, data: &CollationData) -> bool {
        self.header_length = 1 + NUM_SPECIAL_GROUPS;
        let r0 = (CollationFastLatin::VERSION << 8) | self.header_length;
        self.result.push(r0 as u16);
        // The first few reordering groups should be special groups
        // (space, punct, ..., digit) followed by Latn, then Grek and other scripts.
        for i in 0..NUM_SPECIAL_GROUPS {
            self.last_special_primaries[i as usize] = data.get_last_primary_for_group(ReorderCodes::FIRST + i);
            if self.last_special_primaries[i as usize] == 0 {
                // missing data
                return false;
            }
            // Java's StringBuilder.append(int 0) appends the digit '0'
            // (U+0030): reserve a slot for this group.
            self.result.push(0x30);
        }

        self.first_digit_primary = data.get_first_primary_for_group(ReorderCodes::DIGIT);
        self.first_latin_primary = data.get_first_primary_for_group(UScript::LATIN);
        self.last_latin_primary = data.get_last_primary_for_group(UScript::LATIN);
        if self.first_digit_primary == 0 || self.first_latin_primary == 0 {
            // missing data
            return false;
        }
        true
    }

    fn in_same_group(&self, p: i64, q: i64) -> bool {
        // Both or neither need to be encoded as short primaries,
        // so that we can test only one and use the same bit mask.
        if p >= self.first_short_primary {
            return q >= self.first_short_primary;
        } else if q >= self.first_short_primary {
            return false;
        }
        // Both or neither must be potentially-variable,
        // so that we can test only one and determine if both are variable.
        let last_variable_primary = self.last_special_primaries[(NUM_SPECIAL_GROUPS - 1) as usize];
        if p > last_variable_primary {
            return q > last_variable_primary;
        } else if q > last_variable_primary {
            return false;
        }
        // Both will be encoded with long mini primaries.
        // They must be in the same special reordering group,
        // so that we can test only one and determine if both are variable.
        debug_assert!(p != 0 && q != 0);
        let mut i = 0;
        loop {
            // will terminate
            let last_primary = self.last_special_primaries[i];
            if p <= last_primary {
                return q <= last_primary;
            } else if q <= last_primary {
                return false;
            }
            i += 1;
        }
    }

    fn reset_ces(&mut self) {
        self.contraction_ces.remove_all_elements();
        self.unique_ces.remove_all_elements();
        self.short_primary_overflow = false;
        self.result.truncate(self.header_length as usize);
    }

    fn get_ces(&mut self, data: &CollationData) {
        let mut i = 0usize;
        let mut c: i32 = 0;
        loop {
            if c == CollationFastLatin::LATIN_LIMIT {
                c = CollationFastLatin::PUNCT_START;
            } else if c == CollationFastLatin::PUNCT_LIMIT {
                break;
            }
            let d: &CollationData;
            let mut ce32 = data.get_ce32(c);
            if ce32 == Collation::FALLBACK_CE32 {
                d = data.base();
                ce32 = d.get_ce32(c);
            } else {
                d = data;
            }
            if self.get_ces_from_ce32(d, c, ce32) {
                self.char_ces[i][0] = self.ce0;
                self.char_ces[i][1] = self.ce1;
                self.add_unique_ce(self.ce0);
                self.add_unique_ce(self.ce1);
            } else {
                // bail out for c
                self.ce0 = Collation::NO_CE;
                self.ce1 = 0;
                self.char_ces[i][0] = self.ce0;
                self.char_ces[i][1] = self.ce1;
            }
            if c == 0 && !is_contraction_char_ce(self.ce0) {
                // Always map U+0000 to a contraction.
                // Write a contraction list with only a default value if there is no real contraction.
                debug_assert!(self.contraction_ces.is_empty());
                self.add_contraction_entry(CollationFastLatin::CONTR_CHAR_MASK, self.ce0, self.ce1);
                self.char_ces[0][0] = (Collation::NO_CE_PRIMARY << 32) | CONTRACTION_FLAG;
                self.char_ces[0][1] = 0;
            }
            i += 1;
            c += 1;
        }
        // Terminate the last contraction list.
        self.contraction_ces.add_element(CollationFastLatin::CONTR_CHAR_MASK as i64);
    }

    fn get_ces_from_ce32(&mut self, data: &CollationData, c: i32, mut ce32: i32) -> bool {
        ce32 = data.get_final_ce32(ce32);
        self.ce1 = 0;
        if Collation::is_simple_or_long_ce32(ce32) {
            self.ce0 = Collation::ce_from_ce32(ce32);
        } else {
            match Collation::tag_from_ce32(ce32) {
                Collation::LATIN_EXPANSION_TAG => {
                    self.ce0 = Collation::latin_ce0_from_ce32(ce32);
                    self.ce1 = Collation::latin_ce1_from_ce32(ce32);
                }
                Collation::EXPANSION32_TAG => {
                    let index = Collation::index_from_ce32(ce32);
                    let length = Collation::length_from_ce32(ce32);
                    if length <= 2 {
                        self.ce0 = Collation::ce_from_ce32(data.ce32s[index as usize]);
                        if length == 2 {
                            self.ce1 = Collation::ce_from_ce32(data.ce32s[(index + 1) as usize]);
                        }
                    } else {
                        return false;
                    }
                }
                Collation::EXPANSION_TAG => {
                    let index = Collation::index_from_ce32(ce32);
                    let length = Collation::length_from_ce32(ce32);
                    if length <= 2 {
                        self.ce0 = data.ces[index as usize];
                        if length == 2 {
                            self.ce1 = data.ces[(index + 1) as usize];
                        }
                    } else {
                        return false;
                    }
                }
                // Note: We could support PREFIX_TAG (assert c>=0)
                // by recursing on its default CE32 and checking that none of the prefixes starts
                // with a fast Latin character.
                // However, currently (2013) there are only the L-before-middle-dot
                // prefix mappings in the Latin range, and those would be rejected anyway.
                Collation::CONTRACTION_TAG => {
                    debug_assert!(c >= 0);
                    return self.get_ces_from_contraction_ce32(data, ce32);
                }
                Collation::OFFSET_TAG => {
                    debug_assert!(c >= 0);
                    self.ce0 = data.get_ce_from_offset_ce32(c, ce32);
                }
                _ => return false,
            }
        }
        // A mapping can be completely ignorable.
        if self.ce0 == 0 {
            return self.ce1 == 0;
        }
        // We do not support an ignorable ce0 unless it is completely ignorable.
        let p0 = ((self.ce0 as u64) >> 32) as i64;
        if p0 == 0 {
            return false;
        }
        // We only support primaries up to the Latin script.
        if p0 > self.last_latin_primary {
            return false;
        }
        // We support non-common secondary and case weights only together with short primaries.
        let lower32_0 = self.ce0 as i32;
        if p0 < self.first_short_primary {
            let sc0 = lower32_0 & Collation::SECONDARY_AND_CASE_MASK;
            if sc0 != Collation::COMMON_SECONDARY_CE {
                return false;
            }
        }
        // No below-common tertiary weights.
        if (lower32_0 & Collation::ONLY_TERTIARY_MASK) < Collation::COMMON_WEIGHT16 {
            return false;
        }
        if self.ce1 != 0 {
            // Both primaries must be in the same group,
            // or both must get short mini primaries,
            // or a short-primary CE is followed by a secondary CE.
            // This is so that we can test the first primary and use the same mask for both,
            // and determine for both whether they are variable.
            let p1 = ((self.ce1 as u64) >> 32) as i64;
            if if p1 == 0 { p0 < self.first_short_primary } else { !self.in_same_group(p0, p1) } {
                return false;
            }
            let lower32_1 = self.ce1 as i32;
            // No tertiary CEs.
            if ((lower32_1 as u32) >> 16) == 0 {
                return false;
            }
            // We support non-common secondary and case weights
            // only for secondary CEs or together with short primaries.
            if p1 != 0 && p1 < self.first_short_primary {
                let sc1 = lower32_1 & Collation::SECONDARY_AND_CASE_MASK;
                if sc1 != Collation::COMMON_SECONDARY_CE {
                    return false;
                }
            }
            // No below-common tertiary weights.
            if (lower32_0 & Collation::ONLY_TERTIARY_MASK) < Collation::COMMON_WEIGHT16 {
                return false;
            }
        }
        // No quaternary weights.
        ((self.ce0 | self.ce1) & Collation::QUATERNARY_MASK as i64) == 0
    }

    fn get_ces_from_contraction_ce32(&mut self, data: &CollationData, mut ce32: i32) -> bool {
        let trie_index = Collation::index_from_ce32(ce32);
        ce32 = data.get_ce32_from_contexts(trie_index); // Default if no suffix match.
        // Since the original ce32 is not a prefix mapping,
        // the default ce32 must not be another contraction.
        debug_assert!(!Collation::is_contraction_ce32(ce32));
        let contraction_index = self.contraction_ces.size();
        if self.get_ces_from_ce32(data, Collation::SENTINEL_CP, ce32) {
            self.add_contraction_entry(CollationFastLatin::CONTR_CHAR_MASK, self.ce0, self.ce1);
        } else {
            // Bail out for c-without-contraction.
            self.add_contraction_entry(CollationFastLatin::CONTR_CHAR_MASK, Collation::NO_CE, 0);
        }
        // Handle an encodable contraction unless the next contraction is too long
        // and starts with the same character.
        let mut prev_x = -1;
        let mut add_contraction = false;
        let suffixes = CharsTrie::iterator_of(data.contexts.clone(), trie_index + 2, 0);
        for entry in suffixes {
            let suffix = &entry.chars;
            let x = CollationFastLatin::get_char_index(suffix[0]);
            if x < 0 {
                continue;
            } // ignore anything but fast Latin text
            if x == prev_x {
                if add_contraction {
                    // Bail out for all contractions starting with this character.
                    self.add_contraction_entry(x, Collation::NO_CE, 0);
                    add_contraction = false;
                }
                continue;
            }
            if add_contraction {
                self.add_contraction_entry(prev_x, self.ce0, self.ce1);
            }
            ce32 = entry.value;
            if suffix.len() == 1 && self.get_ces_from_ce32(data, Collation::SENTINEL_CP, ce32) {
                add_contraction = true;
            } else {
                self.add_contraction_entry(x, Collation::NO_CE, 0);
                add_contraction = false;
            }
            prev_x = x;
        }
        if add_contraction {
            self.add_contraction_entry(prev_x, self.ce0, self.ce1);
        }
        // Note: There might not be any fast Latin contractions, but
        // we need to enter contraction handling anyway so that we can bail out
        // when there is a non-fast-Latin character following.
        // For example: Danish &Y<<u+umlaut, when we compare Y vs. ü we need to see the
        // following umlaut and bail out, rather than return the difference of Y vs. u.
        self.ce0 = (Collation::NO_CE_PRIMARY << 32) | CONTRACTION_FLAG | contraction_index as i64;
        self.ce1 = 0;
        true
    }

    fn add_contraction_entry(&mut self, x: i32, cce0: i64, cce1: i64) {
        self.contraction_ces.add_element(x as i64);
        self.contraction_ces.add_element(cce0);
        self.contraction_ces.add_element(cce1);
        self.add_unique_ce(cce0);
        self.add_unique_ce(cce1);
    }

    fn add_unique_ce(&mut self, mut ce: i64) {
        if ce == 0 || ((ce as u64) >> 32) as i64 == Collation::NO_CE_PRIMARY {
            return;
        }
        ce &= !(Collation::CASE_MASK as i64); // blank out case bits
        let i = binary_search(self.unique_ces.get_buffer(), self.unique_ces.size(), ce);
        if i < 0 {
            self.unique_ces.insert_element_at(ce, !i);
        }
    }

    fn get_mini_ce(&self, mut ce: i64) -> i32 {
        ce &= !(Collation::CASE_MASK as i64); // blank out case bits
        let index = binary_search(self.unique_ces.get_buffer(), self.unique_ces.size(), ce);
        debug_assert!(index >= 0);
        self.mini_ces.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))[index as usize] as i32
    }

    #[allow(unused_assignments)] // Java's dead stores to prevSecondary/ter, kept as written
    fn encode_unique_ces(&mut self) {
        let n = self.unique_ces.size();
        let mut mini_ces = vec![0u16; n as usize];
        let mut group = 0;
        let mut last_group_primary = self.last_special_primaries[group as usize];
        // The lowest unique CE must be at least a secondary CE.
        debug_assert!(((self.unique_ces.element_ati(0) as i32 as u32) >> 16) != 0);
        let mut prev_primary: i64 = 0;
        let mut prev_secondary = 0;
        let mut pri = 0;
        let mut sec = 0;
        let mut ter = CollationFastLatin::COMMON_TER;
        for i in 0..n {
            let ce = self.unique_ces.element_ati(i);
            // Note: At least one of the p/s/t weights changes from one unique CE to the next.
            // (uniqueCEs does not store case bits.)
            let p = ((ce as u64) >> 32) as i64;
            if p != prev_primary {
                while p > last_group_primary {
                    debug_assert!(pri <= CollationFastLatin::MAX_LONG);
                    // Set the group's header entry to the
                    // last "long primary" in or before the group.
                    self.result[(1 + group) as usize] = pri as u16;
                    group += 1;
                    if group < NUM_SPECIAL_GROUPS {
                        last_group_primary = self.last_special_primaries[group as usize];
                    } else {
                        last_group_primary = 0xffffffff;
                        break;
                    }
                }
                if p < self.first_short_primary {
                    if pri == 0 {
                        pri = CollationFastLatin::MIN_LONG;
                    } else if pri < CollationFastLatin::MAX_LONG {
                        pri += CollationFastLatin::LONG_INC;
                    } else {
                        mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                        continue;
                    }
                } else if pri < CollationFastLatin::MIN_SHORT {
                    pri = CollationFastLatin::MIN_SHORT;
                } else if pri < (CollationFastLatin::MAX_SHORT - CollationFastLatin::SHORT_INC) {
                    // Reserve the highest primary weight for U+FFFF.
                    pri += CollationFastLatin::SHORT_INC;
                } else {
                    self.short_primary_overflow = true;
                    mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                    continue;
                }
                prev_primary = p;
                prev_secondary = Collation::COMMON_WEIGHT16;
                sec = CollationFastLatin::COMMON_SEC;
                ter = CollationFastLatin::COMMON_TER;
            }
            let lower32 = ce as i32;
            let s = ((lower32 as u32) >> 16) as i32;
            if s != prev_secondary {
                if pri == 0 {
                    if sec == 0 {
                        sec = CollationFastLatin::MIN_SEC_HIGH;
                    } else if sec < CollationFastLatin::MAX_SEC_HIGH {
                        sec += CollationFastLatin::SEC_INC;
                    } else {
                        mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                        continue;
                    }
                    prev_secondary = s;
                    ter = CollationFastLatin::COMMON_TER;
                } else if s < Collation::COMMON_WEIGHT16 {
                    if sec == CollationFastLatin::COMMON_SEC {
                        sec = CollationFastLatin::MIN_SEC_BEFORE;
                    } else if sec < CollationFastLatin::MAX_SEC_BEFORE {
                        sec += CollationFastLatin::SEC_INC;
                    } else {
                        mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                        continue;
                    }
                } else if s == Collation::COMMON_WEIGHT16 {
                    sec = CollationFastLatin::COMMON_SEC;
                } else if sec < CollationFastLatin::MIN_SEC_AFTER {
                    sec = CollationFastLatin::MIN_SEC_AFTER;
                } else if sec < CollationFastLatin::MAX_SEC_AFTER {
                    sec += CollationFastLatin::SEC_INC;
                } else {
                    mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                    continue;
                }
                prev_secondary = s;
                ter = CollationFastLatin::COMMON_TER;
            }
            debug_assert!((lower32 & Collation::CASE_MASK) == 0); // blanked out in uniqueCEs
            let t = lower32 & Collation::ONLY_TERTIARY_MASK;
            if t > Collation::COMMON_WEIGHT16 {
                if ter < CollationFastLatin::MAX_TER_AFTER {
                    ter += 1;
                } else {
                    mini_ces[i as usize] = CollationFastLatin::BAIL_OUT as u16;
                    continue;
                }
            }
            if (CollationFastLatin::MIN_LONG..=CollationFastLatin::MAX_LONG).contains(&pri) {
                debug_assert!(sec == CollationFastLatin::COMMON_SEC);
                mini_ces[i as usize] = (pri | ter) as u16;
            } else {
                mini_ces[i as usize] = (pri | sec | ter) as u16;
            }
        }
        self.mini_ces = Some(mini_ces);
    }

    fn encode_char_ces(&mut self) {
        let mini_ces_start = self.result.len();
        for _ in 0..CollationFastLatin::NUM_FAST_CHARS {
            // Java's StringBuilder.append(int 0) appends the digit '0' (U+0030);
            // every slot is overwritten below or by encodeContractions().
            self.result.push(0x30);
        }
        let index_base = self.result.len() as i32;
        for i in 0..CollationFastLatin::NUM_FAST_CHARS as usize {
            let ce = self.char_ces[i][0];
            if is_contraction_char_ce(ce) {
                continue;
            } // defer contraction
            let mut mini_ce = self.encode_two_ces(ce, self.char_ces[i][1]);
            if ((mini_ce as u32) >> 16) > 0 {
                // if ((unsigned)miniCE > 0xffff)
                // Note: There is a chance that this new expansion is the same as a previous one,
                // and if so, then we could reuse the other expansion.
                // However, that seems unlikely.
                let expansion_index = self.result.len() as i32 - index_base;
                if expansion_index > CollationFastLatin::INDEX_MASK {
                    mini_ce = CollationFastLatin::BAIL_OUT;
                } else {
                    self.result.push((mini_ce >> 16) as u16);
                    self.result.push(mini_ce as u16);
                    mini_ce = CollationFastLatin::EXPANSION | expansion_index;
                }
            }
            self.result[mini_ces_start + i] = mini_ce as u16;
        }
    }

    fn encode_contractions(&mut self) {
        // We encode all contraction lists so that the first word of a list
        // terminates the previous list, and we only need one additional terminator at the end.
        let index_base = self.header_length + CollationFastLatin::NUM_FAST_CHARS;
        let first_contraction_index = self.result.len();
        for i in 0..CollationFastLatin::NUM_FAST_CHARS as usize {
            let ce = self.char_ces[i][0];
            if !is_contraction_char_ce(ce) {
                continue;
            }
            let contraction_index = self.result.len() as i32 - index_base;
            if contraction_index > CollationFastLatin::INDEX_MASK {
                self.result[self.header_length as usize + i] = CollationFastLatin::BAIL_OUT as u16;
                continue;
            }
            let mut first_triple = true;
            let mut index = ce as i32 & 0x7fffffff;
            loop {
                let x = self.contraction_ces.element_ati(index);
                if x == CollationFastLatin::CONTR_CHAR_MASK as i64 && !first_triple {
                    break;
                }
                let cce0 = self.contraction_ces.element_ati(index + 1);
                let cce1 = self.contraction_ces.element_ati(index + 2);
                let mini_ce = self.encode_two_ces(cce0, cce1);
                if mini_ce == CollationFastLatin::BAIL_OUT {
                    self.result.push((x | (1 << CollationFastLatin::CONTR_LENGTH_SHIFT)) as u16);
                } else if ((mini_ce as u32) >> 16) == 0 {
                    // if ((unsigned)miniCE <= 0xffff)
                    self.result.push((x | (2 << CollationFastLatin::CONTR_LENGTH_SHIFT)) as u16);
                    self.result.push(mini_ce as u16);
                } else {
                    self.result.push((x | (3 << CollationFastLatin::CONTR_LENGTH_SHIFT)) as u16);
                    self.result.push((mini_ce >> 16) as u16);
                    self.result.push(mini_ce as u16);
                }
                first_triple = false;
                index += 3;
            }
            // Note: There is a chance that this new contraction list is the same as a previous one,
            // and if so, then we could truncate the result and reuse the other list.
            // However, that seems unlikely.
            self.result[self.header_length as usize + i] = (CollationFastLatin::CONTRACTION | contraction_index) as u16;
        }
        if self.result.len() > first_contraction_index {
            // Terminate the last contraction list.
            self.result.push(CollationFastLatin::CONTR_CHAR_MASK as u16);
        }
    }

    fn encode_two_ces(&self, first: i64, second: i64) -> i32 {
        if first == 0 {
            return 0; // completely ignorable
        }
        if first == Collation::NO_CE {
            return CollationFastLatin::BAIL_OUT;
        }
        debug_assert!(((first as u64) >> 32) as i64 != Collation::NO_CE_PRIMARY);

        let mut mini_ce = self.get_mini_ce(first);
        if mini_ce == CollationFastLatin::BAIL_OUT {
            return mini_ce;
        }
        if mini_ce >= CollationFastLatin::MIN_SHORT {
            // Extract & copy the case bits.
            // Shift them from normal CE bits 15..14 to mini CE bits 4..3.
            let mut c = (first as i32 & Collation::CASE_MASK) >> (14 - 3);
            // Only in mini CEs: Ignorable case bits = 0, lowercase = 1.
            c += CollationFastLatin::LOWER_CASE;
            mini_ce |= c;
        }
        if second == 0 {
            return mini_ce;
        }

        let mut mini_ce1 = self.get_mini_ce(second);
        if mini_ce1 == CollationFastLatin::BAIL_OUT {
            return mini_ce1;
        }

        let mut case1 = second as i32 & Collation::CASE_MASK;
        if mini_ce >= CollationFastLatin::MIN_SHORT
            && (mini_ce & CollationFastLatin::SECONDARY_MASK) == CollationFastLatin::COMMON_SEC
        {
            // Try to combine the two mini CEs into one.
            let sec1 = mini_ce1 & CollationFastLatin::SECONDARY_MASK;
            let ter1 = mini_ce1 & CollationFastLatin::TERTIARY_MASK;
            if sec1 >= CollationFastLatin::MIN_SEC_HIGH && case1 == 0 && ter1 == CollationFastLatin::COMMON_TER {
                // sec1>=sec_high implies pri1==0.
                return (mini_ce & !CollationFastLatin::SECONDARY_MASK) | sec1;
            }
        }

        if mini_ce1 <= CollationFastLatin::SECONDARY_MASK || CollationFastLatin::MIN_SHORT <= mini_ce1 {
            // Secondary CE, or a CE with a short primary, copy the case bits.
            case1 = (case1 >> (14 - 3)) + CollationFastLatin::LOWER_CASE;
            mini_ce1 |= case1;
        }
        (mini_ce << 16) | mini_ce1
    }
}
