// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.LocalePriorityList`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::double_to_string;

use crate::jdk::{parse_double, to_rust_string, units};
use crate::util::u_locale::ULocale;

/// `D1`.
const D1: f64 = 1.0;

/// `languageSplitter`.
static LANGUAGE_SPLITTER: LazyLock<JavaPattern> =
    LazyLock::new(|| JavaPattern::compile("\\s*,\\s*", 0).expect("valid pattern"));
/// `weightSplitter`.
static WEIGHT_SPLITTER: LazyLock<JavaPattern> =
    LazyLock::new(|| JavaPattern::compile("\\s*(\\S*)\\s*;\\s*q\\s*=\\s*(\\S*)", 0).expect("valid pattern"));

/// An immutable class that takes a list of locales and weights, sorts the
/// locales by weight (stable for equal weights) and gives a list of the
/// locales, as in an HTTP `Accept-Language` header.
///
/// Equivalent to `com.ibm.icu.util.LocalePriorityList`. The
/// `Collections.unmodifiableMap(LinkedHashMap)` of locales to weights is an
/// insertion-ordered vector of pairs with unique keys (the same iteration
/// order, lookup and equality).
#[derive(Debug, Clone)]
pub struct LocalePriorityList {
    languages_and_weights: Vec<(ULocale, f64)>,
}

/// `myDescendingDouble`: `Double.compareTo`, reversed.
fn my_descending_double(o1: f64, o2: f64) -> Ordering {
    java_double_compare(o1, o2).reverse()
}

/// `Double.compare(double, double)`: the total order of
/// `doubleToLongBits` (`-0.0 < 0.0`, `NaN` greatest and equal to itself).
fn java_double_compare(a: f64, b: f64) -> Ordering {
    let canon = |d: f64| if d.is_nan() { f64::NAN } else { d };
    canon(a).total_cmp(&canon(b))
}

/// `Double.hashCode(double)`.
fn java_double_hash(d: f64) -> i32 {
    let bits = if d.is_nan() { 0x7ff8000000000000u64 } else { d.to_bits() };
    (bits ^ (bits >> 32)) as i32
}

impl LocalePriorityList {
    /// `add(ULocale... locales)`: a builder holding the locales with
    /// weight 1.0.
    pub fn add_locales(locales: &[ULocale]) -> Builder {
        let mut b = Builder::new();
        b.add_locales(locales);
        b
    }

    /// `add(ULocale locale, double weight)`.
    pub fn add_weighted(locale: &ULocale, weight: f64) -> Builder {
        let mut b = Builder::new();
        b.add_weighted(locale, weight);
        b
    }

    /// `add(LocalePriorityList list)`: a builder over an existing list.
    pub fn add_list(list: &Arc<LocalePriorityList>) -> Builder {
        Builder::from_list(list)
    }

    /// `add(String acceptLanguageString)`: a builder holding the locales
    /// of an `Accept-Language` string.
    ///
    /// # Errors
    ///
    /// As [`Builder::add_accept_language`].
    pub fn add_accept_language(accept_language_string: &str) -> Result<Builder> {
        let mut b = Builder::new();
        b.add_accept_language(accept_language_string)?;
        Ok(b)
    }

    /// `getWeight(ULocale locale)`: the weight, `None` for a locale not in
    /// the list.
    pub fn get_weight(&self, locale: &ULocale) -> Option<f64> {
        self.languages_and_weights.iter().find(|(l, _)| l == locale).map(|(_, w)| *w)
    }

    /// `getULocales()`: the locales, in order (an unmodifiable view of the
    /// key set).
    pub fn get_u_locales(&self) -> Vec<ULocale> {
        self.languages_and_weights.iter().map(|(l, _)| l.clone()).collect()
    }

    /// `iterator()`: the locales, in order.
    pub fn iter(&self) -> impl Iterator<Item = &ULocale> {
        self.languages_and_weights.iter().map(|(l, _)| l)
    }

