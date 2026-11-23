// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeMatcher`.

use std::any::Any;

use rucene::error::Result;

use crate::text::replaceable::Replaceable;
use crate::text::unicode_set::UnicodeSet;

/// `U_MISMATCH`: no match.
pub const U_MISMATCH: i32 = 0;
/// `U_PARTIAL_MATCH`: a partial match at the end of an incremental text.
pub const U_PARTIAL_MATCH: i32 = 1;
/// `U_MATCH`: a match.
pub const U_MATCH: i32 = 2;
/// `ETHER`: the code point that matches the end of the text.
pub const ETHER: u16 = 0xFFFF;

/// An object that matches a fixed input string, implementing the
/// `matches()` method.
///
/// Equivalent to the interface `com.ibm.icu.text.UnicodeMatcher`.
/// [`as_any`](Self::as_any) carries Java's casts of a matcher to its
/// class (`(UnicodeSet) m`).
pub trait UnicodeMatcher: Send + Sync + Any {
    /// `matches(Replaceable text, int[] offset, int limit, boolean
    /// incremental)`: `U_MISMATCH`, `U_PARTIAL_MATCH` or `U_MATCH`,
    /// advancing `offset[0]` past a match.
    fn matches(&self, text: &dyn Replaceable, offset: &mut [i32], limit: i32, incremental: bool) -> Result<i32>;

    /// `toPattern(boolean escapeUnprintable)`.
    fn to_pattern(&self, escape_unprintable: bool) -> Vec<u16>;

    /// `matchesIndexValue(int v)`.
    fn matches_index_value(&self, v: i32) -> bool;

    /// `addMatchSetTo(UnicodeSet toUnionTo)`.
    ///
    /// # Errors
    ///
    /// The errors of [`UnicodeSet::add_all`].
    fn add_match_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()>;

    /// The matcher as `Any`, for Java's casts.
    fn as_any(&self) -> &dyn Any;
}
