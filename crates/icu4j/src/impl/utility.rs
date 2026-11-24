// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.Utility`.

use std::sync::LazyLock;

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};

use crate::jdk::{append_code_point, code_point_at, null_pointer, units};
use crate::lang::u_character::UCharacter;
use crate::r#impl::pattern_props::PatternProps;
use crate::text::replaceable::Replaceable;
use crate::text::utf16::UTF16;

/// A Java object as `Utility.arrayEquals(Object, Object)` inspects it at
/// run time: `null`, one of the array types it compares element by
/// element, or another object compared with `equals`.
#[derive(Clone, Copy)]
pub enum JavaObject<'a> {
    /// `null`.
    Null,
    /// An `Object[]`, whose elements are compared recursively.
    Objects(&'a [JavaObject<'a>]),
    /// An `int[]`.
    Ints(&'a [i32]),
    /// A `double[]`.
    Doubles(&'a [f64]),
    /// A `byte[]`.
    Bytes(&'a [i8]),
    /// Any other object, with its `equals(Object)`.
    Other(&'a dyn ObjectEquals),
}

/// `Object.equals(Object)` of an object that is not an array.
pub trait ObjectEquals {
    /// `this.equals(other)`.
    fn equals(&self, other: &JavaObject<'_>) -> bool;
}

/// `APOSTROPHE`.
const APOSTROPHE: u16 = b'\'' as u16;
/// `BACKSLASH`.
const BACKSLASH: u16 = b'\\' as u16;
/// `MAGIC_UNSIGNED`.
const MAGIC_UNSIGNED: i32 = 0x80000000_u32 as i32;
/// `ESCAPE`: the escape character of the run-length encodings.
const ESCAPE: u16 = 0xA5A5;
/// `ESCAPE_BYTE` (package-private).
pub(crate) const ESCAPE_BYTE: i8 = 0xA5_u8 as i8;
/// `HEX_DIGIT` (package-private).
pub(crate) const HEX_DIGIT: [u16; 16] = [
    b'0' as u16,
    b'1' as u16,
    b'2' as u16,
    b'3' as u16,
    b'4' as u16,
    b'5' as u16,
    b'6' as u16,
    b'7' as u16,
    b'8' as u16,
    b'9' as u16,
    b'A' as u16,
    b'B' as u16,
    b'C' as u16,
    b'D' as u16,
    b'E' as u16,
    b'F' as u16,
];
/// `UNESCAPE_MAP`: the C-style escapes, sorted by letter.
const UNESCAPE_MAP: [u16; 16] = [
    /*"   0x22, 0x22 */
    /*'   0x27, 0x27 */
    /*?   0x3F, 0x3F */
    /*\   0x5C, 0x5C */
    /*a*/
    0x61, 0x07, /*b*/ 0x62, 0x08, /*e*/ 0x65, 0x1b, /*f*/ 0x66, 0x0c,
    /*n*/ 0x6E, 0x0a, /*r*/ 0x72, 0x0d, /*t*/ 0x74, 0x09, /*v*/ 0x76, 0x0b,
];
/// `DIGITS` (package-private): the digits of radixes up to 36.
pub(crate) const DIGITS: [u16; 36] = [
    b'0' as u16,
    b'1' as u16,
    b'2' as u16,
    b'3' as u16,
    b'4' as u16,
    b'5' as u16,
    b'6' as u16,
    b'7' as u16,
    b'8' as u16,
    b'9' as u16,
    b'A' as u16,
    b'B' as u16,
    b'C' as u16,
    b'D' as u16,
    b'E' as u16,
    b'F' as u16,
    b'G' as u16,
    b'H' as u16,
    b'I' as u16,
    b'J' as u16,
    b'K' as u16,
    b'L' as u16,
    b'M' as u16,
    b'N' as u16,
    b'O' as u16,
    b'P' as u16,
    b'Q' as u16,
    b'R' as u16,
    b'S' as u16,
    b'T' as u16,
    b'U' as u16,
    b'V' as u16,
    b'W' as u16,
    b'X' as u16,
    b'Y' as u16,
    b'Z' as u16,
];

/// `LINE_SEPARATOR`: `System.getProperty("line.separator")`, the
/// platform's line separator when the property is not set.
pub static LINE_SEPARATOR: LazyLock<String> = LazyLock::new(|| {
    rucene::util::system_property("line.separator").unwrap_or_else(|| {
        if cfg!(windows) {
            "\r\n".into()
        } else {
            "\n".into()
        }
    })
});

/// Miscellaneous ICU utilities: array comparison, run-length string
/// encodings, escaping and unescaping, hexadecimal rendering, pattern
/// parsing helpers and rule quoting.
///
/// Equivalent to `com.ibm.icu.impl.Utility`.
///
/// `LINE_SEPARATOR`, a public mutable static in Java that ICU never
/// reassigns, is the read-only [`LINE_SEPARATOR`] static.
pub struct Utility;

impl Utility {
    // ------------------------------------------------------------------
    // Array comparison
    // ------------------------------------------------------------------

    /// `arrayEquals(Object[] source, Object target)`: whether `target` is an
    /// `Object[]` of the same length and equal elements; a `null` source
    /// equals only a `null` target.
    pub fn array_equals_objects(
        source: Option<&[JavaObject<'_>]>,
        target: &JavaObject<'_>,
    ) -> bool {
        let Some(source) = source else {
            return matches!(target, JavaObject::Null);
        };
        let JavaObject::Objects(targ) = target else {
            return false;
        };
        source.len() == targ.len()
            && Self::array_region_matches_objects(source, 0, targ, 0, source.len() as i32)
    }

    /// `arrayEquals(int[] source, Object target)`.
    pub fn array_equals_ints(source: Option<&[i32]>, target: &JavaObject<'_>) -> bool {
        let Some(source) = source else {
            return matches!(target, JavaObject::Null);
        };
        let JavaObject::Ints(targ) = target else {
            return false;
        };
        source.len() == targ.len()
            && Self::array_region_matches(source, 0, targ, 0, source.len() as i32)
    }

    /// `arrayEquals(double[] source, Object target)`.
    pub fn array_equals_doubles(source: Option<&[f64]>, target: &JavaObject<'_>) -> bool {
        let Some(source) = source else {
            return matches!(target, JavaObject::Null);
        };
        let JavaObject::Doubles(targ) = target else {
            return false;
        };
        source.len() == targ.len()
            && Self::array_region_matches(source, 0, targ, 0, source.len() as i32)
    }

    /// `arrayEquals(byte[] source, Object target)`.
    pub fn array_equals_bytes(source: Option<&[i8]>, target: &JavaObject<'_>) -> bool {
        let Some(source) = source else {
            return matches!(target, JavaObject::Null);
        };
        let JavaObject::Bytes(targ) = target else {
            return false;
        };
        source.len() == targ.len()
            && Self::array_region_matches(source, 0, targ, 0, source.len() as i32)
    }

    /// `arrayEquals(Object source, Object target)`: the array overload for
    /// the source's run-time type, else `source.equals(target)`.
    pub fn array_equals(source: &JavaObject<'_>, target: &JavaObject<'_>) -> bool {
        match source {
            JavaObject::Null => matches!(target, JavaObject::Null),
            // for some reason, the correct arrayEquals is not being called
            // so do it by hand for now.
            JavaObject::Objects(s) => Self::array_equals_objects(Some(s), target),
            JavaObject::Ints(s) => Self::array_equals_ints(Some(s), target),
            JavaObject::Doubles(s) => Self::array_equals_doubles(Some(s), target),
            JavaObject::Bytes(s) => Self::array_equals_bytes(Some(s), target),
            JavaObject::Other(o) => o.equals(target),
        }
    }

    /// `arrayRegionMatches(Object[] source, int sourceStart, Object[]
    /// target, int targetStart, int len)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when a range is outside its array.
    pub fn array_region_matches_objects(
        source: &[JavaObject<'_>],
        source_start: i32,
        target: &[JavaObject<'_>],
        target_start: i32,
        len: i32,
    ) -> bool {
        let source_end = source_start + len;
        let delta = target_start - source_start;
        for i in source_start..source_end {
            if !Self::array_equals(&source[i as usize], &target[(i + delta) as usize]) {
                return false;
            }
        }
        true
    }

    /// `arrayRegionMatches` for `char[]`, `int[]`, `double[]` and `byte[]`:
    /// whether `len` elements from `source_start` equal (`==`) those from
    /// `target_start`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when a range is outside its array.
    pub fn array_region_matches<T: PartialEq>(
        source: &[T],
        source_start: i32,
        target: &[T],
        target_start: i32,
        len: i32,
    ) -> bool {
        let source_end = source_start + len;
        let delta = target_start - source_start;
        for i in source_start..source_end {
            if source[i as usize] != target[(i + delta) as usize] {
                return false;
            }
        }
        true
    }

    /// `sameObjects(Object a, Object b)`: reference identity, for use where
    /// a `==` between objects is intended.
    pub fn same_objects<T: ?Sized>(a: Option<&T>, b: Option<&T>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                std::ptr::eq(a as *const T as *const u8, b as *const T as *const u8)
            }
            _ => false,
        }
    }

    /// `checkCompare(T a, T b)`: `null` sorts first, else `a.compareTo(b)`
    /// (given as `compare_to`).
    pub fn check_compare<T: ?Sized>(
        a: Option<&T>,
        b: Option<&T>,
        compare_to: impl Fn(&T, &T) -> i32,
    ) -> i32 {
        match (a, b) {
            (None, None) => 0,
            (None, Some(_)) => -1,
            (Some(_), None) => 1,
            (Some(a), Some(b)) => compare_to(a, b),
        }
    }

    /// `checkHash(Object a)`: 0 for `null`, else `a.hashCode()` (given as
    /// `hash_code`).
    pub fn check_hash<T: ?Sized>(a: Option<&T>, hash_code: impl Fn(&T) -> i32) -> i32 {
        a.map_or(0, hash_code)
    }

    // ------------------------------------------------------------------
    // Run-length encoding
    // ------------------------------------------------------------------

    /// `arrayToRLEString(int[] a)`: a run-length encoded string of the
    /// array, two units per `int`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for an empty array.
    pub fn array_to_rle_string_ints(a: &[i32]) -> Vec<u16> {
        let mut buffer = Vec::new();

        Self::append_int(&mut buffer, a.len() as i32);
        let mut run_value = a[0];
        let mut run_length = 1;
        for &s in &a[1..] {
            if s == run_value && run_length < 0xFFFF {
                run_length += 1;
            } else {
                Self::encode_run_int(&mut buffer, run_value, run_length);
                run_value = s;
                run_length = 1;
            }
        }
        Self::encode_run_int(&mut buffer, run_value, run_length);
        buffer
    }

    /// `arrayToRLEString(short[] a)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for an empty array.
    pub fn array_to_rle_string_shorts(a: &[i16]) -> Vec<u16> {
        let mut buffer = Vec::new();
        // for (int i=0; i<a.length; ++i) buffer.append((char) a[i]);
        buffer.push((a.len() >> 16) as u16);
        buffer.push(a.len() as u16);
        let mut run_value = a[0];
        let mut run_length = 1;
        for &s in &a[1..] {
            if s == run_value && run_length < 0xFFFF {
                run_length += 1;
            } else {
                Self::encode_run_short(&mut buffer, run_value, run_length);
                run_value = s;
                run_length = 1;
            }
        }
        Self::encode_run_short(&mut buffer, run_value, run_length);
        buffer
    }

    /// `arrayToRLEString(char[] a)`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for an empty array.
    pub fn array_to_rle_string_chars(a: &[u16]) -> Vec<u16> {
        let mut buffer = Vec::new();
        buffer.push((a.len() >> 16) as u16);
        buffer.push(a.len() as u16);
        let mut run_value = a[0];
        let mut run_length = 1;
        for &s in &a[1..] {
            if s == run_value && run_length < 0xFFFF {
                run_length += 1;
            } else {
                Self::encode_run_short(&mut buffer, run_value as i16, run_length);
                run_value = s;
                run_length = 1;
            }
        }
        Self::encode_run_short(&mut buffer, run_value as i16, run_length);
        buffer
    }

    /// `arrayToRLEString(byte[] a)`: two bytes per unit.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for an empty array.
    pub fn array_to_rle_string_bytes(a: &[i8]) -> Vec<u16> {
        let mut buffer = Vec::new();
        buffer.push((a.len() >> 16) as u16);
        buffer.push(a.len() as u16);
        let mut run_value = a[0];
        let mut run_length = 1;
        let mut state = [0i8; 2];
        for &b in &a[1..] {
            if b == run_value && run_length < 0xFF {
                run_length += 1;
            } else {
                Self::encode_run_byte(&mut buffer, run_value, run_length, &mut state);
                run_value = b;
                run_length = 1;
            }
        }
        Self::encode_run_byte(&mut buffer, run_value, run_length, &mut state);

        // We must save the final byte, if there is one, by padding
        // an extra zero.
        if state[0] != 0 {
            Self::append_encoded_byte(&mut buffer, 0, &mut state);
        }

        buffer
    }

    /// `encodeRun(T buffer, int value, int length)` (private).
    fn encode_run_int(buffer: &mut Vec<u16>, value: i32, mut length: i32) {
        if length < 4 {
            for _ in 0..length {
                if value == ESCAPE as i32 {
                    Self::append_int(buffer, value);
                }
                Self::append_int(buffer, value);
            }
        } else {
            if length == ESCAPE as i32 {
                if value == ESCAPE as i32 {
                    Self::append_int(buffer, ESCAPE as i32);
                }
                Self::append_int(buffer, value);
                length -= 1;
            }
            Self::append_int(buffer, ESCAPE as i32);
            Self::append_int(buffer, length);
            Self::append_int(buffer, value); // Don't need to escape this value
        }
    }

    /// `appendInt(T buffer, int value)` (private): the high then the low
    /// unit.
    fn append_int(buffer: &mut Vec<u16>, value: i32) {
        buffer.push(((value as u32) >> 16) as u16);
        buffer.push((value & 0xFFFF) as u16);
    }

    /// `encodeRun(T buffer, short value, int length)` (private).
    fn encode_run_short(buffer: &mut Vec<u16>, value: i16, mut length: i32) {
        let value_char = value as u16;
        if length < 4 {
            for _ in 0..length {
                if value_char == ESCAPE {
                    buffer.push(ESCAPE);
                }
                buffer.push(value_char);
            }
        } else {
            if length == ESCAPE as i32 {
                if value_char == ESCAPE {
                    buffer.push(ESCAPE);
                }
                buffer.push(value_char);
                length -= 1;
            }
            buffer.push(ESCAPE);
            buffer.push(length as u16);
            buffer.push(value_char); // Don't need to escape this value
        }
    }

    /// `encodeRun(T buffer, byte value, int length, byte[] state)` (private).
    fn encode_run_byte(buffer: &mut Vec<u16>, value: i8, mut length: i32, state: &mut [i8; 2]) {
        if length < 4 {
            for _ in 0..length {
                if value == ESCAPE_BYTE {
                    Self::append_encoded_byte(buffer, ESCAPE_BYTE, state);
                }
                Self::append_encoded_byte(buffer, value, state);
            }
        } else {
            if length as i8 == ESCAPE_BYTE {
                if value == ESCAPE_BYTE {
                    Self::append_encoded_byte(buffer, ESCAPE_BYTE, state);
                }
                Self::append_encoded_byte(buffer, value, state);
                length -= 1;
            }
            Self::append_encoded_byte(buffer, ESCAPE_BYTE, state);
            Self::append_encoded_byte(buffer, length as i8, state);
            Self::append_encoded_byte(buffer, value, state); // Don't need to escape this value
        }
    }

    /// `appendEncodedByte(T buffer, byte value, byte[] state)` (private):
    /// pairs bytes into units.
    fn append_encoded_byte(buffer: &mut Vec<u16>, value: i8, state: &mut [i8; 2]) {
        if state[0] != 0 {
            let c = (((state[1] as i32) << 8) | ((value as i32) & 0xFF)) as u16;
            buffer.push(c);
            state[0] = 0;
        } else {
            state[0] = 1;
            state[1] = value;
        }
    }

    /// `RLEStringToIntArray(String s)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Bad run-length encoded int array")` when the
    /// decoded length or the string length does not match.
    pub fn rle_string_to_int_array(s: &[u16]) -> Result<Vec<i32>> {
        let length = Self::get_int(s, 0);
        let mut array = vec![0i32; length as usize];
        let mut ai = 0usize;
        let mut i = 1;

        let max_i = s.len() as i32 / 2;
        while (ai as i32) < length && i < max_i {
            let mut c = Self::get_int(s, i);
            i += 1;

            if c == ESCAPE as i32 {
                c = Self::get_int(s, i);
                i += 1;
                if c == ESCAPE as i32 {
                    array[ai] = c;
                    ai += 1;
                } else {
                    let run_length = c;
                    let run_value = Self::get_int(s, i);
                    i += 1;
                    for _ in 0..run_length {
                        array[ai] = run_value;
                        ai += 1;
                    }
                }
            } else {
                array[ai] = c;
                ai += 1;
            }
        }

        if ai as i32 != length || i != max_i {
            return Err(LuceneError::illegal_state(
                "Bad run-length encoded int array",
            ));
        }

        Ok(array)
    }

    /// `getInt(String s, int i)` (package-private): the `int` of units
    /// `2i` and `2i + 1`.
    pub(crate) fn get_int(s: &[u16], i: i32) -> i32 {
        ((s[(2 * i) as usize] as i32) << 16) | s[(2 * i + 1) as usize] as i32
    }

    /// `RLEStringToShortArray(String s)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Bad run-length encoded short array")`.
    pub fn rle_string_to_short_array(s: &[u16]) -> Result<Vec<i16>> {
        let length = ((s[0] as i32) << 16) | (s[1] as i32);
        let mut array = vec![0i16; length as usize];
        let mut ai = 0usize;
        let mut i = 2usize;
        while i < s.len() {
            let mut c = s[i];
            if c == ESCAPE {
                i += 1;
                c = s[i];
                if c == ESCAPE {
                    array[ai] = c as i16;
                    ai += 1;
                } else {
                    let run_length = c as i32;
                    i += 1;
                    let run_value = s[i] as i16;
                    for _ in 0..run_length {
                        array[ai] = run_value;
                        ai += 1;
                    }
                }
            } else {
                array[ai] = c as i16;
                ai += 1;
            }
            i += 1;
        }

        if ai as i32 != length {
            return Err(LuceneError::illegal_state(
                "Bad run-length encoded short array",
            ));
        }

        Ok(array)
    }

    /// `RLEStringToCharArray(String s)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Bad run-length encoded short array")` (ICU's
    /// message).
    pub fn rle_string_to_char_array(s: &[u16]) -> Result<Vec<u16>> {
        let length = ((s[0] as i32) << 16) | (s[1] as i32);
        let mut array = vec![0u16; length as usize];
        let mut ai = 0usize;
        let mut i = 2usize;
        while i < s.len() {
            let mut c = s[i];
            if c == ESCAPE {
                i += 1;
                c = s[i];
                if c == ESCAPE {
                    array[ai] = c;
                    ai += 1;
                } else {
                    let run_length = c as i32;
                    i += 1;
                    let run_value = s[i];
                    for _ in 0..run_length {
                        array[ai] = run_value;
                        ai += 1;
                    }
                }
            } else {
                array[ai] = c;
                ai += 1;
            }
            i += 1;
        }

        if ai as i32 != length {
            return Err(LuceneError::illegal_state(
                "Bad run-length encoded short array",
            ));
        }

        Ok(array)
    }

    /// `RLEStringToByteArray(String s)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Bad run-length encoded byte array")` or
    /// `("Excess data in RLE byte array string")`.
    pub fn rle_string_to_byte_array(s: &[u16]) -> Result<Vec<i8>> {
        let length = ((s[0] as i32) << 16) | (s[1] as i32);
        let mut array = vec![0i8; length as usize];
        let mut next_char = true;
        let mut c: u16 = 0;
        let mut node = 0;
        let mut run_length = 0i32;
        let mut i = 2usize;
        let mut ai = 0usize;
        while (ai as i32) < length {
            // This part of the loop places the next byte into the local
            // variable 'b' each time through the loop.  It keeps the
            // current character in 'c' and uses the boolean 'nextChar'
            // to see if we've taken both bytes out of 'c' yet.
            let b: i8;
            if next_char {
                c = s[i];
                i += 1;
                b = (c >> 8) as i8;
                next_char = false;
            } else {
                b = (c & 0xFF) as i8;
                next_char = true;
            }

            // This part of the loop is a tiny state machine which handles
            // the parsing of the run-length encoding.  This would be simpler
            // if we could look ahead, but we can't, so we use 'node' to
            // move between three nodes in the state machine.
            match node {
                0 => {
                    // Normal idle node
                    if b == ESCAPE_BYTE {
                        node = 1;
                    } else {
                        array[ai] = b;
                        ai += 1;
                    }
                }
                1 => {
                    // We have seen one ESCAPE_BYTE; we expect either a second
                    // one, or a run length and value.
                    if b == ESCAPE_BYTE {
                        array[ai] = ESCAPE_BYTE;
                        ai += 1;
                        node = 0;
                    } else {
                        run_length = b as i32;
                        // Interpret signed byte as unsigned
                        if run_length < 0 {
                            run_length += 0x100;
                        }
                        node = 2;
                    }
                }
                2 => {
                    // We have seen an ESCAPE_BYTE and length byte.  We interpret
                    // the next byte as the value to be repeated.
                    for _ in 0..run_length {
                        array[ai] = b;
                        ai += 1;
                    }
                    node = 0;
                }
                _ => {}
            }
        }

        if node != 0 {
            return Err(LuceneError::illegal_state(
                "Bad run-length encoded byte array",
            ));
        }

        if i != s.len() {
            return Err(LuceneError::illegal_state(
                "Excess data in RLE byte array string",
            ));
        }

        Ok(array)
    }

    // ------------------------------------------------------------------
    // Source formatting and escaping
    // ------------------------------------------------------------------

    /// `formatForSource(String s)`: the string as Java source literals of
    /// about 80 columns joined with `+` and the line separator.
    pub fn format_for_source(s: &[u16]) -> Vec<u16> {
        let mut buffer: Vec<u16> = Vec::new();
        let mut i = 0usize;
        while i < s.len() {
            if i > 0 {
                buffer.push(b'+' as u16);
                buffer.extend(units(&LINE_SEPARATOR));
            }
            buffer.extend(units("        \""));
            let mut count = 11;
            while i < s.len() && count < 80 {
                let c = s[i];
                i += 1;
                if c < 0x0020 || c == b'"' as u16 || c == b'\\' as u16 {
                    if c == b'\n' as u16 {
                        buffer.extend(units("\\n"));
                        count += 2;
                    } else if c == b'\t' as u16 {
                        buffer.extend(units("\\t"));
                        count += 2;
                    } else if c == b'\r' as u16 {
                        buffer.extend(units("\\r"));
                        count += 2;
                    } else {
                        // Represent control characters, backslash and double quote
                        // using octal notation; otherwise the string we form
                        // won't compile, since Unicode escape sequences are
                        // processed before tokenization.
                        buffer.push(b'\\' as u16);
                        buffer.push(HEX_DIGIT[((c & 0o700) >> 6) as usize]); // HEX_DIGIT works for octal
                        buffer.push(HEX_DIGIT[((c & 0o070) >> 3) as usize]);
                        buffer.push(HEX_DIGIT[(c & 0o007) as usize]);
                        count += 4;
                    }
                } else if c <= 0x007E {
                    buffer.push(c);
                    count += 1;
                } else {
                    buffer.extend(units("\\u"));
                    buffer.push(HEX_DIGIT[((c & 0xF000) >> 12) as usize]);
                    buffer.push(HEX_DIGIT[((c & 0x0F00) >> 8) as usize]);
                    buffer.push(HEX_DIGIT[((c & 0x00F0) >> 4) as usize]);
                    buffer.push(HEX_DIGIT[(c & 0x000F) as usize]);
                    count += 6;
                }
            }
            buffer.push(b'"' as u16);
        }
        buffer
    }

    /// `format1ForSource(String s)`: the string as one Java source literal.
    pub fn format1_for_source(s: &[u16]) -> Vec<u16> {
        let mut buffer: Vec<u16> = Vec::new();
        buffer.push(b'"' as u16);
        for &c in s {
            if c < 0x0020 || c == b'"' as u16 || c == b'\\' as u16 {
                if c == b'\n' as u16 {
                    buffer.extend(units("\\n"));
                } else if c == b'\t' as u16 {
                    buffer.extend(units("\\t"));
                } else if c == b'\r' as u16 {
                    buffer.extend(units("\\r"));
                } else {
                    // Represent control characters, backslash and double quote
                    // using octal notation; otherwise the string we form
                    // won't compile, since Unicode escape sequences are
                    // processed before tokenization.
                    buffer.push(b'\\' as u16);
                    buffer.push(HEX_DIGIT[((c & 0o700) >> 6) as usize]); // HEX_DIGIT works for octal
                    buffer.push(HEX_DIGIT[((c & 0o070) >> 3) as usize]);
                    buffer.push(HEX_DIGIT[(c & 0o007) as usize]);
                }
            } else if c <= 0x007E {
                buffer.push(c);
            } else {
                buffer.extend(units("\\u"));
                buffer.push(HEX_DIGIT[((c & 0xF000) >> 12) as usize]);
                buffer.push(HEX_DIGIT[((c & 0x0F00) >> 8) as usize]);
                buffer.push(HEX_DIGIT[((c & 0x00F0) >> 4) as usize]);
                buffer.push(HEX_DIGIT[(c & 0x000F) as usize]);
            }
        }
        buffer.push(b'"' as u16);
        buffer
    }

    /// `escape(String s)`: non-ASCII and control code points as `\uhhhh` or
    /// `\Uhhhhhhhh`, backslash doubled.
    pub fn escape_string(s: &[u16]) -> Vec<u16> {
        let mut buf: Vec<u16> = Vec::new();
        let mut i = 0i32;
        while (i as usize) < s.len() {
            let c = code_point_at(s, i);
            i += UTF16::get_char_count(c);
            if (b' ' as i32..=0x007F).contains(&c) {
                if c == b'\\' as i32 {
                    buf.extend(units("\\\\")); // That is, "\\"
                } else {
                    buf.push(c as u16);
                }
            } else {
                let four = c <= 0xFFFF;
                buf.extend(units(if four { "\\u" } else { "\\U" }));
                buf.extend(units(&Self::hex_places(c as i64, if four { 4 } else { 8 })));
            }
        }
        buf
    }

    /// `_digit8(int c)` (private).
    fn digit8(c: i32) -> i32 {
        if (b'0' as i32..=b'7' as i32).contains(&c) {
            return c - b'0' as i32;
        }
        -1
    }

    /// `_digit16(int c)` (private).
    fn digit16(c: i32) -> i32 {
        if (b'0' as i32..=b'9' as i32).contains(&c) {
            return c - b'0' as i32;
        }
        if (b'A' as i32..=b'F' as i32).contains(&c) {
            return c - (b'A' as i32 - 10);
        }
        if (b'a' as i32..=b'f' as i32).contains(&c) {
            return c - (b'a' as i32 - 10);
        }
        -1
    }

    /// `unescapeAndLengthAt(CharSequence s, int offset)`: the code point of
    /// the escape sequence starting at `offset` (after its backslash) and
    /// the sequence's length, packed as `cp << 8 | length`; -1 for an
    /// invalid sequence.
    pub fn unescape_and_length_at(s: &[u16], offset: i32) -> i32 {
        Self::unescape_and_length_at_limit(s, offset, s.len() as i32)
    }

    /// `unescapeAndLengthAt(CharSequence s, int offset, int length)`
    /// (private).
    fn unescape_and_length_at_limit(s: &[u16], mut offset: i32, length: i32) -> i32 {
        let mut result = 0i32;
        let mut n = 0;
        let mut min_dig = 0;
        let mut max_dig = 0;
        let mut bits_per_digit = 4;
        let mut dig;
        let mut braces = false;

        // Check that offset is in range
        if offset < 0 || offset >= length {
            return -1;
        }
        let start = offset;

        // Fetch first UChar after '\\'
        let mut c = s[offset as usize] as i32;
        offset += 1;

        // Convert hexadecimal and octal escapes
        match c {
            0x75 /* 'u' */ => {
                min_dig = 4;
                max_dig = 4;
            }
            0x55 /* 'U' */ => {
                min_dig = 8;
                max_dig = 8;
            }
            0x78 /* 'x' */ => {
                min_dig = 1;
                if offset < length && s[offset as usize] == b'{' as u16 {
                    offset += 1;
                    braces = true;
                    max_dig = 8;
                } else {
                    max_dig = 2;
                }
            }
            _ => {
                dig = Self::digit8(c);
                if dig >= 0 {
                    min_dig = 1;
                    max_dig = 3;
                    n = 1; /* Already have first octal digit */
                    bits_per_digit = 3;
                    result = dig;
                }
            }
        }
        if min_dig != 0 {
            while offset < length && n < max_dig {
                c = s[offset as usize] as i32;
                dig = if bits_per_digit == 3 {
                    Self::digit8(c)
                } else {
                    Self::digit16(c)
                };
                if dig < 0 {
                    break;
                }
                result = (result << bits_per_digit) | dig;
                offset += 1;
                n += 1;
            }
            if n < min_dig {
                return -1;
            }
            if braces {
                if c != b'}' as i32 {
                    return -1;
                }
                offset += 1;
            }
            if !(0..0x110000).contains(&result) {
                return -1;
            }
            // If an escape sequence specifies a lead surrogate, see
            // if there is a trail surrogate after it, either as an
            // escape or as a literal.  If so, join them up into a
            // supplementary.
            if offset < length && UTF16::is_lead_surrogate(result) {
                let mut ahead = offset + 1;
                c = s[offset as usize] as i32;
                if c == b'\\' as i32 && ahead < length {
                    // Calling ourselves recursively may cause a stack overflow if
                    // we have repeated escaped lead surrogates.
                    // Limit the length to 11 ("x{0000DFFF}") after ahead.
                    let mut tail_limit = ahead + 11;
                    if tail_limit > length {
                        tail_limit = length;
                    }
                    let cp_and_length = Self::unescape_and_length_at_limit(s, ahead, tail_limit);
                    if cp_and_length >= 0 {
                        c = cp_and_length >> 8;
                        ahead += cp_and_length & 0xff;
                    }
                }
                if UTF16::is_trail_surrogate(c) {
                    offset = ahead;
                    result = UCharacter::to_code_point(result, c);
                }
            }
            return Self::code_point_and_length_range(result, start, offset);
        }

        // Convert C-style escapes in table
        let mut i = 0;
        while i < UNESCAPE_MAP.len() {
            if c == UNESCAPE_MAP[i] as i32 {
                return Self::code_point_and_length_range(
                    UNESCAPE_MAP[i + 1] as i32,
                    start,
                    offset,
                );
            } else if c < UNESCAPE_MAP[i] as i32 {
                break;
            }
            i += 2;
        }

        // Map \cX to control-X: X & 0x1F
        if c == b'c' as i32 && offset < length {
            c = code_point_at(s, offset);
            return Self::code_point_and_length_range(
                c & 0x1F,
                start,
                offset + crate::jdk::char_count(c),
            );
        }

        // If no special forms are recognized, then consider
        // the backslash to generically escape the next character.
        // Deal with surrogate pairs.
        if UTF16::is_lead_surrogate(c) && offset < length {
            let c2 = s[offset as usize] as i32;
            if UTF16::is_trail_surrogate(c2) {
                offset += 1;
                c = UCharacter::to_code_point(c, c2);
            }
        }
        Self::code_point_and_length_range(c, start, offset)
    }

    /// `codePointAndLength(int c, int length)` (private).
    fn code_point_and_length(c: i32, length: i32) -> i32 {
        debug_assert!((0..=0x10ffff).contains(&c));
        debug_assert!((0..=0xff).contains(&length));
        (c << 8) | length
    }

    /// `codePointAndLength(int c, int start, int limit)` (private).
    fn code_point_and_length_range(c: i32, start: i32, limit: i32) -> i32 {
        Self::code_point_and_length(c, limit - start)
    }

    /// `cpFromCodePointAndLength(int cpAndLength)`.
    pub fn cp_from_code_point_and_length(cp_and_length: i32) -> i32 {
        debug_assert!(cp_and_length >= 0);
        cp_and_length >> 8
    }

    /// `lengthFromCodePointAndLength(int cpAndLength)`.
    pub fn length_from_code_point_and_length(cp_and_length: i32) -> i32 {
        debug_assert!(cp_and_length >= 0);
        cp_and_length & 0xff
    }

    /// `unescape(CharSequence s)`: the string with its backslash escapes
    /// converted.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid escape sequence <text>")` for an
    /// invalid escape.
    pub fn unescape(s: &[u16]) -> Result<Vec<u16>> {
        let mut buf: Option<Vec<u16>> = None;
        let mut i = 0usize;
        while i < s.len() {
            let c = s[i];
            i += 1;
            if c == b'\\' as u16 {
                let b = buf.get_or_insert_with(|| {
                    let mut v = Vec::with_capacity(s.len());
                    v.extend_from_slice(&s[..i - 1]);
                    v
                });
                let cp_and_length = Self::unescape_and_length_at(s, i as i32);
                if cp_and_length < 0 {
                    return Err(LuceneError::illegal_argument(format!(
                        "Invalid escape sequence {}",
                        String::from_utf16_lossy(&s[i - 1..(i + 9).min(s.len())])
                    )));
                }
                append_code_point(b, cp_and_length >> 8)?;
                i += (cp_and_length & 0xff) as usize;
            } else if let Some(b) = buf.as_mut() {
                // We could optimize this further by appending whole substrings between escapes.
                b.push(c);
            }
        }
        match buf {
            // No escapes in s.
            None => Ok(s.to_vec()),
            Some(b) => Ok(b),
        }
    }

    /// `unescapeLeniently(CharSequence s)`: as [`unescape`](Self::unescape),
    /// an invalid escape kept as is.
    pub fn unescape_leniently(s: &[u16]) -> Vec<u16> {
        let mut buf: Option<Vec<u16>> = None;
        let mut i = 0usize;
        while i < s.len() {
            let c = s[i];
            i += 1;
            if c == b'\\' as u16 {
                let b = buf.get_or_insert_with(|| {
                    let mut v = Vec::with_capacity(s.len());
                    v.extend_from_slice(&s[..i - 1]);
                    v
                });
                let cp_and_length = Self::unescape_and_length_at(s, i as i32);
                if cp_and_length < 0 {
                    b.push(c);
                } else {
                    append_code_point(b, cp_and_length >> 8)
                        .expect("an unescaped code point is in 0..=0x10ffff");
                    i += (cp_and_length & 0xff) as usize;
                }
            } else if let Some(b) = buf.as_mut() {
                // We could optimize this further by appending whole substrings between escapes.
                b.push(c);
            }
        }
        match buf {
            // No escapes in s.
            None => s.to_vec(),
            Some(b) => b,
        }
    }

    // ------------------------------------------------------------------
    // Hexadecimal rendering
    // ------------------------------------------------------------------

    /// `hex(long ch)`: [`hex_places`](Self::hex_places) with 4 places.
    pub fn hex(ch: i64) -> String {
        Self::hex_places(ch, 4)
    }

    /// `hex(long i, int places)`: upper-case hexadecimal, zero-padded to
    /// `places`, with a leading `-` for a negative value.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when padding is needed and `places`
    /// is above 16.
    pub fn hex_places(mut i: i64, places: i32) -> String {
        if i == i64::MIN {
            return "-8000000000000000".to_string();
        }
        let negative = i < 0;
        if negative {
            i = -i;
        }
        let mut result = format!("{i:X}");
        if (result.len() as i32) < places {
            let pad = "0000000000000000";
            assert!(
                places as usize <= pad.len(),
                "java.lang.StringIndexOutOfBoundsException: begin {}, end {}, length 16",
                result.len(),
                places
            );
            result = format!("{}{}", &pad[result.len()..places as usize], result);
        }
        if negative {
            return format!("-{result}");
        }
        result
    }

    /// `hex(CharSequence s)`: the code points as 4-digit hexadecimal
    /// numbers separated by commas.
    pub fn hex_str(s: &[u16]) -> String {
        let mut out = Vec::new();
        Self::hex_to(s, 4, &units(","), true, &mut out);
        String::from_utf16_lossy(&out)
    }

    /// `hex(S s, int width, U separator, boolean useCodePoints, T result)`:
    /// appends the code points (or code units) of `s` in hexadecimal of at
    /// least `width` digits, separated by `separator`, and returns `result`.
    pub fn hex_to<'r>(
        s: &[u16],
        width: i32,
        separator: &[u16],
        use_code_points: bool,
        result: &'r mut Vec<u16>,
    ) -> &'r mut Vec<u16> {
        if use_code_points {
            let mut i = 0i32;
            while (i as usize) < s.len() {
                let cp = code_point_at(s, i);
                if i != 0 {
                    result.extend_from_slice(separator);
                }
                result.extend(units(&Self::hex_places(cp as i64, width)));
                i += UTF16::get_char_count(cp);
            }
        } else {
            for (i, &c) in s.iter().enumerate() {
                if i != 0 {
                    result.extend_from_slice(separator);
                }
                result.extend(units(&Self::hex_places(c as i64, width)));
            }
        }
        result
    }

    /// `hex(byte[] o, int start, int end, String separator)`.
    pub fn hex_bytes(o: &[i8], start: i32, end: i32, separator: &str) -> String {
        let mut result = String::new();
        for i in start..end {
            if i != 0 {
                result.push_str(separator);
            }
            result.push_str(&Self::hex(o[i as usize] as i64));
        }
        result
    }

    /// `hex(S s, int width, S separator)`: code points.
    pub fn hex_with_separator(s: &[u16], width: i32, separator: &[u16]) -> Vec<u16> {
        let mut out = Vec::new();
        Self::hex_to(s, width, separator, true, &mut out);
        out
    }

    // ------------------------------------------------------------------
    // Splitting and lookup
    // ------------------------------------------------------------------

    /// `split(String s, char divider, String[] output)`: the pieces between
    /// dividers, the remaining output slots set to empty strings.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when there are more pieces than
    /// slots.
    pub fn split_into(s: &[u16], divider: u16, output: &mut [Vec<u16>]) {
        let mut last = 0usize;
        let mut current = 0usize;
        let mut i = 0usize;
        while i < s.len() {
            if s[i] == divider {
                output[current] = s[last..i].to_vec();
                current += 1;
                last = i + 1;
            }
            i += 1;
        }
        output[current] = s[last..i].to_vec();
        current += 1;
        while current < output.len() {
            output[current] = Vec::new();
            current += 1;
        }
    }

    /// `split(String s, char divider)`: the pieces between dividers,
    /// trailing empty pieces included.
    pub fn split(s: &[u16], divider: u16) -> Vec<Vec<u16>> {
        let mut last = 0usize;
        let mut output = Vec::new();
        let mut i = 0usize;
        while i < s.len() {
            if s[i] == divider {
                output.push(s[last..i].to_vec());
                last = i + 1;
            }
            i += 1;
        }
        output.push(s[last..i].to_vec());
        output
    }

    /// `lookup(String source, String[] target)`: the index of `source` in
    /// `target`, or -1.
    pub fn lookup(source: &[u16], target: &[&[u16]]) -> i32 {
        for (i, t) in target.iter().enumerate() {
            if source == *t {
                return i as i32;
            }
        }
        -1
    }

    /// `parseChar(String id, int[] pos, char ch)`: skips white space at
    /// `pos[0]` and then `ch`; on a mismatch restores `pos[0]` and returns
    /// false.
    pub fn parse_char(id: &[u16], pos: &mut [i32], ch: u16) -> bool {
        let start = pos[0];
        pos[0] = PatternProps::skip_white_space(id, pos[0]);
        if pos[0] == id.len() as i32 || id[pos[0] as usize] != ch {
            pos[0] = start;
            return false;
        }
        pos[0] += 1;
        true
    }

    /// `parsePattern(String pat, Replaceable text, int index, int limit)`:
    /// matches `pat` (where `~` is optional white space) against `text`
    /// from `index`; the index after the match, or -1.
    pub fn parse_pattern_replaceable(
        pat: &[u16],
        text: &dyn Replaceable,
        mut index: i32,
        limit: i32,
    ) -> i32 {
        let mut ipat = 0i32;

        // empty pattern matches immediately
        if ipat == pat.len() as i32 {
            return index;
        }

        let mut cpat = code_point_at(pat, ipat);

        while index < limit {
            let c = text.char32_at(index);

            // parse \s*
            if cpat == b'~' as i32 {
                if PatternProps::is_white_space(c) {
                    index += UTF16::get_char_count(c);
                    continue;
                } else {
                    ipat += 1;
                    if ipat == pat.len() as i32 {
                        return index; // success; c unparsed
                    }
                    // fall thru; get next cpat
                }
            }
            // parse literal
            else if c == cpat {
                let n = UTF16::get_char_count(c);
                index += n;
                ipat += n;
                if ipat == pat.len() as i32 {
                    return index; // success; c parsed
                }
                // fall thru; get next cpat
            }
            // match failure of literal
            else {
                return -1;
            }

            cpat = UTF16::char_at(pat, ipat);
        }

        -1 // text ended before end of pat
    }

    // ------------------------------------------------------------------
    // Numbers
    // ------------------------------------------------------------------

    /// `recursiveAppendNumber(T result, int n, int radix, int minDigits)`
    /// (private).
    fn recursive_append_number(result: &mut Vec<u16>, n: i32, radix: i32, min_digits: i32) {
        let digit = n % radix;

        if n >= radix || min_digits > 1 {
            Self::recursive_append_number(result, n / radix, radix, min_digits - 1);
        }

        result.push(DIGITS[digit as usize]);
    }

    /// `appendNumber(T result, int n, int radix, int minDigits)`: appends
    /// `n` in `radix` with at least `min_digits` digits.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal radix <radix>")` outside
    /// `2..=36`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for `i32::MIN`, whose negation
    /// overflows, as in Java.
    pub fn append_number(
        result: &mut Vec<u16>,
        n: i32,
        radix: i32,
        min_digits: i32,
    ) -> Result<&mut Vec<u16>> {
        if !(2..=36).contains(&radix) {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal radix {radix}"
            )));
        }

        let mut abs = n;

        if n < 0 {
            abs = n.wrapping_neg();
            result.push(b'-' as u16);
        }

        Self::recursive_append_number(result, abs, radix, min_digits);

        Ok(result)
    }

    /// `isUnprintable(int c)`: true outside `0x20..=0x7E`.
    pub fn is_unprintable(c: i32) -> bool {
        //0x20 = 32 and 0x7E = 126
        !(0x20..=0x7E).contains(&c)
    }

    /// `shouldAlwaysBeEscaped(int c)`: controls, C1 controls, surrogates,
    /// noncharacters and values above `0x10ffff`.
    pub fn should_always_be_escaped(c: i32) -> bool {
        if c < 0x20 {
            true // C0 control codes
        } else if c <= 0x7e {
            false // printable ASCII
        } else if c <= 0x9f {
            true // C1 control codes
        } else if c < 0xd800 {
            false // most of the BMP
        } else if c <= 0xdfff || (0xfdd0..=0xfdef).contains(&c) || (c & 0xfffe) == 0xfffe {
            true // surrogate or noncharacter code points
        } else {
            c > 0x10ffff // non-code point integer value
        }
    }

    /// `escapeUnprintable(T result, int c)`: escapes `c` when it is
    /// unprintable; returns whether it did.
    pub fn escape_unprintable(result: &mut Vec<u16>, c: i32) -> bool {
        if Self::is_unprintable(c) {
            Self::escape(result, c);
            return true;
        }
        false
    }

    /// `escape(T result, int c)`: appends `\uhhhh` or `\Uhhhhhhhh`.
    pub fn escape(result: &mut Vec<u16>, c: i32) -> &mut Vec<u16> {
        result.push(b'\\' as u16);
        if (c & !0xFFFF) != 0 {
            result.push(b'U' as u16);
            result.push(DIGITS[(0xF & (c >> 28)) as usize]);
            result.push(DIGITS[(0xF & (c >> 24)) as usize]);
            result.push(DIGITS[(0xF & (c >> 20)) as usize]);
            result.push(DIGITS[(0xF & (c >> 16)) as usize]);
        } else {
            result.push(b'u' as u16);
        }
        result.push(DIGITS[(0xF & (c >> 12)) as usize]);
        result.push(DIGITS[(0xF & (c >> 8)) as usize]);
        result.push(DIGITS[(0xF & (c >> 4)) as usize]);
        result.push(DIGITS[(0xF & c) as usize]);
        result
    }

    /// `quotedIndexOf(String text, int start, int limit, String
    /// setOfChars)`: the first index in `start..limit` of a unit of
    /// `set_of_chars` outside quotes and backslash escapes, or -1.
    pub fn quoted_index_of(text: &[u16], start: i32, limit: i32, set_of_chars: &[u16]) -> i32 {
        let mut i = start;
        while i < limit {
            let c = text[i as usize];
            if c == BACKSLASH {
                i += 1;
            } else if c == APOSTROPHE {
                loop {
                    i += 1;
                    if !(i < limit && text[i as usize] != APOSTROPHE) {
                        break;
                    }
                }
            } else if set_of_chars.contains(&c) {
                return i;
            }
            i += 1;
        }
        -1
    }

    /// `appendToRule(StringBuffer rule, int c, boolean isLiteral, boolean
    /// escapeUnprintable, StringBuffer quoteBuf)`: appends `c` to a rule,
    /// quoting and escaping it as needed; `c == -1` flushes `quote_buf`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from `appendCodePoint` for an invalid code
    /// point.
    pub fn append_to_rule(
        rule: &mut Vec<u16>,
        c: i32,
        is_literal: bool,
        escape_unprintable: bool,
        quote_buf: &mut Vec<u16>,
    ) -> Result<()> {
        // If we are escaping unprintables, then escape them outside
        // quotes.  \\u and \\U are not recognized within quotes.  The same
        // logic applies to literals, but literals are never escaped.
        if is_literal || (escape_unprintable && Utility::is_unprintable(c)) {
            if !quote_buf.is_empty() {
                // We prefer backslash APOSTROPHE to double APOSTROPHE
                // (more readable, less similar to ") so if there are
                // double APOSTROPHEs at the ends, we pull them outside
                // of the quote.

                // If the first thing in the quoteBuf is APOSTROPHE
                // (doubled) then pull it out.
                while quote_buf.len() >= 2
                    && quote_buf[0] == APOSTROPHE
                    && quote_buf[1] == APOSTROPHE
                {
                    rule.push(BACKSLASH);
                    rule.push(APOSTROPHE);
                    quote_buf.drain(0..2);
                }
                // If the last thing in the quoteBuf is APOSTROPHE
                // (doubled) then remove and count it and add it after.
                let mut trailing_count = 0;
                while quote_buf.len() >= 2
                    && quote_buf[quote_buf.len() - 2] == APOSTROPHE
                    && quote_buf[quote_buf.len() - 1] == APOSTROPHE
                {
                    quote_buf.truncate(quote_buf.len() - 2);
                    trailing_count += 1;
                }
                if !quote_buf.is_empty() {
                    rule.push(APOSTROPHE);
                    rule.extend_from_slice(quote_buf);
                    rule.push(APOSTROPHE);
                    quote_buf.clear();
                }
                while trailing_count > 0 {
                    trailing_count -= 1;
                    rule.push(BACKSLASH);
                    rule.push(APOSTROPHE);
                }
            }
            if c != -1 {
                // Since spaces are ignored during parsing, they are
                // emitted only for readability.  We emit one here
                // only if there isn't already one at the end of the
                // rule.
                if c == b' ' as i32 {
                    let len = rule.len();
                    if len > 0 && rule[len - 1] != b' ' as u16 {
                        rule.push(b' ' as u16);
                    }
                } else if !escape_unprintable || !Utility::escape_unprintable(rule, c) {
                    append_code_point(rule, c)?;
                }
            }
        }
        // Escape ' and '\' and don't begin a quote just for them
        else if quote_buf.is_empty() && (c == APOSTROPHE as i32 || c == BACKSLASH as i32) {
            rule.push(BACKSLASH);
            rule.push(c as u16);
        }
        // Specials (printable ascii that isn't [0-9a-zA-Z]) and
        // whitespace need quoting.  Also append stuff to quotes if we are
        // building up a quoted substring already.
        else if !quote_buf.is_empty()
            || ((0x0021..=0x007E).contains(&c)
                && !((0x0030..=0x0039).contains(&c)
                    || (0x0041..=0x005A).contains(&c)
                    || (0x0061..=0x007A).contains(&c)))
            || PatternProps::is_white_space(c)
        {
            append_code_point(quote_buf, c)?;
            // Double ' within a quote
            if c == APOSTROPHE as i32 {
                quote_buf.push(c as u16);
            }
        }
        // Otherwise just append
        else {
            append_code_point(rule, c)?;
        }
        Ok(())
    }

    /// `appendToRule(StringBuffer rule, String text, boolean isLiteral,
    /// boolean escapeUnprintable, StringBuffer quoteBuf)`: each unit of
    /// `text`.
    ///
    /// # Errors
    ///
    /// As [`append_to_rule`](Self::append_to_rule).
    pub fn append_to_rule_str(
        rule: &mut Vec<u16>,
        text: &[u16],
        is_literal: bool,
        escape_unprintable: bool,
        quote_buf: &mut Vec<u16>,
    ) -> Result<()> {
        for &c in text {
            // Okay to process in 16-bit code units here
            Self::append_to_rule(rule, c as i32, is_literal, escape_unprintable, quote_buf)?;
        }
        Ok(())
    }

    /// `compareUnsigned(int source, int target)`: -1, 0 or 1 comparing the
    /// values as unsigned.
    pub fn compare_unsigned(mut source: i32, mut target: i32) -> i32 {
        source = source.wrapping_add(MAGIC_UNSIGNED);
        target = target.wrapping_add(MAGIC_UNSIGNED);
        if source < target {
            -1
        } else if source > target {
            1
        } else {
            0
        }
    }

    /// `highBit(int n)`: the index of the highest set bit, -1 for `n <= 0`.
    pub fn high_bit(mut n: i32) -> i8 {
        if n <= 0 {
            return -1;
        }

        let mut bit: i8 = 0;

        if n >= 1 << 16 {
            n >>= 16;
            bit += 16;
        }

        if n >= 1 << 8 {
            n >>= 8;
            bit += 8;
        }

        if n >= 1 << 4 {
            n >>= 4;
            bit += 4;
        }

        if n >= 1 << 2 {
            n >>= 2;
            bit += 2;
        }

        if n >= 1 << 1 {
            bit += 1;
        }

        bit
    }

    /// `valueOf(int[] source)`: the string of the code points.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn value_of(source: &[i32]) -> Result<Vec<u16>> {
        // TODO: Investigate why this method is not on UTF16 class
        let mut result = Vec::with_capacity(source.len());
        for &cp in source {
            append_code_point(&mut result, cp)?;
        }
        Ok(result)
    }

    /// `repeat(String s, int count)`: `s` repeated `count` times.
    pub fn repeat(s: &[u16], count: i32) -> Vec<u16> {
        if count <= 0 {
            return Vec::new();
        }
        if count == 1 {
            return s.to_vec();
        }
        let mut result = Vec::new();
        for _ in 0..count {
            result.extend_from_slice(s);
        }
        result
    }

    /// `splitString(String src, String target)`:
    /// `src.split("\\Q" + target + "\\E")`, a split on the literal `target` with `String.split`'s
    /// rules (no leading empty piece for a zero-width match at the start,
    /// trailing empty pieces dropped, the whole input when nothing matches).
    pub fn split_string(src: &[u16], target: &[u16]) -> Vec<Vec<u16>> {
        let mut list: Vec<Vec<u16>> = Vec::new();
        let mut index = 0usize;
        let mut matched = false;
        let mut pos = 0usize;
        let n = src.len();
        let m = target.len();
        while pos + m <= n {
            if src[pos..pos + m] == *target {
                if m == 0 && pos == 0 {
                    // no empty leading substring for a zero-width match at
                    // the beginning
                    pos += 1;
                    continue;
                }
                list.push(src[index..pos].to_vec());
                matched = true;
                index = pos + m;
                pos = if m == 0 { pos + 1 } else { pos + m };
            } else {
                pos += 1;
            }
        }
        if !matched {
            return vec![src.to_vec()];
        }
        list.push(src[index..].to_vec());
        while list.last().is_some_and(Vec::is_empty) {
            list.pop();
        }
        list
    }

    /// `splitWhitespace(String src)`: `src.split("\\s+")`.
    pub fn split_whitespace(src: &[u16]) -> Vec<Vec<u16>> {
        static WS: LazyLock<JavaPattern> =
            LazyLock::new(|| JavaPattern::compile("\\s+", 0).expect("\\s+ is a valid pattern"));
        WS.split(src, 0)
    }

    /// `fromHex(String string, int minLength, String separator)`: the code
    /// points written in hexadecimal between matches of `separator`
    /// (`\s+` when `None`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid separator pattern or a part
    /// shorter than `min_length`; `NumberFormatException` for a part that is
    /// not hexadecimal.
    pub fn from_hex(string: &[u16], min_length: i32, separator: Option<&str>) -> Result<Vec<u16>> {
        let pattern = JavaPattern::compile(separator.unwrap_or("\\s+"), 0)
            .map_err(|e| LuceneError::illegal_argument(e.to_string()))?;
        Self::from_hex_pattern(string, min_length, &pattern)
    }

    /// `fromHex(String string, int minLength, Pattern separator)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("code point too short: <part>")`;
    /// `NumberFormatException` for a part that is not hexadecimal;
    /// `IllegalArgumentException` from `appendCodePoint`.
    pub fn from_hex_pattern(
        string: &[u16],
        min_length: i32,
        separator: &JavaPattern,
    ) -> Result<Vec<u16>> {
        let mut buffer = Vec::new();
        let parts = separator.split(string, 0);
        for part in parts {
            if (part.len() as i32) < min_length {
                return Err(LuceneError::illegal_argument(format!(
                    "code point too short: {}",
                    String::from_utf16_lossy(&part)
                )));
            }
            let cp = crate::jdk::parse_int(&part, 16)?;
            append_code_point(&mut buffer, cp)?;
        }
        Ok(buffer)
    }

    /// `addExact(int x, int y)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("integer overflow")`.
    pub fn add_exact(x: i32, y: i32) -> Result<i32> {
        let r = x.wrapping_add(y);
        // HD 2-12 Overflow iff both arguments have the opposite sign of the result
        if ((x ^ r) & (y ^ r)) < 0 {
            return Err(crate::jdk::arithmetic("integer overflow"));
        }
        Ok(r)
    }

    /// `charSequenceEquals(CharSequence a, CharSequence b)`: equal units,
    /// `null` equal only to `null`.
    pub fn char_sequence_equals(a: Option<&[u16]>, b: Option<&[u16]>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// `charSequenceHashCode(CharSequence value)`: `String.hashCode()` of
    /// the units.
    pub fn char_sequence_hash_code(value: &[u16]) -> i32 {
        let mut hash = 0i32;
        for &c in value {
            hash = hash.wrapping_mul(31).wrapping_add(c as i32);
        }
        hash
    }

    /// `appendTo(CharSequence string, A appendable)`.
    pub fn append_to<'a>(string: &[u16], appendable: &'a mut Vec<u16>) -> &'a mut Vec<u16> {
        appendable.extend_from_slice(string);
        appendable
    }

    /// `joinStrings(CharSequence delimiter, Iterable<? extends CharSequence>
    /// elements)`: the non-null elements joined with the delimiter.
    ///
    /// # Errors
    ///
    /// `NullPointerException("Delimiter or elements is null")`.
    pub fn join_strings<'e>(
        delimiter: Option<&[u16]>,
        elements: Option<impl IntoIterator<Item = Option<&'e [u16]>>>,
    ) -> Result<Vec<u16>> {
        let (Some(delimiter), Some(elements)) = (delimiter, elements) else {
            return Err(null_pointer("Delimiter or elements is null"));
        };
        let mut buf = Vec::new();
        let mut is_first_elem = true;
        for element in elements.into_iter().flatten() {
            if !is_first_elem {
                buf.extend_from_slice(delimiter);
            } else {
                is_first_elem = false;
            }
            buf.extend_from_slice(element);
        }
        Ok(buf)
    }
}

