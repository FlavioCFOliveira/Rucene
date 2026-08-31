//! Unicode conversions ported from `org.apache.lucene.util.UnicodeUtil`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`UnicodeUtil`] | `UnicodeUtil` |
//! | [`UTF8CodePoint`] | `UnicodeUtil.UTF8CodePoint` |
//!
//! # Java `char` in Rust
//!
//! A Java `char` is a UTF-16 code unit, not a Unicode scalar value, so every
//! function that Java declares over `char[]` is ported over `&[u16]` and every
//! function declared over `CharSequence` is ported over `&str` read through
//! [`str::encode_utf16`]. That keeps surrogate handling — the entire point of
//! most of these routines — byte-for-byte identical to Lucene's.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::util::BytesRef;

/// Unicode conversion helpers.
///
/// Port of `org.apache.lucene.util.UnicodeUtil`.
pub struct UnicodeUtil;

/// First UTF-16 high-surrogate code unit. `UnicodeUtil.UNI_SUR_HIGH_START`.
pub const UNI_SUR_HIGH_START: u32 = 0xD800;
/// Last UTF-16 high-surrogate code unit. `UnicodeUtil.UNI_SUR_HIGH_END`.
pub const UNI_SUR_HIGH_END: u32 = 0xDBFF;
/// First UTF-16 low-surrogate code unit. `UnicodeUtil.UNI_SUR_LOW_START`.
pub const UNI_SUR_LOW_START: u32 = 0xDC00;
/// Last UTF-16 low-surrogate code unit. `UnicodeUtil.UNI_SUR_LOW_END`.
pub const UNI_SUR_LOW_END: u32 = 0xDFFF;
/// The Unicode replacement character. `UnicodeUtil.UNI_REPLACEMENT_CHAR`.
pub const UNI_REPLACEMENT_CHAR: u32 = 0xFFFD;
/// Maximum number of UTF-8 bytes a single UTF-16 code unit expands to.
///
/// `UnicodeUtil.MAX_UTF8_BYTES_PER_CHAR`.
pub const MAX_UTF8_BYTES_PER_CHAR: usize = 3;

/// Highest code point in the Basic Multilingual Plane.
const UNI_MAX_BMP: u32 = 0x0000_FFFF;
const HALF_SHIFT: u32 = 10;
const HALF_MASK: u32 = 0x3FF;
/// `Character.MIN_SUPPLEMENTARY_CODE_POINT - (UNI_SUR_HIGH_START << 10) - UNI_SUR_LOW_START`.
const SURROGATE_OFFSET: i32 =
    0x1_0000 - ((UNI_SUR_HIGH_START as i32) << HALF_SHIFT) - UNI_SUR_LOW_START as i32;

const LEAD_SURROGATE_SHIFT: u32 = 10;
const TRAIL_SURROGATE_MASK: u32 = 0x3FF;
const TRAIL_SURROGATE_MIN_VALUE: u32 = 0xDC00;
const LEAD_SURROGATE_MIN_VALUE: u32 = 0xD800;
const SUPPLEMENTARY_MIN_VALUE: u32 = 0x10000;
const LEAD_SURROGATE_OFFSET: u32 =
    LEAD_SURROGATE_MIN_VALUE - (SUPPLEMENTARY_MIN_VALUE >> LEAD_SURROGATE_SHIFT);

/// A term that sorts after every legal UTF-8 term.
///
/// Port of `UnicodeUtil.BIG_TERM`. Lucene's own comment notes this constant is
/// unrelated to the rest of the class and should live elsewhere; it is kept
/// here for parity.
pub fn big_term() -> BytesRef {
    BytesRef::new(vec![0xFFu8; 10])
}

/// A decoded UTF-8 code point together with the number of bytes it occupied.
///
/// Port of `UnicodeUtil.UTF8CodePoint`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UTF8CodePoint {
    /// The decoded code point.
    pub code_point: u32,
    /// How many bytes the encoded form occupied.
    pub num_bytes: usize,
}

