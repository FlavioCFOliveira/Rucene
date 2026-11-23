// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CurrencyPluralInfo`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::{java_identity_hash_code_stand_in, java_string_hash_code, JavaHashMap};
use rucene_queryparser::jdk::locale::Locale;
use crate::util::u_locale::Category;

use crate::jdk::index_of_unit;
use crate::r#impl::currency_data;
use crate::r#impl::number::decimal_format_properties::string_hash_code;
use crate::text::number_format::{self, NUMBERSTYLE};
use crate::text::plural_rules::{FixedDecimal, PluralRules};
use crate::util::u_locale::ULocale;

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 1;

/// `tripleCurrencyStr`.
const TRIPLE_CURRENCY_STR: [u16; 3] = [0xA4, 0xA4, 0xA4];

/// `defaultCurrencyPluralPattern`: `"0.## ¤¤¤"` with a leading U+0000
/// (`{0, '.', '#', '#', ' ', 0xA4, 0xA4, 0xA4}`).
const DEFAULT_CURRENCY_PLURAL_PATTERN: [u16; 8] = [0, 0x2E, 0x23, 0x23, 0x20, 0xA4, 0xA4, 0xA4];

/// This class represents the information needed by DecimalFormat to format
/// currency plural, such as "3.00 US dollars" or "1.00 US dollar".
/// DecimalFormat creates for itself an instance of CurrencyPluralInfo from
/// its locale data. If you need to change any of these symbols, you can get
/// the CurrencyPluralInfo object from your DecimalFormat and modify it.
///
/// Equivalent to `com.ibm.icu.text.CurrencyPluralInfo` (`Cloneable`,
/// `Serializable`): [`Clone`] is `clone()` (the pattern map is copied, the
/// plural rules shared).
#[derive(Debug, Clone)]
pub struct CurrencyPluralInfo {
    /// Map from plural count to currency plural pattern, for example
    /// one->"{0} {1}", other->"{0} {1}s".
    plural_count_to_currency_unit_pattern: JavaHashMap<Vec<u16>>,
    /// The plural rule used to format currency plural.
    plural_rules: Option<Arc<PluralRules>>,
    /// locale
    ulocale: ULocale,
}

