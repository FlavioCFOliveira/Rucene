// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UTF16`.

use rucene::error::{LuceneError, Result};

use crate::jdk::{array_index_out_of_bounds, index_out_of_bounds, string_index_out_of_bounds};
use crate::text::replaceable::Replaceable;

/// Standalone utility class providing UTF-16 code-point and surrogate
/// operations on strings, string buffers and `char` arrays.
///
/// Equivalent to `com.ibm.icu.text.UTF16`. Java overloads a method for
/// `String`, `CharSequence`, `StringBuffer` and `char[]` (with a
/// `start`/`limit` range); here the overloads that behave alike share one
/// function over `&[u16]`, the others get a suffix: `_buffer` for the
/// `StringBuffer` overload where its checks or messages differ, `_chars` for
/// the `char[]` overload, `_replaceable` for the `Replaceable` one.
///
/// The nested class `UTF16.StringComparator` compares case-insensitively
/// through `Normalizer.cmpEquivFold` and uses `Normalizer`'s comparison
/// options; it is ported with `Normalizer` (rmp task #646).
pub struct UTF16;

impl UTF16 {
    /// `SINGLE_CHAR_BOUNDARY`: value returned by `bounds` for a code unit
    /// that is not part of a surrogate pair.
    pub const SINGLE_CHAR_BOUNDARY: i32 = 1;
    /// `LEAD_SURROGATE_BOUNDARY`: `bounds` at the lead unit of a pair.
    pub const LEAD_SURROGATE_BOUNDARY: i32 = 2;
    /// `TRAIL_SURROGATE_BOUNDARY`: `bounds` at the trail unit of a pair.
    pub const TRAIL_SURROGATE_BOUNDARY: i32 = 5;
    /// `CODEPOINT_MIN_VALUE`.
    pub const CODEPOINT_MIN_VALUE: i32 = 0;
    /// `CODEPOINT_MAX_VALUE`.
    pub const CODEPOINT_MAX_VALUE: i32 = 0x10ffff;
    /// `SUPPLEMENTARY_MIN_VALUE`.
    pub const SUPPLEMENTARY_MIN_VALUE: i32 = 0x10000;
    /// `LEAD_SURROGATE_MIN_VALUE`.
    pub const LEAD_SURROGATE_MIN_VALUE: i32 = 0xD800;
    /// `TRAIL_SURROGATE_MIN_VALUE`.
    pub const TRAIL_SURROGATE_MIN_VALUE: i32 = 0xDC00;
    /// `LEAD_SURROGATE_MAX_VALUE`.
    pub const LEAD_SURROGATE_MAX_VALUE: i32 = 0xDBFF;
    /// `TRAIL_SURROGATE_MAX_VALUE`.
    pub const TRAIL_SURROGATE_MAX_VALUE: i32 = 0xDFFF;
    /// `SURROGATE_MIN_VALUE`.
    pub const SURROGATE_MIN_VALUE: i32 = Self::LEAD_SURROGATE_MIN_VALUE;
    /// `SURROGATE_MAX_VALUE`.
    pub const SURROGATE_MAX_VALUE: i32 = Self::TRAIL_SURROGATE_MAX_VALUE;

    /// `LEAD_SURROGATE_BITMASK`.
    const LEAD_SURROGATE_BITMASK: i32 = 0xFFFFFC00_u32 as i32;
    /// `TRAIL_SURROGATE_BITMASK`.
    const TRAIL_SURROGATE_BITMASK: i32 = 0xFFFFFC00_u32 as i32;
    /// `SURROGATE_BITMASK`.
    const SURROGATE_BITMASK: i32 = 0xFFFFF800_u32 as i32;
    /// `LEAD_SURROGATE_BITS`.
    const LEAD_SURROGATE_BITS: i32 = 0xD800;
    /// `TRAIL_SURROGATE_BITS`.
    const TRAIL_SURROGATE_BITS: i32 = 0xDC00;
    /// `SURROGATE_BITS`.
    const SURROGATE_BITS: i32 = 0xD800;
    /// `LEAD_SURROGATE_SHIFT_`.
    const LEAD_SURROGATE_SHIFT: i32 = 10;
    /// `TRAIL_SURROGATE_MASK_`.
    const TRAIL_SURROGATE_MASK: i32 = 0x3FF;
    /// `LEAD_SURROGATE_OFFSET_`.
    const LEAD_SURROGATE_OFFSET: i32 =
        Self::LEAD_SURROGATE_MIN_VALUE - (Self::SUPPLEMENTARY_MIN_VALUE >> Self::LEAD_SURROGATE_SHIFT);

    // ------------------------------------------------------------------
    // charAt
    // ------------------------------------------------------------------

    /// `charAt(String source, int offset16)` and `charAt(CharSequence,
    /// int)`: the code point at `offset16`; at a lead (trail) surrogate
    /// paired with the next (previous) unit, the supplementary code point.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside the text.
    pub fn char_at(source: &[u16], offset16: i32) -> i32 {
        let single = source[offset16 as usize];
        if (single as i32) < Self::LEAD_SURROGATE_MIN_VALUE {
            return single as i32;
        }
        Self::char_at_slow(source, offset16, single)
    }

    /// `_charAt(String|CharSequence, int, char)` (private).
    fn char_at_slow(source: &[u16], mut offset16: i32, single: u16) -> i32 {
        if single as i32 > Self::TRAIL_SURROGATE_MAX_VALUE {
            return single as i32;
        }

        // Convert the UTF-16 surrogate pair if necessary.
        // For simplicity in usage, and because the frequency of pairs is
        // low, look both directions.

        if single as i32 <= Self::LEAD_SURROGATE_MAX_VALUE {
            offset16 += 1;
            if source.len() as i32 != offset16 {
                let trail = source[offset16 as usize] as i32;
                if (Self::TRAIL_SURROGATE_MIN_VALUE..=Self::TRAIL_SURROGATE_MAX_VALUE).contains(&trail) {
                    return crate::jdk::to_code_point(single, trail as u16);
                }
            }
        } else {
            offset16 -= 1;
            if offset16 >= 0 {
                // single is a trail surrogate so
                let lead = source[offset16 as usize] as i32;
                if (Self::LEAD_SURROGATE_MIN_VALUE..=Self::LEAD_SURROGATE_MAX_VALUE).contains(&lead) {
                    return crate::jdk::to_code_point(lead as u16, single);
                }
            }
        }
        single as i32 // return unmatched surrogate
    }

