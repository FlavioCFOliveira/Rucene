// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.range.StandardPluralRanges`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;

use crate::jdk::to_rust_string;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;
use crate::util::u_resource_type_mismatch_exception::type_mismatch;

/// The plural form of a range from the plural forms of its two ends, per
/// CLDR `pluralRanges` data.
///
/// Equivalent to `com.ibm.icu.impl.number.range.StandardPluralRanges`.
/// Instances are shared as `Arc`s ([`DEFAULT`] is one shared instance, as
/// Java's constant).
#[derive(Debug, Clone, Default)]
pub struct StandardPluralRanges {
    flat_triples: Vec<StandardPlural>,
    num_triples: i32,
}

/// The `volatile` cache `languageToSet` (Java's `null` is `None`).
static LANGUAGE_TO_SET: Mutex<Option<Arc<HashMap<String, String>>>> = Mutex::new(None);

/// `StandardPluralRanges.DEFAULT`: the empty instance (every range
/// resolves to `OTHER`).
pub static DEFAULT: LazyLock<Arc<StandardPluralRanges>> =
    LazyLock::new(|| Arc::new(StandardPluralRanges::default()));

/// `StandardPluralRanges.PluralRangeSetsDataSink`.
struct PluralRangeSetsDataSink<'a> {
    output: &'a mut HashMap<String, String>,
}

impl Sink for PluralRangeSetsDataSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, value) {
            let k = key.to_java_string();
            debug_assert!(ULocale::new(&k)
                .map(|l| l.get_language() == k)
                .unwrap_or(false));
            self.output.insert(k, value.to_java_string());
            i += 1;
        }
        Ok(())
    }
}

/// `StandardPluralRanges.PluralRangesDataSink`.
struct PluralRangesDataSink<'a> {
    output: &'a mut StandardPluralRanges,
}

impl Sink for PluralRangesDataSink<'_> {
    fn put(&mut self, _key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let entries_array = value.get_array()?;
        self.output.set_capacity(entries_array.get_size());
        let mut i = 0;
        while entries_array.get_value(i, value) {
            let plural_forms_array = value.get_array()?;
            if plural_forms_array.get_size() != 3 {
                return Err(type_mismatch(
                    "Expected 3 elements in pluralRanges.txt array",
                ));
            }
            plural_forms_array.get_value(0, value);
            let first = StandardPlural::from_string(&to_rust_string(&value.get_string()?))?;
            plural_forms_array.get_value(1, value);
            let second = StandardPlural::from_string(&to_rust_string(&value.get_string()?))?;
            plural_forms_array.get_value(2, value);
            let result = StandardPlural::from_string(&to_rust_string(&value.get_string()?))?;
            self.output.add_plural_range(first, second, result);
            i += 1;
        }
        Ok(())
    }
}

fn plural_ranges_bundle() -> Result<Arc<ICUResourceBundle>> {
    match UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "pluralRanges")? {
        UResourceBundle::Icu(b) => Ok(b),
        UResourceBundle::Wrapper(_) => panic!(
            "java.lang.ClassCastException: class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle"
        ),
    }
}

/// `getLanguageToSet()`.
fn get_language_to_set() -> Result<Arc<HashMap<String, String>>> {
    let candidate = LANGUAGE_TO_SET
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let candidate = match candidate {
        Some(c) => c,
        None => {
            let mut map = HashMap::new();
            let mut sink = PluralRangeSetsDataSink { output: &mut map };
            let resource = plural_ranges_bundle()?;
            resource.get_all_items_with_fallback("locales", &mut sink)?;
            Arc::new(map)
        }
    };
    // Check if another thread set languageToSet in the mean time
    let mut guard = LANGUAGE_TO_SET.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(candidate);
    }
    Ok(guard.clone().unwrap_or_default())
}

/// `getPluralRangesData(String set, StandardPluralRanges out)`.
fn get_plural_ranges_data(set: &str, out: &mut StandardPluralRanges) -> Result<()> {
    let resource = plural_ranges_bundle()?;
    let mut sb = String::new();
    sb.push_str("rules/");
    sb.push_str(set);
    let key = sb;
    let mut sink = PluralRangesDataSink { output: out };
    resource.get_all_items_with_fallback(&key, &mut sink)
}

impl StandardPluralRanges {
    /// `forLocale(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` or `UResourceTypeMismatchException` for
    /// broken data.
    pub fn for_locale(locale: &ULocale) -> Result<Arc<StandardPluralRanges>> {
        Self::for_set(Self::get_set_for_locale(locale)?.as_deref())
    }

    /// `forSet(String set)`: [`DEFAULT`] for Java's `null` set.
    ///
    /// # Errors
    ///
    /// As [`for_locale`](Self::for_locale).
    pub fn for_set(set: Option<&str>) -> Result<Arc<StandardPluralRanges>> {
        let mut result = StandardPluralRanges::default();
        let Some(set) = set else {
            return Ok(DEFAULT.clone());
        };
        get_plural_ranges_data(set, &mut result)?;
        Ok(Arc::new(result))
    }

    /// `getSetForLocale(ULocale locale)`: the rule set of the locale's
    /// language, `None` when it has none.
    ///
    /// # Errors
    ///
    /// As [`for_locale`](Self::for_locale).
    pub fn get_set_for_locale(locale: &ULocale) -> Result<Option<String>> {
        Ok(get_language_to_set()?.get(&locale.get_language()).cloned())
    }

    fn add_plural_range(
        &mut self,
        first: StandardPlural,
        second: StandardPlural,
        result: StandardPlural,
    ) {
        let n = (3 * self.num_triples) as usize;
        self.flat_triples[n] = first;
        self.flat_triples[n + 1] = second;
        self.flat_triples[n + 2] = result;
        self.num_triples += 1;
    }

    fn set_capacity(&mut self, length: i32) {
        // Java's `new StandardPlural[length*3]` holds nulls; every slot read
        // by `resolve` is written by `addPluralRange` first.
        self.flat_triples = vec![StandardPlural::Other; (length * 3) as usize];
    }

    /// `resolve(StandardPlural first, StandardPlural second)`: the plural
    /// form of the range, `OTHER` when no rule matches.
    pub fn resolve(&self, first: StandardPlural, second: StandardPlural) -> StandardPlural {
        for i in 0..self.num_triples as usize {
            if first == self.flat_triples[3 * i] && second == self.flat_triples[3 * i + 1] {
                return self.flat_triples[3 * i + 2];
            }
        }
        // Default fallback
        StandardPlural::Other
    }
}
