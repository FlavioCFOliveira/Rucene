// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 **********************************************************************
 * Copyright (c) 2001-2016, International Business Machines
 * Corporation and others.  All Rights Reserved.
 **********************************************************************
 * Created: May 26, 2001
 * Since: ICU 2.2
 **********************************************************************
 */

//! `com.ibm.icu.util.Currency`.

use std::any::Any;
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::currency::Currency as JavaCurrency;
use crate::jdk::{to_rust_string, units, ParsePosition};
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::icu_cache::ICUCache;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{icu_data_class_loader, ICUResourceBundle};
use crate::r#impl::simple_cache::SimpleCache;
use crate::r#impl::soft_cache::SoftCache;
use crate::r#impl::static_unicode_sets::StaticUnicodeSets;
use crate::r#impl::text_trie_map::{ResultHandler, TextTrieMap};
use crate::text::currency_display_names::{self, CurrencyDisplayNames};
use crate::text::currency_meta_info::{self, CurrencyFilter};
use crate::util::currency_service_shim::CurrencyServiceShim;
use crate::util::measure_unit::{CurrencyFactory, CustomSymbols, MeasureUnit, MeasureUnitProxy, UnitClass};
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::UResourceBundle;

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = -5839973855554750484;

/// Selector for getName() indicating a symbolic name for a currency, such
/// as "$" for USD. Equivalent to `SYMBOL_NAME`.
pub const SYMBOL_NAME: i32 = 0;

/// Selector for getName() indicating the long name for a currency, such as
/// "US Dollar" for USD. Equivalent to `LONG_NAME`.
pub const LONG_NAME: i32 = 1;

/// Selector for getName() indicating the plural long name for a currency,
/// such as "US dollar" for USD in "1 US dollar", and "US dollars" for USD in
/// "2 US dollars". Equivalent to `PLURAL_LONG_NAME`.
pub const PLURAL_LONG_NAME: i32 = 2;

/// Selector for getName() indicating the narrow currency symbol. Equivalent
/// to `NARROW_SYMBOL_NAME`.
pub const NARROW_SYMBOL_NAME: i32 = 3;

/// Selector for getName() indicating the formal currency symbol.
/// Equivalent to `FORMAL_SYMBOL_NAME`.
pub const FORMAL_SYMBOL_NAME: i32 = 4;

/// Selector for getName() indicating the variant currency symbol.
/// Equivalent to `VARIANT_SYMBOL_NAME`.
pub const VARIANT_SYMBOL_NAME: i32 = 5;

/// Currency Usage used for Decimal Format. Equivalent to
/// `Currency.CurrencyUsage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CurrencyUsage {
    /// a setting to specify currency usage which determines currency digit
    /// and rounding for standard usage, for example: "50.00 NT$"
    Standard,
    /// a setting to specify currency usage which determines currency digit
    /// and rounding for cash usage, for example: "50 NT$"
    Cash,
}

/// A registry key returned by `registerInstance` (Java `Object`).
pub type RegistryKey = Arc<dyn Any + Send + Sync>;

/// The service shim of the currency registration service. Equivalent to
/// the package-private abstract class `Currency.ServiceShim`.
pub trait ServiceShim: Send + Sync {
    /// Equivalent to `getAvailableULocales()`.
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>>;
    /// Equivalent to `getAvailableLocales()`.
    fn get_available_locales(&self) -> Result<Vec<Locale>>;
    /// Equivalent to `createInstance(ULocale)`.
    fn create_instance(&self, l: &ULocale) -> Result<Option<Currency>>;
    /// Equivalent to `registerInstance(Currency, ULocale)`.
    fn register_instance(&self, c: Currency, l: &ULocale) -> Result<RegistryKey>;
    /// Equivalent to `unregister(Object)`.
    fn unregister(&self, f: &RegistryKey) -> bool;
}

/// `shim` (`null` until `getShim()` creates it).
static SHIM: Mutex<Option<Arc<dyn ServiceShim>>> = Mutex::new(None);

