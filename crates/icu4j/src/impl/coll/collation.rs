// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.Collation`.

/// Collation v2 basic definitions and static helper functions.
///
/// Equivalent to `com.ibm.icu.impl.coll.Collation`: the constants and
/// static helpers are associated items of this unit struct. Java `int`
/// values are `i32`, `long` values `i64`, with the same bit patterns.
#[derive(Debug)]
pub struct Collation;

/// A Java `int` hexadecimal literal with the high bit set.
const fn h(v: u32) -> i32 {
    v as i32
}

impl Collation {
    /// Indicates the end of input when a code point is expected.
    pub const SENTINEL_CP: i32 = -1;

    /// `LESS` (ICU4C `UCOL_LESS`).
    pub const LESS: i32 = -1;
    /// `EQUAL`.
    pub const EQUAL: i32 = 0;
    /// `GREATER`.
    pub const GREATER: i32 = 1;

    /// Merge-sort-key separator / terminator byte.
    pub const TERMINATOR_BYTE: i32 = 0;
    /// Level separator byte.
    pub const LEVEL_SEPARATOR_BYTE: i32 = 1;
    /// The secondary/tertiary lower limit for tailoring before any root elements.
    pub const BEFORE_WEIGHT16: i32 = 0x100;
    /// Merge-sort-key separator.
    pub const MERGE_SEPARATOR_BYTE: i32 = 2;
    /// `MERGE_SEPARATOR_PRIMARY` (U+FFFE).
    pub const MERGE_SEPARATOR_PRIMARY: i64 = 0x02000000;
    /// `MERGE_SEPARATOR_CE32` (U+FFFE).
    pub const MERGE_SEPARATOR_CE32: i32 = 0x02000505;
    /// Primary compression low terminator.
    pub const PRIMARY_COMPRESSION_LOW_BYTE: i32 = 3;
    /// Primary compression high terminator.
    pub const PRIMARY_COMPRESSION_HIGH_BYTE: i32 = 0xff;
    /// Default secondary/tertiary weight lead byte.
    pub const COMMON_BYTE: i32 = 5;
    /// `COMMON_WEIGHT16`.
    pub const COMMON_WEIGHT16: i32 = 0x0500;
    /// Middle 16 bits of a CE with a common secondary weight.
    pub const COMMON_SECONDARY_CE: i32 = 0x05000000;
    /// Lower 16 bits of a CE with a common tertiary weight.
    pub const COMMON_TERTIARY_CE: i32 = 0x0500;
    /// Lower 32 bits of a CE with common secondary and tertiary weights.
    pub const COMMON_SEC_AND_TER_CE: i32 = 0x05000500;
    /// `SECONDARY_MASK`.
    pub const SECONDARY_MASK: i32 = h(0xffff0000);
    /// `CASE_MASK`.
    pub const CASE_MASK: i32 = 0xc000;
    /// `SECONDARY_AND_CASE_MASK`.
    pub const SECONDARY_AND_CASE_MASK: i32 = Self::SECONDARY_MASK | Self::CASE_MASK;
    /// Only the 2*6 bits for the pure tertiary weight.
    pub const ONLY_TERTIARY_MASK: i32 = 0x3f3f;
    /// Only the secondary & tertiary bits; no case, no quaternary.
    pub const ONLY_SEC_TER_MASK: i32 = Self::SECONDARY_MASK | Self::ONLY_TERTIARY_MASK;
    /// Case bits and tertiary bits.
    pub const CASE_AND_TERTIARY_MASK: i32 = Self::CASE_MASK | Self::ONLY_TERTIARY_MASK;
    /// `QUATERNARY_MASK`.
    pub const QUATERNARY_MASK: i32 = 0xc0;
    /// Case bits and quaternary bits.
    pub const CASE_AND_QUATERNARY_MASK: i32 = Self::CASE_MASK | Self::QUATERNARY_MASK;

