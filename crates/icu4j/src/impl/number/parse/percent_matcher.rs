// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.PercentMatcher`.

use std::sync::{Arc, LazyLock};

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::symbol_matcher::{impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl};
use crate::r#impl::static_unicode_sets::Key;
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;

static DEFAULT: LazyLock<Arc<PercentMatcher>> = LazyLock::new(|| Arc::new(PercentMatcher { base: SymbolMatcher::from_key(Key::PercentSign) }));

/// Equivalent to `com.ibm.icu.impl.number.parse.PercentMatcher`.
#[derive(Debug)]
pub struct PercentMatcher {
    base: SymbolMatcher,
}

impl PercentMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols)`.
    pub fn get_instance(symbols: &DecimalFormatSymbols) -> Arc<PercentMatcher> {
        let symbol_string = symbols.get_percent_string();
        if DEFAULT.base.uni_set.contains_str(symbol_string) {
            DEFAULT.clone()
        } else {
            Arc::new(PercentMatcher { base: SymbolMatcher::new(symbol_string, DEFAULT.base.uni_set) })
        }
    }
}

impl SymbolMatcherImpl for PercentMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, result: &ParsedNumber) -> bool {
        0 != (result.flags & parsed_number::FLAG_PERCENT)
    }

    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber) {
        result.flags |= parsed_number::FLAG_PERCENT;
        result.set_chars_consumed(segment);
    }
}

impl_symbol_matcher!(PercentMatcher, "<PercentMatcher>", false);
