// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.text.CurrencyMetaInfo`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene_queryparser::jdk::date::Date;

use crate::r#impl::grego::Grego;
use crate::r#impl::icu_currency_meta_info::ICUCurrencyMetaInfo;
use crate::util::currency::CurrencyUsage;

/// The (`impl`, `hasData`) pair the static initializer sets.
static IMPL: LazyLock<(Arc<dyn CurrencyMetaInfo>, bool)> = LazyLock::new(|| {
    // Java: Class.forName("com.ibm.icu.impl.ICUCurrencyMetaInfo").newInstance(),
    // any Throwable falling back to the base class. The class is resolved
    // directly (no reflection; precedent "forName tables").
    match ICUCurrencyMetaInfo::new() {
        Ok(temp) => (Arc::new(temp) as Arc<dyn CurrencyMetaInfo>, true),
        Err(_) => (Arc::new(BaseCurrencyMetaInfo) as Arc<dyn CurrencyMetaInfo>, false),
    }
});

/// `defaultDigits` (protected): 2 fraction digits, no rounding increment.
pub const DEFAULT_DIGITS: CurrencyDigits = CurrencyDigits { fraction_digits: 2, rounding_increment: 0 };

/// Provides information about currencies that is not specific to a locale.
///
/// A note about currency dates. The CLDR data provides data to the day,
/// inclusive. The date information used by CurrencyInfo and CurrencyFilter
/// is represented by milliseconds, which is overly precise. These times are
/// in GMT, so queries involving dates should use GMT times, but more
/// generally you should avoid relying on time of day in queries.
///
/// Equivalent to `com.ibm.icu.text.CurrencyMetaInfo`: the class (whose
/// methods answer empty data) is this trait with those bodies as provided
/// methods; its data-backed subclass is
/// [`ICUCurrencyMetaInfo`]. The statics are [`get_instance`],
/// [`get_instance_no_substitute`] and [`has_data`].
pub trait CurrencyMetaInfo: fmt::Debug + Send + Sync {
    /// Returns the properties of currencies that match the filter.
    /// Equivalent to `currencyInfo(CurrencyFilter)`.
    fn currency_info(&self, _filter: &CurrencyFilter) -> Vec<CurrencyInfo> {
        Vec::new()
    }

    /// Returns the ISO codes of currencies that match the filter.
    /// Equivalent to `currencies(CurrencyFilter)`.
    fn currencies(&self, _filter: &CurrencyFilter) -> Vec<String> {
        Vec::new()
    }

    /// Returns the region codes of regions that match the filter.
    /// Equivalent to `regions(CurrencyFilter)`.
    fn regions(&self, _filter: &CurrencyFilter) -> Vec<String> {
        Vec::new()
    }

    /// Returns the CurrencyDigits for the currency code. Equivalent to
    /// `currencyDigits(String)`.
    fn currency_digits(&self, iso_code: &str) -> CurrencyDigits {
        self.currency_digits_usage(iso_code, CurrencyUsage::Standard)
    }

    /// Returns the CurrencyDigits for the currency code with Context Usage.
    /// Equivalent to `currencyDigits(String, CurrencyUsage)`.
    fn currency_digits_usage(&self, _iso_code: &str, _currency_usage: CurrencyUsage) -> CurrencyDigits {
        DEFAULT_DIGITS
    }
}

/// The base class itself (`new CurrencyMetaInfo()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BaseCurrencyMetaInfo;

impl CurrencyMetaInfo for BaseCurrencyMetaInfo {}

/// Returns the unique instance of the currency meta info. Equivalent to
/// `getInstance()`.
pub fn get_instance() -> Arc<dyn CurrencyMetaInfo> {
    IMPL.0.clone()
}

/// Returns the unique instance of the currency meta info, or `None` if
/// `noSubstitute` is true and there is no data. Equivalent to
/// `getInstance(boolean)`.
pub fn get_instance_no_substitute(_no_substitute: bool) -> Option<Arc<dyn CurrencyMetaInfo>> {
    if IMPL.1 {
        Some(IMPL.0.clone())
    } else {
        None
    }
}

/// Returns true if there is data for the currency meta info. Equivalent to
/// `hasData()`.
pub fn has_data() -> bool {
    IMPL.1
}

/// A filter used to select which currency info is returned. Equivalent to
/// `CurrencyMetaInfo.CurrencyFilter` (immutable, public final fields).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurrencyFilter {
    /// The region to filter on. If null, accepts any region.
    pub region: Option<String>,
    /// The currency to filter on. If null, accepts any currency.
    pub currency: Option<String>,
    /// The from date to filter on (as milliseconds). Accepts any currency on
    /// or after this date.
    pub from: i64,
    /// The to date to filter on (as milliseconds). Accepts any currency on
    /// or before this date.
    pub to: i64,
    /// true if we are filtering only for currencies used as legal tender.
    pub tender_only: bool,
}

/// `CurrencyFilter.ALL`.
const ALL: CurrencyFilter =
    CurrencyFilter { region: None, currency: None, from: i64::MIN, to: i64::MAX, tender_only: false };

