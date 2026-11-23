// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationSettings`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::shared_object::SharedObject;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::text::collator::{self, ReorderCodes};

/// Collation settings/options/attributes.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationSettings` (a
/// `SharedObject`). The reorder table, ranges and codes are shared
/// [`Arc`] slices: Java shares those arrays between clones because it only
/// replaces them, never modifies them; [`Clone`] copies
/// `fastLatinPrimaries`, as Java's `clone()` does.
#[derive(Debug, Clone)]
pub struct CollationSettings {
    /// Options bit set.
    pub options: i32,
    /// Variable-top primary weight.
    pub variable_top: i64,
    /// 256-byte table for reordering permutation of primary lead bytes;
    /// `None` if no reordering.
    pub reorder_table: Option<Arc<[i8]>>,
    /// Limit of last reordered range. 0 if no reordering or no split bytes.
    pub min_high_no_reorder: i64,
    /// Primary-weight range limits & offsets (`reorderRanges`).
    pub reorder_ranges: Option<Arc<[i64]>>,
    /// Array of reorder codes; ignored if `reorder_table` is `None`.
    pub reorder_codes: Arc<[i32]>,
    /// Options for CollationFastLatin. Negative if disabled.
    pub fast_latin_options: i32,
    /// `fastLatinPrimaries` (mutable contents, length
    /// `CollationFastLatin.LATIN_LIMIT`).
    pub fast_latin_primaries: Vec<u16>,
}

impl SharedObject for CollationSettings {}

impl Default for CollationSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for CollationSettings {
    /// `equals(Object)`.
    fn eq(&self, o: &Self) -> bool {
        if self.options != o.options {
            return false;
        }
        if (self.options & Self::ALTERNATE_MASK) != 0 && self.variable_top != o.variable_top {
            return false;
        }
        if self.reorder_codes[..] != o.reorder_codes[..] {
            return false;
        }
        true
    }
}

impl CollationSettings {
    /// Options bit 0: Perform the FCD check on the input text and deliver
    /// normalized text.
    pub const CHECK_FCD: i32 = 1;
    /// Options bit 1: Numeric collation.
    pub const NUMERIC: i32 = 2;
    /// "Shifted" alternate handling, see `ALTERNATE_MASK`.
    pub const SHIFTED: i32 = 4;
    /// Options bits 3..2: Alternate-handling mask.
    pub const ALTERNATE_MASK: i32 = 0xc;
    /// Options bits 6..4: The 3-bit maxVariable value bit field is shifted
    /// by this value.
    pub const MAX_VARIABLE_SHIFT: i32 = 4;
    /// maxVariable options bit mask before shifting.
    pub const MAX_VARIABLE_MASK: i32 = 0x70;
    /// Options bit 8: Sort uppercase first if caseLevel or caseFirst is on.
    pub const UPPER_FIRST: i32 = 0x100;
    /// Options bit 9: Keep the case bits in the tertiary weight.
    pub const CASE_FIRST: i32 = 0x200;
    /// Options bit mask for caseFirst and upperFirst, before shifting.
    pub const CASE_FIRST_AND_UPPER_MASK: i32 = Self::CASE_FIRST | Self::UPPER_FIRST;
    /// Options bit 10: Insert the case level between the secondary and
    /// tertiary levels.
    pub const CASE_LEVEL: i32 = 0x400;
    /// Options bit 11: Compare secondary weights backwards. ("French
    /// secondary")
    pub const BACKWARD_SECONDARY: i32 = 0x800;
    /// Options bits 15..12: The 4-bit strength value bit field is shifted by
    /// this value.
    pub const STRENGTH_SHIFT: i32 = 12;
    /// Strength options bit mask before shifting.
    pub const STRENGTH_MASK: i32 = 0xf000;

    /// maxVariable values
    pub const MAX_VAR_SPACE: i32 = 0;
    /// `MAX_VAR_PUNCT`.
    pub const MAX_VAR_PUNCT: i32 = 1;
    /// `MAX_VAR_SYMBOL`.
    pub const MAX_VAR_SYMBOL: i32 = 2;
    /// `MAX_VAR_CURRENCY`.
    pub const MAX_VAR_CURRENCY: i32 = 3;

    /// `CollationSettings()`.
    pub fn new() -> Self {
        Self {
            options: (collator::TERTIARY << Self::STRENGTH_SHIFT) // DEFAULT_STRENGTH
                | (Self::MAX_VAR_PUNCT << Self::MAX_VARIABLE_SHIFT),
            variable_top: 0,
            reorder_table: None,
            min_high_no_reorder: 0,
            reorder_ranges: None,
            reorder_codes: Arc::from(Vec::new()),
            fast_latin_options: -1,
            fast_latin_primaries: vec![0; 0x180],
        }
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut h = self.options << 8;
        if (self.options & Self::ALTERNATE_MASK) != 0 {
            h ^= self.variable_top as i32;
        }
        h ^= self.reorder_codes.len() as i32;
        for (i, &code) in self.reorder_codes.iter().enumerate() {
            h ^= code.wrapping_shl(i as u32);
        }
        h
    }

