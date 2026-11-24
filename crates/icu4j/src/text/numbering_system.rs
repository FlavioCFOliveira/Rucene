// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NumberingSystem`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{to_rust_string, units};
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::soft_cache::SoftCache;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// `OTHER_NS_KEYWORDS`.
const OTHER_NS_KEYWORDS: [&str; 3] = ["native", "traditional", "finance"];

/// Defines numbering systems: either a radix and a string of digits
/// (numeric), or a set of RBNF rules (algorithmic).
///
/// Equivalent to `com.ibm.icu.text.NumberingSystem`. Instances are shared
/// as `Arc`s (Java hands out the cached objects). The description is
/// UTF-16 (`desc`, a digit string or a rule-set name); `name` is `None`
/// for Java's `null` (instances made by
/// [`get_instance_with`](Self::get_instance_with)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberingSystem {
    desc: Vec<u16>,
    radix: i32,
    algorithmic: bool,
    name: Option<String>,
}

/// `NumberingSystem.LATIN`: the default numbering system (`latn`), Java's
/// `lookupInstanceByName("latn")`, `None` for its `null` when the data
/// lacks it.
pub static LATIN: LazyLock<Option<Arc<NumberingSystem>>> =
    LazyLock::new(|| match lookup_instance_by_name("latn") {
        Ok(ns) => ns,
        Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
    });

/// `NumberingSystem.LocaleLookupData`.
#[derive(Debug, Clone)]
struct LocaleLookupData {
    locale: ULocale,
    numbers_keyword: String,
}

/// `cachedLocaleData`.
static CACHED_LOCALE_DATA: LazyLock<SoftCache<String, Arc<NumberingSystem>, LocaleLookupData>> =
    LazyLock::new(|| {
        SoftCache::new(|_key: &String, data: LocaleLookupData| {
            Ok(Some(lookup_instance_by_locale(&data)?))
        })
    });

/// `cachedStringData`: the `createInstance` of Java answers `null` for an
/// unknown name, which `SoftCache` returns without caching it.
static CACHED_STRING_DATA: LazyLock<SoftCache<String, Arc<NumberingSystem>, ()>> =
    LazyLock::new(|| SoftCache::new(|key: &String, _unused: ()| lookup_instance_by_name(key)));

impl Default for NumberingSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn code_point_count(s: &[u16]) -> i32 {
    let mut n = 0;
    let mut i = 0;
    while i < s.len() {
        if crate::jdk::is_high_surrogate(s[i])
            && i + 1 < s.len()
            && crate::jdk::is_low_surrogate(s[i + 1])
        {
            i += 2;
        } else {
            i += 1;
        }
        n += 1;
    }
    n
}

impl NumberingSystem {
    /// `new NumberingSystem()`: radix 10, numeric, `0123456789`, `latn`.
    pub fn new() -> Self {
        Self {
            radix: 10,
            algorithmic: false,
            desc: units("0123456789"),
            name: Some("latn".to_string()),
        }
    }

    /// `getInstance(int radix_in, boolean isAlgorithmic_in, String
    /// desc_in)`: a numbering system without a name.
    ///
    /// # Errors
    ///
    /// As [`get_instance_named`](Self::get_instance_named).
    pub fn get_instance_with(
        radix_in: i32,
        is_algorithmic_in: bool,
        desc_in: &[u16],
    ) -> Result<NumberingSystem> {
        Self::get_instance_named(None, radix_in, is_algorithmic_in, desc_in)
    }