impl CurrencyFilter {
    fn new(region: Option<String>, currency: Option<String>, from: i64, to: i64, tender_only: bool) -> Self {
        Self { region, currency, from, to, tender_only }
    }

    /// Returns a filter that accepts all currency data. Equivalent to
    /// `all()`.
    pub fn all() -> CurrencyFilter {
        ALL
    }

    /// Returns a filter that accepts all currencies in use as of the current
    /// date. Equivalent to `now()`.
    pub fn now() -> CurrencyFilter {
        ALL.with_date(&Date::now())
    }

    /// Returns a filter that accepts all currencies ever used in the given
    /// region. Equivalent to `onRegion(String)`.
    pub fn on_region(region: Option<&str>) -> CurrencyFilter {
        ALL.with_region(region)
    }

    /// Returns a filter that accepts the given currency. Equivalent to
    /// `onCurrency(String)`.
    pub fn on_currency(currency: Option<&str>) -> CurrencyFilter {
        ALL.with_currency(currency)
    }

    /// Returns a filter that accepts all currencies in use on the given
    /// date. Equivalent to `onDate(Date)`.
    pub fn on_date(date: &Date) -> CurrencyFilter {
        ALL.with_date(date)
    }

    /// Returns a filter that accepts all currencies that were in use at some
    /// point between the given dates (`None` for no limit). Equivalent to
    /// `onDateRange(Date, Date)`.
    pub fn on_date_range(from: Option<&Date>, to: Option<&Date>) -> CurrencyFilter {
        ALL.with_date_range(from, to)
    }

    /// Returns a filter that accepts all currencies in use on the given date
    /// (milliseconds). Equivalent to `onDate(long)`.
    pub fn on_date_millis(date: i64) -> CurrencyFilter {
        ALL.with_date_millis(date)
    }

    /// Returns a filter that accepts all currencies that were in use at some
    /// point between the given dates (milliseconds). Equivalent to
    /// `onDateRange(long, long)`.
    pub fn on_date_range_millis(from: i64, to: i64) -> CurrencyFilter {
        ALL.with_date_range_millis(from, to)
    }

    /// Returns a CurrencyFilter for finding currencies that were either once
    /// used, are used, or will be used as tender. Equivalent to
    /// `onTender()`.
    pub fn on_tender() -> CurrencyFilter {
        ALL.with_tender()
    }

    /// Returns a copy of this filter, with the specified region. Equivalent
    /// to `withRegion(String)`.
    pub fn with_region(&self, region: Option<&str>) -> CurrencyFilter {
        CurrencyFilter::new(region.map(str::to_string), self.currency.clone(), self.from, self.to, self.tender_only)
    }

    /// Returns a copy of this filter, with the specified currency.
    /// Equivalent to `withCurrency(String)`.
    pub fn with_currency(&self, currency: Option<&str>) -> CurrencyFilter {
        CurrencyFilter::new(self.region.clone(), currency.map(str::to_string), self.from, self.to, self.tender_only)
    }

    /// Returns a copy of this filter, with from and to set to the given
    /// date. Equivalent to `withDate(Date)`.
    pub fn with_date(&self, date: &Date) -> CurrencyFilter {
        CurrencyFilter::new(self.region.clone(), self.currency.clone(), date.get_time(), date.get_time(), self.tender_only)
    }

    /// Returns a copy of this filter, with from and to set to the given
    /// dates. Equivalent to `withDateRange(Date, Date)`.
    pub fn with_date_range(&self, from: Option<&Date>, to: Option<&Date>) -> CurrencyFilter {
        let from_long = from.map_or(i64::MIN, Date::get_time);
        let to_long = to.map_or(i64::MAX, Date::get_time);
        CurrencyFilter::new(self.region.clone(), self.currency.clone(), from_long, to_long, self.tender_only)
    }

    /// Returns a copy of this filter that accepts all currencies in use on
    /// the given date. Equivalent to `withDate(long)`.
    pub fn with_date_millis(&self, date: i64) -> CurrencyFilter {
        CurrencyFilter::new(self.region.clone(), self.currency.clone(), date, date, self.tender_only)
    }

    /// Returns a copy of this filter that accepts all currencies that were
    /// in use at some point between the given dates. Equivalent to
    /// `withDateRange(long, long)`.
    pub fn with_date_range_millis(&self, from: i64, to: i64) -> CurrencyFilter {
        CurrencyFilter::new(self.region.clone(), self.currency.clone(), from, to, self.tender_only)
    }

    /// Returns a copy of this filter that filters for currencies that were
    /// either once used, are used, or will be used as tender. Equivalent to
    /// `withTender()`.
    pub fn with_tender(&self) -> CurrencyFilter {
        CurrencyFilter::new(self.region.clone(), self.currency.clone(), self.from, self.to, true)
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut hc: i32 = 0;
        if let Some(region) = &self.region {
            hc = string_hash(region);
        }
        if let Some(currency) = &self.currency {
            hc = hc.wrapping_mul(31).wrapping_add(string_hash(currency));
        }
        hc = hc.wrapping_mul(31).wrapping_add(self.from as i32);
        hc = hc.wrapping_mul(31).wrapping_add(((self.from as u64) >> 32) as i32);
        hc = hc.wrapping_mul(31).wrapping_add(self.to as i32);
        hc = hc.wrapping_mul(31).wrapping_add(((self.to as u64) >> 32) as i32);
        hc = hc.wrapping_mul(31).wrapping_add(if self.tender_only { 1 } else { 0 });
        hc
    }
}

