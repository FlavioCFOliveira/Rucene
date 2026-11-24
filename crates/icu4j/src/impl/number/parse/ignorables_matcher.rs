// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.IgnorablesMatcher`.

use std::sync::{Arc, LazyLock};

use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::number::parse::symbol_matcher::{
    impl_symbol_matcher, SymbolMatcher, SymbolMatcherImpl,
};
use crate::r#impl::static_unicode_sets::{Key, StaticUnicodeSets};
use crate::r#impl::string_segment::StringSegment;
use crate::text::unicode_set::UnicodeSet;

static DEFAULT: LazyLock<Arc<IgnorablesMatcher>> = LazyLock::new(|| {
    Arc::new(IgnorablesMatcher::new(StaticUnicodeSets::get(
        Key::DefaultIgnorables,
    )))
});
static STRICT: LazyLock<Arc<IgnorablesMatcher>> = LazyLock::new(|| {
    Arc::new(IgnorablesMatcher::new(StaticUnicodeSets::get(
        Key::StrictIgnorables,
    )))
});
static JAVA_COMPATIBILITY: LazyLock<Arc<IgnorablesMatcher>> =
    LazyLock::new(|| Arc::new(IgnorablesMatcher::new(StaticUnicodeSets::get(Key::Empty))));

/// Matches ignorable characters (`Flexible`).
///
/// Equivalent to `com.ibm.icu.impl.number.parse.IgnorablesMatcher`.
#[derive(Debug)]
pub struct IgnorablesMatcher {
    base: SymbolMatcher,
}

impl IgnorablesMatcher {
    /// Equivalent to `getInstance(int)`.
    pub fn get_instance(parse_flags: i32) -> Arc<IgnorablesMatcher> {
        if 0 != (parse_flags & ParsingUtils::PARSE_FLAG_JAVA_COMPATIBILITY_IGNORABLES) {
            JAVA_COMPATIBILITY.clone()
        } else if 0 != (parse_flags & ParsingUtils::PARSE_FLAG_STRICT_IGNORABLES) {
            STRICT.clone()
        } else {
            DEFAULT.clone()
        }
    }

    fn new(ignorables: &'static UnicodeSet) -> Self {
        IgnorablesMatcher {
            base: SymbolMatcher::new(&[], ignorables),
        }
    }

    /// Equivalent to `getSet()`.
    pub fn get_set(&self) -> &'static UnicodeSet {
        self.base.get_set()
    }
}

impl SymbolMatcherImpl for IgnorablesMatcher {
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

impl_symbol_matcher!(IgnorablesMatcher, "<IgnorablesMatcher>", true);