    /// `UNASSIGNED_IMPLICIT_BYTE` (compressible).
    pub const UNASSIGNED_IMPLICIT_BYTE: i32 = 0xfe;
    /// First unassigned: AlphabeticIndex overflow boundary.
    pub const FIRST_UNASSIGNED_PRIMARY: i64 = 0xfe040200;
    /// `TRAIL_WEIGHT_BYTE` (not compressible).
    pub const TRAIL_WEIGHT_BYTE: i32 = 0xff;
    /// `FIRST_TRAILING_PRIMARY` (`[first trailing]`).
    pub const FIRST_TRAILING_PRIMARY: i64 = 0xff020200;
    /// `MAX_PRIMARY` (U+FFFF).
    pub const MAX_PRIMARY: i64 = 0xffff0000;
    /// `MAX_REGULAR_CE32` (U+FFFF).
    pub const MAX_REGULAR_CE32: i32 = h(0xffff0505);

    /// Primary of U+FFFD and illegal UTF-8 byte sequences.
    pub const FFFD_PRIMARY: i64 = Self::MAX_PRIMARY - 0x20000;
    /// CE32 of U+FFFD.
    pub const FFFD_CE32: i32 = Self::MAX_REGULAR_CE32 - 0x20000;

    /// A CE32 is special if its low byte is this or greater.
    pub const SPECIAL_CE32_LOW_BYTE: i32 = 0xc0;
    /// `FALLBACK_CE32`.
    pub const FALLBACK_CE32: i32 = Self::SPECIAL_CE32_LOW_BYTE;
    /// Low byte of a long-primary special CE32.
    pub const LONG_PRIMARY_CE32_LOW_BYTE: i32 = 0xc1;

    /// `UNASSIGNED_CE32`: compute an unassigned-implicit CE.
    pub const UNASSIGNED_CE32: i32 = h(0xffffffff);

    /// Value for "no CE32" ("no expansion").
    pub const NO_CE32: i32 = 1;

    /// `NO_CE_PRIMARY` (not a left-adjusted weight).
    pub const NO_CE_PRIMARY: i64 = 1;
    /// `NO_CE_WEIGHT16` (weight of `LEVEL_SEPARATOR_BYTE`).
    pub const NO_CE_WEIGHT16: i32 = 0x0100;
    /// `NO_CE`.
    pub const NO_CE: i64 = 0x101000100;

    /// Sort key levels.
    pub const NO_LEVEL: i32 = 0;
    /// `PRIMARY_LEVEL`.
    pub const PRIMARY_LEVEL: i32 = 1;
    /// `SECONDARY_LEVEL`.
    pub const SECONDARY_LEVEL: i32 = 2;
    /// `CASE_LEVEL`.
    pub const CASE_LEVEL: i32 = 3;
    /// `TERTIARY_LEVEL`.
    pub const TERTIARY_LEVEL: i32 = 4;
    /// `QUATERNARY_LEVEL`.
    pub const QUATERNARY_LEVEL: i32 = 5;
    /// `IDENTICAL_LEVEL`.
    pub const IDENTICAL_LEVEL: i32 = 6;
    /// Beyond sort key bytes.
    pub const ZERO_LEVEL: i32 = 7;

    /// Sort key level flags: `xx_FLAG = 1 << xx_LEVEL`.
    pub const NO_LEVEL_FLAG: i32 = 1;
    /// `PRIMARY_LEVEL_FLAG`.
    pub const PRIMARY_LEVEL_FLAG: i32 = 2;
    /// `SECONDARY_LEVEL_FLAG`.
    pub const SECONDARY_LEVEL_FLAG: i32 = 4;
    /// `CASE_LEVEL_FLAG`.
    pub const CASE_LEVEL_FLAG: i32 = 8;
    /// `TERTIARY_LEVEL_FLAG`.
    pub const TERTIARY_LEVEL_FLAG: i32 = 0x10;
    /// `QUATERNARY_LEVEL_FLAG`.
    pub const QUATERNARY_LEVEL_FLAG: i32 = 0x20;
    /// `IDENTICAL_LEVEL_FLAG`.
    pub const IDENTICAL_LEVEL_FLAG: i32 = 0x40;
    /// `ZERO_LEVEL_FLAG`.
    pub const ZERO_LEVEL_FLAG: i32 = 0x80;