    /// The private `getInstance(String name_in, int radix_in, boolean
    /// isAlgorithmic_in, String desc_in)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid radix for numbering system")`
    /// for a radix below 2; `IllegalArgumentException("Invalid digit
    /// string for numbering system")` for a numeric system whose
    /// description is not `radix` code points or not 10 code points.
    fn get_instance_named(
        name_in: Option<&str>,
        radix_in: i32,
        is_algorithmic_in: bool,
        desc_in: &[u16],
    ) -> Result<NumberingSystem> {
        if radix_in < 2 {
            return Err(LuceneError::illegal_argument(
                "Invalid radix for numbering system",
            ));
        }
        if !is_algorithmic_in
            && (code_point_count(desc_in) != radix_in || !Self::is_valid_digit_string(desc_in))
        {
            return Err(LuceneError::illegal_argument(
                "Invalid digit string for numbering system",
            ));
        }
        let mut ns = NumberingSystem::new();
        ns.radix = radix_in;
        ns.algorithmic = is_algorithmic_in;
        ns.desc = desc_in.to_vec();
        ns.name = name_in.map(str::to_string);
        Ok(ns)
    }

    /// `getInstance(Locale inLocale)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_for`](Self::get_instance_for).
    pub fn get_instance_for_locale(in_locale: Option<&Locale>) -> Result<Arc<NumberingSystem>> {
        let ulocale = ULocale::for_locale(in_locale)?;
        match ulocale {
            Some(l) => Self::get_instance_for(&l),
            None => Err(LuceneError::null_pointer()),
        }
    }

    /// `getInstance(ULocale locale)`: the numbering system of the locale's
    /// `numbers` keyword, or of its default numbering system.
    ///
    /// # Errors
    ///
    /// The errors of reading the locale data, other than a missing
    /// resource (which answers the default system).
    pub fn get_instance_for(locale: &ULocale) -> Result<Arc<NumberingSystem>> {
        // Check for @numbers
        let mut ns_resolved = true;
        let mut numbers_keyword = locale.get_keyword_value("numbers");
        match &numbers_keyword {
            Some(k) => {
                for keyword in OTHER_NS_KEYWORDS {
                    if k == keyword {
                        ns_resolved = false;
                        break;
                    }
                }
            }
            None => {
                numbers_keyword = Some("default".to_string());
                ns_resolved = false;
            }
        }
        let mut numbers_keyword = numbers_keyword.unwrap_or_default();

        if ns_resolved {
            if let Some(ns) = Self::get_instance_by_name(&numbers_keyword)? {
                return Ok(ns);
            }
            // If the @numbers keyword points to a bogus numbering system name,
            // we return the default for the locale.
            numbers_keyword = "default".to_string();
        }

        // Attempt to get the numbering system from the cache
        let base_name = locale.get_base_name();
        // TODO: Caching by locale+numbersKeyword could yield a large cache.
        // Try to load for each locale the mappings from OTHER_NS_KEYWORDS and default
        // to real numbering system names; can we get those from supplemental data?
        // Then look up those mappings for the locale and resolve the keyword.
        let key = format!("{base_name}@numbers={numbers_keyword}");
        let locale_lookup_data = LocaleLookupData {
            locale: locale.clone(),
            numbers_keyword,
        };
        CACHED_LOCALE_DATA
            .get_instance(key, locale_lookup_data)?
            .ok_or_else(|| {
                LuceneError::null_pointer_with_message("NumberingSystem cache returned null")
            })
    }

    /// `getInstance()`: the numbering system of the default `FORMAT`
    /// locale.
    ///
    /// # Errors
    ///
    /// As [`get_instance_for`](Self::get_instance_for).
    pub fn get_instance() -> Result<Arc<NumberingSystem>> {
        Self::get_instance_for(&ULocale::get_default_category(Category::Format))
    }

    /// `getInstanceByName(String name)`: the numbering system named
    /// `name`, `None` (Java's `null`) when there is none.
    ///
    /// # Errors
    ///
    /// The errors of reading the data, other than a missing resource.
    pub fn get_instance_by_name(name: &str) -> Result<Option<Arc<NumberingSystem>>> {
        CACHED_STRING_DATA.get_instance(name.to_string(), ())
    }