impl Utility {
    /// `parsePattern(String rule, int pos, int limit, String pattern, int[]
    /// parsedInts)`: parses a pattern string (`' '` one white space then
    /// optional ones, `'~'` optional white space, `'#'` an integer, other
    /// characters lowercase-insensitive literals); returns the position
    /// after the match, or -1.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `parsed_ints` is too short.
    pub fn parse_pattern(
        rule: &[u16],
        mut pos: i32,
        limit: i32,
        pattern: &[u16],
        parsed_ints: &mut [i32],
    ) -> i32 {
        // TODO Update this to handle surrogates
        let mut p = [0i32; 1];
        let mut int_count = 0usize; // number of integers parsed
        for &cpat in pattern {
            match cpat {
                0x20 /* ' ' */ => {
                    if pos >= limit {
                        return -1;
                    }
                    let c = rule[pos as usize];
                    pos += 1;
                    if !PatternProps::is_white_space(c as i32) {
                        return -1;
                    }
                    // FALL THROUGH to skipWhitespace
                    pos = PatternProps::skip_white_space(rule, pos);
                }
                0x7E /* '~' */ => {
                    pos = PatternProps::skip_white_space(rule, pos);
                }
                0x23 /* '#' */ => {
                    p[0] = pos;
                    parsed_ints[int_count] = Self::parse_integer(rule, &mut p, limit);
                    int_count += 1;
                    if p[0] == pos {
                        // Syntax error; failed to parse integer
                        return -1;
                    }
                    pos = p[0];
                }
                _ => {
                    if pos >= limit {
                        return -1;
                    }
                    let c = crate::lang::u_character::UCharacter::to_lower_case_cp(rule[pos as usize] as i32) as u16;
                    pos += 1;
                    if c != cpat {
                        return -1;
                    }
                }
            }
        }
        pos
    }

