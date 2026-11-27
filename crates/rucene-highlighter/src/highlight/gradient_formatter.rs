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

//! Port of `GradientFormatter.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use rucene::error::{LuceneError, Result};

use crate::highlight::formatter::Formatter;
use crate::highlight::token_group::TokenGroup;
use crate::java::{character_digit, substring, units};

/// Formats text with different color intensity depending on the score of the
/// term.
///
/// Equivalent to `org.apache.lucene.search.highlight.GradientFormatter`.
#[derive(Debug, Clone)]
pub struct GradientFormatter {
    max_score: f32,
    pub(crate) fg_r_min: i32,
    pub(crate) fg_g_min: i32,
    pub(crate) fg_b_min: i32,
    pub(crate) fg_r_max: i32,
    pub(crate) fg_g_max: i32,
    pub(crate) fg_b_max: i32,
    /// Equivalent to the protected field `highlightForeground`.
    pub highlight_foreground: bool,
    pub(crate) bg_r_min: i32,
    pub(crate) bg_g_min: i32,
    pub(crate) bg_b_min: i32,
    pub(crate) bg_r_max: i32,
    pub(crate) bg_g_max: i32,
    pub(crate) bg_b_max: i32,
    /// Equivalent to the protected field `highlightBackground`.
    pub highlight_background: bool,
}

const HEX_DIGITS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];

impl GradientFormatter {
    /// Sole constructor; a colour is a hex RGB string such as `#FFFFFF`, and
    /// `None` (Java's `null`) for either colour of a pair disables that
    /// highlighting.
    ///
    /// Equivalent to `new GradientFormatter(float, String, String, String,
    /// String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a colour is not 7 characters long, and
    /// `NumberFormatException` when it is not hexadecimal.
    pub fn new(
        max_score: f32,
        min_foreground_color: Option<&str>,
        max_foreground_color: Option<&str>,
        min_background_color: Option<&str>,
        max_background_color: Option<&str>,
    ) -> Result<Self> {
        let mut this = Self {
            max_score: 0.0,
            fg_r_min: 0,
            fg_g_min: 0,
            fg_b_min: 0,
            fg_r_max: 0,
            fg_g_max: 0,
            fg_b_max: 0,
            highlight_foreground: false,
            bg_r_min: 0,
            bg_g_min: 0,
            bg_b_min: 0,
            bg_r_max: 0,
            bg_g_max: 0,
            bg_b_max: 0,
            highlight_background: false,
        };
        this.highlight_foreground =
            min_foreground_color.is_some() && max_foreground_color.is_some();
        if let (true, Some(min), Some(max)) = (
            this.highlight_foreground,
            min_foreground_color,
            max_foreground_color,
        ) {
            let min = units(min);
            let max = units(max);
            if min.len() != 7 {
                return Err(LuceneError::illegal_argument(
                    "minForegroundColor is not 7 bytes long eg a hex RGB value such as #FFFFFF",
                ));
            }
            if max.len() != 7 {
                return Err(LuceneError::illegal_argument(
                    "minForegroundColor is not 7 bytes long eg a hex RGB value such as #FFFFFF",
                ));
            }
            this.fg_r_min = Self::hex_to_int(&substring(&min, 1, 3)?)?;
            this.fg_g_min = Self::hex_to_int(&substring(&min, 3, 5)?)?;
            this.fg_b_min = Self::hex_to_int(&substring(&min, 5, 7)?)?;

            this.fg_r_max = Self::hex_to_int(&substring(&max, 1, 3)?)?;
            this.fg_g_max = Self::hex_to_int(&substring(&max, 3, 5)?)?;
            this.fg_b_max = Self::hex_to_int(&substring(&max, 5, 7)?)?;
        }

        this.highlight_background =
            min_background_color.is_some() && max_background_color.is_some();
        if let (true, Some(min), Some(max)) = (
            this.highlight_background,
            min_background_color,
            max_background_color,
        ) {
            let min = units(min);
            let max = units(max);
            if min.len() != 7 {
                return Err(LuceneError::illegal_argument(
                    "minBackgroundColor is not 7 bytes long eg a hex RGB value such as #FFFFFF",
                ));
            }
            if max.len() != 7 {
                return Err(LuceneError::illegal_argument(
                    "minBackgroundColor is not 7 bytes long eg a hex RGB value such as #FFFFFF",
                ));
            }
            this.bg_r_min = Self::hex_to_int(&substring(&min, 1, 3)?)?;
            this.bg_g_min = Self::hex_to_int(&substring(&min, 3, 5)?)?;
            this.bg_b_min = Self::hex_to_int(&substring(&min, 5, 7)?)?;

            this.bg_r_max = Self::hex_to_int(&substring(&max, 1, 3)?)?;
            this.bg_g_max = Self::hex_to_int(&substring(&max, 3, 5)?)?;
            this.bg_b_max = Self::hex_to_int(&substring(&max, 5, 7)?)?;
        }
        this.max_score = max_score;
        Ok(this)
    }