    /// `equals(Object o)`: equal maps (the same keys with equal `Double`
    /// values, in any order).
    pub fn java_equals(&self, that: &LocalePriorityList) -> bool {
        self.languages_and_weights.len() == that.languages_and_weights.len()
            && self.languages_and_weights.iter().all(|(l, w)| {
                that.get_weight(l).is_some_and(|tw| java_double_compare(*w, tw) == Ordering::Equal)
            })
    }

    /// `hashCode()`: `AbstractMap.hashCode`, the sum of
    /// `key.hashCode() ^ value.hashCode()`.
    pub fn hash_code(&self) -> i32 {
        self.languages_and_weights
            .iter()
            .fold(0i32, |h, (l, w)| h.wrapping_add(l.hash_code() ^ java_double_hash(*w)))
    }
}

impl PartialEq for LocalePriorityList {
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

/// `toString()`: `locale;q=weight` items separated by `", "`, the weight
/// omitted when it is 1.0.
impl fmt::Display for LocalePriorityList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = String::new();
        for (language, weight) in &self.languages_and_weights {
            if !result.is_empty() {
                result.push_str(", ");
            }
            result.push_str(&language.to_string());
            if *weight != 1.0 {
                result.push_str(";q=");
                result.push_str(&double_to_string(*weight));
            }
        }
        f.write_str(&result)
    }
}

/// Builder for a [`LocalePriorityList`].
///
/// Equivalent to `LocalePriorityList.Builder`.
#[derive(Debug)]
pub struct Builder {
    /// `languageToWeight`: the insertion-ordered `LinkedHashMap`, `None`
    /// after `build()`.
    language_to_weight: Option<Vec<(ULocale, f64)>>,
    built: Option<Arc<LocalePriorityList>>,
    /// other than 1.0
    has_weights: bool,
}

impl Builder {
    /// `Builder()` (private).
    fn new() -> Self {
        Self {
            language_to_weight: Some(Vec::new()),
            built: None,
            has_weights: false,
        }
    }

    /// `Builder(LocalePriorityList list)` (private).
    fn from_list(list: &Arc<LocalePriorityList>) -> Self {
        let mut has_weights = false;
        for &(_, value) in &list.languages_and_weights {
            let weight = value;
            debug_assert!(0.0 < weight && weight <= 1.0);
            if weight != 1.0 {
                has_weights = true;
                break;
            }
        }
        Self {
            language_to_weight: None,
            built: Some(list.clone()),
            has_weights,
        }
    }

    /// `build()`: the list, ordered by decreasing weight, with the weights
    /// replaced by 1.0.
    pub fn build(&mut self) -> Arc<LocalePriorityList> {
        self.build_preserving(false)
    }

    /// `build(boolean preserveWeights)`.
    pub fn build_preserving(&mut self, preserve_weights: bool) -> Arc<LocalePriorityList> {
        if let Some(built) = &self.built {
            // Calling build() again without changing anything in between.
            // Just return the same immutable list.
            return built.clone();
        }
        let language_to_weight = self.language_to_weight.take().expect("set while built is null");
        let temp: Vec<(ULocale, f64)>;
        if self.has_weights {
            // Walk through the input list, collecting the items with the same weights.
            // (TreeMap<Double, List<ULocale>>(myDescendingDouble): kept sorted by key.)
            let mut weight_to_languages: Vec<(f64, Vec<ULocale>)> = Vec::new();
            for (lang, weight) in &language_to_weight {
                match weight_to_languages.binary_search_by(|(k, _)| my_descending_double(*k, *weight)) {
                    Ok(i) => weight_to_languages[i].1.push(lang.clone()),
                    Err(i) => weight_to_languages.insert(i, (*weight, vec![lang.clone()])),
                }
            }
            // We now have a bunch of items sorted by weight, then chronologically.
            // We can now create a list in the right order.
            if weight_to_languages.len() <= 1 {
                // There is at most one weight.
                if weight_to_languages.is_empty() || weight_to_languages[0].0 == 1.0 {
                    self.has_weights = false;
                }
                temp = language_to_weight;
            } else {
                let mut t: Vec<(ULocale, f64)> = Vec::new();
                for (key, langs) in weight_to_languages {
                    let weight = if preserve_weights { key } else { D1 };
                    for lang in langs {
                        put(&mut t, lang, weight);
                    }
                }
                temp = t;
            }
        } else {
            // Nothing to sort.
            temp = language_to_weight;
        }
        let built = Arc::new(LocalePriorityList {
            languages_and_weights: temp,
        });
        self.built = Some(built.clone());
        built
    }

