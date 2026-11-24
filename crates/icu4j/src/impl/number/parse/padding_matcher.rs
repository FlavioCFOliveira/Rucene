// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.PaddingMatcher`.

use std::sync::Arc;

use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::symbol_matcher::{
    impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl,
};
use crate::r#impl::string_segment::StringSegment;
use crate::text::unicode_set::EMPTY;

/// Matches the padding string (`Flexible`).
///
/// Equivalent to `com.ibm.icu.impl.number.parse.PaddingMatcher`.
#[derive(Debug)]
pub struct PaddingMatcher {
    base: SymbolMatcher,
}

impl PaddingMatcher {
    /// Equivalent to `getInstance(String)`.
    pub fn get_instance(pad_string: &[u16]) -> Arc<PaddingMatcher> {
        Arc::new(PaddingMatcher {
            base: SymbolMatcher::new(pad_string, &EMPTY),
        })
    }
}

impl SymbolMatcherImpl for PaddingMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, _result: &ParsedNumber) -> bool {
        false
    }

    fn accept(&self, _segment: &StringSegment, _result: &mut ParsedNumber) {
        // No-op
    }
}

impl_symbol_matcher!(PaddingMatcher, "<PaddingMatcher>", true);
