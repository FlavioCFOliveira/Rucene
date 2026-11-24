// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.AffixTokenMatcherFactory`.

use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::number::parse::combined_currency_matcher::CombinedCurrencyMatcher;
use crate::r#impl::number::parse::ignorables_matcher::IgnorablesMatcher;
use crate::r#impl::number::parse::minus_sign_matcher::MinusSignMatcher;
use crate::r#impl::number::parse::percent_matcher::PercentMatcher;
use crate::r#impl::number::parse::permille_matcher::PermilleMatcher;
use crate::r#impl::number::parse::plus_sign_matcher::PlusSignMatcher;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::currency::Currency;
use crate::util::u_locale::ULocale;

/// Small helper class that generates matchers for individual tokens for
/// AffixPatternMatcher.
///
/// In Java, this is done by having a single class that contains the
/// factory methods; in C++, it is done by having a template matcher
/// warehouse.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.AffixTokenMatcherFactory`.
#[derive(Debug, Clone)]
pub struct AffixTokenMatcherFactory {
    /// `currency`.
    pub currency: Currency,
    /// `symbols`.
    pub symbols: Arc<DecimalFormatSymbols>,
    /// `ignorables`.
    pub ignorables: Arc<IgnorablesMatcher>,
    /// `locale`.
    pub locale: ULocale,
    /// `parseFlags`.
    pub parse_flags: i32,
}

impl AffixTokenMatcherFactory {
    /// Equivalent to `minusSign()`.
    pub fn minus_sign(&self) -> Arc<MinusSignMatcher> {
        MinusSignMatcher::get_instance(&self.symbols, true)
    }

    /// Equivalent to `plusSign()`.
    pub fn plus_sign(&self) -> Arc<PlusSignMatcher> {
        PlusSignMatcher::get_instance(&self.symbols, true)
    }

    /// Equivalent to `percent()`.
    pub fn percent(&self) -> Arc<PercentMatcher> {
        PercentMatcher::get_instance(&self.symbols)
    }

    /// Equivalent to `permille()`.
    pub fn permille(&self) -> Arc<PermilleMatcher> {
        PermilleMatcher::get_instance(&self.symbols)
    }

    /// Equivalent to `currency()`.
    ///
    /// # Errors
    ///
    /// The errors of `CombinedCurrencyMatcher.getInstance`.
    pub fn currency(&self) -> Result<Arc<CombinedCurrencyMatcher>> {
        CombinedCurrencyMatcher::get_instance(&self.currency, &self.symbols, self.parse_flags)
    }

    /// Equivalent to `ignorables()`.
    pub fn ignorables(&self) -> Arc<IgnorablesMatcher> {
        self.ignorables.clone()
    }
}