impl CurrencyPluralInfo {
    /// Create a CurrencyPluralInfo object for the default `FORMAT` locale.
    /// Equivalent to `CurrencyPluralInfo()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn new() -> Result<Self> {
        Self::new_ulocale(&ULocale::get_default_category(Category::Format))
    }

    /// Create a CurrencyPluralInfo object for the given locale. Equivalent
    /// to `CurrencyPluralInfo(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn new_locale(locale: &Locale) -> Result<Self> {
        let uloc = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::new_ulocale(&uloc)
    }

    /// Create a CurrencyPluralInfo object for the given locale. Equivalent
    /// to `CurrencyPluralInfo(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn new_ulocale(locale: &ULocale) -> Result<Self> {
        let mut info = CurrencyPluralInfo {
            plural_count_to_currency_unit_pattern: JavaHashMap::new(),
            plural_rules: None,
            ulocale: locale.clone(),
        };
        info.initialize(locale)?;
        Ok(info)
    }

    /// Gets a CurrencyPluralInfo instance for the default locale. Equivalent
    /// to `getInstance()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance() -> Result<Self> {
        Self::new()
    }

    /// Gets a CurrencyPluralInfo instance for the given locale. Equivalent
    /// to `getInstance(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance_locale(locale: &Locale) -> Result<Self> {
        Self::new_locale(locale)
    }

    /// Gets a CurrencyPluralInfo instance for the given locale. Equivalent
    /// to `getInstance(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance_ulocale(locale: &ULocale) -> Result<Self> {
        Self::new_ulocale(locale)
    }

    /// Gets plural rules of this locale, used for currency plural format.
    /// Equivalent to `getPluralRules()`.
    pub fn get_plural_rules(&self) -> Option<Arc<PluralRules>> {
        self.plural_rules.clone()
    }

    /// Given a plural count, gets currency plural pattern of this locale,
    /// used for currency plural format. Equivalent to
    /// `getCurrencyPluralPattern(String)`.
    pub fn get_currency_plural_pattern(&self, plural_count: &str) -> Vec<u16> {
        let mut currency_plural_pattern = self.plural_count_to_currency_unit_pattern.get(plural_count).cloned();
        if currency_plural_pattern.is_none() {
            // fall back to "other"
            if plural_count != "other" {
                currency_plural_pattern = self.plural_count_to_currency_unit_pattern.get("other").cloned();
            }
            if currency_plural_pattern.is_none() {
                // no currencyUnitPatterns defined,
                // fallback to predefined default.
                // This should never happen when ICU resource files are
                // available, since currencyUnitPattern of "other" is always
                // defined in root.
                currency_plural_pattern = Some(DEFAULT_CURRENCY_PLURAL_PATTERN.to_vec());
            }
        }
        currency_plural_pattern.unwrap_or_default()
    }

    /// Get locale. Equivalent to `getLocale()`.
    pub fn get_locale(&self) -> &ULocale {
        &self.ulocale
    }

    /// Set plural rules. These are initially set in the constructor based on
    /// the locale, and usually do not need to be changed. Equivalent to
    /// `setPluralRules(String)`.
    pub fn set_plural_rules(&mut self, rule_description: &str) {
        self.plural_rules = PluralRules::create_rules(rule_description);
    }

    /// Set currency plural patterns. These are initially set in the
    /// constructor based on the locale, and usually do not need to be
    /// changed. Equivalent to `setCurrencyPluralPattern(String, String)`.
    pub fn set_currency_plural_pattern(&mut self, plural_count: &str, pattern: &[u16]) {
        self.plural_count_to_currency_unit_pattern.insert(plural_count.to_string(), pattern.to_vec());
    }

    /// Set locale. This also sets both the plural rules and the currency
    /// plural patterns to be the defaults for the locale. Equivalent to
    /// `setLocale(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn set_locale(&mut self, loc: &ULocale) -> Result<()> {
        self.ulocale = loc.clone();
        self.initialize(loc)
    }

    /// Override equals. Equivalent to `equals(Object)`.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` when the plural rules are `null`.
    pub fn java_equals(&self, other: &CurrencyPluralInfo) -> bool {
        let rules = self.plural_rules.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        rules.equals(other.plural_rules.as_deref())
            && self.plural_count_to_currency_unit_pattern == other.plural_count_to_currency_unit_pattern
    }

    /// Override hashCode. Equivalent to `hashCode()`.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` when the plural rules are `null`.
    pub fn hash_code(&self) -> i32 {
        let mut map_hash = 0i32;
        for (k, v) in self.plural_count_to_currency_unit_pattern.iter() {
            map_hash = map_hash.wrapping_add(java_string_hash_code(k) ^ string_hash_code(v));
        }
        let rules = self.plural_rules.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        map_hash ^ rules.hash_code() ^ self.ulocale.hash_code()
    }

    /// `toString()`: `Object.toString()` (the class does not override it),
    /// with the identity hash stand-in.
    pub fn to_java_string(&self) -> String {
        format!(
            "com.ibm.icu.text.CurrencyPluralInfo@{:x}",
            java_identity_hash_code_stand_in("com.ibm.icu.text.CurrencyPluralInfo")
        )
    }

    /// Given a number, returns the keyword of the first rule that applies
    /// to the number. Equivalent to the deprecated package-private
    /// `select(double)`.
    pub(crate) fn select_double(&self, number: f64) -> String {
        self.plural_rules
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .select_double(number)
    }

    /// Given a number, returns the keyword of the first rule that applies
    /// to the number. Equivalent to the internal `select(FixedDecimal)`.
    pub fn select(&self, number_info: &FixedDecimal) -> String {
        self.plural_rules
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .select(number_info)
    }

    /// Currency plural pattern iterator. Equivalent to the internal
    /// `pluralPatternIterator()`: the plural counts, in the map's order.
    pub fn plural_pattern_iterator(&self) -> Vec<String> {
        self.plural_count_to_currency_unit_pattern.keys().cloned().collect()
    }

    fn initialize(&mut self, uloc: &ULocale) -> Result<()> {
        self.ulocale = uloc.clone();
        self.plural_rules = Some(PluralRules::for_locale(uloc)?);
        self.setup_currency_plural_pattern(uloc)
    }

    fn setup_currency_plural_pattern(&mut self, uloc: &ULocale) -> Result<()> {
        self.plural_count_to_currency_unit_pattern = JavaHashMap::new();

        let mut number_style_pattern = number_format::get_pattern(uloc, NUMBERSTYLE)?;
        // Split the number style pattern into pos and neg if applicable
        let separator_index = index_of_unit(&number_style_pattern, u16::from(b';'), 0);
        let mut neg_number_pattern: Option<Vec<u16>> = None;
        if separator_index != -1 {
            neg_number_pattern = Some(number_style_pattern[separator_index as usize + 1..].to_vec());
            number_style_pattern.truncate(separator_index as usize);
        }
        let map = currency_data::provider()
            .get_instance(Some(uloc), true)?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
            .get_unit_patterns()?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        for (plural_count, pattern) in map.iter() {
            let pattern = crate::jdk::units(pattern);
            let pattern_with_number = replace(&pattern, &[0x7B, 0x30, 0x7D], &number_style_pattern); // "{0}"
            let mut pattern_with_currency_sign = replace(&pattern_with_number, &[0x7B, 0x31, 0x7D], &TRIPLE_CURRENCY_STR); // "{1}"
            if let Some(neg_number_pattern) = neg_number_pattern.as_ref() {
                let neg_pattern = &pattern;
                let neg_with_number = replace(neg_pattern, &[0x7B, 0x30, 0x7D], neg_number_pattern);
                let neg_with_curr_sign = replace(&neg_with_number, &[0x7B, 0x31, 0x7D], &TRIPLE_CURRENCY_STR);
                let mut pos_neg_patterns = pattern_with_currency_sign;
                pos_neg_patterns.push(u16::from(b';'));
                pos_neg_patterns.extend_from_slice(&neg_with_curr_sign);
                pattern_with_currency_sign = pos_neg_patterns;
            }
            self.plural_count_to_currency_unit_pattern.insert(plural_count.clone(), pattern_with_currency_sign);
        }
        Ok(())
    }
}

/// `String.replace(CharSequence target, CharSequence replacement)`.
fn replace(s: &[u16], target: &[u16], replacement: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if i + target.len() <= s.len() && s[i..i + target.len()] == *target {
            out.extend_from_slice(replacement);
            i += target.len();
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}
