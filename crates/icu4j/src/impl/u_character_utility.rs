// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UCharacterUtility`.

/// Internal character utility class for simple data type conversion and
/// String parsing functions.
///
/// Equivalent to `com.ibm.icu.impl.UCharacterUtility`. Java's `byte`
/// arrays are `&[i8]`; the `StringBuffer` targets are `&mut Vec<u16>`.
pub struct UCharacterUtility;

const NON_CHARACTER_SUFFIX_MIN_3_0: i32 = 0xFFFE;
const NON_CHARACTER_MIN_3_1: i32 = 0xFDD0;
const NON_CHARACTER_MAX_3_1: i32 = 0xFDEF;

impl UCharacterUtility {
    /// `isNonCharacter(int ch)`: whether the code point is a noncharacter
    /// (`xxFFFE`/`xxFFFF` or `FDD0..FDEF`).
    pub fn is_non_character(ch: i32) -> bool {
        if (ch & NON_CHARACTER_SUFFIX_MIN_3_0) == NON_CHARACTER_SUFFIX_MIN_3_0 {
            return true;
        }
        (NON_CHARACTER_MIN_3_1..=NON_CHARACTER_MAX_3_1).contains(&ch)
    }

    /// `toInt(char msc, char lsc)` (package-private).
    pub(crate) fn to_int(msc: u16, lsc: u16) -> i32 {
        ((msc as i32) << 16) | lsc as i32
    }

    /// `getNullTermByteSubString(StringBuffer str, byte[] array, int index)`
    /// (package-private): appends the null-terminated bytes at `index` and
    /// returns the index after the terminator.
    pub(crate) fn get_null_term_byte_sub_string(
        str: &mut Vec<u16>,
        array: &[i8],
        mut index: i32,
    ) -> i32 {
        let mut b: i8 = 1;
        while b != 0 {
            b = array[index as usize];
            if b != 0 {
                str.push((b as i32 & 0x00FF) as u16);
            }
            index += 1;
        }
        index
    }

    /// `compareNullTermByteSubString(String str, byte[] array, int strindex,
    /// int aindex)` (package-private): the index in `str` after a match, -1
    /// on mismatch.
    pub(crate) fn compare_null_term_byte_sub_string(
        str: &[u16],
        array: &[i8],
        mut strindex: i32,
        mut aindex: i32,
    ) -> i32 {
        let mut b: i8 = 1;
        let length = str.len() as i32;
        while b != 0 {
            b = array[aindex as usize];
            aindex += 1;
            if b == 0 {
                break;
            }
            // if we have reached the end of the string and yet the array has not
            // reached the end of their substring yet, abort
            if strindex == length || (str[strindex as usize] != (b as i32 & 0xFF) as u16) {
                return -1;
            }
            strindex += 1;
        }
        strindex
    }

    /// `skipNullTermByteSubString(byte[] array, int index, int skipcount)`
    /// (package-private).
    pub(crate) fn skip_null_term_byte_sub_string(
        array: &[i8],
        mut index: i32,
        skipcount: i32,
    ) -> i32 {
        for _ in 0..skipcount {
            let mut b: i8 = 1;
            while b != 0 {
                b = array[index as usize];
                index += 1;
            }
        }
        index
    }

    /// `skipByteSubString(byte[] array, int index, int length, byte
    /// skipend)` (package-private): the number of bytes up to and including
    /// `skipend`.
    pub(crate) fn skip_byte_sub_string(array: &[i8], index: i32, length: i32, skipend: i8) -> i32 {
        let mut result = 0;
        while result < length {
            let b = array[(index + result) as usize];
            if b == skipend {
                result += 1;
                break;
            }
            result += 1;
        }
        result
    }
}
