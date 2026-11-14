/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The static methods of `java.lang.Character` the module calls.
//!
//! The predicates and `getNumericValue` read the tables of JDK 25 (Unicode
//! 16.0) in `character_data`, generated from the JDK's own answers; the
//! UTF-16 helpers follow the JDK's specification. A code point is an `i32`,
//! as Java's `int` code point is.

use super::character_data::{DIGIT, LETTER, NUMERIC_VALUE, WHITESPACE};
use super::character_type_data::{
    LATIN1_LOWER_CASE, LATIN1_UPPER_CASE, LOWER_CASE, TYPES, UPPER_CASE,
};
use super::unicode_block_data::{BLOCKS, BLOCK_NAMES};
use rucene::error::{LuceneError, Result};

/// `Character.MIN_SUPPLEMENTARY_CODE_POINT`.
pub const MIN_SUPPLEMENTARY_CODE_POINT: i32 = 0x10000;

/// Whether `code_point` lies in one of the inclusive `ranges`, which are
/// sorted and disjoint.
fn in_ranges(ranges: &[(u32, u32)], code_point: i32) -> bool {
    let Ok(c) = u32::try_from(code_point) else {
        return false;
    };
    ranges
        .binary_search_by(|&(first, last)| {
            if last < c {
                std::cmp::Ordering::Less
            } else if first > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `Character.isLetter(int)`: whether the general category of `code_point`
/// is `Lu`, `Ll`, `Lt`, `Lm` or `Lo`, as JDK 25 answers.
pub fn is_letter(code_point: i32) -> bool {
    in_ranges(LETTER, code_point)
}

/// `Character.isDigit(int)`: whether the general category of `code_point`
/// is `Nd`, as JDK 25 answers.
pub fn is_digit(code_point: i32) -> bool {
    in_ranges(DIGIT, code_point)
}

/// `Character.isWhitespace(int)`, as JDK 25 answers: a space, line or
/// paragraph separator other than a no-break space (U+00A0, U+2007, U+202F),
/// or one of U+0009-U+000D and U+001C-U+001F.
///
/// It is not Rust's [`char::is_whitespace`] (Unicode's `White_Space`
/// property), which includes the no-break spaces and U+0085 and excludes
/// U+001C-U+001F.
pub fn is_whitespace(code_point: i32) -> bool {
    in_ranges(WHITESPACE, code_point)
}

/// `Character.getNumericValue(int)`: the `int` value `code_point`
/// represents, `-2` when its value is not a non-negative integer, and `-1`
/// when it has none, as JDK 25 answers (the Latin letters, half- and
/// full-width, count from 10 for `a`/`A`).
pub fn get_numeric_value(code_point: i32) -> i32 {
    let Ok(c) = u32::try_from(code_point) else {
        return -1;
    };
    match NUMERIC_VALUE.binary_search_by(|&(first, last, _, _)| {
        if last < c {
            std::cmp::Ordering::Less
        } else if first > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => {
            let (first, _, value, step) = NUMERIC_VALUE[index];
            if value == -2 {
                -2
            } else {
                value + step * (c - first) as i32
            }
        }
        Err(_) => -1,
    }
}

/// `Character.isUpperCase(int)`: general category `Lu` or the
/// `Other_Uppercase` property, as JDK 25 answers.
pub fn is_upper_case(code_point: i32) -> bool {
    in_ranges(UPPER_CASE, code_point)
}

/// `Character.isLowerCase(int)`: general category `Ll` or the
/// `Other_Lowercase` property, as JDK 25 answers.
pub fn is_lower_case(code_point: i32) -> bool {
    in_ranges(LOWER_CASE, code_point)
}

/// `String.toUpperCase(Locale.ROOT)` of the UTF-16 units `units`: the full,
/// locale-independent Unicode uppercase mapping (`ß` becomes `SS`).
///
/// Each character maps with [`char::to_uppercase`], the Unicode 16.0
/// `UnicodeData` and unconditional `SpecialCasing` mappings JDK 25 applies
/// for the root locale; a lone surrogate, which Java keeps, is kept.
pub fn to_upper_case_root(units: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(units.len());
    let mut buf = [0u16; 2];
    for decoded in char::decode_utf16(units.iter().copied()) {
        match decoded {
            Ok(c) => {
                for upper in c.to_uppercase() {
                    out.extend_from_slice(upper.encode_utf16(&mut buf));
                }
            }
            Err(e) => out.push(e.unpaired_surrogate()),
        }
    }
    out
}

/// `Character.isHighSurrogate(char)`.
pub fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

/// `Character.isLowSurrogate(char)`.
pub fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// `Character.codePointAt(char[], int, int)`: the code point at `index` of
/// the UTF-16 units `a`, a surrogate pair that ends before `limit` joined;
/// any other unit, a lone surrogate included, is taken as it is.
///
/// # Panics
///
/// Where Java throws `IndexOutOfBoundsException`: `index` not below
/// `limit`, or `limit` past the end of `a`.
pub fn code_point_at(a: &[u16], index: usize, limit: usize) -> i32 {
    assert!(
        index < limit && limit <= a.len(),
        "java.lang.IndexOutOfBoundsException: index {index}, limit {limit}, length {}",
        a.len()
    );
    let c1 = a[index];
    if is_high_surrogate(c1) && index + 1 < limit {
        let c2 = a[index + 1];
        if is_low_surrogate(c2) {
            return ((i32::from(c1) - 0xD800) << 10) + (i32::from(c2) - 0xDC00) + 0x10000;
        }
    }
    i32::from(c1)
}

/// The `java.lang.IndexOutOfBoundsException` (no message) the JDK's
/// `Character` UTF-16 helpers throw for a range they cannot read.
fn index_out_of_bounds() -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class: "java.lang.IndexOutOfBoundsException",
        message: None,
        cause: None,
    })
}

/// `Character.codePointAt(char[], int, int)`, raising Java's exception as an
/// error rather than a panic: [`code_point_at`] for a caller (a JFlex
/// scanner) that propagates the JDK's `IndexOutOfBoundsException`.
///
/// # Errors
///
/// `java.lang.IndexOutOfBoundsException`, without a message, when `index`
/// is not below `limit` or `limit` is past the end of `a`.
pub fn checked_code_point_at(a: &[u16], index: usize, limit: usize) -> rucene::error::Result<i32> {
    if index >= limit || limit > a.len() {
        return Err(index_out_of_bounds());
    }
    Ok(code_point_at(a, index, limit))
}

/// `Character.offsetByCodePoints(char[] a, int start, int count, int index,
/// int codePointOffset)`: the index within the subarray `a[start..start +
/// count]` that is `code_point_offset` code points away from `index`. A
/// surrogate pair inside the range counts as one code point, an unpaired
/// surrogate as one code point each.
///
/// # Errors
///
/// `java.lang.IndexOutOfBoundsException`, without a message, as the JDK
/// throws it: the subarray past the end of `a`, `index` outside
/// `start..=start + count`, or fewer code points than `code_point_offset`
/// asks for between `index` and the matching end of the range.
pub fn offset_by_code_points(
    a: &[u16],
    start: usize,
    count: usize,
    index: usize,
    code_point_offset: i32,
) -> rucene::error::Result<usize> {
    if count > a.len().saturating_sub(start)
        || start > a.len()
        || index < start
        || index > start + count
    {
        return Err(index_out_of_bounds());
    }
    let mut x = index;
    if code_point_offset >= 0 {
        let limit = start + count;
        let mut i = 0;
        while x < limit && i < code_point_offset {
            let unit = a[x];
            x += 1;
            if is_high_surrogate(unit) && x < limit && is_low_surrogate(a[x]) {
                x += 1;
            }
            i += 1;
        }
        if i < code_point_offset {
            return Err(index_out_of_bounds());
        }
    } else {
        let mut i = code_point_offset;
        while x > start && i < 0 {
            x -= 1;
            if is_low_surrogate(a[x]) && x > start && is_high_surrogate(a[x - 1]) {
                x -= 1;
            }
            i += 1;
        }
        if i < 0 {
            return Err(index_out_of_bounds());
        }
    }
    Ok(x)
}

/// `Character.charCount(int)`: the number of UTF-16 units `code_point`
/// takes, 2 for a supplementary character and 1 otherwise.
pub fn char_count(code_point: i32) -> usize {
    if code_point >= MIN_SUPPLEMENTARY_CODE_POINT {
        2
    } else {
        1
    }
}

/// The `char` a term buffer holds for `code_point`, which Java writes with
/// `Character.toChars(int, char[], int)`.
///
/// **Divergence.** `rucene`'s `CharTermAttribute` holds Unicode scalar
/// values where Java's holds UTF-16 units (see that trait's documentation): a
/// lone surrogate, which Java stores as it is, cannot be a `char`, and is
/// stored as U+FFFD, the replacement `rucene` applies wherever a lone
/// surrogate meets a `char`.
pub fn to_term_char(code_point: i32) -> char {
    u32::try_from(code_point)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// `Character.UPPERCASE_LETTER`, general category `Lu`.
pub const UPPERCASE_LETTER: u8 = 1;
/// `Character.LOWERCASE_LETTER`, general category `Ll`.
pub const LOWERCASE_LETTER: u8 = 2;
/// `Character.TITLECASE_LETTER`, general category `Lt`.
pub const TITLECASE_LETTER: u8 = 3;
/// `Character.MODIFIER_LETTER`, general category `Lm`.
pub const MODIFIER_LETTER: u8 = 4;
/// `Character.OTHER_LETTER`, general category `Lo`.
pub const OTHER_LETTER: u8 = 5;
/// `Character.NON_SPACING_MARK`, general category `Mn`.
pub const NON_SPACING_MARK: u8 = 6;
/// `Character.ENCLOSING_MARK`, general category `Me`.
pub const ENCLOSING_MARK: u8 = 7;
/// `Character.COMBINING_SPACING_MARK`, general category `Mc`.
pub const COMBINING_SPACING_MARK: u8 = 8;
/// `Character.DECIMAL_DIGIT_NUMBER`, general category `Nd`.
pub const DECIMAL_DIGIT_NUMBER: u8 = 9;
/// `Character.LETTER_NUMBER`, general category `Nl`.
pub const LETTER_NUMBER: u8 = 10;
/// `Character.OTHER_NUMBER`, general category `No`.
pub const OTHER_NUMBER: u8 = 11;
/// `Character.SPACE_SEPARATOR`, general category `Zs`.
pub const SPACE_SEPARATOR: u8 = 12;
/// `Character.FORMAT`, general category `Cf`.
pub const FORMAT: u8 = 16;
/// `Character.SURROGATE`, general category `Cs`.
pub const SURROGATE: u8 = 19;

/// `Character.getType(int)`: the general category of `code_point`, as the
/// value of Java's `Character` category constant, as JDK 25 answers (a
/// surrogate code unit is [`SURROGATE`]; a value outside the code space is
/// `UNASSIGNED`, 0).
pub fn get_type(code_point: i32) -> u8 {
    let Ok(c) = u32::try_from(code_point) else {
        return 0;
    };
    match TYPES.binary_search_by(|&(first, last, _)| {
        if last < c {
            std::cmp::Ordering::Less
        } else if first > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => TYPES[index].2,
        Err(_) => 0,
    }
}

/// `Character.isLowerCase(int)` for a code point below U+0100, as JDK 25
/// answers: general category `Ll` or the `Other_Lowercase` property (so
/// U+00AA and U+00BA too).
///
/// # Panics
///
/// When `code_point` is not below U+0100, which no caller passes.
pub fn is_lower_case_latin1(code_point: i32) -> bool {
    assert!(
        (0..0x100).contains(&code_point),
        "only Latin-1 data is held"
    );
    LATIN1_LOWER_CASE.contains(&(code_point as u8))
}

/// `Character.isUpperCase(int)` for a code point below U+0100, as JDK 25
/// answers.
///
/// # Panics
///
/// When `code_point` is not below U+0100, which no caller passes.
pub fn is_upper_case_latin1(code_point: i32) -> bool {
    assert!(
        (0..0x100).contains(&code_point),
        "only Latin-1 data is held"
    );
    LATIN1_UPPER_CASE.contains(&(code_point as u8))
}

/// The UTF-16 units of `chars`, a term buffer of `char`s, as Java's `char[]`
/// term buffer holds them.
pub fn chars_to_utf16(chars: &[char]) -> Vec<u16> {
    let mut units = Vec::with_capacity(chars.len());
    let mut buf = [0u16; 2];
    for c in chars {
        units.extend_from_slice(c.encode_utf16(&mut buf));
    }
    units
}

/// The number of UTF-16 units of `chars`: Java's `CharTermAttribute.length()`
/// for the same term.
pub fn utf16_length(chars: &[char]) -> usize {
    chars.iter().map(|c| c.len_utf16()).sum()
}

/// The `char`s a term buffer holds for the UTF-16 units `units`.
///
/// **Divergence.** A lone surrogate, which Java's `char[]` holds as it is,
/// cannot be a `char` and becomes U+FFFD, the replacement `rucene` applies
/// wherever a lone surrogate meets a `char` (see [`to_term_char`]).
pub fn utf16_to_term_chars(units: &[u16]) -> Vec<char> {
    char::decode_utf16(units.iter().copied())
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// `Character.toLowerCase(char)`: the simple lowercase mapping of the UTF-16
/// unit `unit` (a surrogate maps to itself), `(char)
/// Character.toLowerCase((int) ch)`.
pub fn to_lower_case_unit(unit: u16) -> u16 {
    rucene::util::automaton::case_folding::to_lower_case(i32::from(unit)) as u16
}

/// `Character.toUpperCase(char)`: the simple uppercase mapping of the UTF-16
/// unit `unit` (a surrogate maps to itself), `(char)
/// Character.toUpperCase((int) ch)`.
pub fn to_upper_case_unit(unit: u16) -> u16 {
    rucene::util::automaton::case_folding::to_upper_case(i32::from(unit)) as u16
}

/// `java.lang.Character.UnicodeBlock`: one block of the Unicode Standard, as
/// JDK 25 (Unicode 16.0) defines them.
///
/// Java's blocks are singletons compared by identity; a block here is its
/// canonical name (`UnicodeBlock.toString()`), which identifies it as
/// uniquely, so `==` answers what Java's `==` answers. The blocks are the
/// JDK's own answers, generated by `tools/translate/JdkUnicodeBlockData.java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnicodeBlock {
    name: &'static str,
}

impl UnicodeBlock {
    /// `Character.UnicodeBlock.DEVANAGARI`.
    pub const DEVANAGARI: UnicodeBlock = UnicodeBlock { name: "DEVANAGARI" };
    /// `Character.UnicodeBlock.BENGALI`.
    pub const BENGALI: UnicodeBlock = UnicodeBlock { name: "BENGALI" };
    /// `Character.UnicodeBlock.GURMUKHI`.
    pub const GURMUKHI: UnicodeBlock = UnicodeBlock { name: "GURMUKHI" };
    /// `Character.UnicodeBlock.GUJARATI`.
    pub const GUJARATI: UnicodeBlock = UnicodeBlock { name: "GUJARATI" };
    /// `Character.UnicodeBlock.ORIYA`.
    pub const ORIYA: UnicodeBlock = UnicodeBlock { name: "ORIYA" };
    /// `Character.UnicodeBlock.TAMIL`.
    pub const TAMIL: UnicodeBlock = UnicodeBlock { name: "TAMIL" };
    /// `Character.UnicodeBlock.TELUGU`.
    pub const TELUGU: UnicodeBlock = UnicodeBlock { name: "TELUGU" };
    /// `Character.UnicodeBlock.KANNADA`.
    pub const KANNADA: UnicodeBlock = UnicodeBlock { name: "KANNADA" };
    /// `Character.UnicodeBlock.MALAYALAM`.
    pub const MALAYALAM: UnicodeBlock = UnicodeBlock { name: "MALAYALAM" };

    /// `Character.UnicodeBlock.of(char)`: the block of the UTF-16 unit `c`,
    /// or `None` (Java's `null`) when it lies in no block.
    pub fn of_char(c: u16) -> Option<UnicodeBlock> {
        Self::lookup(u32::from(c))
    }

    /// `Character.UnicodeBlock.of(int)`: the block of `code_point`, or `None`
    /// (Java's `null`) when it lies in no block.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Not a valid Unicode code point: 0x%X")` when
    /// `code_point` is not a valid code point, as Java throws.
    pub fn of(code_point: i32) -> Result<Option<UnicodeBlock>> {
        match u32::try_from(code_point) {
            Ok(c) if c <= 0x10FFFF => Ok(Self::lookup(c)),
            _ => Err(LuceneError::illegal_argument(format!(
                "Not a valid Unicode code point: 0x{:X}",
                code_point
            ))),
        }
    }

    /// The block of a valid code point, read from the JDK's runs.
    fn lookup(c: u32) -> Option<UnicodeBlock> {
        BLOCKS
            .binary_search_by(|&(first, last, _)| {
                if last < c {
                    std::cmp::Ordering::Less
                } else if first > c {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
            .map(|index| UnicodeBlock {
                name: BLOCK_NAMES[usize::from(BLOCKS[index].2)],
            })
    }
}

impl std::fmt::Display for UnicodeBlock {
    /// `UnicodeBlock.toString()`: the block's canonical name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}
