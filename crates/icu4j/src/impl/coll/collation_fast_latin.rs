// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationFastLatin`.

use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::lang::u_script::UScript;
use crate::text::collator::{self, ReorderCodes};

/// Fast Latin string comparison for common Latin text.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationFastLatin` (static).
#[derive(Debug)]
pub struct CollationFastLatin;

/// Java's `int >>> 16`.
fn ushr16(x: i32) -> i32 {
    ((x as u32) >> 16) as i32
}

impl CollationFastLatin {
    /// Fast Latin format version (one byte 1..FF).
    pub const VERSION: i32 = 2;
    /// `LATIN_MAX`.
    pub const LATIN_MAX: i32 = 0x17f;
    /// `LATIN_LIMIT`.
    pub const LATIN_LIMIT: i32 = Self::LATIN_MAX + 1;
    /// UTF-8 lead byte of LATIN_MAX.
    pub const LATIN_MAX_UTF8_LEAD: i32 = 0xc5;
    /// `PUNCT_START`.
    pub const PUNCT_START: i32 = 0x2000;
    /// `PUNCT_LIMIT`.
    pub const PUNCT_LIMIT: i32 = 0x2040;
    /// excludes U+FFFE & U+FFFF
    pub const NUM_FAST_CHARS: i32 = Self::LATIN_LIMIT + (Self::PUNCT_LIMIT - Self::PUNCT_START);
    /// bits 15..10
    pub const SHORT_PRIMARY_MASK: i32 = 0xfc00;
    /// bits 9..0 for expansions & contractions
    pub const INDEX_MASK: i32 = 0x3ff;
    /// bits 9..5
    pub const SECONDARY_MASK: i32 = 0x3e0;
    /// bits 4..3
    pub const CASE_MASK: i32 = 0x18;
    /// bits 15..3
    pub const LONG_PRIMARY_MASK: i32 = 0xfff8;
    /// bits 2..0
    pub const TERTIARY_MASK: i32 = 7;
    /// `CASE_AND_TERTIARY_MASK`.
    pub const CASE_AND_TERTIARY_MASK: i32 = Self::CASE_MASK | Self::TERTIARY_MASK;
    /// 0xfc00fc00
    pub const TWO_SHORT_PRIMARIES_MASK: i32 = (Self::SHORT_PRIMARY_MASK << 16) | Self::SHORT_PRIMARY_MASK;
    /// 0xfff8fff8
    pub const TWO_LONG_PRIMARIES_MASK: i32 = (Self::LONG_PRIMARY_MASK << 16) | Self::LONG_PRIMARY_MASK;
    /// 0x3e003e0
    pub const TWO_SECONDARIES_MASK: i32 = (Self::SECONDARY_MASK << 16) | Self::SECONDARY_MASK;
    /// 0x180018
    pub const TWO_CASES_MASK: i32 = (Self::CASE_MASK << 16) | Self::CASE_MASK;
    /// 0x70007
    pub const TWO_TERTIARIES_MASK: i32 = (Self::TERTIARY_MASK << 16) | Self::TERTIARY_MASK;
    /// Contraction with one fast Latin character.
    pub const CONTRACTION: i32 = 0x400;
    /// An expansion encodes two CEs.
    pub const EXPANSION: i32 = 0x800;
    /// Smallest long mini primary.
    pub const MIN_LONG: i32 = 0xc00;
    /// `LONG_INC`.
    pub const LONG_INC: i32 = 8;
    /// `MAX_LONG`.
    pub const MAX_LONG: i32 = 0xff8;
    /// Smallest short mini primary.
    pub const MIN_SHORT: i32 = 0x1000;
    /// `SHORT_INC`.
    pub const SHORT_INC: i32 = 0x400;
    /// `MAX_SHORT`.
    pub const MAX_SHORT: i32 = Self::SHORT_PRIMARY_MASK;
    /// must add SEC_OFFSET
    pub const MIN_SEC_BEFORE: i32 = 0;
    /// `SEC_INC`.
    pub const SEC_INC: i32 = 0x20;
    /// 5 before common
    pub const MAX_SEC_BEFORE: i32 = Self::MIN_SEC_BEFORE + 4 * Self::SEC_INC;
    /// `COMMON_SEC`.
    pub const COMMON_SEC: i32 = Self::MAX_SEC_BEFORE + Self::SEC_INC;
    /// `MIN_SEC_AFTER`.
    pub const MIN_SEC_AFTER: i32 = Self::COMMON_SEC + Self::SEC_INC;
    /// 6 after common
    pub const MAX_SEC_AFTER: i32 = Self::MIN_SEC_AFTER + 5 * Self::SEC_INC;
    /// 20 high secondaries
    pub const MIN_SEC_HIGH: i32 = Self::MAX_SEC_AFTER + Self::SEC_INC;
    /// `MAX_SEC_HIGH`.
    pub const MAX_SEC_HIGH: i32 = Self::SECONDARY_MASK;
    /// `SEC_OFFSET`.
    pub const SEC_OFFSET: i32 = Self::SEC_INC;
    /// `COMMON_SEC_PLUS_OFFSET`.
    pub const COMMON_SEC_PLUS_OFFSET: i32 = Self::COMMON_SEC + Self::SEC_OFFSET;
    /// 0x200020
    pub const TWO_SEC_OFFSETS: i32 = (Self::SEC_OFFSET << 16) | Self::SEC_OFFSET;
    /// `TWO_COMMON_SEC_PLUS_OFFSET`.
    pub const TWO_COMMON_SEC_PLUS_OFFSET: i32 = (Self::COMMON_SEC_PLUS_OFFSET << 16) | Self::COMMON_SEC_PLUS_OFFSET;
    /// case bits include this offset
    pub const LOWER_CASE: i32 = 8;
    /// 0x80008
    pub const TWO_LOWER_CASES: i32 = (Self::LOWER_CASE << 16) | Self::LOWER_CASE;
    /// must add TER_OFFSET
    pub const COMMON_TER: i32 = 0;
    /// 7 after common
    pub const MAX_TER_AFTER: i32 = 7;
    /// `TER_OFFSET`.
    pub const TER_OFFSET: i32 = Self::SEC_OFFSET;
    /// `COMMON_TER_PLUS_OFFSET`.
    pub const COMMON_TER_PLUS_OFFSET: i32 = Self::COMMON_TER + Self::TER_OFFSET;
    /// `TWO_TER_OFFSETS`.
    pub const TWO_TER_OFFSETS: i32 = (Self::TER_OFFSET << 16) | Self::TER_OFFSET;
    /// `TWO_COMMON_TER_PLUS_OFFSET`.
    pub const TWO_COMMON_TER_PLUS_OFFSET: i32 = (Self::COMMON_TER_PLUS_OFFSET << 16) | Self::COMMON_TER_PLUS_OFFSET;
    /// `MERGE_WEIGHT`.
    pub const MERGE_WEIGHT: i32 = 3;
    /// end of string
    pub const EOS: i32 = 2;
    /// `BAIL_OUT`.
    pub const BAIL_OUT: i32 = 1;
    /// Contraction result first word bits 8..0 contain the second contraction character.
    pub const CONTR_CHAR_MASK: i32 = 0x1ff;
    /// Contraction result first word bits 15..9 contain the result length.
    pub const CONTR_LENGTH_SHIFT: i32 = 9;
    /// Comparison return value when the regular comparison must be used.
    pub const BAIL_OUT_RESULT: i32 = -2;