    /// `getAvailableNames()`: the names of every numbering system in the
    /// data.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data is missing.
    pub fn get_available_names() -> Result<Vec<String>> {
        let numbering_systems_info =
            UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "numberingSystems")?;
        let ns_current = numbering_systems_info.get("numberingSystems")?;
        let mut output = Vec::new();
        let mut it = ns_current.get_iterator();
        while it.has_next() {
            let temp = it.next()?;
            if let Some(ns_name) = temp.get_key() {
                output.push(ns_name);
            }
        }
        Ok(output)
    }

    /// `isValidDigitString(String str)`: whether `str` has exactly 10 code
    /// points.
    pub fn is_valid_digit_string(str: &[u16]) -> bool {
        code_point_count(str) == 10
    }

    /// `getRadix()`.
    pub fn get_radix(&self) -> i32 {
        self.radix
    }

    /// `getDescription()`: the digits (numeric) or the RBNF rule set name
    /// (algorithmic).
    pub fn get_description(&self) -> &[u16] {
        &self.desc
    }

    /// `getName()`.
    pub fn get_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `isAlgorithmic()`.
    pub fn is_algorithmic(&self) -> bool {
        self.algorithmic
    }
}

/// `lookupInstanceByLocale(LocaleLookupData localeLookupData)`.
fn lookup_instance_by_locale(
    locale_lookup_data: &LocaleLookupData,
) -> Result<Arc<NumberingSystem>> {
    let locale = &locale_lookup_data.locale;
    let rb = match UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale)) {
        Ok(UResourceBundle::Icu(b)) => match b.get_with_fallback("NumberElements") {
            Ok(r) => r,
            Err(e) if is_missing_resource(&e) => return Ok(Arc::new(NumberingSystem::new())),
            Err(e) => return Err(e),
        },
        Ok(UResourceBundle::Wrapper(_)) => panic!(
            "java.lang.ClassCastException: class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle"
        ),
        Err(e) if is_missing_resource(&e) => return Ok(Arc::new(NumberingSystem::new())),
        Err(e) => return Err(e),
    };

    let mut numbers_keyword = locale_lookup_data.numbers_keyword.clone();
    let mut resolved_numbering_system: Option<Vec<u16>> = None;
    loop {
        match rb.get_string_with_fallback(&numbers_keyword) {
            Ok(s) => {
                resolved_numbering_system = Some(s);
                break;
            }
            Err(e) if is_missing_resource(&e) => {
                // Fall back as needed
                if numbers_keyword == "native" || numbers_keyword == "finance" {
                    numbers_keyword = "default".to_string();
                } else if numbers_keyword == "traditional" {
                    numbers_keyword = "native".to_string();
                } else {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }

    let mut ns = None;
    if let Some(r) = resolved_numbering_system {
        ns = NumberingSystem::get_instance_by_name(&to_rust_string(&r))?;
    }
    Ok(ns.unwrap_or_else(|| Arc::new(NumberingSystem::new())))
}

/// `lookupInstanceByName(String name)`: `None` (Java's `null`) when the
/// data has no such system.
fn lookup_instance_by_name(name: &str) -> Result<Option<Arc<NumberingSystem>>> {
    let read = || -> Result<(i32, bool, Vec<u16>)> {
        let numbering_systems_info =
            UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "numberingSystems")?;
        let ns_current = numbering_systems_info.get("numberingSystems")?;
        let ns_top = ns_current.get(name)?;
        let description = ns_top.get_string_by_key("desc")?;
        let ns_radix_bundle = ns_top.get("radix")?;
        let ns_alg_bundle = ns_top.get("algorithmic")?;
        let radix = ns_radix_bundle.get_int()?;
        let algorithmic = ns_alg_bundle.get_int()?;
        Ok((radix, algorithmic == 1, description))
    };
    let (radix, is_algorithmic, description) = match read() {
        Ok(v) => v,
        Err(e) if is_missing_resource(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(Arc::new(NumberingSystem::get_instance_named(
        Some(name),
        radix,
        is_algorithmic,
        &description,
    )?)))
}
