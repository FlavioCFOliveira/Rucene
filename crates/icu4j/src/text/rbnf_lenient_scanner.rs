// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RbnfLenientScanner`.

/// Used by `RuleBasedNumberFormat` to do lenient parsing.
///
/// Equivalent to the deprecated interface
/// `com.ibm.icu.text.RbnfLenientScanner`.
pub trait RbnfLenientScanner: std::fmt::Debug + Send + Sync {
    /// `allIgnorable(String s)`: whether every character of `s` is
    /// ignorable.
    fn all_ignorable(&self, s: &[u16]) -> bool;

    /// `prefixLength(String str, String prefix)`: the number of units of
    /// `str` the prefix matches, 0 when it does not.
    fn prefix_length(&self, str: &[u16], prefix: &[u16]) -> i32;

    /// `findText(String str, String key, int startingAt)`: the position
    /// and length of the match of `key` in `str`, `[-1, 0]` when none.
    fn find_text(&self, str: &[u16], key: &[u16], starting_at: i32) -> [i32; 2];
}
