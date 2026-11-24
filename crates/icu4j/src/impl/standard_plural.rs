// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2015, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.StandardPlural`.

use std::fmt;

use rucene::error::{LuceneError, Result};

/// Standard CLDR plural form/category constants.
///
/// Equivalent to `com.ibm.icu.impl.StandardPlural`. Keywords are given as
/// Rust strings (they are ASCII names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StandardPlural {
    /// `ZERO`.
    Zero,
    /// `ONE`.
    One,
    /// `TWO`.
    Two,
    /// `FEW`.
    Few,
    /// `MANY`.
    Many,
    /// `OTHER`.
    Other,
    /// `EQ_0`: CLDR plural form for an explicit `=0` value.
    Eq0,
    /// `EQ_1`: CLDR plural form for an explicit `=1` value.
    Eq1,
}

impl StandardPlural {
    /// `OTHER_INDEX`: the "other" form's ordinal.
    pub const OTHER_INDEX: i32 = StandardPlural::Other as i32;

    /// `VALUES`: all forms, in ordinal order.
    pub const VALUES: [StandardPlural; 8] = [
        StandardPlural::Zero,
        StandardPlural::One,
        StandardPlural::Two,
        StandardPlural::Few,
        StandardPlural::Many,
        StandardPlural::Other,
        StandardPlural::Eq0,
        StandardPlural::Eq1,
    ];

    /// `COUNT`: the number of standard plural forms/categories.
    pub const COUNT: i32 = 8;

    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// `getKeyword()`: the lowercase CLDR keyword string for the form.
    pub fn get_keyword(self) -> &'static str {
        match self {
            StandardPlural::Zero => "zero",
            StandardPlural::One => "one",
            StandardPlural::Two => "two",
            StandardPlural::Few => "few",
            StandardPlural::Many => "many",
            StandardPlural::Other => "other",
            StandardPlural::Eq0 => "=0",
            StandardPlural::Eq1 => "=1",
        }
    }

    /// `orNullFromString(CharSequence keyword)`: the form for a keyword
    /// (`"0"`/`"1"` too), `None` if not a known keyword.
    pub fn or_null_from_string(keyword: &str) -> Option<StandardPlural> {
        let k: Vec<u16> = keyword.encode_utf16().collect();
        match k.len() {
            1 => {
                if k[0] == b'0' as u16 {
                    return Some(StandardPlural::Eq0);
                } else if k[0] == b'1' as u16 {
                    return Some(StandardPlural::Eq1);
                }
            }
            2 => {
                if keyword == "=0" {
                    return Some(StandardPlural::Eq0);
                } else if keyword == "=1" {
                    return Some(StandardPlural::Eq1);
                }
            }
            3 => {
                if keyword == "one" {
                    return Some(StandardPlural::One);
                } else if keyword == "two" {
                    return Some(StandardPlural::Two);
                } else if keyword == "few" {
                    return Some(StandardPlural::Few);
                }
            }
            4 => {
                if keyword == "many" {
                    return Some(StandardPlural::Many);
                } else if keyword == "zero" {
                    return Some(StandardPlural::Zero);
                }
            }
            5
                if keyword == "other" => {
                    return Some(StandardPlural::Other);
                }
            _ => {}
        }
        None
    }

    /// `orOtherFromString(CharSequence keyword)`: the form, or `OTHER`.
    pub fn or_other_from_string(keyword: &str) -> StandardPlural {
        Self::or_null_from_string(keyword).unwrap_or(StandardPlural::Other)
    }

    /// `fromString(CharSequence keyword)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException(keyword)` if not a known keyword.
    pub fn from_string(keyword: &str) -> Result<StandardPlural> {
        Self::or_null_from_string(keyword).ok_or_else(|| LuceneError::illegal_argument(keyword))
    }

    /// `indexOrNegativeFromString(CharSequence keyword)`: the ordinal, or
    /// -1.
    pub fn index_or_negative_from_string(keyword: &str) -> i32 {
        Self::or_null_from_string(keyword).map_or(-1, StandardPlural::ordinal)
    }

    /// `indexOrOtherIndexFromString(CharSequence keyword)`.
    pub fn index_or_other_index_from_string(keyword: &str) -> i32 {
        Self::or_null_from_string(keyword).map_or(StandardPlural::Other.ordinal(), StandardPlural::ordinal)
    }

    /// `indexFromString(CharSequence keyword)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException(keyword)` if not a known keyword.
    pub fn index_from_string(keyword: &str) -> Result<i32> {
        Self::from_string(keyword).map(StandardPlural::ordinal)
    }
}

/// `toString()`: the constant name (`Enum.toString`).
impl fmt::Display for StandardPlural {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StandardPlural::Zero => "ZERO",
            StandardPlural::One => "ONE",
            StandardPlural::Two => "TWO",
            StandardPlural::Few => "FEW",
            StandardPlural::Many => "MANY",
            StandardPlural::Other => "OTHER",
            StandardPlural::Eq0 => "EQ_0",
            StandardPlural::Eq1 => "EQ_1",
        })
    }
}