    /// `resetReordering()`.
    pub fn reset_reordering(&mut self) {
        // When we turn off reordering, we want to set a null permutation
        // rather than a no-op permutation.
        self.reorder_table = None;
        self.min_high_no_reorder = 0;
        self.reorder_ranges = None;
        self.reorder_codes = Arc::from(Vec::new());
    }

    /// `aliasReordering(CollationData, int[], int, byte[])`.
    ///
    /// # Errors
    ///
    /// The exceptions of [`set_reordering`](Self::set_reordering) when the
    /// data must be regenerated.
    pub fn alias_reordering(
        &mut self,
        data: Option<&CollationData>,
        codes_and_ranges: &[i32],
        codes_length: i32,
        table: Option<Arc<[i8]>>,
    ) -> Result<()> {
        let codes: Arc<[i32]> = Arc::from(&codes_and_ranges[..codes_length as usize]);
        let ranges_start = codes_length;
        let ranges_limit = codes_and_ranges.len() as i32;
        let ranges_length = ranges_limit - ranges_start;
        if let Some(table) = table.as_ref().filter(|table| {
            if ranges_length == 0 {
                !Self::reorder_table_has_split_bytes(table)
            } else {
                ranges_length >= 2
                    // The first offset must be 0. The last offset must not be 0.
                    && (codes_and_ranges[ranges_start as usize] & 0xffff) == 0
                    && (codes_and_ranges[(ranges_limit - 1) as usize] & 0xffff) != 0
            }
        }) {
            self.reorder_table = Some(table.clone());
            self.reorder_codes = codes;
            // Drop ranges before the first split byte. They are reordered by the table.
            // This then speeds up reordering of the remaining ranges.
            let mut first_split_byte_range_index = ranges_start;
            while first_split_byte_range_index < ranges_limit
                && (codes_and_ranges[first_split_byte_range_index as usize] & 0xff0000) == 0
            {
                // The second byte of the primary limit is 0.
                first_split_byte_range_index += 1;
            }
            if first_split_byte_range_index == ranges_limit {
                debug_assert!(!Self::reorder_table_has_split_bytes(table));
                self.min_high_no_reorder = 0;
                self.reorder_ranges = None;
            } else {
                debug_assert!(
                    table[((codes_and_ranges[first_split_byte_range_index as usize] as u32) >> 24) as usize] == 0
                );
                self.min_high_no_reorder = (codes_and_ranges[(ranges_limit - 1) as usize] as i64) & 0xffff0000;
                self.set_reorder_ranges(
                    codes_and_ranges,
                    first_split_byte_range_index,
                    ranges_limit - first_split_byte_range_index,
                );
            }
            return Ok(());
        }
        // Regenerate missing data.
        let data = data.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        self.set_reordering(data, &codes)
    }

    /// `setReordering(CollationData, int[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`/`ICUException` of
    /// `CollationData.makeReorderRanges`.
    pub fn set_reordering(&mut self, data: &CollationData, codes: &[i32]) -> Result<()> {
        if codes.is_empty() || (codes.len() == 1 && codes[0] == ReorderCodes::NONE) {
            self.reset_reordering();
            return Ok(());
        }
        let mut ranges_list = UVector32::new();
        data.make_reorder_ranges(codes, &mut ranges_list)?;
        let mut ranges_length = ranges_list.size();
        if ranges_length == 0 {
            self.reset_reordering();
            return Ok(());
        }
        let ranges = ranges_list.get_buffer();
        // ranges[] contains at least two (limit, offset) pairs.
        // The first offset must be 0. The last offset must not be 0.
        // Separators (at the low end) and trailing weights (at the high end)
        // are never reordered.
        debug_assert!(ranges_length >= 2);
        debug_assert!((ranges[0] & 0xffff) == 0 && (ranges[(ranges_length - 1) as usize] & 0xffff) != 0);
        self.min_high_no_reorder = (ranges[(ranges_length - 1) as usize] as i64) & 0xffff0000;

        // Write the lead byte permutation table.
        // Set a 0 for each lead byte that has a range boundary in the middle.
        let mut table = vec![0i8; 256];
        let mut b: i32 = 0;
        let mut first_split_byte_range_index: i32 = -1;
        for i in 0..ranges_length {
            let pair = ranges[i as usize];
            let limit1 = ((pair as u32) >> 24) as i32;
            while b < limit1 {
                table[b as usize] = b.wrapping_add(pair) as i8;
                b += 1;
            }
            // Check the second byte of the limit.
            if (pair & 0xff0000) != 0 {
                table[limit1 as usize] = 0;
                b = limit1 + 1;
                if first_split_byte_range_index < 0 {
                    first_split_byte_range_index = i;
                }
            }
        }
        while b <= 0xff {
            table[b as usize] = b as i8;
            b += 1;
        }
        let ranges_start;
        if first_split_byte_range_index < 0 {
            // The lead byte permutation table alone suffices for reordering.
            ranges_start = 0;
            ranges_length = 0;
        } else {
            // Remove the ranges below the first split byte.
            ranges_start = first_split_byte_range_index;
            ranges_length -= first_split_byte_range_index;
        }
        let ranges = ranges.to_vec();
        self.set_reorder_arrays(Some(codes), &ranges, ranges_start, ranges_length, Some(Arc::from(table)));
        Ok(())
    }