/// Number of UTF-8 bytes introduced by each possible lead byte, or
/// [`i32::MIN`] when the byte cannot start a sequence.
///
/// Port of `UnicodeUtil.utf8CodeLength`, borrowed by Lucene from Python 3.1.2
/// and modified to reject the 5- and 6-byte sequences reserved by RFC 3629 as
/// well as the invalid bytes `0xFE` and `0xFF`.
static UTF8_CODE_LENGTH: [i32; 256] = build_utf8_code_length();

const fn build_utf8_code_length() -> [i32; 256] {
    let v = i32::MIN;
    let mut table = [v; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = if i < 0x80 {
            1
        } else if i < 0xC0 {
            v
        } else if i < 0xE0 {
            2
        } else if i < 0xF0 {
            3
        } else if i < 0xF8 {
            4
        } else {
            v
        };
        i += 1;
    }
    table
}

impl UnicodeUtil {
    /// Encodes `source[offset..offset + length]` (UTF-16 code units) as UTF-8
    /// into `out`, returning the number of bytes written.
    ///
    /// Unpaired or out-of-order surrogates are replaced by U+FFFD, exactly as
    /// `UnicodeUtil.UTF16toUTF8(char[], int, int, byte[])` does.
    pub fn utf16_to_utf8(source: &[u16], offset: usize, length: usize, out: &mut [u8]) -> usize {
        let mut upto = 0usize;
        let mut i = offset;
        let end = offset + length;

        while i < end {
            let code = source[i] as i32;
            i += 1;

            if code < 0x80 {
                out[upto] = code as u8;
                upto += 1;
            } else if code < 0x800 {
                out[upto] = (0xC0 | (code >> 6)) as u8;
                out[upto + 1] = (0x80 | (code & 0x3F)) as u8;
                upto += 2;
            } else if !(0xD800..=0xDFFF).contains(&code) {
                out[upto] = (0xE0 | (code >> 12)) as u8;
                out[upto + 1] = (0x80 | ((code >> 6) & 0x3F)) as u8;
                out[upto + 2] = (0x80 | (code & 0x3F)) as u8;
                upto += 3;
            } else {
                // Surrogate pair: confirm a valid high surrogate.
                if code < 0xDC00 && i < end {
                    let mut utf32 = source[i] as i32;
                    // Confirm a valid low surrogate and write the pair.
                    if (0xDC00..=0xDFFF).contains(&utf32) {
                        utf32 = (code << 10) + utf32 + SURROGATE_OFFSET;
                        i += 1;
                        out[upto] = (0xF0 | (utf32 >> 18)) as u8;
                        out[upto + 1] = (0x80 | ((utf32 >> 12) & 0x3F)) as u8;
                        out[upto + 2] = (0x80 | ((utf32 >> 6) & 0x3F)) as u8;
                        out[upto + 3] = (0x80 | (utf32 & 0x3F)) as u8;
                        upto += 4;
                        continue;
                    }
                }
                // Replace an unpaired surrogate or an out-of-order low
                // surrogate with the substitution character.
                out[upto] = 0xEF;
                out[upto + 1] = 0xBF;
                out[upto + 2] = 0xBD;
                upto += 3;
            }
        }
        upto
    }

    /// Encodes the UTF-16 view of `s` as UTF-8 into `out` starting at
    /// `out_offset`, returning the position just past the last byte written.
    ///
    /// Port of `UnicodeUtil.UTF16toUTF8(CharSequence, int, int, byte[], int)`.
    pub fn utf16_to_utf8_str(
        s: &str,
        offset: usize,
        length: usize,
        out: &mut [u8],
        out_offset: usize,
    ) -> usize {
        let units: Vec<u16> = s.encode_utf16().collect();
        let written = Self::utf16_to_utf8(&units, offset, length, &mut out[out_offset..]);
        out_offset + written
    }

