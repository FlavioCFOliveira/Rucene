// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.MinusSignMatcher`.

use std::sync::{Arc, LazyLock};

use crate::r#impl::number::parse::parsed_number::{self, ParsedNumber};
use crate::r#impl::number::parse::symbol_matcher::{impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl};
use crate::r#impl::static_unicode_sets::Key;
use crate::r#impl::string_segment::StringSegment;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;

static DEFAULT: LazyLock<Arc<MinusSignMatcher>> = LazyLock::new(|| Arc::new(MinusSignMatcher::default_matcher(false)));
static DEFAULT_ALLOW_TRAILING: LazyLock<Arc<MinusSignMatcher>> = LazyLock::new(|| Arc::new(MinusSignMatcher::default_matcher(true)));

/// Equivalent to `com.ibm.icu.impl.number.parse.MinusSignMatcher`.
#[derive(Debug)]
pub struct MinusSignMatcher {
    base: SymbolMatcher,
    allow_trailing: bool,
}

impl MinusSignMatcher {
    /// Equivalent to `getInstance(DecimalFormatSymbols, boolean)`.
    pub fn get_instance(symbols: &DecimalFormatSymbols, allow_trailing: bool) -> Arc<MinusSignMatcher> {
        let symbol_string = symbols.get_minus_sign_string();
        if DEFAULT.base.uni_set.contains_str(symbol_string) {
            if allow_trailing {
                DEFAULT_ALLOW_TRAILING.clone()
            } else {
                DEFAULT.clone()
            }
        } else {
            Arc::new(MinusSignMatcher { base: SymbolMatcher::new(symbol_string, DEFAULT.base.uni_set), allow_trailing })
        }
    }

    fn default_matcher(allow_trailing: bool) -> Self {
        MinusSignMatcher { base: SymbolMatcher::from_key(Key::MinusSign), allow_trailing }
    }
}

impl SymbolMatcherImpl for MinusSignMatcher {
    fn symbol(&self) -> &SymbolMatcher {
        &self.base
    }

    fn is_disabled(&self, result: &ParsedNumber) -> bool {
        !self.allow_trailing && result.seen_number()
    }

    fn accept(&self, segment: &StringSegment, result: &mut ParsedNumber) {
        result.flags |= parsed_number::FLAG_NEGATIVE;
        result.set_chars_consumed(segment);
    }
}

impl_symbol_matcher!(MinusSignMatcher, "<MinusSignMatcher>", false);