    /// `charAt(StringBuffer source, int offset16)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException(offset16)` when `offset16` is
    /// outside the buffer.
    pub fn char_at_buffer(source: &[u16], mut offset16: i32) -> i32 {
        if offset16 < 0 || offset16 >= source.len() as i32 {
            string_index_out_of_bounds(offset16);
        }

        let single = source[offset16 as usize];
        if !Self::is_surrogate(single as i32) {
            return single as i32;
        }

        // Convert the UTF-16 surrogate pair if necessary.
        // For simplicity in usage, and because the frequency of pairs is
        // low, look both directions.

        if single as i32 <= Self::LEAD_SURROGATE_MAX_VALUE {
            offset16 += 1;
            if source.len() as i32 != offset16 {
                let trail = source[offset16 as usize];
                if Self::is_trail_surrogate(trail as i32) {
                    return crate::jdk::to_code_point(single, trail);
                }
            }
        } else {
            offset16 -= 1;
            if offset16 >= 0 {
                // single is a trail surrogate so
                let lead = source[offset16 as usize];
                if Self::is_lead_surrogate(lead as i32) {
                    return crate::jdk::to_code_point(lead, single);
                }
            }
        }
        single as i32 // return unmatched surrogate
    }

    /// `charAt(char[] source, int start, int limit, int offset16)`:
    /// `offset16` relative to `start`, the text being
    /// `source[start..limit]`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException(start + offset16)` when it is outside
    /// `start..limit`.
    pub fn char_at_chars(source: &[u16], start: i32, limit: i32, mut offset16: i32) -> i32 {
        offset16 += start;
        if offset16 < start || offset16 >= limit {
            array_index_out_of_bounds(offset16);
        }

        let single = source[offset16 as usize];
        if !Self::is_surrogate(single as i32) {
            return single as i32;
        }

        // Convert the UTF-16 surrogate pair if necessary.
        // For simplicity in usage, and because the frequency of pairs is
        // low, look both directions.
        if single as i32 <= Self::LEAD_SURROGATE_MAX_VALUE {
            offset16 += 1;
            if offset16 >= limit {
                return single as i32;
            }
            let trail = source[offset16 as usize];
            if Self::is_trail_surrogate(trail as i32) {
                return crate::jdk::to_code_point(single, trail);
            }
        } else {
            // isTrailSurrogate(single), so
            if offset16 == start {
                return single as i32;
            }
            offset16 -= 1;
            let lead = source[offset16 as usize];
            if Self::is_lead_surrogate(lead as i32) {
                return crate::jdk::to_code_point(lead, single);
            }
        }
        single as i32 // return unmatched surrogate
    }

    /// `charAt(Replaceable source, int offset16)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException(offset16)` when `offset16` is
    /// outside the text.
    pub fn char_at_replaceable(source: &dyn Replaceable, mut offset16: i32) -> i32 {
        if offset16 < 0 || offset16 >= source.length() {
            string_index_out_of_bounds(offset16);
        }

        let single = source.char_at(offset16);
        if !Self::is_surrogate(single as i32) {
            return single as i32;
        }

        // Convert the UTF-16 surrogate pair if necessary.
        // For simplicity in usage, and because the frequency of pairs is
        // low, look both directions.

        if single as i32 <= Self::LEAD_SURROGATE_MAX_VALUE {
            offset16 += 1;
            if source.length() != offset16 {
                let trail = source.char_at(offset16);
                if Self::is_trail_surrogate(trail as i32) {
                    return crate::jdk::to_code_point(single, trail);
                }
            }
        } else {
            offset16 -= 1;
            if offset16 >= 0 {
                // single is a trail surrogate so
                let lead = source.char_at(offset16);
                if Self::is_lead_surrogate(lead as i32) {
                    return crate::jdk::to_code_point(lead, single);
                }
            }
        }
        single as i32 // return unmatched surrogate
    }

    /// `getCharCount(int char32)`: 2 for a supplementary code point, else 1.
    pub fn get_char_count(char32: i32) -> i32 {
        if char32 < Self::SUPPLEMENTARY_MIN_VALUE {
            return 1;
        }
        2
    }

    // ------------------------------------------------------------------
    // bounds
    // ------------------------------------------------------------------

    /// `bounds(String source, int offset16)` and `bounds(StringBuffer,
    /// int)`: [`SINGLE_CHAR_BOUNDARY`](Self::SINGLE_CHAR_BOUNDARY),
    /// [`LEAD_SURROGATE_BOUNDARY`](Self::LEAD_SURROGATE_BOUNDARY) or
    /// [`TRAIL_SURROGATE_BOUNDARY`](Self::TRAIL_SURROGATE_BOUNDARY).
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside the text.
    pub fn bounds(source: &[u16], mut offset16: i32) -> i32 {
        let ch = source[offset16 as usize];
        if Self::is_surrogate(ch as i32) {
            if Self::is_lead_surrogate(ch as i32) {
                offset16 += 1;
                if offset16 < source.len() as i32 && Self::is_trail_surrogate(source[offset16 as usize] as i32) {
                    return Self::LEAD_SURROGATE_BOUNDARY;
                }
            } else {
                // isTrailSurrogate(ch), so
                offset16 -= 1;
                if offset16 >= 0 && Self::is_lead_surrogate(source[offset16 as usize] as i32) {
                    return Self::TRAIL_SURROGATE_BOUNDARY;
                }
            }
        }
        Self::SINGLE_CHAR_BOUNDARY
    }

    /// `bounds(char[] source, int start, int limit, int offset16)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException(start + offset16)` when it is outside
    /// `start..limit`.
    pub fn bounds_chars(source: &[u16], start: i32, limit: i32, mut offset16: i32) -> i32 {
        offset16 += start;
        if offset16 < start || offset16 >= limit {
            array_index_out_of_bounds(offset16);
        }
        let ch = source[offset16 as usize];
        if Self::is_surrogate(ch as i32) {
            if Self::is_lead_surrogate(ch as i32) {
                offset16 += 1;
                if offset16 < limit && Self::is_trail_surrogate(source[offset16 as usize] as i32) {
                    return Self::LEAD_SURROGATE_BOUNDARY;
                }
            } else {
                // isTrailSurrogate(ch), so
                offset16 -= 1;
                if offset16 >= start && Self::is_lead_surrogate(source[offset16 as usize] as i32) {
                    return Self::TRAIL_SURROGATE_BOUNDARY;
                }
            }
        }
        Self::SINGLE_CHAR_BOUNDARY
    }

