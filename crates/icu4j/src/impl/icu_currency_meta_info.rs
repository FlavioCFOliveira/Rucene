// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUCurrencyMetaInfo`.

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::to_rust_string;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{icu_data_class_loader, ICUResourceBundle};
use crate::text::currency_meta_info::{CurrencyDigits, CurrencyFilter, CurrencyInfo, CurrencyMetaInfo};
use crate::text::rule_based_number_format::npe;
use crate::util::currency::CurrencyUsage;
use crate::util::u_resource_bundle::UResourceBundle;

/// `Region` (private): the region feature bit.
const REGION: i32 = 1;
/// `Currency` (private): the currency feature bit.
const CURRENCY: i32 = 2;
/// `Date` (private): the date feature bit.
const DATE: i32 = 4;
/// `Tender` (private): the tender feature bit.
const TENDER: i32 = 8;
/// `Everything` (private).
const EVERYTHING: i32 = i32::MAX;

/// `MASK` (private): the low 32 bits of a `long`.
const MASK: i64 = 4294967295;

/// The result of a resource access in a method whose Java signature
/// declares no exception: an unchecked exception of the resource code
/// surfaces as a panic carrying its text.
fn unchecked<T>(r: Result<T>) -> T {
    r.unwrap_or_else(|e| panic!("{e}"))
}

/// A non-`null` bundle (Java's dereference of the result of `at(...)`).
fn non_null(b: Option<Arc<ICUResourceBundle>>) -> Arc<ICUResourceBundle> {
    b.unwrap_or_else(|| panic!("{}", npe()))
}

/// ICU's currency meta info data.
///
/// Equivalent to `com.ibm.icu.impl.ICUCurrencyMetaInfo` (a
/// `CurrencyMetaInfo`). The trait's methods are infallible, as their Java
/// signatures: an unchecked exception of the resource data surfaces as a
/// panic carrying its text.
#[derive(Debug)]
pub struct ICUCurrencyMetaInfo {
    /// `regionInfo`: the `CurrencyMap` table (`None` is Java's `null`).
    region_info: Option<Arc<ICUResourceBundle>>,
    /// `digitInfo`: the `CurrencyMeta` table (`None` is Java's `null`).
    digit_info: Option<Arc<ICUResourceBundle>>,
}

impl ICUCurrencyMetaInfo {
    /// Loads the `CurrencyMap` and `CurrencyMeta` tables of the currency
    /// `supplementalData`. Equivalent to `ICUCurrencyMetaInfo()`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` without the data.
    pub fn new() -> Result<Self> {
        let bundle = ICUResourceBundle::get_bundle_instance(
            Some(ICUData::ICU_CURR_BASE_NAME),
            "supplementalData",
            icu_data_class_loader(),
            false,
        )?;
        let bundle = UResourceBundle::Icu(bundle);
        let region_info = match bundle.find_top_level("CurrencyMap")? {
            Some(b) => Some(b.as_icu()?.clone()),
            None => None,
        };
        let digit_info = match bundle.find_top_level("CurrencyMeta")? {
            Some(b) => Some(b.as_icu()?.clone()),
            None => None,
        };
        Ok(ICUCurrencyMetaInfo { region_info, digit_info })
    }

    /// `collect(Collector<T> collector, CurrencyFilter filter)` (private).
    fn collect<T>(&self, collector: &mut dyn Collector<T>, filter: &CurrencyFilter) -> Vec<T> {
        // We rely on the fact that the data lists the regions in order, and the
        // priorities in order within region.  This means we don't need
        // to sort the results to ensure the ordering matches the spec.

        let mut needed = collector.collects();
        if filter.region.is_some() {
            needed |= REGION;
        }
        if filter.currency.is_some() {
            needed |= CURRENCY;
        }
        if filter.from != i64::MIN || filter.to != i64::MAX {
            needed |= DATE;
        }
        if filter.tender_only {
            needed |= TENDER;
        }

        if needed != 0 {
            let region_info = self.region_info.as_ref().unwrap_or_else(|| panic!("{}", npe()));
            if let Some(region) = &filter.region {
                if let Some(b) = unchecked(region_info.find_with_fallback(region)) {
                    self.collect_region(collector, filter, needed, &b);
                }
            } else {
                for i in 0..region_info.get_size() {
                    let b = non_null(unchecked(region_info.at(i)));
                    self.collect_region(collector, filter, needed, &b);
                }
            }
        }

        collector.get_list()
    }