/// The current shim, if any (Java reads the static field `shim`).
fn current_shim() -> Option<Arc<dyn ServiceShim>> {
    SHIM.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `getShim()`: Java loads `com.ibm.icu.util.CurrencyServiceShim` by name
/// (`Class.forName`, then `newInstance()`); the class is resolved directly
/// (no reflection; precedent "forName tables").
fn get_shim() -> Result<Arc<dyn ServiceShim>> {
    // Note: this instantiation is safe on loose thread semantics.
    // If we get two instances of the shim, the last one wins.
    let mut guard = SHIM.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match CurrencyServiceShim::new() {
            Ok(s) => *guard = Some(Arc::new(s)),
            Err(e) => {
                // if(DEBUG) e.printStackTrace(); (DEBUG is false unless the
                // "currency" debug flag is set)
                return Err(LuceneError::unchecked_with_message("java.lang.RuntimeException", Some(e.to_string())));
            }
        }
    }
    Ok(guard.clone().expect("set above"))
}

/// `CURRENCY_NAME_CACHE`.
static CURRENCY_NAME_CACHE: LazyLock<SimpleCache<ULocale, TrieVec>> =
    LazyLock::new(SimpleCache::new);

/// The cached `List<TextTrieMap<CurrencyStringInfo>>`: element 0 the symbol
/// trie, element 1 the name trie.
type TrieVec = Arc<Vec<Arc<TextTrieMap<CurrencyStringInfo>>>>;

/// `regionCurrencyCache`.
static REGION_CURRENCY_CACHE: LazyLock<SoftCache<String, Currency, ()>> =
    LazyLock::new(|| SoftCache::new(|key: &String, _unused: ()| Currency::load_currency(key)));

/// `UND`.
static UND: LazyLock<ULocale> = LazyLock::new(|| ULocale::new("und").expect("und is a valid locale"));

/// `POW10`.
const POW10: [i32; 10] = [1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000];

/// `ALL_TENDER_CODES` (a soft reference never cleared, crate precedent).
static ALL_TENDER_CODES: Mutex<Option<Arc<Vec<String>>>> = Mutex::new(None);

/// `ALL_CODES_AS_SET` (a soft reference never cleared, crate precedent).
static ALL_CODES_AS_SET: Mutex<Option<Arc<HashSet<String>>>> = Mutex::new(None);

/// A class encapsulating a currency, as defined by ISO 4217. A *Currency*
/// object can be created given a *Locale* or given an ISO 4217 code. Once
/// created, the *Currency* object can return various data necessary to its
/// proper display.
///
/// Equivalent to `com.ibm.icu.util.Currency` (a `MeasureUnit` subclass): a
/// view over a [`MeasureUnit`] whose runtime class is `Currency` (or its
/// subclass `CustomSymbolCurrency`, whose overrides of `getName`,
/// `getCurrencyCode`, `hashCode` and `equals` read the [`CustomSymbols`]
/// the unit carries). It dereferences to the unit for the inherited members.
/// A `java.util.Currency` is the emulation [`JavaCurrency`].
#[derive(Debug, Clone, PartialEq)]
pub struct Currency(MeasureUnit);

impl Currency {
    /// Constructs a currency object for the given ISO 4217 3-letter code.
    /// This constructor assumes that the code is valid. Equivalent to the
    /// protected `Currency(String)`.
    pub(crate) fn new(the_iso_code: &str) -> Self {
        // isoCode is kept for readResolve() and Currency class no longer
        // use it. So this statement is not really necessary, but just
        // having it here.
        Currency(MeasureUnit::with_class(
            Some("currency".to_string()),
            Some(the_iso_code.to_string()),
            UnitClass::Currency { iso_code: the_iso_code.to_string(), custom: None },
        ))
    }

    /// A currency of the subclass `CustomSymbolCurrency`: `super(isoCode)`
    /// plus the two symbols.
    pub(crate) fn new_custom(iso_code: &str, symbol1: Vec<u16>, symbol2: Vec<u16>) -> Self {
        Currency(MeasureUnit::with_class(
            Some("currency".to_string()),
            Some(iso_code.to_string()),
            UnitClass::Currency { iso_code: iso_code.to_string(), custom: Some(CustomSymbols { symbol1, symbol2 }) },
        ))
    }