    /// Special-CE32 tags, from bits 3..0 of a special 32-bit CE.
    pub const FALLBACK_TAG: i32 = 0;
    /// `LONG_PRIMARY_TAG`.
    pub const LONG_PRIMARY_TAG: i32 = 1;
    /// `LONG_SECONDARY_TAG`.
    pub const LONG_SECONDARY_TAG: i32 = 2;
    /// `RESERVED_TAG_3`.
    pub const RESERVED_TAG_3: i32 = 3;
    /// `LATIN_EXPANSION_TAG`.
    pub const LATIN_EXPANSION_TAG: i32 = 4;
    /// `EXPANSION32_TAG`.
    pub const EXPANSION32_TAG: i32 = 5;
    /// `EXPANSION_TAG`.
    pub const EXPANSION_TAG: i32 = 6;
    /// `BUILDER_DATA_TAG`.
    pub const BUILDER_DATA_TAG: i32 = 7;
    /// `PREFIX_TAG`.
    pub const PREFIX_TAG: i32 = 8;
    /// `CONTRACTION_TAG`.
    pub const CONTRACTION_TAG: i32 = 9;
    /// `DIGIT_TAG`.
    pub const DIGIT_TAG: i32 = 10;
    /// `U0000_TAG`.
    pub const U0000_TAG: i32 = 11;
    /// `HANGUL_TAG`.
    pub const HANGUL_TAG: i32 = 12;
    /// `LEAD_SURROGATE_TAG`.
    pub const LEAD_SURROGATE_TAG: i32 = 13;
    /// `OFFSET_TAG`.
    pub const OFFSET_TAG: i32 = 14;
    /// `IMPLICIT_TAG`.
    pub const IMPLICIT_TAG: i32 = 15;

    /// `isAssignedCE32(int)`.
    pub fn is_assigned_ce32(ce32: i32) -> bool {
        ce32 != Self::FALLBACK_CE32 && ce32 != Self::UNASSIGNED_CE32
    }

    /// The maximum length of an expansion.
    pub const MAX_EXPANSION_LENGTH: i32 = 31;
    /// The maximum index into the expansion and context tables.
    pub const MAX_INDEX: i32 = 0x7ffff;

    /// Contraction data: no single-code-point match.
    pub const CONTRACT_SINGLE_CP_NO_MATCH: i32 = 0x100;
    /// Contraction data: some next character has lccc != 0.
    pub const CONTRACT_NEXT_CCC: i32 = 0x200;
    /// Contraction data: some contraction ends with tccc != 0.
    pub const CONTRACT_TRAILING_CCC: i32 = 0x400;

    /// For `HANGUL_TAG`: no Jamo has a special CE32.
    pub const HANGUL_NO_SPECIAL_JAMO: i32 = 0x100;

    /// For `LEAD_SURROGATE_TAG`.
    pub const LEAD_ALL_UNASSIGNED: i32 = 0;
    /// `LEAD_ALL_FALLBACK`.
    pub const LEAD_ALL_FALLBACK: i32 = 0x100;
    /// `LEAD_MIXED`.
    pub const LEAD_MIXED: i32 = 0x200;
    /// `LEAD_TYPE_MASK`.
    pub const LEAD_TYPE_MASK: i32 = 0x300;

    /// `makeLongPrimaryCE32(long)`.
    pub fn make_long_primary_ce32(p: i64) -> i32 {
        (p | Self::LONG_PRIMARY_CE32_LOW_BYTE as i64) as i32
    }

    /// Turns the long-primary CE32 into a primary weight pppppp00.
    pub fn primary_from_long_primary_ce32(ce32: i32) -> i64 {
        (ce32 as i64) & 0xffffff00
    }

    /// `ceFromLongPrimaryCE32(int)`.
    pub fn ce_from_long_primary_ce32(ce32: i32) -> i64 {
        (((ce32 & h(0xffffff00)) as i64) << 32) | Self::COMMON_SEC_AND_TER_CE as i64
    }