    /// Returns the number of UTF-8 bytes the UTF-16 range
    /// `[offset, offset + len)` of `s` needs.
    ///
    /// Port of `UnicodeUtil.calcUTF16toUTF8Length`.
    pub fn calc_utf16_to_utf8_length(s: &str, offset: usize, len: usize) -> usize {
        let units: Vec<u16> = s.encode_utf16().collect();
        Self::calc_utf16_to_utf8_length_units(&units, offset, len)
    }

    /// Returns the number of UTF-8 bytes the UTF-16 code units
    /// `[offset, offset + len)` need.
    pub fn calc_utf16_to_utf8_length_units(units: &[u16], offset: usize, len: usize) -> usize {
        let end = offset + len;
        let mut res = 0usize;
        let mut i = offset;
        while i < end {
            let code = units[i] as i32;
            if code < 0x80 {
                res += 1;
            } else if code < 0x800 {
                res += 2;
            } else if !(0xD800..=0xDFFF).contains(&code) {
                res += 3;
            } else {
                if code < 0xDC00 && i < end - 1 {
                    let utf32 = units[i + 1] as i32;
                    if (0xDC00..=0xDFFF).contains(&utf32) {
                        i += 1;
                        res += 4;
                        i += 1;
                        continue;
                    }
                }
                res += 3;
            }
            i += 1;
        }
        res
    }

    /// Returns whether the UTF-16 code units form a well-paired sequence.
    ///
    /// Port of `UnicodeUtil.validUTF16String(char[], int)`.
    pub fn valid_utf16_string(s: &[u16], size: usize) -> bool {
        let mut i = 0usize;
        while i < size {
            let ch = s[i] as u32;
            if (UNI_SUR_HIGH_START..=UNI_SUR_HIGH_END).contains(&ch) {
                if i < size - 1 {
                    i += 1;
                    let next_ch = s[i] as u32;
                    if !(UNI_SUR_LOW_START..=UNI_SUR_LOW_END).contains(&next_ch) {
                        // Unmatched high surrogate.
                        return false;
                    }
                } else {
                    // Unmatched high surrogate.
                    return false;
                }
            } else if (UNI_SUR_LOW_START..=UNI_SUR_LOW_END).contains(&ch) {
                // Unmatched low surrogate.
                return false;
            }
            i += 1;
        }
        true
    }

    /// Counts the code points in a UTF-8 encoded [`BytesRef`].
    ///
    /// Port of `UnicodeUtil.codePointCount`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the bytes are not valid
    /// UTF-8, which is Java's bare `IllegalArgumentException`.
    pub fn code_point_count(utf8: &BytesRef) -> Result<usize> {
        let mut pos = utf8.offset;
        let limit = pos + utf8.length;
        let bytes = &utf8.bytes;

        let mut code_point_count = 0usize;
        while pos < limit {
            let v = bytes[pos] as u32;
            if v < 0x80 {
                pos += 1;
            } else if v >= 0xC0 {
                if v < 0xE0 {
                    pos += 2;
                } else if v < 0xF0 {
                    pos += 3;
                } else if v < 0xF8 {
                    pos += 4;
                } else {
                    // 5- and 6-byte sequences are invalid.
                    return Err(LuceneError::IllegalArgument(
                        "invalid UTF-8 lead byte".to_string(),
                    ));
                }
            } else {
                return Err(LuceneError::IllegalArgument(
                    "invalid UTF-8 lead byte".to_string(),
                ));
            }
            code_point_count += 1;
        }

        // Check we did not go over the limit on the last character.
        if pos > limit {
            return Err(LuceneError::IllegalArgument(
                "truncated UTF-8 sequence".to_string(),
            ));
        }

        Ok(code_point_count)
    }

