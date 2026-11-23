// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeReplacer`.

use rucene::error::Result;

use crate::text::replaceable::Replaceable;
use crate::text::unicode_set::UnicodeSet;

/// An object that replaces a span of text, the output side of a
/// transliteration rule.
///
/// Equivalent to the package-private interface
/// `com.ibm.icu.text.UnicodeReplacer`.
pub trait UnicodeReplacer: Send + Sync {
    /// `replace(Replaceable text, int start, int limit, int[] cursor)`:
    /// replaces `start..limit` of `text` with the output text and returns
    /// its length; `cursor[0]` receives the new cursor position when the
    /// replacer has a cursor.
    ///
    /// # Errors
    ///
    /// The errors of the replacement (a nested transliteration).
    fn replace(&self, text: &dyn Replaceable, start: i32, limit: i32, cursor: &mut [i32]) -> Result<i32>;

    /// `toReplacerPattern(boolean escapeUnprintable)`: the replacer as rule
    /// source.
    ///
    /// # Errors
    ///
    /// The errors of `Utility.appendToRule` (invalid code point).
    fn to_replacer_pattern(&self, escape_unprintable: bool) -> Result<Vec<u16>>;

    /// `addReplacementSetTo(UnicodeSet toUnionTo)`: unions into
    /// `to_union_to` the characters this replacer may output.
    ///
    /// # Errors
    ///
    /// The errors of the `UnicodeSet` operations.
    fn add_replacement_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()>;
}