    /// `makeLongSecondaryCE32(int)`.
    pub fn make_long_secondary_ce32(lower32: i32) -> i32 {
        lower32 | Self::SPECIAL_CE32_LOW_BYTE | Self::LONG_SECONDARY_TAG
    }

    /// `ceFromLongSecondaryCE32(int)`.
    pub fn ce_from_long_secondary_ce32(ce32: i32) -> i64 {
        (ce32 as i64) & 0xffffff00
    }

    /// Makes a special CE32 with tag, index and length.
    pub fn make_ce32_from_tag_index_and_length(tag: i32, index: i32, length: i32) -> i32 {
        (index << 13) | (length << 8) | Self::SPECIAL_CE32_LOW_BYTE | tag
    }

    /// Makes a special CE32 with only tag and index.
    pub fn make_ce32_from_tag_and_index(tag: i32, index: i32) -> i32 {
        (index << 13) | Self::SPECIAL_CE32_LOW_BYTE | tag
    }

    /// `isSpecialCE32(int)`.
    pub fn is_special_ce32(ce32: i32) -> bool {
        (ce32 & 0xff) >= Self::SPECIAL_CE32_LOW_BYTE
    }

    /// `tagFromCE32(int)`.
    pub fn tag_from_ce32(ce32: i32) -> i32 {
        ce32 & 0xf
    }

    /// `hasCE32Tag(int, int)`.
    pub fn has_ce32_tag(ce32: i32, tag: i32) -> bool {
        Self::is_special_ce32(ce32) && Self::tag_from_ce32(ce32) == tag
    }

    /// `isLongPrimaryCE32(int)`.
    pub fn is_long_primary_ce32(ce32: i32) -> bool {
        Self::has_ce32_tag(ce32, Self::LONG_PRIMARY_TAG)
    }

    /// `isSimpleOrLongCE32(int)`.
    pub fn is_simple_or_long_ce32(ce32: i32) -> bool {
        !Self::is_special_ce32(ce32)
            || Self::tag_from_ce32(ce32) == Self::LONG_PRIMARY_TAG
            || Self::tag_from_ce32(ce32) == Self::LONG_SECONDARY_TAG
    }

    /// `isSelfContainedCE32(int)`.
    pub fn is_self_contained_ce32(ce32: i32) -> bool {
        !Self::is_special_ce32(ce32)
            || Self::tag_from_ce32(ce32) == Self::LONG_PRIMARY_TAG
            || Self::tag_from_ce32(ce32) == Self::LONG_SECONDARY_TAG
            || Self::tag_from_ce32(ce32) == Self::LATIN_EXPANSION_TAG
    }

    /// `isPrefixCE32(int)`.
    pub fn is_prefix_ce32(ce32: i32) -> bool {
        Self::has_ce32_tag(ce32, Self::PREFIX_TAG)
    }

    /// `isContractionCE32(int)`.
    pub fn is_contraction_ce32(ce32: i32) -> bool {
        Self::has_ce32_tag(ce32, Self::CONTRACTION_TAG)
    }

    /// `ce32HasContext(int)`.
    pub fn ce32_has_context(ce32: i32) -> bool {
        Self::is_special_ce32(ce32)
            && (Self::tag_from_ce32(ce32) == Self::PREFIX_TAG || Self::tag_from_ce32(ce32) == Self::CONTRACTION_TAG)
    }

    /// Gets the primary CE of a Latin mini expansion.
    pub fn latin_ce0_from_ce32(ce32: i32) -> i64 {
        (((ce32 & h(0xff000000)) as i64) << 32) | Self::COMMON_SECONDARY_CE as i64 | ((ce32 & 0xff0000) >> 8) as i64
    }

    /// Gets the secondary CE of a Latin mini expansion.
    pub fn latin_ce1_from_ce32(ce32: i32) -> i64 {
        (((ce32 as i64) & 0xff00) << 16) | Self::COMMON_TERTIARY_CE as i64
    }

    /// `indexFromCE32(int)`.
    pub fn index_from_ce32(ce32: i32) -> i32 {
        ((ce32 as u32) >> 13) as i32
    }

    /// `lengthFromCE32(int)`.
    pub fn length_from_ce32(ce32: i32) -> i32 {
        (ce32 >> 8) & 31
    }

