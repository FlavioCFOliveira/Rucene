// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationData`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::r#impl::normalizer2_impl::Normalizer2Impl;
use crate::r#impl::trie2_32::Trie2_32;
use crate::lang::u_script::UScript;
use crate::text::collator::ReorderCodes;
use crate::text::unicode_set::UnicodeSet;
use crate::util::icu_exception::ICUException;

/// Collation data container. Immutable data created by a
/// CollationDataBuilder, or loaded from a file, or deserialized from API
/// output.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationData`. Arrays that Java
/// aliases between a tailoring and its base data (never modified after
/// loading) are shared [`Arc`] slices; Java's `null` arrays that are only
/// indexed (`ce32s`, `ces`, `contexts`) are empty slices, and the
/// nullable ones that are tested for `null` are [`Option`]s.
#[derive(Debug, Clone)]
pub struct CollationData {
    /// Main lookup trie.
    pub trie: Option<Arc<Trie2_32>>,
    /// Array of CE32 values. At index 0 there must be CE32(U+0000) to
    /// support U+0000's special-tag for NUL-termination handling.
    pub ce32s: Arc<[i32]>,
    /// Array of CE values for expansions and OFFSET_TAG.
    pub ces: Arc<[i64]>,
    /// Array of prefix and contraction-suffix matching data.
    pub contexts: Arc<[u16]>,
    /// Base collation data, or `None` if this data itself is a base.
    pub base: Option<Arc<CollationData>>,
    /// Simple array of JAMO_CE32S_LENGTH=19+21+27 CE32s, one per canonical
    /// Jamo L/V/T. They are normally simple CE32s, rarely expansions.
    pub jamo_ce32s: Arc<[i32]>,
    /// `nfcImpl`.
    pub nfc_impl: Arc<Normalizer2Impl>,
    /// The single-byte primary weight (xx000000) for numeric collation.
    pub numeric_primary: i64,
    /// 256 flags for which primary-weight lead bytes are compressible.
    pub compressible_bytes: Arc<[bool]>,
    /// Set of code points that are unsafe for starting string comparison
    /// after an identical prefix, or in backwards CE iteration.
    pub unsafe_backward_set: Option<Arc<UnicodeSet>>,
    /// Fast Latin table for common-Latin-text string comparisons.
    pub fast_latin_table: Option<Arc<[u16]>>,
    /// Header portion of the fastLatinTable.
    pub fast_latin_table_header: Option<Arc<[u16]>>,
    /// Data for scripts and reordering groups: number of scripts.
    pub num_scripts: i32,
    /// The length of scriptsIndex is numScripts+16.
    pub scripts_index: Arc<[u16]>,
    /// Start primary weight (top 16 bits only) for a group/script/reserved
    /// range indexed by scriptsIndex.
    pub script_starts: Arc<[u16]>,
    /// Collation elements in the root collator; `None` in tailorings.
    pub root_elements: Option<Arc<[i64]>>,
}

const EMPTY_INT_ARRAY: [i32; 0] = [];

impl CollationData {
    /// `REORDER_RESERVED_BEFORE_LATIN`.
    pub const REORDER_RESERVED_BEFORE_LATIN: i32 = ReorderCodes::FIRST + 14;
    /// `REORDER_RESERVED_AFTER_LATIN`.
    pub const REORDER_RESERVED_AFTER_LATIN: i32 = ReorderCodes::FIRST + 15;
    /// `MAX_NUM_SPECIAL_REORDER_CODES`.
    pub const MAX_NUM_SPECIAL_REORDER_CODES: i32 = 8;
    /// `JAMO_CE32S_LENGTH`.
    pub const JAMO_CE32S_LENGTH: i32 = 19 + 21 + 27;

    /// `CollationData(Normalizer2Impl nfc)`.
    pub fn new(nfc: Arc<Normalizer2Impl>) -> Self {
        Self {
            trie: None,
            ce32s: Arc::from(Vec::new()),
            ces: Arc::from(Vec::new()),
            contexts: Arc::from(Vec::new()),
            base: None,
            jamo_ce32s: Arc::from(vec![0; Self::JAMO_CE32S_LENGTH as usize]),
            nfc_impl: nfc,
            numeric_primary: 0x12000000,
            compressible_bytes: Arc::from(Vec::new()),
            unsafe_backward_set: None,
            fast_latin_table: None,
            fast_latin_table_header: None,
            num_scripts: 0,
            scripts_index: Arc::from(Vec::new()),
            script_starts: Arc::from(Vec::new()),
            root_elements: None,
        }
    }

