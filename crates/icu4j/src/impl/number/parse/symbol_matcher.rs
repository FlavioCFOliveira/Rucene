// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.SymbolMatcher`.

use rucene::error::Result;

use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::static_unicode_sets::{Key, StaticUnicodeSets};
use crate::r#impl::string_segment::StringSegment;
use crate::text::unicode_set::UnicodeSet;

/// The state of the abstract class `SymbolMatcher`: its `string` and
/// `uniSet`.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.SymbolMatcher`; each
/// subclass holds one and implements [`SymbolMatcherImpl`], whose
/// [`symbol_match`]/[`symbol_smoke_test`] are the inherited `match` and
/// `smokeTest`.
#[derive(Debug, Clone)]
pub struct SymbolMatcher {
    /// `string`.
    pub(crate) string: Vec<u16>,
    /// `uniSet`.
    pub(crate) uni_set: &'static UnicodeSet,
}

impl SymbolMatcher {
    /// Equivalent to `SymbolMatcher(String, UnicodeSet)`.
    pub(crate) fn new(symbol_string: &[u16], symbol_uni_set: &'static UnicodeSet) -> Self {
        SymbolMatcher {
            string: symbol_string.to_vec(),
            uni_set: symbol_uni_set,
        }
    }

    /// Equivalent to `SymbolMatcher(StaticUnicodeSets.Key)`.
    pub(crate) fn from_key(key: Key) -> Self {
        SymbolMatcher {
            string: Vec::new(),
            uni_set: StaticUnicodeSets::get(key),
        }
    }

    /// Equivalent to `getSet()`.
    pub fn get_set(&self) -> &'static UnicodeSet {
        self.uni_set
    }
}

/// The abstract members of `SymbolMatcher`.
pub trait SymbolMatcherImpl {
    /// The inherited state.
    fn symbol(&self) -> &SymbolMatcher;
    /// Equivalent to `isDisabled(ParsedNumber)`.
    fn is_disabled(&self, result: &ParsedNumber) -> bool;
    /// Equivalent to `accept(StringSegment, ParsedNumber)`.
    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber);
}

/// `SymbolMatcher.match(StringSegment, ParsedNumber)`.
///
/// # Errors
///
/// The `IllegalArgumentException` of `UnicodeSet.contains` (never for the
/// code points of a string).
pub fn symbol_match<M: SymbolMatcherImpl + ?Sized>(
    m: &M,
    segment: &mut StringSegment,
    result: &mut ParsedNumber,
) -> Result<bool> {
    // Smoke test first; this matcher might be disabled.
    if m.is_disabled(result) {
        return Ok(false);
    }

    // Test the string first in order to consume trailing chars greedily.
    let base = m.symbol();
    let mut overlap = 0;
    if !base.string.is_empty() {
        overlap = segment.get_common_prefix_length(&base.string);
        if overlap == base.string.len() as i32 {
            segment.adjust_offset(base.string.len() as i32);
            m.accept(segment, result);
            return Ok(false);
        }
    }

    if segment.starts_with_set(base.uni_set) {
        segment.adjust_offset_by_code_point();
        m.accept(segment, result);
        return Ok(false);
    }

    Ok(overlap == segment.length())
}

/// `SymbolMatcher.smokeTest(StringSegment)`.
pub fn symbol_smoke_test(base: &SymbolMatcher, segment: &StringSegment) -> bool {
    segment.starts_with_set(base.uni_set) || segment.starts_with(Some(&base.string))
}

/// Implements `NumberParseMatcher` for a `SymbolMatcher` subclass: the
/// inherited `match`, `smokeTest` and empty `postProcess`, the subclass's
/// `toString()` text, and whether it is `Flexible`.
macro_rules! impl_symbol_matcher {
    ($ty:ty, $text:literal, $flexible:literal) => {
        impl $crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher for $ty {
            fn match_(
                &self,
                segment: &mut $crate::r#impl::string_segment::StringSegment,
                result: &mut $crate::r#impl::number::parse::parsed_number::ParsedNumber,
            ) -> rucene::error::Result<bool> {
                $crate::r#impl::number::parse::symbol_matcher::symbol_match(self, segment, result)
            }

            fn smoke_test(&self, segment: &$crate::r#impl::string_segment::StringSegment) -> bool {
                $crate::r#impl::number::parse::symbol_matcher::symbol_smoke_test(
                    $crate::r#impl::number::parse::symbol_matcher::SymbolMatcherImpl::symbol(self),
                    segment,
                )
            }

            fn post_process(
                &self,
                _result: &mut $crate::r#impl::number::parse::parsed_number::ParsedNumber,
            ) -> rucene::error::Result<()> {
                Ok(())
            }

            fn is_flexible(&self) -> bool {
                $flexible
            }

            fn to_java_string(&self) -> String {
                $text.to_string()
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}
pub(crate) use impl_symbol_matcher;