    /// `isSurrogate(int codePoint)`.
    pub fn is_surrogate(code_point: i32) -> bool {
        (code_point & Self::SURROGATE_BITMASK) == Self::SURROGATE_BITS
    }

    /// `isTrailSurrogate(int codePoint)`.
    pub fn is_trail_surrogate(code_point: i32) -> bool {
        (code_point & Self::TRAIL_SURROGATE_BITMASK) == Self::TRAIL_SURROGATE_BITS
    }

    /// `isLeadSurrogate(int codePoint)`.
    pub fn is_lead_surrogate(code_point: i32) -> bool {
        (code_point & Self::LEAD_SURROGATE_BITMASK) == Self::LEAD_SURROGATE_BITS
    }

    /// `getLeadSurrogate(int char32)`: the lead unit of a supplementary code
    /// point, else 0.
    pub fn get_lead_surrogate(char32: i32) -> u16 {
        if char32 >= Self::SUPPLEMENTARY_MIN_VALUE {
            return Self::LEAD_SURROGATE_OFFSET.wrapping_add(char32 >> Self::LEAD_SURROGATE_SHIFT) as u16;
        }
        0
    }

    /// `getTrailSurrogate(int char32)`: the trail unit of a supplementary
    /// code point, else `(char) char32`.
    pub fn get_trail_surrogate(char32: i32) -> u16 {
        if char32 >= Self::SUPPLEMENTARY_MIN_VALUE {
            return (Self::TRAIL_SURROGATE_MIN_VALUE + (char32 & Self::TRAIL_SURROGATE_MASK)) as u16;
        }
        char32 as u16
    }

    // ------------------------------------------------------------------
    // valueOf
    // ------------------------------------------------------------------

