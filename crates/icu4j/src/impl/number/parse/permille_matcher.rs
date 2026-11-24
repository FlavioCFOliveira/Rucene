// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.PermilleMatcher`.

use std::sync::{Arc, LazyLock};

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::symbol_matcher::{
    impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl,
};
use crate::r#impl::static_unicode_sets::Key;
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;

static DEFAULT: LazyLock<Arc<PermilleMatcher>> = LazyLock::new(|| {
    Arc::new(PermilleMatcher {
        base: SymbolMatcher::from_key(Key::PermilleSign),
    })
});

/// Equivalent to `com.ibm.icu.impl.number.parse.PermilleMatcher`.
#[derive(Debug)]
pub struct PermilleMatcher {
    base: SymbolMatcher,
}

impl PermilleMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols)`.
    pub fn get_instance(symbols: &DecimalFormatSymbols) -> Arc<PermilleMatcher> {
        let symbol_string = symbols.get_per_mill_string();
        if DEFAULT.base.uni_set.contains_str(symbol_string) {
            DEFAULT.clone()
        } else {
            Arc::new(PermilleMatcher {
                base: SymbolMatcher::new(symbol_string, DEFAULT.base.uni_set),
            })
        }
    }
}

impl SymbolMatcherImpl for PermilleMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, result: &ParsedNumber) -> bool {
        0 != (result.flags & parsed_number::FLAG_PERMILLE)
    }

    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber) {
        result.flags |= parsed_number::FLAG_PERMILLE;
        result.set_chars_consumed(segment);
    }
}

impl_symbol_matcher!(PermilleMatcher, "<PermilleMatcher>", false);