    /// `collectRegion(Collector<T> collector, CurrencyFilter filter, int
    /// needed, ICUResourceBundle b)` (private).
    fn collect_region<T>(&self, collector: &mut dyn Collector<T>, filter: &CurrencyFilter, needed: i32, b: &Arc<ICUResourceBundle>) {
        let region = b.get_key().map(str::to_string);
        if needed == REGION {
            collector.collect(b.get_key().map(str::to_string), None, 0, 0, -1, false);
            return;
        }

        for i in 0..b.get_size() {
            let r = non_null(unchecked(b.at(i)));
            if r.get_size() == 0 {
                // AQ[0] is an empty array instead of a table, so the bundle is null.
                // There's no data here, so we skip this entirely.
                // We'd do a type test, but the ResourceArray type is private.
                continue;
            }
            let mut currency: Option<String> = None;
            let mut from = i64::MIN;
            let mut to = i64::MAX;
            let mut tender = true;

            if (needed & CURRENCY) != 0 {
                let curr_bundle = non_null(unchecked(r.at_key("id")));
                let c = to_rust_string(&unchecked(curr_bundle.get_string()));
                if let Some(fc) = &filter.currency {
                    if *fc != c {
                        continue;
                    }
                }
                currency = Some(c);
            }

            if (needed & DATE) != 0 {
                from = self.get_date(unchecked(r.at_key("from")), i64::MIN, false);
                to = self.get_date(unchecked(r.at_key("to")), i64::MAX, true);
                // In the data, to is always > from.  This means that when we have a range
                // from == to, the comparisons below will always do the right thing, despite
                // the range being technically empty.  It really should be [from, from+1) but
                // this way we don't need to fiddle with it.
                if filter.from > to {
                    continue;
                }
                if filter.to < from {
                    continue;
                }
            }
            if (needed & TENDER) != 0 {
                let tender_bundle = unchecked(r.at_key("tender"));
                tender = match tender_bundle {
                    None => true,
                    Some(t) => unchecked(t.get_string()) == crate::jdk::units("true"),
                };
                if filter.tender_only && !tender {
                    continue;
                }
            }

            // data lists elements in priority order, so 'i' suffices
            collector.collect(region.clone(), currency, from, to, i, tender);
        }
    }

    /// `getDate(ICUResourceBundle b, long defaultValue, boolean endOfDay)`
    /// (private): the date of a two-int vector, `default_value` without
    /// one.
    fn get_date(&self, b: Option<Arc<ICUResourceBundle>>, default_value: i64, _end_of_day: bool) -> i64 {
        match b {
            None => default_value,
            Some(b) => {
                let values = unchecked(b.get_int_vector());
                (i64::from(values[0]) << 32) | (i64::from(values[1]) & MASK)
            }
        }
    }
}

impl CurrencyMetaInfo for ICUCurrencyMetaInfo {
    fn currency_info(&self, filter: &CurrencyFilter) -> Vec<CurrencyInfo> {
        self.collect(&mut InfoCollector::default(), filter)
    }

    fn currencies(&self, filter: &CurrencyFilter) -> Vec<String> {
        self.collect(&mut CurrencyCollector::default(), filter)
    }

    fn regions(&self, filter: &CurrencyFilter) -> Vec<String> {
        self.collect(&mut RegionCollector::default(), filter)
    }

    fn currency_digits(&self, iso_code: &str) -> CurrencyDigits {
        self.currency_digits_usage(iso_code, CurrencyUsage::Standard)
    }