    /// Java's `(Currency) unit` cast.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` when `unit` is not a `Currency`.
    pub fn cast(unit: MeasureUnit) -> Currency {
        Self::try_cast(&unit).unwrap_or_else(|| {
            panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.util.Currency",
                unit.java_class_name()
            )
        })
    }

    /// `unit instanceof Currency ? (Currency) unit : null`.
    pub fn try_cast(unit: &MeasureUnit) -> Option<Currency> {
        match unit.unit_class() {
            UnitClass::Currency { .. } => Some(Currency(unit.clone())),
            _ => None,
        }
    }

    /// The unit this view stands for.
    pub fn into_measure_unit(self) -> MeasureUnit {
        self.0
    }

    /// The unit this view stands for.
    pub fn as_measure_unit(&self) -> &MeasureUnit {
        &self.0
    }

    /// The `CustomSymbolCurrency` fields, when this is one.
    pub(crate) fn custom_symbols(&self) -> Option<&CustomSymbols> {
        match self.0.unit_class() {
            UnitClass::Currency { custom, .. } => custom.as_ref(),
            _ => None,
        }
    }

    fn sub_type(&self) -> &str {
        self.0.get_subtype().unwrap_or("")
    }

    /// Returns a currency object for the default currency in the given
    /// locale. Equivalent to `getInstance(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of locale conversion and of the currency data.
    pub fn get_instance_locale(locale: &Locale) -> Result<Option<Currency>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::get_instance_ulocale(&ulocale)
    }

    /// Returns a currency object for the default currency in the given
    /// locale (`None` when the region has none). Equivalent to
    /// `getInstance(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the currency data and of the registration service.
    pub fn get_instance_ulocale(locale: &ULocale) -> Result<Option<Currency>> {
        if let Some(currency) = locale.get_keyword_value("currency") {
            return Self::get_instance(Some(&currency)).map(Some);
        }

        match current_shim() {
            None => Self::create_currency(locale),
            Some(shim) => shim.create_instance(locale),
        }
    }

    /// Returns an array of Strings which contain the currency identifiers
    /// that are valid for the given locale on the given date. If there are
    /// no such identifiers, returns `None`. Equivalent to
    /// `getAvailableCurrencyCodes(ULocale, Date)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale and currency data.
    pub fn get_available_currency_codes(loc: &ULocale, d: &Date) -> Result<Option<Vec<String>>> {
        let region = ULocale::get_region_for_supplemental_data(loc, false)?;
        let filter = CurrencyFilter::on_date(d).with_region(Some(&region));
        let list = Self::get_tender_currencies(&filter);
        // Note: Prior to 4.4 the spec didn't say that we return null if
        // there are no results, but the test assumed it did.  Kept the
        // behavior.
        if list.is_empty() {
            return Ok(None);
        }
        Ok(Some(list))
    }

    /// Equivalent to `getAvailableCurrencyCodes(Locale, Date)`.
    ///
    /// # Errors
    ///
    /// As [`get_available_currency_codes`](Self::get_available_currency_codes).
    pub fn get_available_currency_codes_locale(loc: &Locale, d: &Date) -> Result<Option<Vec<String>>> {
        let ulocale = ULocale::for_locale(Some(loc))?.unwrap_or_else(ULocale::root);
        Self::get_available_currency_codes(&ulocale, d)
    }

    /// Returns the set of available currencies. The returned set of
    /// currencies contains all of the available currencies, including
    /// obsolete ones. Equivalent to `getAvailableCurrencies()` (a `HashSet`
    /// of `Currency`, whose `hashCode` is `31 * type + subType`).
    ///
    /// # Errors
    ///
    /// The errors of `getInstance`.
    pub fn get_available_currencies() -> Result<Vec<Currency>> {
        let info = currency_meta_info::get_instance();
        let list = info.currencies(&CurrencyFilter::all());
        let mut result_set: Vec<Currency> = Vec::with_capacity(list.len());
        for code in &list {
            let c = Self::get_instance(Some(code))?;
            if !result_set.iter().any(|e| e.0.java_equals(&c.0)) {
                result_set.push(c);
            }
        }
        Ok(result_set)
    }

    /// Instantiate a currency from resource data. Equivalent to the
    /// package-private `createCurrency(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale and currency data.
    pub(crate) fn create_currency(loc: &ULocale) -> Result<Option<Currency>> {
        let key = ULocale::get_region_for_supplemental_data(loc, false)?;
        REGION_CURRENCY_CACHE.get_instance(key, ())
    }

    /// `loadCurrency(String)`.
    fn load_currency(key: &str) -> Result<Option<Currency>> {
        let region = key;
        let info = currency_meta_info::get_instance();
        // Note: Prior to ICU 74, we passed tender=false here.
        let mut list = info.currencies(&CurrencyFilter::on_region(Some(region)).with_tender());
        if list.is_empty() {
            list = info.currencies(&CurrencyFilter::on_region(Some(region)));
        }
        if !list.is_empty() {
            let code = &list[0];
            return Self::get_instance(Some(code)).map(Some);
        }
        Ok(None)
    }

    /// Returns a currency object given an ISO 4217 3-letter code. Equivalent
    /// to `getInstance(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input currency code is null.")`,
    /// `IllegalArgumentException("The input currency code is not 3-letter
    /// alphabetic code.")`.
    pub fn get_instance(the_iso_code: Option<&str>) -> Result<Currency> {
        let the_iso_code = match the_iso_code {
            None => return Err(crate::jdk::null_pointer("The input currency code is null.")),
            Some(c) => c,
        };
        if !Self::is_alpha3_code(the_iso_code) {
            return Err(LuceneError::illegal_argument("The input currency code is not 3-letter alphabetic code."));
        }
        Ok(Currency::cast(MeasureUnit::internal_get_instance("currency", &the_iso_code.to_ascii_uppercase())?))
    }

    /// `isAlpha3Code(String)`.
    fn is_alpha3_code(code: &str) -> bool {
        let u = units(code);
        if u.len() != 3 {
            return false;
        }
        for &ch in &u {
            if ch < b'A' as u16 || (ch > b'Z' as u16 && ch < b'a' as u16) || ch > b'z' as u16 {
                return false;
            }
        }
        true
    }

    /// Returns a Currency object based on the currency represented by the
    /// given java.util.Currency. Equivalent to
    /// `fromJavaCurrency(java.util.Currency)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance`](Self::get_instance).
    pub fn from_java_currency(currency: &JavaCurrency) -> Result<Currency> {
        Self::get_instance(Some(&currency.get_currency_code()))
    }

    /// Returns a java.util.Currency object based on the currency represented
    /// by this Currency. Equivalent to `toJavaCurrency()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from `java.util.Currency.getInstance`
    /// when the JDK's ISO 4217 data does not know this currency's code.
    pub fn to_java_currency(&self) -> Result<JavaCurrency> {
        JavaCurrency::get_instance(&self.get_currency_code())
    }

    /// Registers a new currency for the provided locale. Equivalent to
    /// `registerInstance(Currency, ULocale)`.
    ///
    /// # Errors
    ///
    /// The `RuntimeException` of a shim that cannot be created, and the
    /// errors of the service.
    pub fn register_instance(currency: Currency, locale: &ULocale) -> Result<RegistryKey> {
        get_shim()?.register_instance(currency, locale)
    }

    /// Unregister the currency associated with this key (obtained from
    /// registerInstance). Equivalent to `unregister(Object)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("registryKey must not be null")`.
    pub fn unregister(registry_key: Option<&RegistryKey>) -> Result<bool> {
        let registry_key = match registry_key {
            None => return Err(LuceneError::illegal_argument("registryKey must not be null")),
            Some(k) => k,
        };
        match current_shim() {
            None => Ok(false),
            Some(shim) => Ok(shim.unregister(registry_key)),
        }
    }

    /// Return an array of the locales for which a currency is defined.
    /// Equivalent to `getAvailableLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale index data.
    pub fn get_available_locales() -> Result<Vec<Locale>> {
        match current_shim() {
            None => Ok(ICUResourceBundle::get_available_locales()?.as_ref().clone()),
            Some(shim) => shim.get_available_locales(),
        }
    }

    /// Return an array of the ulocales for which a currency is defined.
    /// Equivalent to `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale index data.
    pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
        match current_shim() {
            None => Ok(ICUResourceBundle::get_available_u_locales()?.unwrap_or_default()),
            Some(shim) => shim.get_available_u_locales(),
        }
    }

    /// Given a key and a locale, returns an array of values for the key for
    /// which data exists. If commonlyUsed is true, these are the values that
    /// typically are used with this locale, otherwise these are all values
    /// for which data exists. Equivalent to
    /// `getKeywordValuesForLocale(String, ULocale, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_keyword_values_for_locale(key: &str, locale: &ULocale, commonly_used: bool) -> Result<Vec<String>> {
        // The only keyword we recognize is 'currency'
        if "currency" != key {
            return Ok(Vec::new());
        }

        if !commonly_used {
            // Behavior change from 4.3.3, no longer sort the currencies
            return Ok(Self::get_all_tender_currencies().as_ref().clone());
        }

        // Don't resolve region if the requested locale is 'und', it will
        // resolve to US which we don't want.
        if *UND == *locale {
            return Ok(Vec::new());
        }
        let pref_region = ULocale::get_region_for_supplemental_data(locale, true)?;

        let filter = CurrencyFilter::now().with_region(Some(&pref_region));

        // currencies are in region's preferred order when we're filtering on
        // region, which matches our spec
        let result = Self::get_tender_currencies(&filter);

        // No fallback anymore (change from 4.3.3)
        if result.is_empty() {
            return Ok(Vec::new());
        }

        Ok(result)
    }

    /// Returns the ISO 4217 code for this currency (the international symbol
    /// for a `CustomSymbolCurrency`). Equivalent to `getCurrencyCode()`.
    pub fn get_currency_code(&self) -> String {
        match self.custom_symbols() {
            Some(c) => to_rust_string(&c.symbol2),
            None => self.sub_type().to_string(),
        }
    }

    /// Returns the ISO 4217 numeric code for this currency object (0 when
    /// the code is unknown). Equivalent to `getNumericCode()`.
    ///
    /// # Errors
    ///
    /// The non-`MissingResourceException` errors of the data.
    pub fn get_numeric_code(&self) -> Result<i32> {
        let mut result = 0;
        let lookup = || -> Result<i32> {
            let bundle = UResourceBundle::get_bundle_instance_loader(
                Some(ICUData::ICU_BASE_NAME),
                "currencyNumericCodes",
                icu_data_class_loader(),
            )?;
            let code_map = bundle.get("codeMap")?;
            let num_code = code_map.get(self.sub_type())?;
            num_code.get_int()
        };
        match lookup() {
            Ok(r) => result = r,
            Err(e) if MissingResourceException::of(&e).is_some() => {
                // fall through
            }
            Err(e) => return Err(e),
        }
        Ok(result)
    }

    /// Convenience and compatibility override of getName that requests the
    /// symbol name for the default `DISPLAY` locale. Equivalent to
    /// `getSymbol()`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name data.
    pub fn get_symbol(&self) -> Result<Option<String>> {
        self.get_symbol_ulocale(&ULocale::get_default_category(Category::Display))
    }

    /// Equivalent to `getSymbol(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name data.
    pub fn get_symbol_locale(&self, loc: &Locale) -> Result<Option<String>> {
        let ulocale = ULocale::for_locale(Some(loc))?.unwrap_or_else(ULocale::root);
        self.get_symbol_ulocale(&ulocale)
    }

    /// Equivalent to `getSymbol(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the display-name data.
    pub fn get_symbol_ulocale(&self, uloc: &ULocale) -> Result<Option<String>> {
        self.get_name(uloc, SYMBOL_NAME, None)
    }

    /// Equivalent to `getName(Locale, int, boolean[])`.
    ///
    /// # Errors
    ///
    /// As [`get_name`](Self::get_name).
    pub fn get_name_locale(&self, locale: &Locale, name_style: i32, is_choice_format: Option<&mut [bool]>) -> Result<Option<String>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        self.get_name(&ulocale, name_style, is_choice_format)
    }

    /// Returns the display name for the given currency in the given locale.
    /// Equivalent to `getName(ULocale, int, boolean[])` (overridden by
    /// `CustomSymbolCurrency` for `SYMBOL_NAME`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("bad name style: <style>")`, and the errors
    /// of the display-name data.
    pub fn get_name(&self, locale: &ULocale, name_style: i32, is_choice_format: Option<&mut [bool]>) -> Result<Option<String>> {
        if let Some(custom) = self.custom_symbols() {
            if name_style == SYMBOL_NAME {
                if let Some(c) = is_choice_format {
                    c[0] = false;
                }
                return Ok(Some(to_rust_string(&custom.symbol1)));
            }
        }
        // We no longer support choice format data in names.  Data should
        // not contain choice patterns.
        if let Some(c) = is_choice_format {
            c[0] = false;
        }

        let names = display_names(locale)?;
        let sub_type = self.sub_type();
        match name_style {
            SYMBOL_NAME => names.get_symbol(sub_type),
            NARROW_SYMBOL_NAME => names.get_narrow_symbol(sub_type),
            FORMAL_SYMBOL_NAME => names.get_formal_symbol(sub_type),
            VARIANT_SYMBOL_NAME => names.get_variant_symbol(sub_type),
            LONG_NAME => names.get_name(sub_type),
            _ => Err(LuceneError::illegal_argument(format!("bad name style: {name_style}"))),
        }
    }

    /// Equivalent to `getName(Locale, int, String, boolean[])`.
    ///
    /// # Errors
    ///
    /// As [`get_name_plural`](Self::get_name_plural).
    pub fn get_name_plural_locale(
        &self,
        locale: &Locale,
        name_style: i32,
        plural_count: &str,
        is_choice_format: Option<&mut [bool]>,
    ) -> Result<Option<String>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        self.get_name_plural(&ulocale, name_style, plural_count, is_choice_format)
    }

    /// Returns the display name for the given currency in the given locale,
    /// for the plural count. Equivalent to `getName(ULocale, int, String,
    /// boolean[])`.
    ///
    /// # Errors
    ///
    /// As [`get_name`](Self::get_name).
    pub fn get_name_plural(
        &self,
        locale: &ULocale,
        name_style: i32,
        plural_count: &str,
        is_choice_format: Option<&mut [bool]>,
    ) -> Result<Option<String>> {
        if name_style != PLURAL_LONG_NAME {
            return self.get_name(locale, name_style, is_choice_format);
        }

        // We no longer support choice format
        if let Some(c) = is_choice_format {
            c[0] = false;
        }

        let names = display_names(locale)?;
        names.get_plural_name(self.sub_type(), plural_count)
    }

    /// Returns the display name for this currency in the default locale.
    /// Equivalent to `getDisplayName()`.
    ///
    /// # Errors
    ///
    /// As [`get_name`](Self::get_name).
    pub fn get_display_name(&self) -> Result<Option<String>> {
        self.get_name_locale(&Locale::get_default(), LONG_NAME, None)
    }

    /// Returns the display name for this currency in the provided locale.
    /// Equivalent to `getDisplayName(Locale)`.
    ///
    /// # Errors
    ///
    /// As [`get_name`](Self::get_name).
    pub fn get_display_name_locale(&self, locale: &Locale) -> Result<Option<String>> {
        self.get_name_locale(locale, LONG_NAME, None)
    }

    /// Attempt to parse the given string as a currency, either as a display
    /// name in the given locale, or as a 3-letter ISO 4217 code. If multiple
    /// display names match, then the longest one is selected. Equivalent to
    /// `parse(ULocale, String, int, ParsePosition)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the display-name data.
    pub fn parse(locale: &ULocale, text: &[u16], type_: i32, pos: &mut ParsePosition) -> Result<Option<String>> {
        let currency_trie_vec = Self::get_currency_trie_vec(locale)?;
        let mut max_length;
        let mut iso_result;

        // look for the names
        let currency_name_trie = &currency_trie_vec[1];
        let mut handler = CurrencyNameResultHandler::default();
        currency_name_trie.find(&text[pos.get_index() as usize..], &mut handler)?;
        iso_result = handler.get_best_currency_iso_code().map(str::to_string);
        max_length = handler.get_best_match_length();

        if type_ != LONG_NAME {
            // not long name only
            let currency_symbol_trie = &currency_trie_vec[0];
            let mut handler = CurrencyNameResultHandler::default();
            currency_symbol_trie.find(&text[pos.get_index() as usize..], &mut handler)?;
            if handler.get_best_match_length() > max_length {
                iso_result = handler.get_best_currency_iso_code().map(str::to_string);
                max_length = handler.get_best_match_length();
            }
        }
        let start = pos.get_index();
        pos.set_index(start + max_length);
        Ok(iso_result)
    }

    /// Equivalent to `getParsingTrie(ULocale, int)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the display-name data.
    pub fn get_parsing_trie(locale: &ULocale, type_: i32) -> Result<Arc<TextTrieMap<CurrencyStringInfo>>> {
        let currency_trie_vec = Self::get_currency_trie_vec(locale)?;
        if type_ == LONG_NAME {
            Ok(currency_trie_vec[1].clone())
        } else {
            Ok(currency_trie_vec[0].clone())
        }
    }

    /// `getCurrencyTrieVec(ULocale)`: element 0 is the symbol trie, element
    /// 1 the name trie.
    fn get_currency_trie_vec(locale: &ULocale) -> Result<TrieVec> {
        if let Some(v) = CURRENCY_NAME_CACHE.get(locale) {
            return Ok(v);
        }
        let currency_name_trie = Arc::new(TextTrieMap::new(true));
        let currency_symbol_trie = Arc::new(TextTrieMap::new(false));
        let currency_trie_vec = vec![currency_symbol_trie, currency_name_trie];
        Self::setup_currency_trie_vec(locale, &currency_trie_vec)?;
        let v = Arc::new(currency_trie_vec);
        CURRENCY_NAME_CACHE.put(locale.clone(), v.clone());
        Ok(v)
    }

    /// `setupCurrencyTrieVec(ULocale, List<TextTrieMap<CurrencyStringInfo>>)`.
    fn setup_currency_trie_vec(locale: &ULocale, trie_vec: &[Arc<TextTrieMap<CurrencyStringInfo>>]) -> Result<()> {
        let sym_trie = &trie_vec[0];
        let trie = &trie_vec[1];

        let names = display_names(locale)?;
        for (symbol, iso_code) in names.symbol_map()?.iter() {
            // Register under not just symbol, but under every equivalent
            // symbol as well e.g short width yen and long width yen.
            let symbol_units = units(symbol);
            let key = StaticUnicodeSets::choose_currency(&symbol_units);
            let value = CurrencyStringInfo::new(iso_code.clone(), symbol.clone());
            match key {
                Some(key) => {
                    let equivalents = StaticUnicodeSets::get(key);
                    // The symbol itself is included in the UnicodeSet
                    for equivalent_symbol in equivalents {
                        sym_trie.put(&equivalent_symbol, value.clone());
                    }
                }
                None => {
                    sym_trie.put(&symbol_units, value);
                }
            }
        }
        for (name, iso_code) in names.name_map()?.iter() {
            trie.put(&units(name), CurrencyStringInfo::new(iso_code.clone(), name.clone()));
        }
        Ok(())
    }

    /// Returns the number of the number of fraction digits that should be
    /// displayed for this currency. Equivalent to
    /// `getDefaultFractionDigits()`.
    pub fn get_default_fraction_digits(&self) -> i32 {
        self.get_default_fraction_digits_usage(CurrencyUsage::Standard)
    }

    /// Returns the number of the number of fraction digits that should be
    /// displayed for this currency with Usage. Equivalent to
    /// `getDefaultFractionDigits(CurrencyUsage)`.
    pub fn get_default_fraction_digits_usage(&self, usage: CurrencyUsage) -> i32 {
        let info = currency_meta_info::get_instance();
        let digits = info.currency_digits_usage(self.sub_type(), usage);
        digits.fraction_digits
    }

    /// Returns the rounding increment for this currency, or 0.0 if no
    /// rounding is done by this currency. Equivalent to
    /// `getRoundingIncrement()`.
    pub fn get_rounding_increment(&self) -> f64 {
        self.get_rounding_increment_usage(CurrencyUsage::Standard)
    }

    /// Returns the rounding increment for this currency, or 0.0 if no
    /// rounding is done by this currency with the Usage. Equivalent to
    /// `getRoundingIncrement(CurrencyUsage)`.
    pub fn get_rounding_increment_usage(&self, usage: CurrencyUsage) -> f64 {
        let info = currency_meta_info::get_instance();
        let digits = info.currency_digits_usage(self.sub_type(), usage);

        let data1 = digits.rounding_increment;

        // If there is no rounding return 0.0 to indicate no rounding.
        // This is the high-runner case, by far.
        if data1 == 0 {
            return 0.0;
        }

        let data0 = digits.fraction_digits;

        // If the meta data is invalid, return 0.0 to indicate no rounding.
        if data0 < 0 || data0 >= POW10.len() as i32 {
            return 0.0;
        }

        // Return data[1] / 10^(data[0]). The only actual rounding data, as
        // of this writing, is CHF { 2, 25 }.
        data1 as f64 / POW10[data0 as usize] as f64
    }

    /// Returns the ISO 4217 code for this currency. Equivalent to
    /// `toString()`.
    pub fn to_java_string(&self) -> String {
        self.sub_type().to_string()
    }

    /// `getAllTenderCurrencies()` (`static synchronized`).
    fn get_all_tender_currencies() -> Arc<Vec<String>> {
        let mut guard = ALL_TENDER_CODES.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            // Filter out non-tender currencies which have "from" date set
            // to 9999-12-31 CLDR adds them as placeholders.
            let filter = CurrencyFilter::all();
            *guard = Some(Arc::new(Self::get_tender_currencies(&filter)));
        }
        guard.clone().expect("set above")
    }

    /// `getAllCurrenciesAsSet()` (`static synchronized`).
    fn get_all_currencies_as_set() -> Arc<HashSet<String>> {
        let mut guard = ALL_CODES_AS_SET.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let info = currency_meta_info::get_instance();
            *guard = Some(Arc::new(info.currencies(&CurrencyFilter::all()).into_iter().collect()));
        }
        guard.clone().expect("set above")
    }

    /// Queries if the given ISO 4217 3-letter code is available on the
    /// specified date range. Equivalent to `isAvailable(String, Date,
    /// Date)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("To is before from")`.
    pub fn is_available(code: &str, from: Option<&Date>, to: Option<&Date>) -> Result<bool> {
        if !Self::is_alpha3_code(code) {
            return Ok(false);
        }

        if let (Some(from), Some(to)) = (from, to) {
            if from.after(to) {
                return Err(LuceneError::illegal_argument("To is before from"));
            }
        }

        let code = code.to_ascii_uppercase();
        let is_known = Self::get_all_currencies_as_set().contains(&code);
        if !is_known {
            return Ok(false);
        } else if from.is_none() && to.is_none() {
            return Ok(true);
        }

        // If caller passed a date range, we cannot rely solely on the cache
        let info = currency_meta_info::get_instance();
        let all_active = info.currencies(&CurrencyFilter::on_date_range(from, to).with_currency(Some(&code)));

        Ok(all_active.contains(&code))
    }

    /// Returns the list of remaining tender currencies after a filter is
    /// applied. Equivalent to `getTenderCurrencies(CurrencyFilter)`.
    fn get_tender_currencies(filter: &CurrencyFilter) -> Vec<String> {
        let info = currency_meta_info::get_instance();
        info.currencies(&filter.with_tender())
    }

    /// Equivalent to `writeReplace()`.
    pub fn write_replace(&self) -> MeasureUnitProxy {
        self.0.write_replace()
    }

    /// Equivalent to `readResolve()`: `Currency.getInstance(isoCode)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance`](Self::get_instance).
    pub fn read_resolve(&self) -> Result<Currency> {
        // The old isoCode field used to determine the currency.
        let iso_code = match self.0.unit_class() {
            UnitClass::Currency { iso_code, .. } => iso_code.clone(),
            _ => String::new(),
        };
        Currency::get_instance(Some(&iso_code))
    }

    /// The factory `MeasureUnit.internalGetInstance` uses for currencies.
    #[allow(dead_code)]
    pub(crate) fn factory() -> CurrencyFactory {
        CurrencyFactory
    }
}

