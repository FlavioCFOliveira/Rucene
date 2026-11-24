// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.InfinityMatcher`.

use std::sync::{Arc, LazyLock};

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::symbol_matcher::{
    impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl,
};
use crate::r#impl::static_unicode_sets::Key;
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;

static DEFAULT: LazyLock<Arc<InfinityMatcher>> = LazyLock::new(|| {
    Arc::new(InfinityMatcher {
        base: SymbolMatcher::from_key(Key::InfinitySign),
    })
});

/// Equivalent to `com.ibm.icu.impl.number.parse.InfinityMatcher`.
#[derive(Debug)]
pub struct InfinityMatcher {
    base: SymbolMatcher,
}

impl InfinityMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols)`.
    pub fn get_instance(symbols: &DecimalFormatSymbols) -> Arc<InfinityMatcher> {
        let symbol_string = symbols.get_infinity();
        if DEFAULT.base.uni_set.contains_str(symbol_string) {
            DEFAULT.clone()
        } else {
            Arc::new(InfinityMatcher {
                base: SymbolMatcher::new(symbol_string, DEFAULT.base.uni_set),
            })
        }
    }
}

impl SymbolMatcherImpl for InfinityMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, result: &ParsedNumber) -> bool {
        0 != (result.flags & parsed_number::FLAG_INFINITY)
    }

    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber) {
        result.flags |= parsed_number::FLAG_INFINITY;
        result.set_chars_consumed(segment);
    }
}

impl_symbol_matcher!(InfinityMatcher, "<InfinityMatcher>", false);