    /// `add(LocalePriorityList list)`: adds the locales of a list with
    /// their weights.
    pub fn add_list(&mut self, list: &LocalePriorityList) -> &mut Self {
        for (locale, weight) in &list.languages_and_weights {
            self.add_weighted(locale, *weight);
        }
        self
    }

    /// `add(ULocale locale)`: adds a locale with weight 1.0.
    pub fn add(&mut self, locale: &ULocale) -> &mut Self {
        self.add_weighted(locale, 1.0)
    }

    /// `add(ULocale... locales)`.
    pub fn add_locales(&mut self, locales: &[ULocale]) -> &mut Self {
        for language_code in locales {
            self.add_weighted(language_code, 1.0);
        }
        self
    }

    /// `add(ULocale locale, double weight)`: adds a locale with a weight;
    /// a weight `<= 0` removes it, a weight `>= 1` is 1.0.
    pub fn add_weighted(&mut self, locale: &ULocale, weight: f64) -> &mut Self {
        if self.language_to_weight.is_none() {
            // Builder reuse after build().
            let built = self.built.take().expect("set after build()");
            self.language_to_weight = Some(built.languages_and_weights.clone());
        }
        let language_to_weight = self.language_to_weight.as_mut().expect("set above");
        if let Some(i) = language_to_weight.iter().position(|(l, _)| l == locale) {
            language_to_weight.remove(i);
        }
        let value;
        if weight <= 0.0 {
            return self; // skip zeros
        } else if weight >= 1.0 {
            value = D1;
        } else {
            value = weight;
            self.has_weights = true;
        }
        put(language_to_weight, locale.clone(), value);
        self
    }

    /// `add(String acceptLanguageList)`: adds the items of an
    /// `Accept-Language` string (`locale` or `locale;q=weight`, separated
    /// by commas).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Illegal weight, must be 0..1: " +
    /// weight)`; `NumberFormatException` for an unparsable weight; the
    /// errors of `new ULocale`.
    pub fn add_accept_language(&mut self, accept_language_list: &str) -> Result<&mut Self> {
        let trimmed = java_trim(&units(accept_language_list));
        let items = LANGUAGE_SPLITTER.split(&trimmed, 0);
        for item in &items {
            let mut item_matcher = WEIGHT_SPLITTER.matcher(item);
            if item_matcher.matches() {
                let group1 = item_matcher.group(1)?.unwrap_or_default();
                let group2 = item_matcher.group(2)?.unwrap_or_default();
                let language = ULocale::new(&to_rust_string(&group1))?;
                let weight = parse_double(&to_rust_string(&group2))?;
                if !(0.0..=1.0).contains(&weight) {
                    // do ! for NaN
                    return Err(LuceneError::illegal_argument(format!(
                        "Illegal weight, must be 0..1: {}",
                        double_to_string(weight)
                    )));
                }
                self.add_weighted(&language, weight);
            } else if !item.is_empty() {
                self.add(&ULocale::new(&to_rust_string(item))?);
            }
        }
        Ok(self)
    }
}

/// `LinkedHashMap.put`: replaces the value of an existing key in place,
/// appends a new key.
fn put(map: &mut Vec<(ULocale, f64)>, key: ULocale, value: f64) {
    match map.iter_mut().find(|(l, _)| *l == key) {
        Some(e) => e.1 = value,
        None => map.push((key, value)),
    }
}

/// `String.trim()`: without the leading and trailing units `<= ' '`.
fn java_trim(s: &[u16]) -> Vec<u16> {
    let start = s.iter().position(|&c| c > 0x20).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c > 0x20).map_or(start, |e| e + 1);
    s[start..end].to_vec()
}
