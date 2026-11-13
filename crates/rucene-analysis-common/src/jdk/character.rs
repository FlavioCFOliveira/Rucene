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