    fn trie(&self) -> &Trie2_32 {
        self.trie.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// The base data; Java dereferences `base` without a check.
    pub fn base(&self) -> &CollationData {
        self.base.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `getCE32(int)`.
    pub fn get_ce32(&self, c: i32) -> i32 {
        self.trie().get(c)
    }

    /// `getCE32FromSupplementary(int)`.
    pub fn get_ce32_from_supplementary(&self, c: i32) -> i32 {
        self.trie().get(c) // TODO: port UTRIE2_GET32_FROM_SUPP(trie, c) to Java?
    }

    /// `isDigit(int)`.
    pub fn is_digit(&self, c: i32) -> bool {
        if c < 0x660 {
            (0x30..=0x39).contains(&c)
        } else {
            Collation::has_ce32_tag(self.get_ce32(c), Collation::DIGIT_TAG)
        }
    }

    /// `isUnsafeBackward(int, boolean)`.
    pub fn is_unsafe_backward(&self, c: i32, numeric: bool) -> bool {
        let set = self.unsafe_backward_set.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        set.contains_valid(c) || (numeric && self.is_digit(c))
    }

    /// `isCompressibleLeadByte(int)`.
    pub fn is_compressible_lead_byte(&self, b: i32) -> bool {
        self.compressible_bytes[b as usize]
    }

    /// `isCompressiblePrimary(long)`.
    pub fn is_compressible_primary(&self, p: i64) -> bool {
        self.is_compressible_lead_byte(((p as i32 as u32) >> 24) as i32)
    }

    /// Returns the CE32 from two contexts words.
    pub fn get_ce32_from_contexts(&self, index: i32) -> i32 {
        ((self.contexts[index as usize] as i32) << 16) | self.contexts[(index + 1) as usize] as i32
    }

    /// Returns the CE32 for an indirect special CE32 (e.g., with
    /// DIGIT_TAG). Requires that `ce32` is special.
    pub fn get_indirect_ce32(&self, mut ce32: i32) -> i32 {
        debug_assert!(Collation::is_special_ce32(ce32));
        let tag = Collation::tag_from_ce32(ce32);
        if tag == Collation::DIGIT_TAG {
            // Fetch the non-numeric-collation CE32.
            ce32 = self.ce32s[Collation::index_from_ce32(ce32) as usize];
        } else if tag == Collation::LEAD_SURROGATE_TAG {
            ce32 = Collation::UNASSIGNED_CE32;
        } else if tag == Collation::U0000_TAG {
            // Fetch the normal ce32 for U+0000.
            ce32 = self.ce32s[0];
        }
        ce32
    }

    /// Returns `ce32`, or the CE32 for an indirect special one.
    pub fn get_final_ce32(&self, mut ce32: i32) -> i32 {
        if Collation::is_special_ce32(ce32) {
            ce32 = self.get_indirect_ce32(ce32);
        }
        ce32
    }

    /// Computes a CE from c's ce32 which has the OFFSET_TAG.
    pub fn get_ce_from_offset_ce32(&self, c: i32, ce32: i32) -> i64 {
        let data_ce = self.ces[Collation::index_from_ce32(ce32) as usize];
        Collation::make_ce(Collation::get_three_byte_primary_for_offset_data(c, data_ce))
    }

    /// Returns the single CE that c maps to.
    ///
    /// Equivalent to `getSingleCE(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if c does not map to a single CE.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError` for an unexpected CE32 tag.
    pub fn get_single_ce(&self, c: i32) -> Result<i64> {
        let d: &CollationData;
        let mut ce32 = self.get_ce32(c);
        if ce32 == Collation::FALLBACK_CE32 {
            d = self.base();
            ce32 = d.get_ce32(c);
        } else {
            d = self;
        }
        let not_one = |ce32: i32| {
            LuceneError::unsupported_operation(format!(
                "there is not exactly one collation element for U+{c:04X} (CE32 0x{:08x})",
                ce32 as u32
            ))
        };
        while Collation::is_special_ce32(ce32) {
            match Collation::tag_from_ce32(ce32) {
                Collation::LATIN_EXPANSION_TAG
                | Collation::BUILDER_DATA_TAG
                | Collation::PREFIX_TAG
                | Collation::CONTRACTION_TAG
                | Collation::HANGUL_TAG
                | Collation::LEAD_SURROGATE_TAG => return Err(not_one(ce32)),
                Collation::FALLBACK_TAG | Collation::RESERVED_TAG_3 => {
                    panic!("java.lang.AssertionError: unexpected CE32 tag for U+{c:04X} (CE32 0x{:08x})", ce32 as u32)
                }
                Collation::LONG_PRIMARY_TAG => return Ok(Collation::ce_from_long_primary_ce32(ce32)),
                Collation::LONG_SECONDARY_TAG => return Ok(Collation::ce_from_long_secondary_ce32(ce32)),
                Collation::EXPANSION32_TAG => {
                    if Collation::length_from_ce32(ce32) == 1 {
                        ce32 = d.ce32s[Collation::index_from_ce32(ce32) as usize];
                    } else {
                        return Err(not_one(ce32));
                    }
                }
                Collation::EXPANSION_TAG => {
                    if Collation::length_from_ce32(ce32) == 1 {
                        return Ok(d.ces[Collation::index_from_ce32(ce32) as usize]);
                    } else {
                        return Err(not_one(ce32));
                    }
                }
                Collation::DIGIT_TAG => {
                    // Fetch the non-numeric-collation CE32 and continue.
                    ce32 = d.ce32s[Collation::index_from_ce32(ce32) as usize];
                }
                Collation::U0000_TAG => {
                    debug_assert!(c == 0);
                    // Fetch the normal ce32 for U+0000 and continue.
                    ce32 = d.ce32s[0];
                }
                Collation::OFFSET_TAG => return Ok(d.get_ce_from_offset_ce32(c, ce32)),
                Collation::IMPLICIT_TAG => return Ok(Collation::unassigned_ce_from_code_point(c)),
                _ => unreachable!("tags are 4 bits"),
            }
        }
        Ok(Collation::ce_from_simple_ce32(ce32))
    }

    /// Returns the FCD16 value for code point c. c must be >= 0.
    pub fn get_fcd16(&self, c: i32) -> i32 {
        self.nfc_impl.get_fcd16(c)
    }

    /// Returns the first primary for the script's reordering group.
    pub fn get_first_primary_for_group(&self, script: i32) -> i64 {
        let index = self.get_script_index(script);
        if index == 0 {
            0
        } else {
            (self.script_starts[index as usize] as i64) << 16
        }
    }

    /// Returns the last primary for the script's reordering group.
    pub fn get_last_primary_for_group(&self, script: i32) -> i64 {
        let index = self.get_script_index(script);
        if index == 0 {
            return 0;
        }
        let limit = self.script_starts[(index + 1) as usize] as i64;
        (limit << 16) - 1
    }

    /// Finds the reordering group which contains the primary weight.
    pub fn get_group_for_primary(&self, mut p: i64) -> i32 {
        p >>= 16;
        if p < self.script_starts[1] as i64 || self.script_starts[self.script_starts.len() - 1] as i64 <= p {
            return -1;
        }
        let mut index = 1usize;
        while p >= self.script_starts[index + 1] as i64 {
            index += 1;
        }
        for i in 0..self.num_scripts {
            if self.scripts_index[i as usize] as usize == index {
                return i;
            }
        }
        for i in 0..Self::MAX_NUM_SPECIAL_REORDER_CODES {
            if self.scripts_index[(self.num_scripts + i) as usize] as usize == index {
                return ReorderCodes::FIRST + i;
            }
        }
        -1
    }

    fn get_script_index(&self, mut script: i32) -> i32 {
        if script < 0 {
            0
        } else if script < self.num_scripts {
            self.scripts_index[script as usize] as i32
        } else if script < ReorderCodes::FIRST {
            0
        } else {
            script -= ReorderCodes::FIRST;
            if script < Self::MAX_NUM_SPECIAL_REORDER_CODES {
                self.scripts_index[(self.num_scripts + script) as usize] as i32
            } else {
                0
            }
        }
    }

    /// `getEquivalentScripts(int)`.
    pub fn get_equivalent_scripts(&self, script: i32) -> Vec<i32> {
        let index = self.get_script_index(script);
        if index == 0 {
            return EMPTY_INT_ARRAY.to_vec();
        }
        if script >= ReorderCodes::FIRST {
            // Special groups have no aliases.
            return vec![script];
        }

        let mut length = 0;
        for i in 0..self.num_scripts {
            if self.scripts_index[i as usize] as i32 == index {
                length += 1;
            }
        }
        let mut dest = vec![0; length];
        if length == 1 {
            dest[0] = script;
            return dest;
        }
        length = 0;
        for i in 0..self.num_scripts {
            if self.scripts_index[i as usize] as i32 == index {
                dest[length] = i;
                length += 1;
            }
        }
        dest
    }

    /// Writes the permutation of primary-weight ranges for the given
    /// reordering of scripts and groups.
    ///
    /// Equivalent to `makeReorderRanges(int[], UVector32)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid reorder codes,
    /// `ICUException` when too many scripts are reordered.
    pub fn make_reorder_ranges(&self, reorder: &[i32], ranges: &mut UVector32) -> Result<()> {
        self.make_reorder_ranges_latin(reorder, false, ranges)
    }

    fn make_reorder_ranges_latin(&self, reorder: &[i32], latin_must_move: bool, ranges: &mut UVector32) -> Result<()> {
        ranges.remove_all_elements();
        let mut length = reorder.len();
        if length == 0 || (length == 1 && reorder[0] == UScript::UNKNOWN) {
            return Ok(());
        }

        // Maps each script-or-group range to a new lead byte.
        let mut table = vec![0i16; self.script_starts.len() - 1]; // C++: uint8_t[]

        {
            // Set "don't care" values for reserved ranges.
            let mut index = self.scripts_index
                [(self.num_scripts + Self::REORDER_RESERVED_BEFORE_LATIN - ReorderCodes::FIRST) as usize]
                as usize;
            if index != 0 {
                table[index] = 0xff;
            }
            index = self.scripts_index
                [(self.num_scripts + Self::REORDER_RESERVED_AFTER_LATIN - ReorderCodes::FIRST) as usize]
                as usize;
            if index != 0 {
                table[index] = 0xff;
            }
        }

        // Never reorder special low and high primary lead bytes.
        debug_assert!(self.script_starts.len() >= 2);
        debug_assert!(self.script_starts[0] == 0);
        let mut low_start = self.script_starts[1] as i32;
        debug_assert!(low_start == ((Collation::MERGE_SEPARATOR_BYTE + 1) << 8));
        let mut high_limit = self.script_starts[self.script_starts.len() - 1] as i32;
        debug_assert!(high_limit == (Collation::TRAIL_WEIGHT_BYTE << 8));

        // Get the set of special reorder codes in the input list.
        // This supports a fixed number of special reorder codes;
        // it works for data with codes beyond Collator.ReorderCodes.LIMIT.
        let mut specials = 0;
        for &code in &reorder[..length] {
            let reorder_code = code.wrapping_sub(ReorderCodes::FIRST);
            if (0..Self::MAX_NUM_SPECIAL_REORDER_CODES).contains(&reorder_code) {
                specials |= 1 << reorder_code;
            }
        }

        // Start the reordering with the special low reorder codes that do not occur in the input.
        for i in 0..Self::MAX_NUM_SPECIAL_REORDER_CODES {
            let index = self.scripts_index[(self.num_scripts + i) as usize] as i32;
            if index != 0 && (specials & (1 << i)) == 0 {
                low_start = self.add_low_script_range(&mut table, index, low_start);
            }
        }

        // Skip the reserved range before Latin if Latin is the first script,
        // so that we do not move it unnecessarily.
        let mut skipped_reserved = 0;
        if specials == 0 && reorder[0] == UScript::LATIN && !latin_must_move {
            let index = self.scripts_index[UScript::LATIN as usize] as usize;
            debug_assert!(index != 0);
            let start = self.script_starts[index] as i32;
            debug_assert!(low_start <= start);
            skipped_reserved = start - low_start;
            low_start = start;
        }

        // Reorder according to the input scripts, continuing from the bottom of the primary range.
        let mut has_reorder_to_end = false;
        let mut i = 0;
        while i < length {
            let mut script = reorder[i];
            i += 1;
            if script == UScript::UNKNOWN {
                // Put the remaining scripts at the top.
                has_reorder_to_end = true;
                while i < length {
                    length -= 1;
                    script = reorder[length];
                    if script == UScript::UNKNOWN {
                        // Must occur at most once.
                        return Err(LuceneError::illegal_argument("setReorderCodes(): duplicate UScript.UNKNOWN"));
                    }
                    if script == ReorderCodes::DEFAULT {
                        return Err(LuceneError::illegal_argument(
                            "setReorderCodes(): UScript.DEFAULT together with other scripts",
                        ));
                    }
                    let index = self.get_script_index(script);
                    if index == 0 {
                        continue;
                    }
                    if table[index as usize] != 0 {
                        // Duplicate or equivalent script.
                        return Err(LuceneError::illegal_argument(format!(
                            "setReorderCodes(): duplicate or equivalent script {}",
                            Self::script_code_string(script)
                        )));
                    }
                    high_limit = self.add_high_script_range(&mut table, index, high_limit);
                }
                break;
            }
            if script == ReorderCodes::DEFAULT {
                // The default code must be the only one in the list, and that is handled by the caller.
                // Otherwise it must not be used.
                return Err(LuceneError::illegal_argument(
                    "setReorderCodes(): UScript.DEFAULT together with other scripts",
                ));
            }
            let index = self.get_script_index(script);
            if index == 0 {
                continue;
            }
            if table[index as usize] != 0 {
                // Duplicate or equivalent script.
                return Err(LuceneError::illegal_argument(format!(
                    "setReorderCodes(): duplicate or equivalent script {}",
                    Self::script_code_string(script)
                )));
            }
            low_start = self.add_low_script_range(&mut table, index, low_start);
        }

        // Put all remaining scripts into the middle.
        for i in 1..self.script_starts.len() - 1 {
            let lead_byte = table[i];
            if lead_byte != 0 {
                continue;
            }
            let start = self.script_starts[i] as i32;
            if !has_reorder_to_end && start > low_start {
                // No need to move this script.
                low_start = start;
            }
            low_start = self.add_low_script_range(&mut table, i as i32, low_start);
        }
        if low_start > high_limit {
            if (low_start - (skipped_reserved & 0xff00)) <= high_limit {
                // Try not skipping the before-Latin reserved range.
                return self.make_reorder_ranges_latin(reorder, true, ranges);
            }
            // We need more primary lead bytes than available, despite the reserved ranges.
            return Err(ICUException::with_message(
                "setReorderCodes(): reordering too many partial-primary-lead-byte scripts",
            )
            .error());
        }

        // Turn lead bytes into a list of (limit, offset) pairs.
        // Encode each pair in one list element:
        // Upper 16 bits = limit, lower 16 = signed lead byte offset.
        let mut offset: i32 = 0;
        let mut i = 1usize;
        loop {
            let mut next_offset = offset;
            while i < self.script_starts.len() - 1 {
                let new_lead_byte = table[i] as i32;
                if new_lead_byte == 0xff {
                    // "Don't care" lead byte for reserved range, continue with current offset.
                } else {
                    next_offset = new_lead_byte - ((self.script_starts[i] as i32) >> 8);
                    if next_offset != offset {
                        break;
                    }
                }
                i += 1;
            }
            if offset != 0 || i < self.script_starts.len() - 1 {
                ranges.add_element(((self.script_starts[i] as i32) << 16) | (offset & 0xffff));
            }
            if i == self.script_starts.len() - 1 {
                break;
            }
            offset = next_offset;
            i += 1;
        }
        Ok(())
    }

    fn add_low_script_range(&self, table: &mut [i16], index: i32, mut low_start: i32) -> i32 {
        let start = self.script_starts[index as usize] as i32;
        if (start & 0xff) < (low_start & 0xff) {
            low_start += 0x100;
        }
        table[index as usize] = (low_start >> 8) as i16;
        let limit = self.script_starts[(index + 1) as usize] as i32;
        low_start = ((low_start & 0xff00) + ((limit & 0xff00) - (start & 0xff00))) | (limit & 0xff);
        low_start
    }

    fn add_high_script_range(&self, table: &mut [i16], index: i32, mut high_limit: i32) -> i32 {
        let limit = self.script_starts[(index + 1) as usize] as i32;
        if (limit & 0xff) > (high_limit & 0xff) {
            high_limit -= 0x100;
        }
        let start = self.script_starts[index as usize] as i32;
        high_limit = ((high_limit & 0xff00) - ((limit & 0xff00) - (start & 0xff00))) | (start & 0xff);
        table[index as usize] = (high_limit >> 8) as i16;
        high_limit
    }

    fn script_code_string(script: i32) -> String {
        // Do not use the script name here: We do not want to depend on that data.
        if script < ReorderCodes::FIRST {
            script.to_string()
        } else {
            format!("0x{:x}", script as u32)
        }
    }
}