impl fmt::Display for CurrencyFilter {
    /// Equivalent to `toString()` (`debugString`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&debug_string(
            "CurrencyFilter",
            &[
                ("region", self.region.clone().map(DebugField::Str)),
                ("currency", self.currency.clone().map(DebugField::Str)),
                ("from", Some(DebugField::Long(self.from))),
                ("to", Some(DebugField::Long(self.to))),
                ("tenderOnly", Some(DebugField::Str(self.tender_only.to_string()))),
            ],
        ))
    }
}

/// Represents the raw information about fraction digits and rounding
/// increment. Equivalent to `CurrencyMetaInfo.CurrencyDigits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CurrencyDigits {
    /// Number of fraction digits used to display this currency.
    pub fraction_digits: i32,
    /// Rounding increment used when displaying this currency.
    pub rounding_increment: i32,
}

impl CurrencyDigits {
    /// Constructor for CurrencyDigits. Equivalent to `CurrencyDigits(int,
    /// int)`.
    pub fn new(fraction_digits: i32, rounding_increment: i32) -> Self {
        Self { fraction_digits, rounding_increment }
    }
}

impl fmt::Display for CurrencyDigits {
    /// Equivalent to `toString()` (`debugString`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&debug_string(
            "CurrencyDigits",
            &[
                ("fractionDigits", Some(DebugField::Str(self.fraction_digits.to_string()))),
                ("roundingIncrement", Some(DebugField::Str(self.rounding_increment.to_string()))),
            ],
        ))
    }
}

/// Represents a complete currency info record listing the region, currency,
/// from and to dates, and priority. Equivalent to
/// `CurrencyMetaInfo.CurrencyInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurrencyInfo {
    /// Region code where currency is used.
    pub region: Option<String>,
    /// The three-letter ISO currency code.
    pub code: Option<String>,
    /// Date on which the currency was first officially used in the region.
    pub from: i64,
    /// Date at which the currency stopped being officially used in the
    /// region.
    pub to: i64,
    /// Preference order of currencies being used at the same time in the
    /// region.
    pub priority: i32,
    tender: bool,
}

impl CurrencyInfo {
    /// Equivalent to `CurrencyInfo(String, String, long, long, int)`
    /// (deprecated): a tender currency.
    pub fn new(region: Option<String>, code: Option<String>, from: i64, to: i64, priority: i32) -> Self {
        Self::with_tender(region, code, from, to, priority, true)
    }

    /// Constructs a currency info. Equivalent to `CurrencyInfo(String,
    /// String, long, long, int, boolean)` (internal).
    pub fn with_tender(region: Option<String>, code: Option<String>, from: i64, to: i64, priority: i32, tender: bool) -> Self {
        Self { region, code, from, to, priority, tender }
    }

    /// Determine whether or not this currency was once used, is used, or
    /// will be used as tender in this region. Equivalent to `isTender()`.
    pub fn is_tender(&self) -> bool {
        self.tender
    }
}

impl fmt::Display for CurrencyInfo {
    /// Equivalent to `toString()` (`debugString`: the public fields only).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&debug_string(
            "CurrencyInfo",
            &[
                ("region", self.region.clone().map(DebugField::Str)),
                ("code", self.code.clone().map(DebugField::Str)),
                ("from", Some(DebugField::Long(self.from))),
                ("to", Some(DebugField::Long(self.to))),
                ("priority", Some(DebugField::Str(self.priority.to_string()))),
            ],
        ))
    }
}

/// A public field value `debugString` renders: a `Long` (a date) or any
/// other value's `String.valueOf`.
enum DebugField {
    Long(i64),
    Str(String),
}

/// `dateString(long)`.
fn date_string(date: i64) -> Option<String> {
    if date == i64::MAX || date == i64::MIN {
        return None;
    }
    Some(Grego::time_to_string(date))
}

/// `debugString(Object)`: Java reflects over `getClass().getFields()`, the
/// public fields in declaration order; the callers list them.
fn debug_string(simple_name: &str, fields: &[(&str, Option<DebugField>)]) -> String {
    let mut sb = String::new();
    for (name, v) in fields {
        if let Some(v) = v {
            let s = match v {
                DebugField::Long(l) => date_string(*l),
                DebugField::Str(s) => Some(s.clone()),
            };
            let s = match s {
                None => continue,
                Some(s) => s,
            };
            if !sb.is_empty() {
                sb.push(',');
            }
            sb.push_str(name);
            sb.push_str("='");
            sb.push_str(&s);
            sb.push('\'');
        }
    }
    format!("{simple_name}({sb})")
}

/// `String.hashCode()`.
fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}
