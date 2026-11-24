// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Quantifier`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::{illegal_argument_no_message, units};
use crate::r#impl::utility::Utility;
use crate::text::replaceable::Replaceable;
use crate::text::unicode_matcher::{UnicodeMatcher, U_MATCH, U_MISMATCH, U_PARTIAL_MATCH};
use crate::text::unicode_set::UnicodeSet;

/// A matcher that repeats another matcher between a minimum and a maximum
/// number of times (`*`, `+`, `?` of transliteration rules).
///
/// Equivalent to the package-private class `com.ibm.icu.text.Quantifier`.
pub struct Quantifier {
    matcher: Arc<dyn UnicodeMatcher>,
    min_count: i32,
    max_count: i32,
}

impl std::fmt::Debug for Quantifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Quantifier")
            .field("min_count", &self.min_count)
            .field("max_count", &self.max_count)
            .finish()
    }
}

impl Quantifier {
    /// `MAX`: maximum count a quantifier can have.
    pub const MAX: i32 = i32::MAX;

    /// `Quantifier(UnicodeMatcher theMatcher, int theMinCount, int
    /// theMaxCount)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) for a negative count or a
    /// minimum above the maximum. Java's `null` matcher check is the
    /// non-optional parameter type.
    pub fn new(
        the_matcher: Arc<dyn UnicodeMatcher>,
        the_min_count: i32,
        the_max_count: i32,
    ) -> Result<Self> {
        if the_min_count < 0 || the_max_count < 0 || the_min_count > the_max_count {
            return Err(illegal_argument_no_message());
        }
        Ok(Self {
            matcher: the_matcher,
            min_count: the_min_count,
            max_count: the_max_count,
        })
    }
}

impl UnicodeMatcher for Quantifier {
    /// `matches(Replaceable text, int[] offset, int limit, boolean
    /// incremental)`.
    fn matches(
        &self,
        text: &dyn Replaceable,
        offset: &mut [i32],
        limit: i32,
        incremental: bool,
    ) -> Result<i32> {
        let start = offset[0];
        let mut count = 0;
        while count < self.max_count {
            let pos = offset[0];
            let m = self.matcher.matches(text, offset, limit, incremental)?;
            if m == U_MATCH {
                count += 1;
                if pos == offset[0] {
                    // If offset has not moved we have a zero-width match.
                    // Don't keep matching it infinitely.
                    break;
                }
            } else if incremental && m == U_PARTIAL_MATCH {
                return Ok(U_PARTIAL_MATCH);
            } else {
                break;
            }
        }
        if incremental && offset[0] == limit {
            return Ok(U_PARTIAL_MATCH);
        }
        if count >= self.min_count {
            return Ok(U_MATCH);
        }
        offset[0] = start;
        Ok(U_MISMATCH)
    }

    /// `toPattern(boolean escapeUnprintable)`.
    fn to_pattern(&self, escape_unprintable: bool) -> Vec<u16> {
        let mut result = Vec::new();
        result.extend_from_slice(&self.matcher.to_pattern(escape_unprintable));
        if self.min_count == 0 {
            if self.max_count == 1 {
                result.push(b'?' as u16);
                return result;
            } else if self.max_count == Self::MAX {
                result.push(b'*' as u16);
                return result;
            }
            // else fall through
        } else if self.min_count == 1 && self.max_count == Self::MAX {
            result.push(b'+' as u16);
            return result;
        }
        result.push(b'{' as u16);
        result.extend_from_slice(&units(&Utility::hex_places(self.min_count as i64, 1)));
        result.push(b',' as u16);
        if self.max_count != Self::MAX {
            result.extend_from_slice(&units(&Utility::hex_places(self.max_count as i64, 1)));
        }
        result.push(b'}' as u16);
        result
    }

    /// `matchesIndexValue(int v)`.
    fn matches_index_value(&self, v: i32) -> bool {
        (self.min_count == 0) || self.matcher.matches_index_value(v)
    }

    /// `addMatchSetTo(UnicodeSet toUnionTo)`.
    fn add_match_set_to(&self, to_union_to: &mut UnicodeSet) -> Result<()> {
        if self.max_count > 0 {
            self.matcher.add_match_set_to(to_union_to)?;
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
