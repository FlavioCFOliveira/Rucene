// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and    *
 * others. All Rights Reserved.                                                *
 *******************************************************************************
 */

//! `com.ibm.icu.text.CurrencyDisplayNames`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::currency_data;
use crate::util::u_locale::ULocale;

/// Returns currency names localized for a locale.
///
/// Equivalent to `com.ibm.icu.text.CurrencyDisplayNames` (abstract class).
/// ISO codes and plural keys are Rust strings; names and symbols are Rust
/// strings too, as ICU service display names are in this crate (CLDR data,
/// always well-formed UTF-16). Java's `null` results are `None`; the
/// unmodifiable `Map<String, String>` results are shared `JavaHashMap`s
/// (Java's `HashMap` iteration order). Methods that read resource data
/// return `Result` for the `ICUException`s of malformed data.
pub trait CurrencyDisplayNames: Send + Sync {
    /// `getULocale()`: the locale used to determine how to translate the
    /// currency names.
    fn get_u_locale(&self) -> ULocale;

    /// `getSymbol(String isoCode)`: the symbol for the currency with the
    /// provided ISO code.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_symbol(&self, iso_code: &str) -> Result<Option<String>>;

    /// `getNarrowSymbol(String isoCode)`: the narrow symbol.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_narrow_symbol(&self, iso_code: &str) -> Result<Option<String>>;

    /// `getFormalSymbol(String isoCode)`: the formal symbol.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_formal_symbol(&self, iso_code: &str) -> Result<Option<String>>;

    /// `getVariantSymbol(String isoCode)`: the variant symbol.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_variant_symbol(&self, iso_code: &str) -> Result<Option<String>>;

    /// `getName(String isoCode)`: the 'long name' for the currency.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_name(&self, iso_code: &str) -> Result<Option<String>>;

    /// `getPluralName(String isoCode, String pluralKey)`: a 'plural name'
    /// for the currency.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_plural_name(&self, iso_code: &str, plural_key: &str) -> Result<Option<String>>;

    /// `symbolMap()`: a map from localized symbols to ISO codes.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn symbol_map(&self) -> Result<Arc<JavaHashMap<String>>>;

    /// `nameMap()`: a map from localized names (standard and plural) to ISO
    /// codes.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn name_map(&self) -> Result<Arc<JavaHashMap<String>>>;
}

/// `getInstance(ULocale locale)`: an instance for the locale, with
/// substitution of the ISO code for missing names.
///
/// # Errors
///
/// The errors of loading the currency data.
pub fn get_instance(locale: Option<&ULocale>) -> Result<Option<Arc<dyn CurrencyDisplayNames>>> {
    Ok(currency_data::provider().get_instance(locale, true)?.map(|i| i as Arc<dyn CurrencyDisplayNames>))
}

/// `getInstance(Locale locale)`: as `getInstance(locale, false)`.
///
/// # Errors
///
/// The errors of `ULocale.forLocale` and of loading the data.
pub fn get_instance_locale(locale: Option<&Locale>) -> Result<Option<Arc<dyn CurrencyDisplayNames>>> {
    get_instance_locale_no_substitute(locale, false)
}

/// `getInstance(ULocale locale, boolean noSubstitute)`: an instance for the
/// locale; with `no_substitute`, missing names are `None` (and the result
/// is `None` when the locale has no data).
///
/// # Errors
///
/// The errors of loading the currency data.
pub fn get_instance_no_substitute(
    locale: Option<&ULocale>,
    no_substitute: bool,
) -> Result<Option<Arc<dyn CurrencyDisplayNames>>> {
    Ok(currency_data::provider()
        .get_instance(locale, !no_substitute)?
        .map(|i| i as Arc<dyn CurrencyDisplayNames>))
}

/// `getInstance(Locale locale, boolean noSubstitute)`.
///
/// # Errors
///
/// The errors of `ULocale.forLocale` and of loading the data.
pub fn get_instance_locale_no_substitute(
    locale: Option<&Locale>,
    no_substitute: bool,
) -> Result<Option<Arc<dyn CurrencyDisplayNames>>> {
    let u = ULocale::for_locale(locale)?;
    get_instance_no_substitute(u.as_ref(), no_substitute)
}

/// `hasData()` (internal, deprecated): whether the currency data is
/// available.
pub fn has_data() -> bool {
    currency_data::provider().has_data()
}
