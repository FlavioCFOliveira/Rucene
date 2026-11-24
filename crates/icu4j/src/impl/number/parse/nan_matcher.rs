// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.NanMatcher`.

use std::sync::{Arc, LazyLock};

use crate::jdk::units;
use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::symbol_matcher::{
    impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl,
};
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::unicode_set::EMPTY;

static DEFAULT: LazyLock<Arc<NanMatcher>> =
    LazyLock::new(|| Arc::new(NanMatcher::new(&units("NaN"))));

/// Equivalent to `com.ibm.icu.impl.number.parse.NanMatcher`.
#[derive(Debug)]
pub struct NanMatcher {
    base: SymbolMatcher,
}

impl NanMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols, int)`.
    pub fn get_instance(symbols: &DecimalFormatSymbols, _parse_flags: i32) -> Arc<NanMatcher> {
        let symbol_string = symbols.get_nan();
        if DEFAULT.base.string == symbol_string {
            DEFAULT.clone()
        } else {
            Arc::new(NanMatcher::new(symbol_string))
        }
    }

    fn new(symbol_string: &[u16]) -> Self {
        NanMatcher {
            base: SymbolMatcher::new(symbol_string, &EMPTY),
        }
    }
}

impl SymbolMatcherImpl for NanMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, result: &ParsedNumber) -> bool {
        result.seen_number()
    }

    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber) {
        result.flags |= parsed_number::FLAG_NAN;
        result.set_chars_consumed(segment);
    }
}

impl_symbol_matcher!(NanMatcher, "<NanMatcher>", false);