impl Deref for Currency {
    type Target = MeasureUnit;

    fn deref(&self) -> &MeasureUnit {
        &self.0
    }
}

/// `CurrencyDisplayNames.getInstance(locale)`, which never answers `null`
/// for a locale.
fn display_names(locale: &ULocale) -> Result<Arc<dyn CurrencyDisplayNames>> {
    currency_display_names::get_instance(Some(locale))?
        .ok_or_else(|| LuceneError::null_pointer_with_message("CurrencyDisplayNames.getInstance"))
}

/// Internal class used for parsing currency names and symbols. Equivalent to
/// `Currency.CurrencyStringInfo` (internal, deprecated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyStringInfo {
    iso_code: String,
    currency_string: String,
}

impl CurrencyStringInfo {
    /// Equivalent to `CurrencyStringInfo(String, String)`.
    pub fn new(iso_code: String, currency_string: String) -> Self {
        Self { iso_code, currency_string }
    }

    /// Equivalent to `getISOCode()`.
    pub fn get_iso_code(&self) -> &str {
        &self.iso_code
    }

    /// Equivalent to `getCurrencyString()`.
    pub fn get_currency_string(&self) -> &str {
        &self.currency_string
    }
}

/// Handler for the currency name trie search: keeps the last (longest)
/// match. Equivalent to `Currency.CurrencyNameResultHandler`.
#[derive(Debug, Default)]
struct CurrencyNameResultHandler {
    // The length of longest matching key
    best_match_length: i32,
    // The currency ISO code of longest matching key
    best_currency_iso_code: Option<String>,
}

impl ResultHandler<CurrencyStringInfo> for CurrencyNameResultHandler {
    fn handle_prefix_match(&mut self, match_length: i32, values: &[CurrencyStringInfo]) -> bool {
        if let Some(first) = values.first() {
            // Since the best match criteria is only based on length of key
            // in trie and all the values are mapped to the same key, we only
            // need to examine the first value.
            self.best_currency_iso_code = Some(first.get_iso_code().to_string());
            self.best_match_length = match_length;
        }
        true
    }
}

impl CurrencyNameResultHandler {
    fn get_best_currency_iso_code(&self) -> Option<&str> {
        self.best_currency_iso_code.as_deref()
    }

    fn get_best_match_length(&self) -> i32 {
        self.best_match_length
    }
}