    /// `valueOf(int char32)`: the code point as a string.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` outside
    /// `0..=0x10ffff`.
    pub fn value_of(char32: i32) -> Result<Vec<u16>> {
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Illegal codepoint"));
        }
        Ok(Self::to_string(char32))
    }

    /// `valueOf(String source, int offset16)` and `valueOf(StringBuffer,
    /// int)`: the code point at `offset16` as a string of one or two units.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside the text.
    pub fn value_of_at(source: &[u16], offset16: i32) -> Vec<u16> {
        let o = offset16 as usize;
        match Self::bounds(source, offset16) {
            Self::LEAD_SURROGATE_BOUNDARY => source[o..o + 2].to_vec(),
            Self::TRAIL_SURROGATE_BOUNDARY => source[o - 1..o + 1].to_vec(),
            _ => source[o..o + 1].to_vec(),
        }
    }

    /// `valueOf(char[] source, int start, int limit, int offset16)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `start + offset16` is outside
    /// `start..limit`.
    pub fn value_of_chars(source: &[u16], start: i32, limit: i32, offset16: i32) -> Vec<u16> {
        let o = (start + offset16) as usize;
        match Self::bounds_chars(source, start, limit, offset16) {
            Self::LEAD_SURROGATE_BOUNDARY => source[o..o + 2].to_vec(),
            Self::TRAIL_SURROGATE_BOUNDARY => source[o - 1..o + 1].to_vec(),
            _ => source[o..o + 1].to_vec(),
        }
    }

    // ------------------------------------------------------------------
    // Offsets
    // ------------------------------------------------------------------

    /// `findOffsetFromCodePoint(String source, int offset32)` and the
    /// `StringBuffer` overload: the UTF-16 offset of code point `offset32`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException(offset32)` when it is outside the
    /// text.
    pub fn find_offset_from_code_point(source: &[u16], offset32: i32) -> i32 {
        let size = source.len() as i32;
        let mut result = 0;
        let mut count = offset32;
        if offset32 < 0 || offset32 > size {
            string_index_out_of_bounds(offset32);
        }
        while result < size && count > 0 {
            let ch = source[result as usize];
            if Self::is_lead_surrogate(ch as i32)
                && (result + 1) < size
                && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
            {
                result += 1;
            }

            count -= 1;
            result += 1;
        }
        if count != 0 {
            string_index_out_of_bounds(offset32);
        }
        result
    }

    /// `findOffsetFromCodePoint(char[] source, int start, int limit, int
    /// offset32)`: the offset relative to `start`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException(offset32)` when it is outside the
    /// range.
    pub fn find_offset_from_code_point_chars(source: &[u16], start: i32, limit: i32, offset32: i32) -> i32 {
        let mut result = start;
        let mut count = offset32;
        if offset32 > limit - start {
            array_index_out_of_bounds(offset32);
        }
        while result < limit && count > 0 {
            let ch = source[result as usize];
            if Self::is_lead_surrogate(ch as i32)
                && (result + 1) < limit
                && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
            {
                result += 1;
            }

            count -= 1;
            result += 1;
        }
        if count != 0 {
            array_index_out_of_bounds(offset32);
        }
        result - start
    }

    /// `findCodePointOffset(String source, int offset16)` and the
    /// `StringBuffer` overload: the code point index of the unit at
    /// `offset16` (a trail unit of a pair counts with its lead).
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException(offset16)` when it is outside
    /// `0..=length`.
    pub fn find_code_point_offset(source: &[u16], offset16: i32) -> i32 {
        if offset16 < 0 || offset16 > source.len() as i32 {
            string_index_out_of_bounds(offset16);
        }

        let mut result = 0;
        let mut had_lead_surrogate = false;

        for &ch in &source[..offset16 as usize] {
            if had_lead_surrogate && Self::is_trail_surrogate(ch as i32) {
                had_lead_surrogate = false; // count valid trail as zero
            } else {
                had_lead_surrogate = Self::is_lead_surrogate(ch as i32);
                result += 1; // count others as 1
            }
        }

        if offset16 == source.len() as i32 {
            return result;
        }

        // end of source being the less significant surrogate character
        // shift result back to the start of the supplementary character
        if had_lead_surrogate && Self::is_trail_surrogate(source[offset16 as usize] as i32) {
            result -= 1;
        }

        result
    }

    /// `findCodePointOffset(char[] source, int start, int limit, int
    /// offset16)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException(start + offset16)` when it is above
    /// `limit`.
    pub fn find_code_point_offset_chars(source: &[u16], start: i32, limit: i32, mut offset16: i32) -> i32 {
        offset16 += start;
        if offset16 > limit {
            string_index_out_of_bounds(offset16);
        }

        let mut result = 0;
        let mut had_lead_surrogate = false;

        for i in start..offset16 {
            let ch = source[i as usize];
            if had_lead_surrogate && Self::is_trail_surrogate(ch as i32) {
                had_lead_surrogate = false; // count valid trail as zero
            } else {
                had_lead_surrogate = Self::is_lead_surrogate(ch as i32);
                result += 1; // count others as 1
            }
        }

        if offset16 == limit {
            return result;
        }

        // end of source being the less significant surrogate character
        // shift result back to the start of the supplementary character
        if had_lead_surrogate && Self::is_trail_surrogate(source[offset16 as usize] as i32) {
            result -= 1;
        }

        result
    }

    // ------------------------------------------------------------------
    // append, count, setCharAt, moveCodePointOffset, insert, delete
    // ------------------------------------------------------------------

    /// `append(StringBuffer target, int char32)`: appends the code point.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint: <hex>")` outside
    /// `0..=0x10ffff`.
    pub fn append(target: &mut Vec<u16>, char32: i32) -> Result<&mut Vec<u16>> {
        // Check for irregular values
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal codepoint: {:x}",
                char32 as u32
            )));
        }

        // Write the UTF-16 values
        if char32 >= Self::SUPPLEMENTARY_MIN_VALUE {
            target.push(Self::get_lead_surrogate(char32));
            target.push(Self::get_trail_surrogate(char32));
        } else {
            target.push(char32 as u16);
        }
        Ok(target)
    }

    /// `appendCodePoint(StringBuffer target, int cp)`: [`append`](Self::append).
    ///
    /// # Errors
    ///
    /// As [`append`](Self::append).
    pub fn append_code_point(target: &mut Vec<u16>, cp: i32) -> Result<&mut Vec<u16>> {
        Self::append(target, cp)
    }

    /// `append(char[] target, int limit, int char32)`: writes the code point
    /// at `limit` and returns the new limit.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` outside
    /// `0..=0x10ffff`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when the array is full.
    pub fn append_chars(target: &mut [u16], mut limit: i32, char32: i32) -> Result<i32> {
        // Check for irregular values
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Illegal codepoint"));
        }
        // Write the UTF-16 values
        if char32 >= Self::SUPPLEMENTARY_MIN_VALUE {
            target[limit as usize] = Self::get_lead_surrogate(char32);
            limit += 1;
            target[limit as usize] = Self::get_trail_surrogate(char32);
            limit += 1;
        } else {
            target[limit as usize] = char32 as u16;
            limit += 1;
        }
        Ok(limit)
    }

    /// `countCodePoint(String source)` and the `StringBuffer` overload: the
    /// number of code points, 0 for a `null` or empty text.
    pub fn count_code_point(source: Option<&[u16]>) -> i32 {
        match source {
            None => 0,
            Some(s) if s.is_empty() => 0,
            Some(s) => Self::find_code_point_offset(s, s.len() as i32),
        }
    }

    /// `countCodePoint(char[] source, int start, int limit)`.
    ///
    /// # Panics
    ///
    /// As [`find_code_point_offset_chars`](Self::find_code_point_offset_chars).
    pub fn count_code_point_chars(source: Option<&[u16]>, start: i32, limit: i32) -> i32 {
        match source {
            None => 0,
            Some(s) if s.is_empty() => 0,
            Some(s) => Self::find_code_point_offset_chars(s, start, limit, limit - start),
        }
    }

    /// `setCharAt(StringBuffer target, int offset16, int char32)`: replaces
    /// the code point at `offset16` (the whole pair when it is part of one).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` for an invalid
    /// `char32`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside the
    /// buffer.
    pub fn set_char_at(target: &mut Vec<u16>, mut offset16: i32, char32: i32) -> Result<()> {
        let mut count = 1;
        let single = target[offset16 as usize];

        if Self::is_surrogate(single as i32) {
            // pairs of the surrogate with offset16 at the lead char found
            if Self::is_lead_surrogate(single as i32)
                && (target.len() as i32 > offset16 + 1)
                && Self::is_trail_surrogate(target[(offset16 + 1) as usize] as i32)
            {
                count += 1;
            } else {
                // pairs of the surrogate with offset16 at the trail char
                // found
                if Self::is_trail_surrogate(single as i32)
                    && (offset16 > 0)
                    && Self::is_lead_surrogate(target[(offset16 - 1) as usize] as i32)
                {
                    offset16 -= 1;
                    count += 1;
                }
            }
        }
        let value = Self::value_of(char32)?;
        crate::text::replaceable_string::string_buffer_replace(target, offset16, offset16 + count, &value);
        Ok(())
    }

    /// `setCharAt(char[] target, int limit, int offset16, int char32)`:
    /// replaces the code point at `offset16` in `target[0..limit]` and
    /// returns the new limit.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` for an invalid
    /// `char32`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException(offset16)` when `offset16 >= limit`.
    pub fn set_char_at_chars(target: &mut [u16], limit: i32, mut offset16: i32, char32: i32) -> Result<i32> {
        if offset16 >= limit {
            array_index_out_of_bounds(offset16);
        }
        let mut count = 1;
        let single = target[offset16 as usize];

        if Self::is_surrogate(single as i32) {
            // pairs of the surrogate with offset16 at the lead char found
            if Self::is_lead_surrogate(single as i32)
                && (target.len() as i32 > offset16 + 1)
                && Self::is_trail_surrogate(target[(offset16 + 1) as usize] as i32)
            {
                count += 1;
            } else {
                // pairs of the surrogate with offset16 at the trail char
                // found
                if Self::is_trail_surrogate(single as i32)
                    && (offset16 > 0)
                    && Self::is_lead_surrogate(target[(offset16 - 1) as usize] as i32)
                {
                    offset16 -= 1;
                    count += 1;
                }
            }
        }

        let s = Self::value_of(char32)?;
        let mut result = limit;
        let strlength = s.len() as i32;
        target[offset16 as usize] = s[0];
        if count == strlength {
            if count == 2 {
                target[(offset16 + 1) as usize] = s[1];
            }
        } else {
            // this is not exact match in space, we'll have to do some
            // shifting
            let src = (offset16 + count) as usize;
            let n = (limit - (offset16 + count)) as usize;
            target.copy_within(src..src + n, (offset16 + strlength) as usize);
            if count < strlength {
                // char32 is a supplementary character trying to squeeze into
                // a non-supplementary space
                target[(offset16 + 1) as usize] = s[1];
                result += 1;
                if result < target.len() as i32 {
                    target[result as usize] = 0;
                }
            } else {
                // char32 is a non-supplementary character trying to fill
                // into a supplementary space
                result -= 1;
                target[result as usize] = 0;
            }
        }
        Ok(result)
    }

    /// `moveCodePointOffset(String source, int offset16, int shift32)` and
    /// the `StringBuffer` overload: the offset `shift32` code points from
    /// `offset16`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` or the result is
    /// outside the text.
    pub fn move_code_point_offset(source: &[u16], offset16: i32, shift32: i32) -> i32 {
        let mut result = offset16;
        let size = source.len() as i32;
        let mut count;
        if offset16 < 0 || offset16 > size {
            string_index_out_of_bounds(offset16);
        }
        if shift32 > 0 {
            if shift32.wrapping_add(offset16) > size {
                string_index_out_of_bounds(offset16);
            }
            count = shift32;
            while result < size && count > 0 {
                let ch = source[result as usize];
                if Self::is_lead_surrogate(ch as i32)
                    && (result + 1) < size
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    result += 1;
                }
                count -= 1;
                result += 1;
            }
        } else {
            if offset16.wrapping_add(shift32) < 0 {
                string_index_out_of_bounds(offset16);
            }
            count = shift32.wrapping_neg();
            while count > 0 {
                result -= 1;
                if result < 0 {
                    break;
                }
                let ch = source[result as usize];
                if Self::is_trail_surrogate(ch as i32)
                    && result > 0
                    && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
                {
                    result -= 1;
                }
                count -= 1;
            }
        }
        if count != 0 {
            string_index_out_of_bounds(shift32);
        }
        result
    }

    /// `moveCodePointOffset(char[] source, int start, int limit, int
    /// offset16, int shift32)`: offsets relative to `start`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an invalid range, offset or
    /// shift.
    pub fn move_code_point_offset_chars(
        source: &[u16],
        start: i32,
        limit: i32,
        offset16: i32,
        shift32: i32,
    ) -> i32 {
        let size = source.len() as i32;
        let mut count;
        let mut result = offset16.wrapping_add(start);
        if start < 0 || limit < start {
            string_index_out_of_bounds(start);
        }
        if limit > size {
            string_index_out_of_bounds(limit);
        }
        if offset16 < 0 || result > limit {
            string_index_out_of_bounds(offset16);
        }
        if shift32 > 0 {
            if shift32.wrapping_add(result) > size {
                string_index_out_of_bounds(result);
            }
            count = shift32;
            while result < limit && count > 0 {
                let ch = source[result as usize];
                if Self::is_lead_surrogate(ch as i32)
                    && (result + 1 < limit)
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    result += 1;
                }
                count -= 1;
                result += 1;
            }
        } else {
            if result.wrapping_add(shift32) < start {
                string_index_out_of_bounds(result);
            }
            count = shift32.wrapping_neg();
            while count > 0 {
                result -= 1;
                if result < start {
                    break;
                }
                let ch = source[result as usize];
                if Self::is_trail_surrogate(ch as i32)
                    && result > start
                    && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
                {
                    result -= 1;
                }
                count -= 1;
            }
        }
        if count != 0 {
            string_index_out_of_bounds(shift32);
        }
        result -= start;
        result
    }

    /// `insert(StringBuffer target, int offset16, int char32)`: inserts the
    /// code point, after the pair when `offset16` is at a trail unit.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` for an invalid
    /// `char32`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside
    /// `0..=length`.
    pub fn insert(target: &mut Vec<u16>, mut offset16: i32, char32: i32) -> Result<&mut Vec<u16>> {
        let s = Self::value_of(char32)?;
        if offset16 != target.len() as i32 && Self::bounds(target, offset16) == Self::TRAIL_SURROGATE_BOUNDARY {
            offset16 += 1;
        }
        assert!(
            offset16 >= 0 && offset16 as usize <= target.len(),
            "java.lang.StringIndexOutOfBoundsException: offset {offset16}, length {}",
            target.len()
        );
        let at = offset16 as usize;
        target.splice(at..at, s);
        Ok(target)
    }

    /// `insert(char[] target, int limit, int offset16, int char32)`: inserts
    /// the code point into `target[0..limit]` and returns the new limit.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal codepoint")` for an invalid
    /// `char32`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException(offset16 + size)` when the array has
    /// no room.
    pub fn insert_chars(target: &mut [u16], limit: i32, mut offset16: i32, char32: i32) -> Result<i32> {
        let s = Self::value_of(char32)?;
        if offset16 != limit && Self::bounds_chars(target, 0, limit, offset16) == Self::TRAIL_SURROGATE_BOUNDARY {
            offset16 += 1;
        }
        let size = s.len() as i32;
        if limit + size > target.len() as i32 {
            array_index_out_of_bounds(offset16 + size);
        }
        let o = offset16 as usize;
        target.copy_within(o..limit as usize, o + size as usize);
        target[o] = s[0];
        if size == 2 {
            target[o + 1] = s[1];
        }
        Ok(limit + size)
    }

    /// `delete(StringBuffer target, int offset16)`: removes the code point
    /// at `offset16` (the whole pair when it is part of one).
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when `offset16` is outside the
    /// buffer.
    pub fn delete(target: &mut Vec<u16>, mut offset16: i32) -> &mut Vec<u16> {
        let mut count = 1;
        match Self::bounds(target, offset16) {
            Self::LEAD_SURROGATE_BOUNDARY => {
                count += 1;
            }
            Self::TRAIL_SURROGATE_BOUNDARY => {
                count += 1;
                offset16 -= 1;
            }
            _ => {}
        }
        let end = ((offset16 + count) as usize).min(target.len());
        target.drain(offset16 as usize..end);
        target
    }

    /// `delete(char[] target, int limit, int offset16)`: removes the code
    /// point at `offset16` from `target[0..limit]`, zeroes the freed units
    /// at the end and returns the new limit.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `offset16` is outside the range.
    pub fn delete_chars(target: &mut [u16], limit: i32, mut offset16: i32) -> i32 {
        let mut count = 1;
        match Self::bounds_chars(target, 0, limit, offset16) {
            Self::LEAD_SURROGATE_BOUNDARY => {
                count += 1;
            }
            Self::TRAIL_SURROGATE_BOUNDARY => {
                count += 1;
                offset16 -= 1;
            }
            _ => {}
        }
        let src = (offset16 + count) as usize;
        target.copy_within(src..limit as usize, offset16 as usize);
        target[(limit - count) as usize] = 0;
        limit - count
    }

    // ------------------------------------------------------------------
    // indexOf, lastIndexOf, replace
    // ------------------------------------------------------------------

    /// `indexOf(String source, int char32)`: the first index of the code
    /// point, a surrogate code point matching only unpaired surrogates.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Argument char32 is not a valid
    /// codepoint")`.
    pub fn index_of(source: &[u16], char32: i32) -> Result<i32> {
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Argument char32 is not a valid codepoint"));
        }

        // non-surrogate bmp
        if char32 < Self::LEAD_SURROGATE_MIN_VALUE
            || (char32 > Self::TRAIL_SURROGATE_MAX_VALUE && char32 < Self::SUPPLEMENTARY_MIN_VALUE)
        {
            return Ok(crate::jdk::index_of_unit(source, char32 as u16, 0));
        }
        // surrogate
        if char32 < Self::SUPPLEMENTARY_MIN_VALUE {
            let result = crate::jdk::index_of_unit(source, char32 as u16, 0);
            if result >= 0 {
                if Self::is_lead_surrogate(char32)
                    && (result < source.len() as i32 - 1)
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    return Self::index_of_from(source, char32, result + 1);
                }
                // trail surrogate
                if result > 0 && Self::is_lead_surrogate(source[(result - 1) as usize] as i32) {
                    return Self::index_of_from(source, char32, result + 1);
                }
            }
            return Ok(result);
        }
        // supplementary
        let char32str = Self::to_string(char32);
        Ok(crate::jdk::index_of(source, &char32str, 0))
    }

    /// `indexOf(String source, String str)`: the first index of `str` that
    /// does not split a surrogate pair at either end.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty `str`.
    pub fn index_of_str(source: &[u16], s: &[u16]) -> i32 {
        let str_length = s.len() as i32;
        // non-surrogate ends
        if !Self::is_trail_surrogate(s[0] as i32) && !Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32) {
            return crate::jdk::index_of(source, s, 0);
        }

        let result = crate::jdk::index_of(source, s, 0);
        let result_end = result + str_length;
        if result >= 0 {
            // check last character
            if Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32)
                && (result < source.len() as i32 - 1)
                && Self::is_trail_surrogate(source[(result_end + 1) as usize] as i32)
            {
                return Self::index_of_str_from(source, s, result_end + 1);
            }
            // check first character which is a trail surrogate
            if Self::is_trail_surrogate(s[0] as i32)
                && result > 0
                && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
            {
                return Self::index_of_str_from(source, s, result_end + 1);
            }
        }
        result
    }

    /// `indexOf(String source, int char32, int fromIndex)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Argument char32 is not a valid
    /// codepoint")`.
    pub fn index_of_from(source: &[u16], char32: i32, from_index: i32) -> Result<i32> {
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Argument char32 is not a valid codepoint"));
        }

        // non-surrogate bmp
        if char32 < Self::LEAD_SURROGATE_MIN_VALUE
            || (char32 > Self::TRAIL_SURROGATE_MAX_VALUE && char32 < Self::SUPPLEMENTARY_MIN_VALUE)
        {
            return Ok(crate::jdk::index_of_unit(source, char32 as u16, from_index));
        }
        // surrogate
        if char32 < Self::SUPPLEMENTARY_MIN_VALUE {
            let result = crate::jdk::index_of_unit(source, char32 as u16, from_index);
            if result >= 0 {
                if Self::is_lead_surrogate(char32)
                    && (result < source.len() as i32 - 1)
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    return Self::index_of_from(source, char32, result + 1);
                }
                // trail surrogate
                if result > 0 && Self::is_lead_surrogate(source[(result - 1) as usize] as i32) {
                    return Self::index_of_from(source, char32, result + 1);
                }
            }
            return Ok(result);
        }
        // supplementary
        let char32str = Self::to_string(char32);
        Ok(crate::jdk::index_of(source, &char32str, from_index))
    }

    /// `indexOf(String source, String str, int fromIndex)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty `str`.
    pub fn index_of_str_from(source: &[u16], s: &[u16], from_index: i32) -> i32 {
        let str_length = s.len() as i32;
        // non-surrogate ends
        if !Self::is_trail_surrogate(s[0] as i32) && !Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32) {
            return crate::jdk::index_of(source, s, from_index);
        }

        let result = crate::jdk::index_of(source, s, from_index);
        let result_end = result + str_length;
        if result >= 0 {
            // check last character
            if Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32)
                && (result < source.len() as i32 - 1)
                && Self::is_trail_surrogate(source[result_end as usize] as i32)
            {
                return Self::index_of_str_from(source, s, result_end + 1);
            }
            // check first character which is a trail surrogate
            if Self::is_trail_surrogate(s[0] as i32)
                && result > 0
                && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
            {
                return Self::index_of_str_from(source, s, result_end + 1);
            }
        }
        result
    }

    /// `lastIndexOf(String source, int char32)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Argument char32 is not a valid
    /// codepoint")`.
    pub fn last_index_of(source: &[u16], char32: i32) -> Result<i32> {
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Argument char32 is not a valid codepoint"));
        }

        let from = source.len() as i32 - 1;
        // non-surrogate bmp
        if char32 < Self::LEAD_SURROGATE_MIN_VALUE
            || (char32 > Self::TRAIL_SURROGATE_MAX_VALUE && char32 < Self::SUPPLEMENTARY_MIN_VALUE)
        {
            return Ok(crate::jdk::last_index_of_unit(source, char32 as u16, from));
        }
        // surrogate
        if char32 < Self::SUPPLEMENTARY_MIN_VALUE {
            let result = crate::jdk::last_index_of_unit(source, char32 as u16, from);
            if result >= 0 {
                if Self::is_lead_surrogate(char32)
                    && (result < source.len() as i32 - 1)
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    return Self::last_index_of_from(source, char32, result - 1);
                }
                // trail surrogate
                if result > 0 && Self::is_lead_surrogate(source[(result - 1) as usize] as i32) {
                    return Self::last_index_of_from(source, char32, result - 1);
                }
            }
            return Ok(result);
        }
        // supplementary
        let char32str = Self::to_string(char32);
        Ok(crate::jdk::last_index_of(source, &char32str, source.len() as i32))
    }

    /// `lastIndexOf(String source, String str)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty `str`.
    pub fn last_index_of_str(source: &[u16], s: &[u16]) -> i32 {
        let str_length = s.len() as i32;
        // non-surrogate ends
        if !Self::is_trail_surrogate(s[0] as i32) && !Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32) {
            return crate::jdk::last_index_of(source, s, source.len() as i32);
        }

        let result = crate::jdk::last_index_of(source, s, source.len() as i32);
        if result >= 0 {
            // check last character
            if Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32)
                && (result < source.len() as i32 - 1)
                && Self::is_trail_surrogate(source[(result + str_length + 1) as usize] as i32)
            {
                return Self::last_index_of_str_from(source, s, result - 1);
            }
            // check first character which is a trail surrogate
            if Self::is_trail_surrogate(s[0] as i32)
                && result > 0
                && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
            {
                return Self::last_index_of_str_from(source, s, result - 1);
            }
        }
        result
    }

    /// `lastIndexOf(String source, int char32, int fromIndex)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Argument char32 is not a valid
    /// codepoint")`.
    pub fn last_index_of_from(source: &[u16], char32: i32, from_index: i32) -> Result<i32> {
        if !(Self::CODEPOINT_MIN_VALUE..=Self::CODEPOINT_MAX_VALUE).contains(&char32) {
            return Err(LuceneError::illegal_argument("Argument char32 is not a valid codepoint"));
        }

        // non-surrogate bmp
        if char32 < Self::LEAD_SURROGATE_MIN_VALUE
            || (char32 > Self::TRAIL_SURROGATE_MAX_VALUE && char32 < Self::SUPPLEMENTARY_MIN_VALUE)
        {
            return Ok(crate::jdk::last_index_of_unit(source, char32 as u16, from_index));
        }
        // surrogate
        if char32 < Self::SUPPLEMENTARY_MIN_VALUE {
            let result = crate::jdk::last_index_of_unit(source, char32 as u16, from_index);
            if result >= 0 {
                if Self::is_lead_surrogate(char32)
                    && (result < source.len() as i32 - 1)
                    && Self::is_trail_surrogate(source[(result + 1) as usize] as i32)
                {
                    return Self::last_index_of_from(source, char32, result - 1);
                }
                // trail surrogate
                if result > 0 && Self::is_lead_surrogate(source[(result - 1) as usize] as i32) {
                    return Self::last_index_of_from(source, char32, result - 1);
                }
            }
            return Ok(result);
        }
        // supplementary
        let char32str = Self::to_string(char32);
        Ok(crate::jdk::last_index_of(source, &char32str, from_index))
    }

    /// `lastIndexOf(String source, String str, int fromIndex)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty `str`.
    pub fn last_index_of_str_from(source: &[u16], s: &[u16], from_index: i32) -> i32 {
        let str_length = s.len() as i32;
        // non-surrogate ends
        if !Self::is_trail_surrogate(s[0] as i32) && !Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32) {
            return crate::jdk::last_index_of(source, s, from_index);
        }

        let result = crate::jdk::last_index_of(source, s, from_index);
        if result >= 0 {
            // check last character
            if Self::is_lead_surrogate(s[(str_length - 1) as usize] as i32)
                && (result < source.len() as i32 - 1)
                && Self::is_trail_surrogate(source[(result + str_length) as usize] as i32)
            {
                return Self::last_index_of_str_from(source, s, result - 1);
            }
            // check first character which is a trail surrogate
            if Self::is_trail_surrogate(s[0] as i32)
                && result > 0
                && Self::is_lead_surrogate(source[(result - 1) as usize] as i32)
            {
                return Self::last_index_of_str_from(source, s, result - 1);
            }
        }
        result
    }

    /// `replace(String source, int oldChar32, int newChar32)`: every
    /// occurrence of the old code point replaced by the new one.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when either argument is not in
    /// `1..=0x10ffff`.
    pub fn replace(source: &[u16], old_char32: i32, new_char32: i32) -> Result<Vec<u16>> {
        if old_char32 <= 0 || old_char32 > Self::CODEPOINT_MAX_VALUE {
            return Err(LuceneError::illegal_argument("Argument oldChar32 is not a valid codepoint"));
        }
        if new_char32 <= 0 || new_char32 > Self::CODEPOINT_MAX_VALUE {
            return Err(LuceneError::illegal_argument("Argument newChar32 is not a valid codepoint"));
        }

        let mut index = Self::index_of(source, old_char32)?;
        if index == -1 {
            return Ok(source.to_vec());
        }
        let new_char32_str = Self::to_string(new_char32);
        let mut old_char32_size = 1;
        let new_char32_size = new_char32_str.len() as i32;
        let mut result = source.to_vec();
        let mut result_index = index;

        if old_char32 >= Self::SUPPLEMENTARY_MIN_VALUE {
            old_char32_size = 2;
        }

        while index != -1 {
            let end_result_index = result_index + old_char32_size;
            crate::text::replaceable_string::string_buffer_replace(
                &mut result,
                result_index,
                end_result_index,
                &new_char32_str,
            );
            let last_end_index = index + old_char32_size;
            index = Self::index_of_from(source, old_char32, last_end_index)?;
            result_index += new_char32_size + index - last_end_index;
        }
        Ok(result)
    }

    /// `replace(String source, String oldStr, String newStr)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` for an empty `old_str`.
    pub fn replace_str(source: &[u16], old_str: &[u16], new_str: &[u16]) -> Vec<u16> {
        let mut index = Self::index_of_str(source, old_str);
        if index == -1 {
            return source.to_vec();
        }
        let old_str_size = old_str.len() as i32;
        let new_str_size = new_str.len() as i32;
        let mut result = source.to_vec();
        let mut result_index = index;

        while index != -1 {
            let end_result_index = result_index + old_str_size;
            crate::text::replaceable_string::string_buffer_replace(
                &mut result,
                result_index,
                end_result_index,
                new_str,
            );
            let last_end_index = index + old_str_size;
            index = Self::index_of_str_from(source, old_str, last_end_index);
            result_index += new_str_size + index - last_end_index;
        }
        result
    }

    /// `reverse(StringBuffer source)`: a new buffer with the code points in
    /// reverse order (surrogate pairs kept in order).
    pub fn reverse(source: &[u16]) -> Vec<u16> {
        let length = source.len();
        let mut result = Vec::with_capacity(length);
        let mut i = length;
        while i > 0 {
            i -= 1;
            let ch = source[i];
            if Self::is_trail_surrogate(ch as i32) && i > 0 {
                let ch2 = source[i - 1];
                if Self::is_lead_surrogate(ch2 as i32) {
                    result.push(ch2);
                    result.push(ch);
                    i -= 1;
                    continue;
                }
            }
            result.push(ch);
        }
        result
    }

    /// `hasMoreCodePointsThan(String source, int number)` and the
    /// `StringBuffer` overload: whether the text has more than `number`
    /// code points (always for a negative `number`, never for `null`).
    pub fn has_more_code_points_than(source: Option<&[u16]>, mut number: i32) -> bool {
        if number < 0 {
            return true;
        }
        let Some(source) = source else {
            return false;
        };
        let length = source.len() as i32;

        // length >= 0 known
        // source contains at least (length + 1) / 2 code points: <= 2
        // chars per cp
        if ((length + 1) >> 1) > number {
            return true;
        }

        // check if source does not even contain enough chars
        let mut maxsupplementary = length - number;
        if maxsupplementary <= 0 {
            return false;
        }

        // there are maxsupplementary = length - number more chars than
        // asked-for code points

        // count code points until they exceed and also check that there are
        // no more than maxsupplementary supplementary code points (char pairs)
        let mut start = 0;
        loop {
            if length == 0 {
                return false;
            }
            if number == 0 {
                return true;
            }
            let c = source[start as usize];
            start += 1;
            if Self::is_lead_surrogate(c as i32)
                && start != length
                && Self::is_trail_surrogate(source[start as usize] as i32)
            {
                start += 1;
                maxsupplementary -= 1;
                if maxsupplementary <= 0 {
                    // too many pairs - too few code points
                    return false;
                }
            }
            number -= 1;
        }
    }

    /// `hasMoreCodePointsThan(char[] source, int start, int limit, int
    /// number)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("Start and limit indexes should be
    /// non-negative and start <= limit")` for an invalid range.
    pub fn has_more_code_points_than_chars(source: Option<&[u16]>, mut start: i32, limit: i32, mut number: i32) -> bool {
        let length = limit - start;
        if length < 0 || start < 0 || limit < 0 {
            index_out_of_bounds(Some(
                "Start and limit indexes should be non-negative and start <= limit".to_string(),
            ));
        }
        if number < 0 {
            return true;
        }
        let Some(source) = source else {
            return false;
        };

        // length >= 0 known
        // source contains at least (length + 1) / 2 code points: <= 2
        // chars per cp
        if ((length + 1) >> 1) > number {
            return true;
        }

        // check if source does not even contain enough chars
        let mut maxsupplementary = length - number;
        if maxsupplementary <= 0 {
            return false;
        }

        // there are maxsupplementary = length - number more chars than
        // asked-for code points

        // count code points until they exceed and also check that there are
        // no more than maxsupplementary supplementary code points (char pairs)
        loop {
            if length == 0 {
                return false;
            }
            if number == 0 {
                return true;
            }
            let c = source[start as usize];
            start += 1;
            if Self::is_lead_surrogate(c as i32)
                && start != limit
                && Self::is_trail_surrogate(source[start as usize] as i32)
            {
                start += 1;
                maxsupplementary -= 1;
                if maxsupplementary <= 0 {
                    // too many pairs - too few code points
                    return false;
                }
            }
            number -= 1;
        }
    }

    /// `newString(int[] codePoints, int offset, int count)`: the string of
    /// the code points `codePoints[offset..offset + count]`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) for a negative `count` or an
    /// invalid code point.
    pub fn new_string(code_points: &[i32], offset: i32, count: i32) -> Result<Vec<u16>> {
        if count < 0 {
            return Err(crate::jdk::illegal_argument_no_message());
        }
        let mut chars = vec![0u16; count as usize];
        let mut w = 0usize;
        let e = offset + count;
        let mut r = offset;
        while r < e {
            let cp = code_points[r as usize];
            if !(0..=0x10ffff).contains(&cp) {
                return Err(crate::jdk::illegal_argument_no_message());
            }
            loop {
                // Java writes and catches the IndexOutOfBoundsException of a
                // full array; the same growth happens on the explicit check.
                let needed = if cp < 0x010000 { 1 } else { 2 };
                if w + needed <= chars.len() {
                    if cp < 0x010000 {
                        chars[w] = cp as u16;
                        w += 1;
                    } else {
                        chars[w] = (Self::LEAD_SURROGATE_OFFSET + (cp >> Self::LEAD_SURROGATE_SHIFT)) as u16;
                        chars[w + 1] = (Self::TRAIL_SURROGATE_MIN_VALUE + (cp & Self::TRAIL_SURROGATE_MASK)) as u16;
                        w += 2;
                    }
                    break;
                } else {
                    let newlen = ((code_points.len() as f64 * (w + 2) as f64) / (r - offset + 1) as f64).ceil() as i32;
                    let mut temp = vec![0u16; newlen as usize];
                    temp[..w].copy_from_slice(&chars[..w]);
                    chars = temp;
                }
            }
            r += 1;
        }
        chars.truncate(w);
        Ok(chars)
    }

    /// `getSingleCodePoint(CharSequence s)`: the code point if `s` holds
    /// exactly one, else -1.
    pub fn get_single_code_point(s: Option<&[u16]>) -> i32 {
        let Some(s) = s else {
            return -1;
        };
        if s.is_empty() {
            return -1;
        } else if s.len() == 1 {
            return s[0] as i32;
        } else if s.len() > 2 {
            return -1;
        }

        // at this point, len = 2
        let cp = crate::jdk::code_point_at(s, 0);
        if cp > 0xFFFF {
            // is surrogate pair
            return cp;
        }
        -1
    }

    /// `compareCodePoint(int codePoint, CharSequence s)`: compares the code
    /// point with the string's first code point, then a string longer than
    /// that code point sorts after it; `null` and the empty string sort
    /// before every code point.
    pub fn compare_code_point(code_point: i32, s: Option<&[u16]>) -> i32 {
        let Some(s) = s else {
            return 1;
        };
        let str_len = s.len() as i32;
        if str_len == 0 {
            return 1;
        }
        let second = crate::jdk::code_point_at(s, 0);
        let diff = code_point - second;
        if diff != 0 {
            return diff;
        }
        if str_len == crate::jdk::char_count(code_point) {
            0
        } else {
            -1
        }
    }

    /// `toString(int ch)` (private): the code point as one or two units,
    /// without validation.
    fn to_string(ch: i32) -> Vec<u16> {
        if ch < Self::SUPPLEMENTARY_MIN_VALUE {
            return vec![ch as u16];
        }

        vec![Self::get_lead_surrogate(ch), Self::get_trail_surrogate(ch)]
    }
}
