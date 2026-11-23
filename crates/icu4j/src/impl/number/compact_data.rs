// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.CompactData`.

use std::collections::HashSet;

use rucene::error::Result;

use crate::jdk::units;
use crate::number::compact_notation::CompactCustomData;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::multiplier_producer::MultiplierProducer;
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::compact_decimal_format::CompactStyle;
use crate::text::plural_rules::PluralRules;
use crate::util::icu_exception::ICUException;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// Equivalent to `CompactData.CompactType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompactType {
    /// `DECIMAL`.
    Decimal,
    /// `CURRENCY`.
    Currency,
}

/// `USE_FALLBACK`: the sentinel of a "0" pattern (use the default,
/// non-compact pattern).
const USE_FALLBACK: &str = "<USE FALLBACK>";

/// One slot of `patterns`: Java compares the stored string with the
/// constant `USE_FALLBACK` by identity (`patternString == USE_FALLBACK`), so
/// the sentinel is a distinct variant, not a text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PatternSlot {
    /// The `USE_FALLBACK` constant itself.
    UseFallback,
    /// Any other pattern string.
    Pattern(Vec<u16>),
}

/// `COMPACT_MAX_DIGITS`: the largest magnitude stored.
const COMPACT_MAX_DIGITS: i32 = 20;

/// Datatype for compact notation data. Includes logic for data loading.
///
/// Equivalent to `com.ibm.icu.impl.number.CompactData`.
#[derive(Debug, Clone)]
pub struct CompactData {
    /// `patterns` (`None` for Java's `null`).
    patterns: Vec<Option<PatternSlot>>,
    /// `multipliers`.
    multipliers: Vec<i8>,
    /// `largestMagnitude`.
    largest_magnitude: i8,
    /// `isEmpty`.
    is_empty: bool,
}

impl Default for CompactData {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactData {
    /// Equivalent to `CompactData()`.
    pub fn new() -> Self {
        CompactData {
            patterns: vec![None; ((COMPACT_MAX_DIGITS + 1) * StandardPlural::COUNT) as usize],
            multipliers: vec![0; (COMPACT_MAX_DIGITS + 1) as usize],
            largest_magnitude: 0,
            is_empty: true,
        }
    }

    /// Populates this object with data for a particular locale and
    /// compact type. Equivalent to `populate(ULocale, String, CompactStyle,
    /// CompactType)`.
    ///
    /// # Errors
    ///
    /// `ICUException("Could not load compact decimal data for locale
    /// <locale>")` when no data is found, and the errors of the data
    /// loading.
    pub fn populate(
        &mut self,
        locale: &ULocale,
        ns_name: &str,
        compact_style: CompactStyle,
        compact_type: CompactType,
    ) -> Result<()> {
        debug_assert!(self.is_empty);
        let rb = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale))?;
        let rb = rb.as_icu()?.clone();

        let ns_is_latn = ns_name == "latn";
        let compact_is_short = compact_style == CompactStyle::Short;

        // Fall back to latn numbering system and/or short compact style.
        let mut resource_key = String::new();
        get_resource_bundle_key(ns_name, compact_style, compact_type, &mut resource_key);
        rb.get_all_items_with_fallback_no_fail(&resource_key, &mut CompactDataSink { data: self })?;
        if self.is_empty && !ns_is_latn {
            get_resource_bundle_key("latn", compact_style, compact_type, &mut resource_key);
            rb.get_all_items_with_fallback_no_fail(&resource_key, &mut CompactDataSink { data: self })?;
        }
        if self.is_empty && !compact_is_short {
            get_resource_bundle_key(ns_name, CompactStyle::Short, compact_type, &mut resource_key);
            rb.get_all_items_with_fallback_no_fail(&resource_key, &mut CompactDataSink { data: self })?;
        }
        if self.is_empty && !ns_is_latn && !compact_is_short {
            get_resource_bundle_key("latn", CompactStyle::Short, compact_type, &mut resource_key);
            rb.get_all_items_with_fallback_no_fail(&resource_key, &mut CompactDataSink { data: self })?;
        }