    /// `getCharIndex(char)`.
    pub fn get_char_index(c: u16) -> i32 {
        let c = c as i32;
        if c <= Self::LATIN_MAX {
            c
        } else if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
            c - (Self::PUNCT_START - Self::LATIN_LIMIT)
        } else {
            // Not a fast Latin character.
            // Note: U+FFFE & U+FFFF are forbidden in tailorings
            // and thus do not occur in any contractions.
            -1
        }
    }

    /// Computes the options value for the compare functions and writes the
    /// precomputed primary weights.
    ///
    /// Equivalent to `getOptions(CollationData, CollationSettings, char[])`.
    pub fn get_options(data: &CollationData, settings: &CollationSettings, primaries: &mut [u16]) -> i32 {
        let Some(header) = data.fast_latin_table_header.as_deref() else {
            return -1;
        };
        debug_assert!((header[0] >> 8) as i32 == Self::VERSION);
        if primaries.len() as i32 != Self::LATIN_LIMIT {
            debug_assert!(false);
            return -1;
        }

        let mini_var_top;
        if (settings.options & CollationSettings::ALTERNATE_MASK) == 0 {
            // No mini primaries are variable, set a variableTop just below the
            // lowest long mini primary.
            mini_var_top = Self::MIN_LONG - 1;
        } else {
            let header_length = (header[0] & 0xff) as i32;
            let i = 1 + settings.get_max_variable();
            if i >= header_length {
                return -1; // variableTop >= digits, should not occur
            }
            mini_var_top = header[i as usize] as i32;
        }

        let mut digits_are_reordered = false;
        if settings.has_reordering() {
            let mut prev_start: i64 = 0;
            let mut before_digit_start: i64 = 0;
            let mut digit_start: i64 = 0;
            let mut after_digit_start: i64 = 0;
            for group in ReorderCodes::FIRST..ReorderCodes::FIRST + CollationData::MAX_NUM_SPECIAL_REORDER_CODES {
                let mut start = data.get_first_primary_for_group(group);
                start = settings.reorder(start);
                if group == ReorderCodes::DIGIT {
                    before_digit_start = prev_start;
                    digit_start = start;
                } else if start != 0 {
                    if start < prev_start {
                        // The permutation affects the groups up to Latin.
                        return -1;
                    }
                    // In the future, there might be a special group between digits & Latin.
                    if digit_start != 0 && after_digit_start == 0 && prev_start == before_digit_start {
                        after_digit_start = start;
                    }
                    prev_start = start;
                }
            }
            let mut latin_start = data.get_first_primary_for_group(UScript::LATIN);
            latin_start = settings.reorder(latin_start);
            if latin_start < prev_start {
                return -1;
            }
            if after_digit_start == 0 {
                after_digit_start = latin_start;
            }
            if !(before_digit_start < digit_start && digit_start < after_digit_start) {
                digits_are_reordered = true;
            }
        }

        let table = data.fast_latin_table.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException")); // skip the header
        for c in 0..Self::LATIN_LIMIT as usize {
            let mut p = table[c] as i32;
            if p >= Self::MIN_SHORT {
                p &= Self::SHORT_PRIMARY_MASK;
            } else if p > mini_var_top {
                p &= Self::LONG_PRIMARY_MASK;
            } else {
                p = 0;
            }
            primaries[c] = p as u16;
        }
        if digits_are_reordered || (settings.options & CollationSettings::NUMERIC) != 0 {
            // Bail out for digits.
            for p in &mut primaries[0x30..=0x39] {
                *p = 0;
            }
        }

        // Shift the miniVarTop above other options.
        (mini_var_top << 16) | settings.options
    }

    /// `compareUTF16(char[], char[], int, CharSequence, CharSequence, int)`.
    pub fn compare_utf16(table: &[u16], primaries: &[u16], mut options: i32, left: &[u16], right: &[u16], start_index: i32) -> i32 {
        // This is a modified copy of CollationCompare.compareUpToQuaternary(),
        // optimized for common Latin text.
        // Keep them in sync!

        let variable_top = options >> 16; // see getOptions()
        options &= 0xffff; // needed for CollationSettings.getStrength() to work

        // Check for supported characters, fetch mini CEs, and compare primaries.
        let mut left_index = start_index;
        let mut right_index = start_index;
        let mut left_pair: i32 = 0;
        let mut right_pair: i32 = 0;
        let t = |c: i32| table[c as usize] as i32;
        loop {
            // We fetch CEs until we get a non-ignorable primary or reach the end.
            while left_pair == 0 {
                if left_index == left.len() as i32 {
                    left_pair = Self::EOS;
                    break;
                }
                let c = left[left_index as usize] as i32;
                left_index += 1;
                if c <= Self::LATIN_MAX {
                    left_pair = primaries[c as usize] as i32;
                    if left_pair != 0 {
                        break;
                    }
                    if c <= 0x39 && c >= 0x30 && (options & CollationSettings::NUMERIC) != 0 {
                        return Self::BAIL_OUT_RESULT;
                    }
                    left_pair = t(c);
                } else if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
                    left_pair = t(c - Self::PUNCT_START + Self::LATIN_LIMIT);
                } else {
                    left_pair = Self::lookup(table, c);
                }
                if left_pair >= Self::MIN_SHORT {
                    left_pair &= Self::SHORT_PRIMARY_MASK;
                    break;
                } else if left_pair > variable_top {
                    left_pair &= Self::LONG_PRIMARY_MASK;
                    break;
                } else {
                    let mut pair_and_inc = Self::next_pair(table, c, left_pair, left, left_index);
                    if pair_and_inc < 0 {
                        left_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    left_pair = pair_and_inc as i32;
                    if left_pair == Self::BAIL_OUT {
                        return Self::BAIL_OUT_RESULT;
                    }
                    left_pair = Self::get_primaries(variable_top, left_pair);
                }
            }
            while right_pair == 0 {
                if right_index == right.len() as i32 {
                    right_pair = Self::EOS;
                    break;
                }
                let c = right[right_index as usize] as i32;
                right_index += 1;
                if c <= Self::LATIN_MAX {
                    right_pair = primaries[c as usize] as i32;
                    if right_pair != 0 {
                        break;
                    }
                    if c <= 0x39 && c >= 0x30 && (options & CollationSettings::NUMERIC) != 0 {
                        return Self::BAIL_OUT_RESULT;
                    }
                    right_pair = t(c);
                } else if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
                    right_pair = t(c - Self::PUNCT_START + Self::LATIN_LIMIT);
                } else {
                    right_pair = Self::lookup(table, c);
                }
                if right_pair >= Self::MIN_SHORT {
                    right_pair &= Self::SHORT_PRIMARY_MASK;
                    break;
                } else if right_pair > variable_top {
                    right_pair &= Self::LONG_PRIMARY_MASK;
                    break;
                } else {
                    let mut pair_and_inc = Self::next_pair(table, c, right_pair, right, right_index);
                    if pair_and_inc < 0 {
                        right_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    right_pair = pair_and_inc as i32;
                    if right_pair == Self::BAIL_OUT {
                        return Self::BAIL_OUT_RESULT;
                    }
                    right_pair = Self::get_primaries(variable_top, right_pair);
                }
            }
            if left_pair == right_pair {
                if left_pair == Self::EOS {
                    break;
                }
                left_pair = 0;
                right_pair = 0;
                continue;
            }
            let left_primary = left_pair & 0xffff;
            let right_primary = right_pair & 0xffff;
            if left_primary != right_primary {
                // Return the primary difference.
                return if left_primary < right_primary { Collation::LESS } else { Collation::GREATER };
            }
            if left_pair == Self::EOS {
                break;
            }
            left_pair = ushr16(left_pair);
            right_pair = ushr16(right_pair);
        }
        // In the following, we need to re-fetch each character because we did not buffer the CEs,
        // but we know that the string is well-formed and
        // only contains supported characters and mappings.

        // We might skip the secondary level but continue with the case level
        // which is turned on separately.
        if CollationSettings::get_strength_of(options) >= collator::SECONDARY {
            left_index = start_index;
            right_index = start_index;
            left_pair = 0;
            right_pair = 0;
            loop {
                while left_pair == 0 {
                    if left_index == left.len() as i32 {
                        left_pair = Self::EOS;
                        break;
                    }
                    let c = left[left_index as usize] as i32;
                    left_index += 1;
                    if c <= Self::LATIN_MAX {
                        left_pair = t(c);
                    } else if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
                        left_pair = t(c - Self::PUNCT_START + Self::LATIN_LIMIT);
                    } else {
                        left_pair = Self::lookup(table, c);
                    }
                    if left_pair >= Self::MIN_SHORT {
                        left_pair = Self::get_secondaries_from_one_short_ce(left_pair);
                        break;
                    } else if left_pair > variable_top {
                        left_pair = Self::COMMON_SEC_PLUS_OFFSET;
                        break;
                    } else {
                        let mut pair_and_inc = Self::next_pair(table, c, left_pair, left, left_index);
                        if pair_and_inc < 0 {
                            left_index += 1;
                            pair_and_inc = !pair_and_inc;
                        }
                        left_pair = Self::get_secondaries(variable_top, pair_and_inc as i32);
                    }
                }
                while right_pair == 0 {
                    if right_index == right.len() as i32 {
                        right_pair = Self::EOS;
                        break;
                    }
                    let c = right[right_index as usize] as i32;
                    right_index += 1;
                    if c <= Self::LATIN_MAX {
                        right_pair = t(c);
                    } else if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
                        right_pair = t(c - Self::PUNCT_START + Self::LATIN_LIMIT);
                    } else {
                        right_pair = Self::lookup(table, c);
                    }
                    if right_pair >= Self::MIN_SHORT {
                        right_pair = Self::get_secondaries_from_one_short_ce(right_pair);
                        break;
                    } else if right_pair > variable_top {
                        right_pair = Self::COMMON_SEC_PLUS_OFFSET;
                        break;
                    } else {
                        let mut pair_and_inc = Self::next_pair(table, c, right_pair, right, right_index);
                        if pair_and_inc < 0 {
                            right_index += 1;
                            pair_and_inc = !pair_and_inc;
                        }
                        right_pair = Self::get_secondaries(variable_top, pair_and_inc as i32);
                    }
                }
                if left_pair == right_pair {
                    if left_pair == Self::EOS {
                        break;
                    }
                    left_pair = 0;
                    right_pair = 0;
                    continue;
                }
                let left_secondary = left_pair & 0xffff;
                let right_secondary = right_pair & 0xffff;
                if left_secondary != right_secondary {
                    if (options & CollationSettings::BACKWARD_SECONDARY) != 0 {
                        // Full support for backwards secondary requires backwards contraction matching
                        // and moving backwards between merge separators.
                        return Self::BAIL_OUT_RESULT;
                    }
                    return if left_secondary < right_secondary { Collation::LESS } else { Collation::GREATER };
                }
                if left_pair == Self::EOS {
                    break;
                }
                left_pair = ushr16(left_pair);
                right_pair = ushr16(right_pair);
            }
        }

        if (options & CollationSettings::CASE_LEVEL) != 0 {
            let strength_is_primary = CollationSettings::get_strength_of(options) == collator::PRIMARY;
            left_index = start_index;
            right_index = start_index;
            left_pair = 0;
            right_pair = 0;
            loop {
                while left_pair == 0 {
                    if left_index == left.len() as i32 {
                        left_pair = Self::EOS;
                        break;
                    }
                    let c = left[left_index as usize] as i32;
                    left_index += 1;
                    left_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                    if left_pair < Self::MIN_LONG {
                        let mut pair_and_inc = Self::next_pair(table, c, left_pair, left, left_index);
                        if pair_and_inc < 0 {
                            left_index += 1;
                            pair_and_inc = !pair_and_inc;
                        }
                        left_pair = pair_and_inc as i32;
                    }
                    left_pair = Self::get_cases(variable_top, strength_is_primary, left_pair);
                }
                while right_pair == 0 {
                    if right_index == right.len() as i32 {
                        right_pair = Self::EOS;
                        break;
                    }
                    let c = right[right_index as usize] as i32;
                    right_index += 1;
                    right_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                    if right_pair < Self::MIN_LONG {
                        let mut pair_and_inc = Self::next_pair(table, c, right_pair, right, right_index);
                        if pair_and_inc < 0 {
                            right_index += 1;
                            pair_and_inc = !pair_and_inc;
                        }
                        right_pair = pair_and_inc as i32;
                    }
                    right_pair = Self::get_cases(variable_top, strength_is_primary, right_pair);
                }
                if left_pair == right_pair {
                    if left_pair == Self::EOS {
                        break;
                    }
                    left_pair = 0;
                    right_pair = 0;
                    continue;
                }
                let left_case = left_pair & 0xffff;
                let right_case = right_pair & 0xffff;
                if left_case != right_case {
                    if (options & CollationSettings::UPPER_FIRST) == 0 {
                        return if left_case < right_case { Collation::LESS } else { Collation::GREATER };
                    } else {
                        return if left_case < right_case { Collation::GREATER } else { Collation::LESS };
                    }
                }
                if left_pair == Self::EOS {
                    break;
                }
                left_pair = ushr16(left_pair);
                right_pair = ushr16(right_pair);
            }
        }
        if CollationSettings::get_strength_of(options) <= collator::SECONDARY {
            return Collation::EQUAL;
        }

        // Remove the case bits from the tertiary weight when caseLevel is on or caseFirst is off.
        let with_case_bits = CollationSettings::is_tertiary_with_case_bits(options);

        left_index = start_index;
        right_index = start_index;
        left_pair = 0;
        right_pair = 0;
        loop {
            while left_pair == 0 {
                if left_index == left.len() as i32 {
                    left_pair = Self::EOS;
                    break;
                }
                let c = left[left_index as usize] as i32;
                left_index += 1;
                left_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                if left_pair < Self::MIN_LONG {
                    let mut pair_and_inc = Self::next_pair(table, c, left_pair, left, left_index);
                    if pair_and_inc < 0 {
                        left_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    left_pair = pair_and_inc as i32;
                }
                left_pair = Self::get_tertiaries(variable_top, with_case_bits, left_pair);
            }
            while right_pair == 0 {
                if right_index == right.len() as i32 {
                    right_pair = Self::EOS;
                    break;
                }
                let c = right[right_index as usize] as i32;
                right_index += 1;
                right_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                if right_pair < Self::MIN_LONG {
                    let mut pair_and_inc = Self::next_pair(table, c, right_pair, right, right_index);
                    if pair_and_inc < 0 {
                        right_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    right_pair = pair_and_inc as i32;
                }
                right_pair = Self::get_tertiaries(variable_top, with_case_bits, right_pair);
            }
            if left_pair == right_pair {
                if left_pair == Self::EOS {
                    break;
                }
                left_pair = 0;
                right_pair = 0;
                continue;
            }
            let mut left_tertiary = left_pair & 0xffff;
            let mut right_tertiary = right_pair & 0xffff;
            if left_tertiary != right_tertiary {
                if CollationSettings::sorts_tertiary_upper_case_first(options) {
                    // Pass through EOS and MERGE_WEIGHT
                    // and keep real tertiary weights larger than the MERGE_WEIGHT.
                    // Tertiary CEs (secondary ignorables) are not supported in fast Latin.
                    if left_tertiary > Self::MERGE_WEIGHT {
                        left_tertiary ^= Self::CASE_MASK;
                    }
                    if right_tertiary > Self::MERGE_WEIGHT {
                        right_tertiary ^= Self::CASE_MASK;
                    }
                }
                return if left_tertiary < right_tertiary { Collation::LESS } else { Collation::GREATER };
            }
            if left_pair == Self::EOS {
                break;
            }
            left_pair = ushr16(left_pair);
            right_pair = ushr16(right_pair);
        }
        if CollationSettings::get_strength_of(options) <= collator::TERTIARY {
            return Collation::EQUAL;
        }

        left_index = start_index;
        right_index = start_index;
        left_pair = 0;
        right_pair = 0;
        loop {
            while left_pair == 0 {
                if left_index == left.len() as i32 {
                    left_pair = Self::EOS;
                    break;
                }
                let c = left[left_index as usize] as i32;
                left_index += 1;
                left_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                if left_pair < Self::MIN_LONG {
                    let mut pair_and_inc = Self::next_pair(table, c, left_pair, left, left_index);
                    if pair_and_inc < 0 {
                        left_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    left_pair = pair_and_inc as i32;
                }
                left_pair = Self::get_quaternaries(variable_top, left_pair);
            }
            while right_pair == 0 {
                if right_index == right.len() as i32 {
                    right_pair = Self::EOS;
                    break;
                }
                let c = right[right_index as usize] as i32;
                right_index += 1;
                right_pair = if c <= Self::LATIN_MAX { t(c) } else { Self::lookup(table, c) };
                if right_pair < Self::MIN_LONG {
                    let mut pair_and_inc = Self::next_pair(table, c, right_pair, right, right_index);
                    if pair_and_inc < 0 {
                        right_index += 1;
                        pair_and_inc = !pair_and_inc;
                    }
                    right_pair = pair_and_inc as i32;
                }
                right_pair = Self::get_quaternaries(variable_top, right_pair);
            }
            if left_pair == right_pair {
                if left_pair == Self::EOS {
                    break;
                }
                left_pair = 0;
                right_pair = 0;
                continue;
            }
            let left_quaternary = left_pair & 0xffff;
            let right_quaternary = right_pair & 0xffff;
            if left_quaternary != right_quaternary {
                return if left_quaternary < right_quaternary { Collation::LESS } else { Collation::GREATER };
            }
            if left_pair == Self::EOS {
                break;
            }
            left_pair = ushr16(left_pair);
            right_pair = ushr16(right_pair);
        }
        Collation::EQUAL
    }

    fn lookup(table: &[u16], c: i32) -> i32 {
        debug_assert!(c > Self::LATIN_MAX);
        if Self::PUNCT_START <= c && c < Self::PUNCT_LIMIT {
            table[(c - Self::PUNCT_START + Self::LATIN_LIMIT) as usize] as i32
        } else if c == 0xfffe {
            Self::MERGE_WEIGHT
        } else if c == 0xffff {
            Self::MAX_SHORT | Self::COMMON_SEC | Self::LOWER_CASE | Self::COMMON_TER
        } else {
            Self::BAIL_OUT
        }
    }

    /// Java returns a negative value (`~result`) when the next character is
    /// consumed by a contraction.
    fn next_pair(table: &[u16], _c: i32, ce: i32, s16: &[u16], s_index: i32) -> i64 {
        let t = |i: i32| table[i as usize] as i32;
        if ce >= Self::MIN_LONG || ce < Self::CONTRACTION {
            ce as i64 // simple or special mini CE
        } else if ce >= Self::EXPANSION {
            let index = Self::NUM_FAST_CHARS + (ce & Self::INDEX_MASK);
            ((t(index + 1) as i64) << 16) | t(index) as i64
        } else {
            // Contraction list: Default mapping followed by
            // 0 or more single-character contraction suffix mappings.
            let mut index = Self::NUM_FAST_CHARS + (ce & Self::INDEX_MASK);
            let mut inc = false; // true if the next char is consumed.
            if s_index != s16.len() as i32 {
                // Read the next character.
                let next_index = s_index;
                let mut c2 = s16[next_index as usize] as i32;
                if c2 > Self::LATIN_MAX {
                    if Self::PUNCT_START <= c2 && c2 < Self::PUNCT_LIMIT {
                        c2 = c2 - Self::PUNCT_START + Self::LATIN_LIMIT; // 2000..203F -> 0180..01BF
                    } else if c2 == 0xfffe || c2 == 0xffff {
                        c2 = -1; // U+FFFE & U+FFFF cannot occur in contractions.
                    } else {
                        return Self::BAIL_OUT as i64;
                    }
                }
                // Look for the next character in the contraction suffix list,
                // which is in ascending order of single suffix characters.
                let mut i = index;
                let mut head = t(i); // first skip the default mapping
                let mut x;
                loop {
                    i += head >> Self::CONTR_LENGTH_SHIFT;
                    head = t(i);
                    x = head & Self::CONTR_CHAR_MASK;
                    if x >= c2 {
                        break;
                    }
                }
                if x == c2 {
                    index = i;
                    inc = true;
                }
            }
            // Return the CE or CEs for the default or contraction mapping.
            let length = t(index) >> Self::CONTR_LENGTH_SHIFT;
            if length == 1 {
                return Self::BAIL_OUT as i64;
            }
            let ce = t(index + 1);
            let result: i64 = if length == 2 { ce as i64 } else { ((t(index + 2) as i64) << 16) | ce as i64 };
            if inc {
                !result
            } else {
                result
            }
        }
    }

    fn get_primaries(variable_top: i32, pair: i32) -> i32 {
        let ce = pair & 0xffff;
        if ce >= Self::MIN_SHORT {
            return pair & Self::TWO_SHORT_PRIMARIES_MASK;
        }
        if ce > variable_top {
            return pair & Self::TWO_LONG_PRIMARIES_MASK;
        }
        if ce >= Self::MIN_LONG {
            return 0;
        } // variable
        pair // special mini CE
    }

    fn get_secondaries_from_one_short_ce(mut ce: i32) -> i32 {
        ce &= Self::SECONDARY_MASK;
        if ce < Self::MIN_SEC_HIGH {
            ce + Self::SEC_OFFSET
        } else {
            ((ce + Self::SEC_OFFSET) << 16) | Self::COMMON_SEC_PLUS_OFFSET
        }
    }

    fn get_secondaries(variable_top: i32, mut pair: i32) -> i32 {
        if pair <= 0xffff {
            // one mini CE
            if pair >= Self::MIN_SHORT {
                pair = Self::get_secondaries_from_one_short_ce(pair);
            } else if pair > variable_top {
                pair = Self::COMMON_SEC_PLUS_OFFSET;
            } else if pair >= Self::MIN_LONG {
                pair = 0; // variable
            }
            // else special mini CE
        } else {
            let ce = pair & 0xffff;
            if ce >= Self::MIN_SHORT {
                pair = (pair & Self::TWO_SECONDARIES_MASK) + Self::TWO_SEC_OFFSETS;
            } else if ce > variable_top {
                pair = Self::TWO_COMMON_SEC_PLUS_OFFSET;
            } else {
                debug_assert!(ce >= Self::MIN_LONG);
                pair = 0; // variable
            }
        }
        pair
    }

    fn get_cases(variable_top: i32, strength_is_primary: bool, mut pair: i32) -> i32 {
        // Primary+caseLevel: Ignore case level weights of primary ignorables.
        // Otherwise: Ignore case level weights of secondary ignorables.
        // For details see the comments in the CollationCompare class.
        // Tertiary CEs (secondary ignorables) are not supported in fast Latin.
        if pair <= 0xffff {
            // one mini CE
            if pair >= Self::MIN_SHORT {
                // A high secondary weight means we really have two CEs,
                // a primary CE and a secondary CE.
                let ce = pair;
                pair &= Self::CASE_MASK; // explicit weight of primary CE
                if !strength_is_primary && (ce & Self::SECONDARY_MASK) >= Self::MIN_SEC_HIGH {
                    pair |= Self::LOWER_CASE << 16; // implied weight of secondary CE
                }
            } else if pair > variable_top {
                pair = Self::LOWER_CASE;
            } else if pair >= Self::MIN_LONG {
                pair = 0; // variable
            }
            // else special mini CE
        } else {
            // two mini CEs, same primary groups, neither expands like above
            let ce = pair & 0xffff;
            if ce >= Self::MIN_SHORT {
                if strength_is_primary && (pair & (Self::SHORT_PRIMARY_MASK << 16)) == 0 {
                    pair &= Self::CASE_MASK;
                } else {
                    pair &= Self::TWO_CASES_MASK;
                }
            } else if ce > variable_top {
                pair = Self::TWO_LOWER_CASES;
            } else {
                debug_assert!(ce >= Self::MIN_LONG);
                pair = 0; // variable
            }
        }
        pair
    }

    fn get_tertiaries(variable_top: i32, with_case_bits: bool, mut pair: i32) -> i32 {
        if pair <= 0xffff {
            // one mini CE
            if pair >= Self::MIN_SHORT {
                // A high secondary weight means we really have two CEs,
                // a primary CE and a secondary CE.
                let ce = pair;
                if with_case_bits {
                    pair = (pair & Self::CASE_AND_TERTIARY_MASK) + Self::TER_OFFSET;
                    if (ce & Self::SECONDARY_MASK) >= Self::MIN_SEC_HIGH {
                        pair |= (Self::LOWER_CASE | Self::COMMON_TER_PLUS_OFFSET) << 16;
                    }
                } else {
                    pair = (pair & Self::TERTIARY_MASK) + Self::TER_OFFSET;
                    if (ce & Self::SECONDARY_MASK) >= Self::MIN_SEC_HIGH {
                        pair |= Self::COMMON_TER_PLUS_OFFSET << 16;
                    }
                }
            } else if pair > variable_top {
                pair = (pair & Self::TERTIARY_MASK) + Self::TER_OFFSET;
                if with_case_bits {
                    pair |= Self::LOWER_CASE;
                }
            } else if pair >= Self::MIN_LONG {
                pair = 0; // variable
            }
            // else special mini CE
        } else {
            // two mini CEs, same primary groups, neither expands like above
            let ce = pair & 0xffff;
            if ce >= Self::MIN_SHORT {
                if with_case_bits {
                    pair &= Self::TWO_CASES_MASK | Self::TWO_TERTIARIES_MASK;
                } else {
                    pair &= Self::TWO_TERTIARIES_MASK;
                }
                pair += Self::TWO_TER_OFFSETS;
            } else if ce > variable_top {
                pair = (pair & Self::TWO_TERTIARIES_MASK) + Self::TWO_TER_OFFSETS;
                if with_case_bits {
                    pair |= Self::TWO_LOWER_CASES;
                }
            } else {
                debug_assert!(ce >= Self::MIN_LONG);
                pair = 0; // variable
            }
        }
        pair
    }

    fn get_quaternaries(variable_top: i32, mut pair: i32) -> i32 {
        // Return the primary weight of a variable CE,
        // or the maximum primary weight for a non-variable, not-completely-ignorable CE.
        if pair <= 0xffff {
            // one mini CE
            if pair >= Self::MIN_SHORT {
                // A high secondary weight means we really have two CEs,
                // a primary CE and a secondary CE.
                if (pair & Self::SECONDARY_MASK) >= Self::MIN_SEC_HIGH {
                    pair = Self::TWO_SHORT_PRIMARIES_MASK;
                } else {
                    pair = Self::SHORT_PRIMARY_MASK;
                }
            } else if pair > variable_top {
                pair = Self::SHORT_PRIMARY_MASK;
            } else if pair >= Self::MIN_LONG {
                pair &= Self::LONG_PRIMARY_MASK; // variable
            }
            // else special mini CE
        } else {
            // two mini CEs, same primary groups, neither expands like above
            let ce = pair & 0xffff;
            if ce > variable_top {
                pair = Self::TWO_SHORT_PRIMARIES_MASK;
            } else {
                debug_assert!(ce >= Self::MIN_LONG);
                pair &= Self::TWO_LONG_PRIMARIES_MASK; // variable
            }
        }
        pair
    }
}