    /// `digitFromCE32(int)`.
    pub fn digit_from_ce32(ce32: i32) -> u16 {
        ((ce32 >> 8) & 0xf) as u16
    }

    /// Returns a 64-bit CE from a simple CE32 (not special).
    pub fn ce_from_simple_ce32(ce32: i32) -> i64 {
        // normal form ppppsstt -> pppp0000ss00tt00
        debug_assert!((ce32 & 0xff) < Self::SPECIAL_CE32_LOW_BYTE);
        (((ce32 & h(0xffff0000)) as i64) << 32) | (((ce32 & 0xff00) as i64) << 16) | ((ce32 & 0xff) << 8) as i64
    }

    /// Returns a 64-bit CE from a simple/long-primary/long-secondary CE32.
    pub fn ce_from_ce32(mut ce32: i32) -> i64 {
        let tertiary = ce32 & 0xff;
        if tertiary < Self::SPECIAL_CE32_LOW_BYTE {
            // normal form ppppsstt -> pppp0000ss00tt00
            (((ce32 & h(0xffff0000)) as i64) << 32) | (((ce32 & 0xff00) as i64) << 16) | (tertiary << 8) as i64
        } else {
            ce32 -= tertiary;
            if (tertiary & 0xf) == Self::LONG_PRIMARY_TAG {
                // long-primary form ppppppC1 -> pppppp00050000500
                ((ce32 as i64) << 32) | Self::COMMON_SEC_AND_TER_CE as i64
            } else {
                // long-secondary form ssssttC2 -> 00000000sssstt00
                debug_assert!((tertiary & 0xf) == Self::LONG_SECONDARY_TAG);
                (ce32 as i64) & 0xffffffff
            }
        }
    }

    /// Creates a CE from a primary weight.
    pub fn make_ce(p: i64) -> i64 {
        (p << 32) | Self::COMMON_SEC_AND_TER_CE as i64
    }

    /// Creates a CE from a primary weight, 16-bit secondary/tertiary weights,
    /// and a 2-bit quaternary.
    pub fn make_ce4(p: i64, s: i32, t: i32, q: i32) -> i64 {
        (p << 32) | ((s as i64) << 16) | t as i64 | (q << 6) as i64
    }

    /// Increments a 2-byte primary by a code point offset.
    pub fn inc_two_byte_primary_by_offset(base_primary: i64, is_compressible: bool, mut offset: i32) -> i64 {
        // Extract the second byte, minus the minimum byte value,
        // plus the offset, modulo the number of usable byte values, plus the minimum.
        // Reserve the PRIMARY_COMPRESSION_LOW_BYTE and high byte if necessary.
        let primary: i64;
        if is_compressible {
            offset += ((base_primary >> 16) as i32 & 0xff) - 4;
            primary = (((offset % 251) + 4) << 16) as i64;
            offset /= 251;
        } else {
            offset += ((base_primary >> 16) as i32 & 0xff) - 2;
            primary = (((offset % 254) + 2) << 16) as i64;
            offset /= 254;
        }
        // First byte, assume no further overflow.
        primary | ((base_primary & 0xff000000) + ((offset as i64) << 24))
    }

    /// Increments a 3-byte primary by a code point offset.
    pub fn inc_three_byte_primary_by_offset(base_primary: i64, is_compressible: bool, mut offset: i32) -> i64 {
        // Extract the third byte, minus the minimum byte value,
        // plus the offset, modulo the number of usable byte values, plus the minimum.
        offset += ((base_primary >> 8) as i32 & 0xff) - 2;
        let mut primary = (((offset % 254) + 2) << 8) as i64;
        offset /= 254;
        // Same with the second byte,
        // but reserve the PRIMARY_COMPRESSION_LOW_BYTE and high byte if necessary.
        if is_compressible {
            offset += ((base_primary >> 16) as i32 & 0xff) - 4;
            primary |= (((offset % 251) + 4) << 16) as i64;
            offset /= 251;
        } else {
            offset += ((base_primary >> 16) as i32 & 0xff) - 2;
            primary |= (((offset % 254) + 2) << 16) as i64;
            offset /= 254;
        }
        // First byte, assume no further overflow.
        primary | ((base_primary & 0xff000000) + ((offset as i64) << 24))
    }