        // The last fallback should be guaranteed to return data.
        if self.is_empty {
            return Err(ICUException::with_message(format!(
                "Could not load compact decimal data for locale {}",
                locale
            ))
            .error());
        }
        Ok(())
    }

    /// Populate this object with the custom data. Equivalent to
    /// `populate(Map<String, Map<String, String>>)` (`None` for Java's
    /// `null`, which throws `NullPointerException`).
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the `IllegalArgumentException` of
    /// `StandardPlural.fromString` for an unknown plural keyword.
    pub fn populate_custom(&mut self, powers_to_plurals_to_patterns: Option<&CompactCustomData>) -> Result<()> {
        debug_assert!(self.is_empty);
        let map = powers_to_plurals_to_patterns.ok_or_else(|| crate::jdk::null_pointer("compactCustomData"))?;
        for (key, plurals) in map.iter() {
            let magnitude = (key.encode_utf16().count() as i32 - 1) as i8;
            for (plural_string, pattern) in plurals.iter() {
                let plural = StandardPlural::from_string(plural_string)?;
                let pattern_string = units(pattern);
                let num_zeros = count_zeros(&pattern_string);
                self.patterns[get_index(magnitude as i32, plural)] = Some(PatternSlot::Pattern(pattern_string));
                if num_zeros > 0 {
                    // Save the multiplier only if there is at least one
                    // zero.
                    self.multipliers[magnitude as usize] = (num_zeros - magnitude as i32 - 1) as i8;
                    if magnitude > self.largest_magnitude {
                        self.largest_magnitude = magnitude;
                    }
                    self.is_empty = false;
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `getPattern(int, PluralRules, DecimalQuantity)`: the
    /// pattern for the magnitude, `None` for Java's `null`.
    pub fn get_pattern(&self, magnitude: i32, rules: &PluralRules, dq: &dyn DecimalQuantity) -> Option<Vec<u16>> {
        if magnitude < 0 {
            return None;
        }
        let magnitude = magnitude.min(self.largest_magnitude as i32);
        let mut pattern_string: Option<&PatternSlot> = None;
        if dq.is_has_integer_value() {
            let i = dq.to_long(true);
            if i == 0 {
                pattern_string = self.patterns[get_index(magnitude, StandardPlural::Eq0)].as_ref();
            } else if i == 1 {
                pattern_string = self.patterns[get_index(magnitude, StandardPlural::Eq1)].as_ref();
            }
            if let Some(p) = pattern_string {
                return slot_text(p);
            }
        }
        let plural = dq.get_standard_plural(Some(rules));
        pattern_string = self.patterns[get_index(magnitude, plural)].as_ref();
        if pattern_string.is_none() && plural != StandardPlural::Other {
            // Fall back to "other" plural variant
            pattern_string = self.patterns[get_index(magnitude, StandardPlural::Other)].as_ref();
        }
        // USE_FALLBACK becomes null.
        pattern_string.and_then(slot_text)
    }

    /// Equivalent to `getUniquePatterns(Set<String>)`: adds every pattern
    /// except `USE_FALLBACK` and `null`.
    pub fn get_unique_patterns(&self, output: &mut HashSet<Vec<u16>>) {
        debug_assert!(output.is_empty());
        // NOTE: In C++, this is done more manually with a UVector.
        // In Java, we can take advantage of JDK HashSet.
        for p in self.patterns.iter().flatten() {
            if let PatternSlot::Pattern(text) = p {
                output.insert(text.clone());
            }
        }
        // output.remove(USE_FALLBACK) removes an equal custom string too.
        output.remove(&units(USE_FALLBACK));
    }
}

/// The text of a slot for `getPattern`: `USE_FALLBACK` (identity) is
/// `null`.
fn slot_text(p: &PatternSlot) -> Option<Vec<u16>> {
    match p {
        PatternSlot::UseFallback => None,
        PatternSlot::Pattern(t) => Some(t.clone()),
    }
}

impl MultiplierProducer for CompactData {
    fn get_multiplier(&self, magnitude: i32) -> i32 {
        if magnitude < 0 {
            return 0;
        }
        let magnitude = magnitude.min(self.largest_magnitude as i32);
        self.multipliers[magnitude as usize] as i32
    }
}

/// `getResourceBundleKey(String, CompactStyle, CompactType, StringBuilder)`.
fn get_resource_bundle_key(ns_name: &str, compact_style: CompactStyle, compact_type: CompactType, sb: &mut String) {
    sb.clear();
    sb.push_str("NumberElements/");
    sb.push_str(ns_name);
    sb.push_str(if compact_style == CompactStyle::Short { "/patternsShort" } else { "/patternsLong" });
    sb.push_str(if compact_type == CompactType::Decimal { "/decimalFormat" } else { "/currencyFormat" });
}

/// `getIndex(int, StandardPlural)`.
fn get_index(magnitude: i32, plural: StandardPlural) -> usize {
    (magnitude * StandardPlural::COUNT + plural.ordinal()) as usize
}

/// `countZeros(String)`.
fn count_zeros(pattern_string: &[u16]) -> i32 {
    // NOTE: This strategy for computing the number of zeros is a hack for
    // efficiency. It could break if there are any 0s that aren't part of
    // the main pattern.
    let mut num_zeros = 0;
    for &c in pattern_string {
        if c == '0' as u16 {
            num_zeros += 1;
        } else if num_zeros > 0 {
            break; // zeros should always be contiguous
        }
    }
    num_zeros
}

/// Equivalent to the private class `CompactData.CompactDataSink`.
struct CompactDataSink<'a> {
    /// `data`.
    data: &'a mut CompactData,
}

impl Sink for CompactDataSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _is_root: bool) -> Result<()> {
        // traverse into the table of powers of ten
        let powers_of_ten_table = value.get_table()?;
        let mut i3 = 0;
        while powers_of_ten_table.get_key_and_value(i3, key, value) {
            i3 += 1;
            // Assumes that the keys are always of the form "10000" where
            // the magnitude is the length of the key minus one.  We expect
            // magnitudes to be less than MAX_DIGITS.
            let magnitude = (key.length() - 1) as i8;
            if magnitude as i32 >= COMPACT_MAX_DIGITS {
                continue;
            }
            let mut multiplier = self.data.multipliers[magnitude as usize];

            // Iterate over the plural variants ("one", "other", etc)
            let plural_variants_table = value.get_table()?;
            let mut i4 = 0;
            while plural_variants_table.get_key_and_value(i4, key, value) {
                i4 += 1;
                // Skip this magnitude/plural if we already have it from a
                // child locale. Note: This also skips USE_FALLBACK entries.
                let plural = StandardPlural::from_string(&key.as_string())?;
                let index = get_index(magnitude as i32, plural);
                if self.data.patterns[index].is_some() {
                    continue;
                }

                // The value "0" means that we need to use the default
                // pattern and not fall back to parent locales. Example
                // locale where this is relevant: 'it'.
                let pattern_string = value.get_string()?;
                let slot = if pattern_string == units("0") {
                    PatternSlot::UseFallback
                } else {
                    PatternSlot::Pattern(pattern_string.clone())
                };

                // Save the pattern string. We will parse it lazily.
                let num_zeros_source = match &slot {
                    PatternSlot::UseFallback => units(USE_FALLBACK),
                    PatternSlot::Pattern(t) => t.clone(),
                };
                self.data.patterns[index] = Some(slot);

                // If necessary, compute the multiplier: the difference
                // between the magnitude and the number of zeros in the
                // pattern.
                if multiplier == 0 {
                    let num_zeros = count_zeros(&num_zeros_source);
                    if num_zeros > 0 {
                        // Save the multiplier only if there is at least
                        // one zero.
                        multiplier = (num_zeros - magnitude as i32 - 1) as i8;
                    }
                }
            }

            // Save the multiplier.
            if self.data.multipliers[magnitude as usize] == 0 {
                self.data.multipliers[magnitude as usize] = multiplier;
                if magnitude > self.data.largest_magnitude {
                    self.data.largest_magnitude = magnitude;
                }
                self.data.is_empty = false;
            } else {
                debug_assert!(self.data.multipliers[magnitude as usize] == multiplier);
            }
        }
        Ok(())
    }
}
