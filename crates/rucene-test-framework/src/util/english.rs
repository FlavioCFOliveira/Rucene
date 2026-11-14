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

//! Port of `org.apache.lucene.tests.util.English`.

/// Converts numbers to English strings for testing.
///
/// Equivalent to the final class `org.apache.lucene.tests.util.English`
/// (`@lucene.internal`): no instances (its private constructor is the
/// private unit field).
pub struct English(());

impl English {
    /// `longToEnglish(long i)`.
    pub fn long_to_english(i: i64) -> String {
        let mut result = String::new();
        Self::long_to_english_into(i, &mut result);
        result
    }

    /// `longToEnglish(long i, StringBuilder result)`.
    ///
    /// As in Java, `Long.MIN_VALUE` negates to itself (two's complement) and
    /// prints as `"minus "`.
    pub fn long_to_english_into(mut i: i64, result: &mut String) {
        if i == 0 {
            result.push_str("zero");
            return;
        }
        if i < 0 {
            result.push_str("minus ");
            i = i.wrapping_neg();
        }
        if i >= 1_000_000_000_000_000_000 {
            // quadrillion
            Self::long_to_english_into(i / 1_000_000_000_000_000_000, result);
            result.push_str("quintillion, ");
            i %= 1_000_000_000_000_000_000;
        }
        if i >= 1_000_000_000_000_000 {
            // quadrillion
            Self::long_to_english_into(i / 1_000_000_000_000_000, result);
            result.push_str("quadrillion, ");
            i %= 1_000_000_000_000_000;
        }
        if i >= 1_000_000_000_000 {
            // trillions
            Self::long_to_english_into(i / 1_000_000_000_000, result);
            result.push_str("trillion, ");
            i %= 1_000_000_000_000;
        }
        if i >= 1_000_000_000 {
            // billions
            Self::long_to_english_into(i / 1_000_000_000, result);
            result.push_str("billion, ");
            i %= 1_000_000_000;
        }
        if i >= 1_000_000 {
            // millions
            Self::long_to_english_into(i / 1_000_000, result);
            result.push_str("million, ");
            i %= 1_000_000;
        }
        if i >= 1000 {
            // thousands
            Self::long_to_english_into(i / 1000, result);
            result.push_str("thousand, ");
            i %= 1000;
        }
        if i >= 100 {
            // hundreds
            Self::long_to_english_into(i / 100, result);
            result.push_str("hundred ");
            i %= 100;
        }
        // we know we are smaller here so we can cast
        if i >= 20 {
            result.push_str(match (i as i32) / 10 {
                9 => "ninety",
                8 => "eighty",
                7 => "seventy",
                6 => "sixty",
                5 => "fifty",
                4 => "forty",
                3 => "thirty",
                2 => "twenty",
                _ => "",
            });
            i %= 10;
            if i == 0 {
                result.push(' ');
            } else {
                result.push('-');
            }
        }
        result.push_str(match i as i32 {
            19 => "nineteen ",
            18 => "eighteen ",
            17 => "seventeen ",
            16 => "sixteen ",
            15 => "fifteen ",
            14 => "fourteen ",
            13 => "thirteen ",
            12 => "twelve ",
            11 => "eleven ",
            10 => "ten ",
            9 => "nine ",
            8 => "eight ",
            7 => "seven ",
            6 => "six ",
            5 => "five ",
            4 => "four ",
            3 => "three ",
            2 => "two ",
            1 => "one ",
            _ => "",
        });
    }

    /// `intToEnglish(int i)`.
    pub fn int_to_english(i: i32) -> String {
        let mut result = String::new();
        Self::long_to_english_into(i64::from(i), &mut result);
        result
    }

    /// `intToEnglish(int i, StringBuilder result)`.
    pub fn int_to_english_into(i: i32, result: &mut String) {
        Self::long_to_english_into(i64::from(i), result);
    }
}
