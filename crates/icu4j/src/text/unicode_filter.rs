// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UnicodeFilter`.

use rucene::error::Result;

use crate::text::replaceable::Replaceable;
use crate::text::unicode_matcher::{UnicodeMatcher, U_MATCH, U_MISMATCH, U_PARTIAL_MATCH};
use crate::text::utf16::UTF16;

/// A matcher of single code points, the basis of transliterator filters.
///
/// Equivalent to the abstract class `com.ibm.icu.text.UnicodeFilter`:
/// [`contains`](Self::contains) is its abstract method and
/// [`filter_matches`](Self::filter_matches) its `matches` implementation,
/// which a subclass's `matches` reaches through `super.matches`.
pub trait UnicodeFilter: UnicodeMatcher {
    /// `contains(int c)`: whether the filter contains the code point.
    ///
    /// # Errors
    ///
    /// The errors of the implementation (`UnicodeSet` rejects an invalid
    /// code point).
    fn contains(&self, c: i32) -> Result<bool>;

    /// `UnicodeFilter.matches(Replaceable text, int[] offset, int limit,
    /// boolean incremental)`.
    ///
    /// # Errors
    ///
    /// The errors of [`contains`](Self::contains).
    fn filter_matches(
        &self,
        text: &dyn Replaceable,
        offset: &mut [i32],
        limit: i32,
        incremental: bool,
    ) -> Result<i32> {
        let c;
        if offset[0] < limit && {
            c = text.char32_at(offset[0]);
            self.contains(c)?
        } {
            offset[0] += UTF16::get_char_count(c);
            return Ok(U_MATCH);
        }
        if offset[0] > limit && self.contains(text.char32_at(offset[0]))? {
            // Backup offset by 1, unless the preceding character is a
            // surrogate pair -- then backup by 2 (keep offset pointing at
            // the lead surrogate).
            offset[0] -= 1;
            if offset[0] >= 0 {
                offset[0] -= UTF16::get_char_count(text.char32_at(offset[0])) - 1;
            }
            return Ok(U_MATCH);
        }
        if incremental && offset[0] == limit {
            return Ok(U_PARTIAL_MATCH);
        }
        Ok(U_MISMATCH)
    }
}