    fn currency_digits_usage(&self, iso_code: &str, currency_purpose: CurrencyUsage) -> CurrencyDigits {
        let digit_info = self.digit_info.as_ref().unwrap_or_else(|| panic!("{}", npe()));
        let b = match unchecked(digit_info.find_with_fallback(iso_code)) {
            Some(b) => b,
            None => non_null(unchecked(digit_info.find_with_fallback("DEFAULT"))),
        };
        let data = unchecked(b.get_int_vector());
        if currency_purpose == CurrencyUsage::Cash {
            CurrencyDigits::new(data[2], data[3])
        } else if currency_purpose == CurrencyUsage::Standard {
            CurrencyDigits::new(data[0], data[1])
        } else {
            CurrencyDigits::new(data[0], data[1])
        }
    }
}

/// Utility, just because I don't like the n^2 behavior of using
/// list.contains to build a list of unique items. If we used java 6 we
/// could use their class for this. Equivalent to `UniqueList<T>` (private
/// static).
#[derive(Debug)]
struct UniqueList<T> {
    seen: HashSet<T>,
    list: Vec<T>,
}

impl<T> Default for UniqueList<T> {
    fn default() -> Self {
        UniqueList { seen: HashSet::new(), list: Vec::new() }
    }
}

impl<T: Eq + Hash + Clone> UniqueList<T> {
    fn add(&mut self, value: T) {
        if !self.seen.contains(&value) {
            self.list.push(value.clone());
            self.seen.insert(value);
        }
    }

    fn list(&self) -> Vec<T> {
        self.list.clone()
    }
}

/// `Collector<T>` (private interface).
trait Collector<T> {
    /// A bitmask of Region/Currency/Date indicating which features we
    /// collect. Equivalent to `collects()`.
    fn collects(&self) -> i32;

    /// Called with data passed by filter. Values not collected by filter
    /// should be ignored. `region` the region code (`None` if ignored),
    /// `currency` the currency code (`None` if ignored), `from` start time
    /// (0 if ignored), `to` end time (0 if ignored), `priority` priority
    /// (-1 if ignored), `tender` true if currency is legal tender.
    /// Equivalent to `collect(String region, String currency, long from,
    /// long to, int priority, boolean tender)`.
    fn collect(&mut self, region: Option<String>, currency: Option<String>, from: i64, to: i64, priority: i32, tender: bool);

    /// Return the list of unique items in the order in which we encountered
    /// them for the first time. Equivalent to `getList()`.
    fn get_list(&self) -> Vec<T>;
}

/// `InfoCollector` (private static). Data is already unique by
/// region/priority, so we don't need to be concerned about duplicates.
#[derive(Debug, Default)]
struct InfoCollector {
    result: Vec<CurrencyInfo>,
}

impl Collector<CurrencyInfo> for InfoCollector {
    fn collect(&mut self, region: Option<String>, currency: Option<String>, from: i64, to: i64, priority: i32, tender: bool) {
        self.result.push(CurrencyInfo::with_tender(region, currency, from, to, priority, tender));
    }

    fn get_list(&self) -> Vec<CurrencyInfo> {
        self.result.clone()
    }

    fn collects(&self) -> i32 {
        EVERYTHING
    }
}

/// `RegionCollector` (private static).
#[derive(Debug, Default)]
struct RegionCollector {
    result: UniqueList<Option<String>>,
}

impl Collector<String> for RegionCollector {
    fn collect(&mut self, region: Option<String>, _currency: Option<String>, _from: i64, _to: i64, _priority: i32, _tender: bool) {
        self.result.add(region);
    }

    fn collects(&self) -> i32 {
        REGION
    }

    fn get_list(&self) -> Vec<String> {
        self.result
            .list()
            .into_iter()
            .map(|r| r.expect("every region table of CurrencyMap has a key"))
            .collect()
    }
}

/// `CurrencyCollector` (private static).
#[derive(Debug, Default)]
struct CurrencyCollector {
    result: UniqueList<Option<String>>,
}

impl Collector<String> for CurrencyCollector {
    fn collect(&mut self, _region: Option<String>, currency: Option<String>, _from: i64, _to: i64, _priority: i32, _tender: bool) {
        self.result.add(currency);
    }

    fn collects(&self) -> i32 {
        CURRENCY
    }

    fn get_list(&self) -> Vec<String> {
        self.result
            .list()
            .into_iter()
            .map(|c| c.expect("the Currency bit of this collector makes collectRegion read every currency"))
            .collect()
    }
}