    fn set_reorder_arrays(
        &mut self,
        codes: Option<&[i32]>,
        ranges: &[i32],
        ranges_start: i32,
        ranges_length: i32,
        table: Option<Arc<[i8]>>,
    ) {
        // Very different from C++. See the comments after the reorderCodes declaration.
        let codes: Arc<[i32]> = Arc::from(codes.unwrap_or(&[]));
        debug_assert!(codes.is_empty() == table.is_none());
        self.reorder_table = table;
        self.reorder_codes = codes;
        self.set_reorder_ranges(ranges, ranges_start, ranges_length);
    }

    fn set_reorder_ranges(&mut self, ranges: &[i32], mut ranges_start: i32, ranges_length: i32) {
        if ranges_length == 0 {
            self.reorder_ranges = None;
        } else {
            let mut reorder_ranges = vec![0i64; ranges_length as usize];
            let mut i = 0;
            loop {
                reorder_ranges[i] = (ranges[ranges_start as usize] as i64) & 0xffffffff;
                i += 1;
                ranges_start += 1;
                if i >= ranges_length as usize {
                    break;
                }
            }
            self.reorder_ranges = Some(Arc::from(reorder_ranges));
        }
    }

    /// `copyReorderingFrom(CollationSettings)`.
    pub fn copy_reordering_from(&mut self, other: &CollationSettings) {
        if !other.has_reordering() {
            self.reset_reordering();
            return;
        }
        self.min_high_no_reorder = other.min_high_no_reorder;
        self.reorder_table = other.reorder_table.clone();
        self.reorder_ranges = other.reorder_ranges.clone();
        self.reorder_codes = other.reorder_codes.clone();
    }

    /// `hasReordering()`.
    pub fn has_reordering(&self) -> bool {
        self.reorder_table.is_some()
    }

    fn reorder_table_has_split_bytes(table: &[i8]) -> bool {
        debug_assert!(table[0] == 0);
        table[1..256].contains(&0)
    }

    /// `reorder(long)`.
    pub fn reorder(&self, p: i64) -> i64 {
        let table = self.reorder_table.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let b = table[((p as i32 as u32) >> 24) as usize];
        if b != 0 || p <= Collation::NO_CE_PRIMARY {
            (((b as i64) & 0xff) << 24) | (p & 0xffffff)
        } else {
            self.reorder_ex(p)
        }
    }

    fn reorder_ex(&self, p: i64) -> i64 {
        debug_assert!(self.min_high_no_reorder > 0);
        if p >= self.min_high_no_reorder {
            return p;
        }
        // Round up p so that its lower 16 bits are >= any offset bits.
        // Then compare q directly with (limit, offset) pairs.
        let q = p | 0xffff;
        let ranges = self.reorder_ranges.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let mut i = 0;
        let mut r;
        loop {
            r = ranges[i];
            if q >= r {
                i += 1;
            } else {
                break;
            }
        }
        p + (((r as i16) as i64) << 24)
    }

    /// `setStrength(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal strength value.
    pub fn set_strength(&mut self, value: i32) -> Result<()> {
        let no_strength = self.options & !Self::STRENGTH_MASK;
        match value {
            collator::PRIMARY | collator::SECONDARY | collator::TERTIARY | collator::QUATERNARY | collator::IDENTICAL => {
                self.options = no_strength | (value << Self::STRENGTH_SHIFT);
                Ok(())
            }
            _ => Err(LuceneError::illegal_argument(format!("illegal strength value {value}"))),
        }
    }

    /// `setStrengthDefault(int)`.
    pub fn set_strength_default(&mut self, default_options: i32) {
        let no_strength = self.options & !Self::STRENGTH_MASK;
        self.options = no_strength | (default_options & Self::STRENGTH_MASK);
    }

    /// `getStrength(int)` (static).
    pub fn get_strength_of(options: i32) -> i32 {
        options >> Self::STRENGTH_SHIFT
    }

    /// `getStrength()`.
    pub fn get_strength(&self) -> i32 {
        Self::get_strength_of(self.options)
    }

