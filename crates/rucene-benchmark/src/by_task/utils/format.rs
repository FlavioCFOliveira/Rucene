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

//! The types declared in `Format.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.utils`.

use std::sync::OnceLock;

use rucene_queryparser::jdk::decimal_format::DecimalFormat;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format;

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.Format`:
/// formatting utilities (for reports).
pub struct Format;

/// `padd`.
const PADD: &str = "                                                 ";

/// `numFormat`: three `NumberFormat.getInstance(Locale.ROOT)` with 0, 1 and
/// 2 fraction digits.
fn num_format(num_frac_digits: usize) -> &'static DecimalFormat {
    static NUM_FORMAT: OnceLock<[DecimalFormat; 3]> = OnceLock::new();
    &NUM_FORMAT.get_or_init(|| {
        let make = |digits: i32| {
            let mut f = number_format::get_instance(&Locale::root())
                .expect("Locale.ROOT has a number format");
            f.set_maximum_fraction_digits(digits);
            f.set_minimum_fraction_digits(digits);
            f
        };
        [make(0), make(1), make(2)]
    })[num_frac_digits]
}

/// `numFormat[numFracDigits].format(f)`; `numFracDigits` must be 0, 1 or
/// 2 (Java throws `ArrayIndexOutOfBoundsException` otherwise).
fn format_number(num_frac_digits: i32, f: f64) -> String {
    num_format(num_frac_digits as usize)
        .format_double(f)
        .expect("HALF_EVEN rounding never throws")
}

/// The last `n` UTF-16 units of `s` (`s.substring(s.length() - n)`).
fn tail(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(&units[units.len() - n..])
}

/// The first `n` UTF-16 units of `s` (`s.substring(0, n)`).
fn head(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(&units[..n])
}

fn col_len(col: &str) -> usize {
    crate::jdk::utf16_len(col)
}

impl Format {
    /// `format(int numFracDigits, float f, String col)`: pad a number from
    /// left.
    pub fn format_float(num_frac_digits: i32, f: f32, col: &str) -> String {
        Self::format_double(num_frac_digits, f64::from(f), col)
    }

    /// `format(int numFracDigits, double f, String col)`.
    pub fn format_double(num_frac_digits: i32, f: f64, col: &str) -> String {
        let res = format!("{PADD}{}", format_number(num_frac_digits, f));
        tail(&res, col_len(col))
    }

    /// `formatPaddRight(int numFracDigits, float f, String col)`: pad a
    /// number from right.
    pub fn format_padd_right_float(num_frac_digits: i32, f: f32, col: &str) -> String {
        Self::format_padd_right_double(num_frac_digits, f64::from(f), col)
    }

    /// `formatPaddRight(int numFracDigits, double f, String col)`.
    pub fn format_padd_right_double(num_frac_digits: i32, f: f64, col: &str) -> String {
        let res = format!("{}{PADD}", format_number(num_frac_digits, f));
        head(&res, col_len(col))
    }

    /// `format(int n, String col)`: pad a number from left.
    pub fn format_int(n: i32, col: &str) -> String {
        let res = format!("{PADD}{n}");
        tail(&res, col_len(col))
    }

    /// `format(String s, String col)`: pad a string from right.
    pub fn format_str(s: &str, col: &str) -> String {
        let s1 = format!("{s}{PADD}");
        let n = col_len(col).min(crate::jdk::utf16_len(&s1));
        head(&s1, n)
    }

    /// `formatPaddLeft(String s, String col)`: pad a string from left.
    pub fn format_padd_left(s: &str, col: &str) -> String {
        let res = format!("{PADD}{s}");
        tail(&res, col_len(col))
    }
}