    /// `parseInteger(String rule, int[] pos, int limit)`: parses a decimal,
    /// octal (leading `0`) or hex (leading `0x`) integer at `pos[0]`,
    /// advancing it past the digits; 0 on overflow.
    pub fn parse_integer(rule: &[u16], pos: &mut [i32], limit: i32) -> i32 {
        let mut count = 0;
        let mut value: i32 = 0;
        let mut p = pos[0];
        let mut radix = 10;

        let ox: [u16; 2] = [b'0' as u16, b'x' as u16];
        let matches_ox = p >= 0
            && (p as usize) + 2 <= rule.len()
            && rule[p as usize] == ox[0]
            && (rule[p as usize + 1] == ox[1]
                || rucene_analysis_common::jdk::character::to_upper_case_unit(
                    rule[p as usize + 1],
                ) == b'X' as u16
                || rucene_analysis_common::jdk::character::to_lower_case_unit(
                    rucene_analysis_common::jdk::character::to_upper_case_unit(
                        rule[p as usize + 1],
                    ),
                ) == b'x' as u16);
        if matches_ox {
            p += 2;
            radix = 16;
        } else if p < limit && rule[p as usize] == b'0' as u16 {
            p += 1;
            count = 1;
            radix = 8;
        }

        while p < limit {
            let d =
                crate::lang::u_character::UCharacter::digit_radix(rule[p as usize] as i32, radix);
            p += 1;
            if d < 0 {
                p -= 1;
                break;
            }
            count += 1;
            let v = value.wrapping_mul(radix).wrapping_add(d);
            if v <= value {
                // If there are too many input digits, at some point
                // the value will go negative, e.g., if we have seen
                // "0x8000000" already and there is another '0', when
                // we parse the next 0 the value will go negative.
                return 0;
            }
            value = v;
        }
        if count > 0 {
            pos[0] = p;
        }
        value
    }