    /// Decodes a UTF-8 [`BytesRef`] into UTF-32 code points, returning how many
    /// were written.
    ///
    /// Port of `UnicodeUtil.UTF8toUTF32(BytesRef, int[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] on an invalid lead byte.
    pub fn utf8_to_utf32(utf8: &BytesRef, ints: &mut [i32]) -> Result<usize> {
        let mut utf32_count = 0usize;
        let mut utf8_upto = utf8.offset;
        let bytes = &utf8.bytes;
        let utf8_limit = utf8.offset + utf8.length;
        while utf8_upto < utf8_limit {
            let cp = Self::code_point_at(bytes, utf8_upto)?;
            ints[utf32_count] = cp.code_point as i32;
            utf32_count += 1;
            utf8_upto += cp.num_bytes;
        }
        Ok(utf32_count)
    }

    /// Decodes the UTF-8 code point starting at `pos`.
    ///
    /// Port of `UnicodeUtil.codePointAt(byte[], int, UTF8CodePoint)`; Java
    /// threads a reusable instance through the call, which a returned
    /// `Copy` struct makes unnecessary here.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the lead byte cannot start
    /// a UTF-8 sequence.
    pub fn code_point_at(utf8: &[u8], pos: usize) -> Result<UTF8CodePoint> {
        let lead_byte = utf8[pos] as usize;
        let num_bytes = UTF8_CODE_LENGTH[lead_byte];
        let mut v: u32 = match num_bytes {
            1 => {
                return Ok(UTF8CodePoint {
                    code_point: lead_byte as u32,
                    num_bytes: 1,
                })
            }
            2 => (lead_byte & 31) as u32, // 5 useful bits
            3 => (lead_byte & 15) as u32, // 4 useful bits
            4 => (lead_byte & 7) as u32,  // 3 useful bits
            _ => {
                return Err(LuceneError::IllegalArgument(format!(
                    "Invalid UTF8 header byte: 0x{lead_byte:x}"
                )))
            }
        };

        let num_bytes = num_bytes as usize;
        let limit = pos + num_bytes;
        let mut p = pos + 1;
        while p < limit {
            v = (v << 6) | (utf8[p] & 63) as u32;
            p += 1;
        }
        Ok(UTF8CodePoint {
            code_point: v,
            num_bytes,
        })
    }