    /// Decrements a 2-byte primary by one range step (1..0x7f).
    pub fn dec_two_byte_primary_by_one_step(mut base_primary: i64, is_compressible: bool, step: i32) -> i64 {
        // Extract the second byte, minus the minimum byte value,
        // minus the step, modulo the number of usable byte values, plus the minimum.
        // Reserve the PRIMARY_COMPRESSION_LOW_BYTE and high byte if necessary.
        // Assume no further underflow for the first byte.
        debug_assert!(0 < step && step <= 0x7f);
        let mut byte2 = ((base_primary >> 16) as i32 & 0xff) - step;
        if is_compressible {
            if byte2 < 4 {
                byte2 += 251;
                base_primary -= 0x1000000;
            }
        } else if byte2 < 2 {
            byte2 += 254;
            base_primary -= 0x1000000;
        }
        (base_primary & 0xff000000) | (byte2 << 16) as i64
    }

    /// Decrements a 3-byte primary by one range step (1..0x7f).
    pub fn dec_three_byte_primary_by_one_step(mut base_primary: i64, is_compressible: bool, step: i32) -> i64 {
        // Extract the third byte, minus the minimum byte value,
        // minus the step, modulo the number of usable byte values, plus the minimum.
        debug_assert!(0 < step && step <= 0x7f);
        let mut byte3 = ((base_primary >> 8) as i32 & 0xff) - step;
        if byte3 >= 2 {
            return (base_primary & 0xffff0000) | (byte3 << 8) as i64;
        }
        byte3 += 254;
        // Same with the second byte,
        // but reserve the PRIMARY_COMPRESSION_LOW_BYTE and high byte if necessary.
        let mut byte2 = ((base_primary >> 16) as i32 & 0xff) - 1;
        if is_compressible {
            if byte2 < 4 {
                byte2 = 0xfe;
                base_primary -= 0x1000000;
            }
        } else if byte2 < 2 {
            byte2 = 0xff;
            base_primary -= 0x1000000;
        }
        // First byte, assume no further underflow.
        (base_primary & 0xff000000) | (byte2 << 16) as i64 | (byte3 << 8) as i64
    }

    /// Computes a 3-byte primary for c's OFFSET_TAG data "CE".
    pub fn get_three_byte_primary_for_offset_data(c: i32, data_ce: i64) -> i64 {
        let p = ((data_ce as u64) >> 32) as i64; // three-byte primary pppppp00
        let lower32 = data_ce as i32; // base code point b & step s: bbbbbbss (bit 7: isCompressible)
        let offset = (c - (lower32 >> 8)) * (lower32 & 0x7f); // delta * increment
        let is_compressible = (lower32 & 0x80) != 0;
        Collation::inc_three_byte_primary_by_offset(p, is_compressible, offset)
    }

    /// Returns the unassigned-character implicit primary weight for any valid code point c.
    pub fn unassigned_primary_from_code_point(mut c: i32) -> i64 {
        // Create a gap before U+0000. Use c=-1 for [first unassigned].
        c += 1;
        // Fourth byte: 18 values, every 14th byte value (gap of 13).
        let mut primary: i64 = (2 + (c % 18) * 14) as i64;
        c /= 18;
        // Third byte: 254 values.
        primary |= ((2 + (c % 254)) << 8) as i64;
        c /= 254;
        // Second byte: 251 values 04..FE excluding the primary compression bytes.
        primary |= ((4 + (c % 251)) << 16) as i64;
        // One lead byte covers all code points (c < 0x1182B4 = 1*251*254*18).
        primary | ((Self::UNASSIGNED_IMPLICIT_BYTE as i64) << 24)
    }

    /// `unassignedCEFromCodePoint(int)`.
    pub fn unassigned_ce_from_code_point(c: i32) -> i64 {
        Self::make_ce(Self::unassigned_primary_from_code_point(c))
    }
}
