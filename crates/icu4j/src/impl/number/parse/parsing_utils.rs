// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.ParsingUtils`.

use rucene::error::Result;

use crate::jdk::code_point_at;
use crate::text::unicode_set::UnicodeSet;

/// Parse flags and lead-code-point helpers of the number parser.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.ParsingUtils` (static
/// members only).
pub struct ParsingUtils;

impl ParsingUtils {
    /// `PARSE_FLAG_IGNORE_CASE`.
    pub const PARSE_FLAG_IGNORE_CASE: i32 = 0x0001;
    /// `PARSE_FLAG_MONETARY_SEPARATORS`.
    pub const PARSE_FLAG_MONETARY_SEPARATORS: i32 = 0x0002;
    /// `PARSE_FLAG_STRICT_SEPARATORS`.
    pub const PARSE_FLAG_STRICT_SEPARATORS: i32 = 0x0004;
    /// `PARSE_FLAG_STRICT_GROUPING_SIZE`.
    pub const PARSE_FLAG_STRICT_GROUPING_SIZE: i32 = 0x0008;
    /// `PARSE_FLAG_INTEGER_ONLY`.
    pub const PARSE_FLAG_INTEGER_ONLY: i32 = 0x0010;
    /// `PARSE_FLAG_GROUPING_DISABLED`.
    pub const PARSE_FLAG_GROUPING_DISABLED: i32 = 0x0020;
    // public static final int PARSE_FLAG_FRACTION_GROUPING_ENABLED = 0x0040; // see #10794
    /// `PARSE_FLAG_INCLUDE_UNPAIRED_AFFIXES`.
    pub const PARSE_FLAG_INCLUDE_UNPAIRED_AFFIXES: i32 = 0x0080;
    /// `PARSE_FLAG_USE_FULL_AFFIXES`.
    pub const PARSE_FLAG_USE_FULL_AFFIXES: i32 = 0x0100;
    /// `PARSE_FLAG_EXACT_AFFIX`.
    pub const PARSE_FLAG_EXACT_AFFIX: i32 = 0x0200;
    /// `PARSE_FLAG_PLUS_SIGN_ALLOWED`.
    pub const PARSE_FLAG_PLUS_SIGN_ALLOWED: i32 = 0x0400;
    // public static final int PARSE_FLAG_OPTIMIZE = 0x0800; // no longer used
    /// `PARSE_FLAG_FORCE_BIG_DECIMAL`.
    pub const PARSE_FLAG_FORCE_BIG_DECIMAL: i32 = 0x1000;
    /// `PARSE_FLAG_NO_FOREIGN_CURRENCIES`.
    pub const PARSE_FLAG_NO_FOREIGN_CURRENCIES: i32 = 0x2000;
    /// `PARSE_FLAG_ALLOW_INFINITE_RECURSION`.
    pub const PARSE_FLAG_ALLOW_INFINITE_RECURSION: i32 = 0x4000;
    /// `PARSE_FLAG_STRICT_IGNORABLES`.
    pub const PARSE_FLAG_STRICT_IGNORABLES: i32 = 0x8000;
    /// `PARSE_FLAG_JAVA_COMPATIBILITY_IGNORABLES`.
    pub const PARSE_FLAG_JAVA_COMPATIBILITY_IGNORABLES: i32 = 0x10000;

    /// `putLeadCodePoints(UnicodeSet input, UnicodeSet output)`: adds
    /// every code point of `input` and the first code point of each of its
    /// non-empty strings.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when `output` is frozen.
    pub fn put_lead_code_points(input: &UnicodeSet, output: &mut UnicodeSet) -> Result<()> {
        for range in input.ranges() {
            output.add_range(range.codepoint, range.codepoint_end)?;
        }
        for str in input.strings() {
            if !str.is_empty() {
                output.add(code_point_at(str, 0))?;
            }
        }
        Ok(())
    }

    /// `putLeadCodePoint(String input, UnicodeSet output)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when `output` is frozen.
    pub fn put_lead_code_point(input: &[u16], output: &mut UnicodeSet) -> Result<()> {
        if !input.is_empty() {
            output.add(code_point_at(input, 0))?;
        }
        Ok(())
    }
}