    /// Equivalent to the protected `getForegroundColorString(float)`.
    pub fn get_foreground_color_string(&self, score: f32) -> String {
        let r_val = self.get_color_val(self.fg_r_min, self.fg_r_max, score);
        let g_val = self.get_color_val(self.fg_g_min, self.fg_g_max, score);
        let b_val = self.get_color_val(self.fg_b_min, self.fg_b_max, score);
        let mut sb = String::from("#");
        sb.push_str(&Self::int_to_hex(r_val));
        sb.push_str(&Self::int_to_hex(g_val));
        sb.push_str(&Self::int_to_hex(b_val));
        sb
    }

    /// Equivalent to the protected `getBackgroundColorString(float)`.
    pub fn get_background_color_string(&self, score: f32) -> String {
        let r_val = self.get_color_val(self.bg_r_min, self.bg_r_max, score);
        let g_val = self.get_color_val(self.bg_g_min, self.bg_g_max, score);
        let b_val = self.get_color_val(self.bg_b_min, self.bg_b_max, score);
        let mut sb = String::from("#");
        sb.push_str(&Self::int_to_hex(r_val));
        sb.push_str(&Self::int_to_hex(g_val));
        sb.push_str(&Self::int_to_hex(b_val));
        sb
    }

    fn get_color_val(&self, color_min: i32, color_max: i32, score: f32) -> i32 {
        if color_min == color_max {
            return color_min;
        }
        let scale = (color_min - color_max).wrapping_abs() as f32;
        let rel_score_percent = self.max_score.min(score) / self.max_score;
        let col_score = scale * rel_score_percent;
        color_min.min(color_max).wrapping_add(col_score as i32)
    }

    fn int_to_hex(i: i32) -> String {
        let mut s = String::with_capacity(2);
        s.push(HEX_DIGITS[((i & 0xF0) >> 4) as usize]);
        s.push(HEX_DIGITS[(i & 0x0F) as usize]);
        s
    }

    /// Converts a hex string into an int. Integer.parseInt(hex, 16) assumes
    /// the input is nonnegative unless there is a preceding minus sign. This
    /// method reads the input as twos complement instead, so if the input is
    /// 8 bytes long, it will correctly restore a negative int produced by
    /// Integer.toHexString() but not necessarily one produced by
    /// Integer.toString(x,16) since that method will produce a string like
    /// '-FF' for negative integer values.
    ///
    /// Equivalent to the static `hexToInt(String)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` (without a message) when `hex` is longer than
    /// 16 characters or holds a character that is not a hex digit.
    pub fn hex_to_int(hex: &str) -> Result<i32> {
        let hex = units(hex);
        let len = hex.len();
        if len > 16 {
            return Err(LuceneError::number_format(String::new()));
        }

        let mut l: i32 = 0;
        for &unit in &hex {
            l = l.wrapping_shl(4);
            let c = character_digit(unit as i32, 16);
            if c < 0 {
                return Err(LuceneError::number_format(String::new()));
            }
            l |= c;
        }
        Ok(l)
    }
}

impl Formatter for GradientFormatter {
    fn highlight_term(&self, original_text: &str, token_group: &TokenGroup) -> String {
        if token_group.get_total_score() == 0.0 {
            return original_text.to_string();
        }
        let score = token_group.get_total_score();
        if score == 0.0 {
            return original_text.to_string();
        }
        let mut sb = String::new();
        sb.push_str("<font ");
        if self.highlight_foreground {
            sb.push_str("color=\"");
            sb.push_str(&self.get_foreground_color_string(score));
            sb.push_str("\" ");
        }
        if self.highlight_background {
            sb.push_str("bgcolor=\"");
            sb.push_str(&self.get_background_color_string(score));
            sb.push_str("\" ");
        }
        sb.push('>');
        sb.push_str(original_text);
        sb.push_str("</font>");
        sb
    }
}