    /// `parseUnicodeIdentifier(String str, int[] pos)`: the Unicode
    /// identifier at `pos[0]` (advancing it), `None` when none starts
    /// there.
    pub fn parse_unicode_identifier(str: &[u16], pos: &mut [i32]) -> Option<Vec<u16>> {
        // assert(pos[0] < str.length());
        let mut buf: Vec<u16> = Vec::new();
        let mut p = pos[0];
        while p < str.len() as i32 {
            let ch = crate::jdk::code_point_at(str, p);
            if buf.is_empty() {
                if crate::lang::u_character::UCharacter::is_unicode_identifier_start(ch) {
                    crate::jdk::append_code_point(&mut buf, ch)
                        .expect("a code point read from text");
                } else {
                    return None;
                }
            } else if crate::lang::u_character::UCharacter::is_unicode_identifier_part(ch) {
                crate::jdk::append_code_point(&mut buf, ch).expect("a code point read from text");
            } else {
                break;
            }
            p += UTF16::get_char_count(ch);
        }
        pos[0] = p;
        Some(buf)
    }

    /// `parseNumber(String text, int[] pos, int radix)`: parses an unsigned
    /// number in the radix at `pos[0]` (advancing it); -1 when there is no
    /// digit or on overflow.
    pub fn parse_number(text: &[u16], pos: &mut [i32], radix: i32) -> i32 {
        // assert(pos[0] >= 0);
        // assert(radix >= 2);
        // assert(radix <= 36);
        let mut n: i32 = 0;
        let mut p = pos[0];
        while p < text.len() as i32 {
            let ch = crate::jdk::code_point_at(text, p);
            let d = crate::lang::u_character::UCharacter::digit_radix(ch, radix);
            if d < 0 {
                break;
            }
            n = radix.wrapping_mul(n).wrapping_add(d);
            // ASSUME that when a 32-bit integer overflows it becomes
            // negative.  E.g., 214748364 * 10 + 8 => negative value.
            if n < 0 {
                return -1;
            }
            p += 1;
        }
        if p == pos[0] {
            return -1;
        }
        pos[0] = p;
        n
    }

    /// `appendToRule(StringBuffer rule, UnicodeMatcher matcher, boolean
    /// escapeUnprintable, StringBuffer quoteBuf)`: appends the matcher's
    /// pattern as a literal (nothing for `None`).
    ///
    /// # Errors
    ///
    /// As [`append_to_rule`](Self::append_to_rule).
    pub fn append_to_rule_matcher(
        rule: &mut Vec<u16>,
        matcher: Option<&dyn crate::text::unicode_matcher::UnicodeMatcher>,
        escape_unprintable: bool,
        quote_buf: &mut Vec<u16>,
    ) -> Result<()> {
        if let Some(matcher) = matcher {
            Self::append_to_rule_str(
                rule,
                &matcher.to_pattern(escape_unprintable),
                true,
                escape_unprintable,
                quote_buf,
            )?;
        }
        Ok(())
    }
}