    /// Sets the options bit for an on/off attribute.
    pub fn set_flag(&mut self, bit: i32, value: bool) {
        if value {
            self.options |= bit;
        } else {
            self.options &= !bit;
        }
    }

    /// `setFlagDefault(int, int)`.
    pub fn set_flag_default(&mut self, bit: i32, default_options: i32) {
        self.options = (self.options & !bit) | (default_options & bit);
    }

    /// `getFlag(int)`.
    pub fn get_flag(&self, bit: i32) -> bool {
        (self.options & bit) != 0
    }

    /// `setCaseFirst(int)`.
    pub fn set_case_first(&mut self, value: i32) {
        debug_assert!(value == 0 || value == Self::CASE_FIRST || value == Self::CASE_FIRST_AND_UPPER_MASK);
        let no_case_first = self.options & !Self::CASE_FIRST_AND_UPPER_MASK;
        self.options = no_case_first | value;
    }

    /// `setCaseFirstDefault(int)`.
    pub fn set_case_first_default(&mut self, default_options: i32) {
        let no_case_first = self.options & !Self::CASE_FIRST_AND_UPPER_MASK;
        self.options = no_case_first | (default_options & Self::CASE_FIRST_AND_UPPER_MASK);
    }

    /// `getCaseFirst()`.
    pub fn get_case_first(&self) -> i32 {
        self.options & Self::CASE_FIRST_AND_UPPER_MASK
    }

    /// `setAlternateHandlingShifted(boolean)`.
    pub fn set_alternate_handling_shifted(&mut self, value: bool) {
        let no_alternate = self.options & !Self::ALTERNATE_MASK;
        if value {
            self.options = no_alternate | Self::SHIFTED;
        } else {
            self.options = no_alternate;
        }
    }

    /// `setAlternateHandlingDefault(int)`.
    pub fn set_alternate_handling_default(&mut self, default_options: i32) {
        let no_alternate = self.options & !Self::ALTERNATE_MASK;
        self.options = no_alternate | (default_options & Self::ALTERNATE_MASK);
    }

    /// `getAlternateHandling()`.
    pub fn get_alternate_handling(&self) -> bool {
        (self.options & Self::ALTERNATE_MASK) != 0
    }

    /// `setMaxVariable(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an illegal maxVariable value.
    pub fn set_max_variable(&mut self, value: i32, default_options: i32) -> Result<()> {
        let no_max = self.options & !Self::MAX_VARIABLE_MASK;
        match value {
            Self::MAX_VAR_SPACE | Self::MAX_VAR_PUNCT | Self::MAX_VAR_SYMBOL | Self::MAX_VAR_CURRENCY => {
                self.options = no_max | (value << Self::MAX_VARIABLE_SHIFT);
                Ok(())
            }
            -1 => {
                self.options = no_max | (default_options & Self::MAX_VARIABLE_MASK);
                Ok(())
            }
            _ => Err(LuceneError::illegal_argument(format!("illegal maxVariable value {value}"))),
        }
    }

    /// `getMaxVariable()`.
    pub fn get_max_variable(&self) -> i32 {
        (self.options & Self::MAX_VARIABLE_MASK) >> Self::MAX_VARIABLE_SHIFT
    }

    /// Include case bits in the tertiary level if caseLevel=off and
    /// caseFirst!=off.
    pub fn is_tertiary_with_case_bits(options: i32) -> bool {
        (options & (Self::CASE_LEVEL | Self::CASE_FIRST)) == Self::CASE_FIRST
    }

    /// `getTertiaryMask(int)`.
    pub fn get_tertiary_mask(options: i32) -> i32 {
        // Remove the case bits from the tertiary weight when caseLevel is on or caseFirst is off.
        if Self::is_tertiary_with_case_bits(options) {
            Collation::CASE_AND_TERTIARY_MASK
        } else {
            Collation::ONLY_TERTIARY_MASK
        }
    }

    /// `sortsTertiaryUpperCaseFirst(int)`.
    pub fn sorts_tertiary_upper_case_first(options: i32) -> bool {
        // On tertiary level, consider case bits and sort uppercase first
        // if caseLevel is off and caseFirst==upperFirst.
        (options & (Self::CASE_LEVEL | Self::CASE_FIRST_AND_UPPER_MASK)) == Self::CASE_FIRST_AND_UPPER_MASK
    }

    /// `dontCheckFCD()`.
    pub fn dont_check_fcd(&self) -> bool {
        (self.options & Self::CHECK_FCD) == 0
    }

    /// `hasBackwardSecondary()`.
    pub fn has_backward_secondary(&self) -> bool {
        (self.options & Self::BACKWARD_SECONDARY) != 0
    }

    /// `isNumeric()`.
    pub fn is_numeric(&self) -> bool {
        (self.options & Self::NUMERIC) != 0
    }
}