    /// Builds a [`String`] from `count` code points starting at `offset`.
    ///
    /// Port of `UnicodeUtil.newString(int[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a value is negative, above
    /// `0x10FFFF`, or is an unpaired surrogate — the last of which Java accepts
    /// because a Java `String` may hold lone surrogates and a Rust `String` may
    /// not. That is the only behavioural difference.
    pub fn new_string(code_points: &[i32], offset: usize, count: usize) -> Result<String> {
        let mut out = String::with_capacity(count);
        for &cp in &code_points[offset..offset + count] {
            if !(0..=0x10FFFF).contains(&cp) {
                return Err(LuceneError::IllegalArgument(format!(
                    "invalid code point: {cp}"
                )));
            }
            match char::from_u32(cp as u32) {
                Some(c) => out.push(c),
                None => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "unpaired surrogate code point: {cp}"
                    )))
                }
            }
        }
        Ok(out)
    }

    /// Renders the UTF-16 code units of `s` for debugging.
    ///
    /// Port of `UnicodeUtil.toHexString(String)`.
    pub fn to_hex_string(s: &str) -> String {
        let mut sb = String::new();
        for (i, ch) in s.encode_utf16().enumerate() {
            let ch = ch as u32;
            if i > 0 {
                sb.push(' ');
            }
            if ch < 128 {
                sb.push(char::from_u32(ch).unwrap_or(char::REPLACEMENT_CHARACTER));
            } else {
                if (UNI_SUR_HIGH_START..=UNI_SUR_HIGH_END).contains(&ch) {
                    sb.push_str("H:");
                } else if (UNI_SUR_LOW_START..=UNI_SUR_LOW_END).contains(&ch) {
                    sb.push_str("L:");
                } else if ch > UNI_SUR_LOW_END {
                    if ch == 0xffff {
                        sb.push_str("F:");
                    } else {
                        sb.push_str("E:");
                    }
                }
                sb.push_str("0x");
                sb.push_str(&format!("{ch:x}"));
            }
        }
        sb
    }

    /// Decodes `utf8[offset..offset + length]` into UTF-16 code units, returning
    /// how many were written.
    ///
    /// Port of `UnicodeUtil.UTF8toUTF16(byte[], int, int, char[])`.
    pub fn utf8_to_utf16(utf8: &[u8], offset: usize, length: usize, out: &mut [u16]) -> usize {
        let mut out_offset = 0usize;
        let mut offset = offset;
        let limit = offset + length;
        while offset < limit {
            let b = utf8[offset] as u32;
            offset += 1;
            if b < 0xc0 {
                debug_assert!(b < 0x80);
                out[out_offset] = b as u16;
                out_offset += 1;
            } else if b < 0xe0 {
                out[out_offset] = (((b & 0x1f) << 6) + (utf8[offset] & 0x3f) as u32) as u16;
                offset += 1;
                out_offset += 1;
            } else if b < 0xf0 {
                out[out_offset] = (((b & 0xf) << 12)
                    + (((utf8[offset] & 0x3f) as u32) << 6)
                    + (utf8[offset + 1] & 0x3f) as u32) as u16;
                offset += 2;
                out_offset += 1;
            } else {
                debug_assert!(b < 0xf8, "b = 0x{b:x}");
                let ch = ((b & 0x7) << 18)
                    + (((utf8[offset] & 0x3f) as u32) << 12)
                    + (((utf8[offset + 1] & 0x3f) as u32) << 6)
                    + (utf8[offset + 2] & 0x3f) as u32;
                offset += 3;
                if ch < UNI_MAX_BMP {
                    out[out_offset] = ch as u16;
                    out_offset += 1;
                } else {
                    let ch_half = ch - 0x0010000;
                    out[out_offset] = ((ch_half >> 10) + 0xD800) as u16;
                    out[out_offset + 1] = ((ch_half & HALF_MASK) + 0xDC00) as u16;
                    out_offset += 2;
                }
            }
        }
        out_offset
    }

    /// Decodes a [`BytesRef`] into UTF-16 code units.
    ///
    /// Port of `UnicodeUtil.UTF8toUTF16(BytesRef, char[])`.
    pub fn utf8_to_utf16_ref(bytes_ref: &BytesRef, chars: &mut [u16]) -> usize {
        Self::utf8_to_utf16(&bytes_ref.bytes, bytes_ref.offset, bytes_ref.length, chars)
    }

    /// Returns the maximum number of UTF-8 bytes `utf16_length` code units can
    /// expand to.
    ///
    /// Port of `UnicodeUtil.maxUTF8Length`.
    ///
    /// # Panics
    ///
    /// Panics on overflow, which is Java's `Math.multiplyExact`.
    pub fn max_utf8_length(utf16_length: usize) -> usize {
        utf16_length
            .checked_mul(MAX_UTF8_BYTES_PER_CHAR)
            .expect("INVARIANT: maxUTF8Length overflows, as Math.multiplyExact would")
    }

    /// Encodes a single code point as a UTF-16 code unit pair, appending to
    /// `out`. Used by [`Self::new_string`]-style conversions.
    ///
    /// Not a Lucene method; it exposes the surrogate arithmetic that
    /// `UnicodeUtil.newString` performs inline so callers working in UTF-16 can
    /// reuse it.
    pub fn append_code_point_utf16(cp: u32, out: &mut Vec<u16>) {
        if cp < SUPPLEMENTARY_MIN_VALUE {
            out.push(cp as u16);
        } else {
            out.push((LEAD_SURROGATE_OFFSET + (cp >> LEAD_SURROGATE_SHIFT)) as u16);
            out.push((TRAIL_SURROGATE_MIN_VALUE + (cp & TRAIL_SURROGATE_MASK)) as u16);
        }
    }
}
