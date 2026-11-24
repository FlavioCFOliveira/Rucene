// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.ICUCurrencyDisplayInfoProvider` (icu4j `currdata`
//! module, packaged in `icu4j-77.1.jar`).

use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::to_rust_string;
use crate::r#impl::currency_data::{
    CurrencyDisplayInfo, CurrencyDisplayInfoProvider, CurrencyFormatInfo, CurrencySpacingInfo,
    SpacingPattern, SpacingType,
};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{ICUResourceBundle, OpenType};
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::currency_display_names::CurrencyDisplayNames;
use crate::util::icu_exception::ICUException;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// The ICU data provider of currency display information.
///
/// Equivalent to `com.ibm.icu.impl.ICUCurrencyDisplayInfoProvider`. The
/// `volatile` single-entry cache is a lock-held `Option`.
#[derive(Debug, Default)]
pub struct ICUCurrencyDisplayInfoProvider {
    currency_display_info_cache: Mutex<Option<Arc<ICUCurrencyDisplayInfo>>>,
}

impl ICUCurrencyDisplayInfoProvider {
    /// `new ICUCurrencyDisplayInfoProvider()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CurrencyDisplayInfoProvider for ICUCurrencyDisplayInfoProvider {
    fn get_instance(
        &self,
        locale: Option<&ULocale>,
        with_fallback: bool,
    ) -> Result<Option<Arc<dyn CurrencyDisplayInfo>>> {
        // Make sure the locale is non-null (this can happen during deserialization):
        let locale = match locale {
            Some(l) => l.clone(),
            None => ULocale::root(),
        };
        let cached = self
            .currency_display_info_cache
            .lock()
            .expect("cache lock")
            .clone();
        let instance = match cached {
            Some(i) if i.locale == locale && i.fallback == with_fallback => i,
            _ => {
                let rb = if with_fallback {
                    ICUResourceBundle::get_bundle_instance_ulocale(
                        Some(ICUData::ICU_CURR_BASE_NAME),
                        Some(&locale),
                        OpenType::LocaleDefaultRoot,
                    )?
                } else {
                    match ICUResourceBundle::get_bundle_instance_ulocale(
                        Some(ICUData::ICU_CURR_BASE_NAME),
                        Some(&locale),
                        OpenType::LocaleOnly,
                    ) {
                        Ok(rb) => rb,
                        Err(e) if is_missing_resource(&e) => return Ok(None),
                        Err(e) => return Err(e),
                    }
                };
                let instance = Arc::new(ICUCurrencyDisplayInfo::new(locale, rb, with_fallback));
                *self.currency_display_info_cache.lock().expect("cache lock") =
                    Some(instance.clone());
                instance
            }
        };
        Ok(Some(instance))
    }

    fn has_data(&self) -> bool {
        true
    }
}

/// `ICUCurrencyDisplayInfo.FormattingData`: a single-currency cache entry.
#[derive(Debug, Clone)]
struct FormattingData {
    iso_code: String,
    display_name: Option<String>,
    symbol: Option<String>,
    format_info: Option<Arc<CurrencyFormatInfo>>,
}

impl FormattingData {
    fn new(iso_code: &str) -> Self {
        Self {
            iso_code: iso_code.to_string(),
            display_name: None,
            symbol: None,
            format_info: None,
        }
    }
}

/// `ICUCurrencyDisplayInfo.VariantSymbol`: a single-currency cache entry.
#[derive(Debug, Clone)]
struct VariantSymbol {
    iso_code: String,
    variant: String,
    symbol: Option<String>,
}

/// `ICUCurrencyDisplayInfo.ParsingData`: the full-table caches.
#[derive(Debug, Default)]
struct ParsingData {
    symbol_to_iso_code: JavaHashMap<String>,
    name_to_iso_code: JavaHashMap<String>,
}

/// Currency display information read from the `curr` resource tree.
///
/// Equivalent to `ICUCurrencyDisplayInfoProvider.ICUCurrencyDisplayInfo`
/// (package-private). Its `volatile` single-entry caches are lock-held
/// options; the `SoftReference` to the parsing data is never cleared (as
/// the crate's other soft caches, allowed by the `SoftReference` contract).
#[derive(Debug)]
#[allow(clippy::type_complexity)] // type mirrors the Java signature
pub struct ICUCurrencyDisplayInfo {
    locale: ULocale,
    fallback: bool,
    rb: Arc<ICUResourceBundle>,
    formatting_data_cache: Mutex<Option<Arc<FormattingData>>>,
    variant_symbol_cache: Mutex<Option<Arc<VariantSymbol>>>,
    plurals_data_cache: Mutex<Option<Arc<Vec<Option<String>>>>>,
    parsing_data_cache: Mutex<Option<Arc<(Arc<JavaHashMap<String>>, Arc<JavaHashMap<String>>)>>>,
    unit_patterns_cache: Mutex<Option<Arc<JavaHashMap<String>>>>,
    spacing_info_cache: Mutex<Option<Arc<CurrencySpacingInfo>>>,
}

impl ICUCurrencyDisplayInfo {
    /// `new ICUCurrencyDisplayInfo(ULocale locale, ICUResourceBundle rb,
    /// boolean fallback)`.
    pub fn new(locale: ULocale, rb: Arc<ICUResourceBundle>, fallback: bool) -> Self {
        Self {
            locale,
            fallback,
            rb,
            formatting_data_cache: Mutex::new(None),
            variant_symbol_cache: Mutex::new(None),
            plurals_data_cache: Mutex::new(None),
            parsing_data_cache: Mutex::new(None),
            unit_patterns_cache: Mutex::new(None),
            spacing_info_cache: Mutex::new(None),
        }
    }

    // ==========================================
    // END PUBLIC API -- START DATA FRONTEND
    // ==========================================

    fn fetch_formatting_data(&self, iso_code: &str) -> Result<Arc<FormattingData>> {
        let cached = self
            .formatting_data_cache
            .lock()
            .expect("cache lock")
            .clone();
        match cached {
            Some(r) if r.iso_code == iso_code => Ok(r),
            _ => {
                let mut sink = CurrencySink::new(!self.fallback, EntrypointTable::Currencies);
                sink.formatting_data = Some(FormattingData::new(iso_code));
                self.rb.get_all_items_with_fallback_no_fail(
                    &format!("Currencies/{iso_code}"),
                    &mut sink,
                )?;
                let result = Arc::new(sink.formatting_data.take().expect("set above"));
                *self.formatting_data_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }

    fn fetch_variant_symbol(&self, iso_code: &str, variant: &str) -> Result<Arc<VariantSymbol>> {
        let cached = self
            .variant_symbol_cache
            .lock()
            .expect("cache lock")
            .clone();
        match cached {
            Some(r) if r.iso_code == iso_code && r.variant == variant => Ok(r),
            _ => {
                let mut sink = CurrencySink::new(!self.fallback, EntrypointTable::CurrencyVariant);
                sink.variant_symbol = Some(VariantSymbol {
                    iso_code: iso_code.to_string(),
                    variant: variant.to_string(),
                    symbol: None,
                });
                self.rb.get_all_items_with_fallback_no_fail(
                    &format!("Currencies%{variant}/{iso_code}"),
                    &mut sink,
                )?;
                let result = Arc::new(sink.variant_symbol.take().expect("set above"));
                *self.variant_symbol_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }

    fn fetch_plurals_data(&self, iso_code: &str) -> Result<Arc<Vec<Option<String>>>> {
        let cached = self.plurals_data_cache.lock().expect("cache lock").clone();
        match cached {
            Some(r) if r[0].as_deref() == Some(iso_code) => Ok(r),
            _ => {
                let mut result = vec![None; 1 + StandardPlural::COUNT as usize];
                result[0] = Some(iso_code.to_string());
                let mut sink = CurrencySink::new(!self.fallback, EntrypointTable::CurrencyPlurals);
                sink.plurals_data = Some(result);
                self.rb.get_all_items_with_fallback_no_fail(
                    &format!("CurrencyPlurals/{iso_code}"),
                    &mut sink,
                )?;
                let result = Arc::new(sink.plurals_data.take().expect("set above"));
                *self.plurals_data_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }

    #[allow(clippy::type_complexity)] // type mirrors the Java signature
    fn fetch_parsing_data(
        &self,
    ) -> Result<Arc<(Arc<JavaHashMap<String>>, Arc<JavaHashMap<String>>)>> {
        let cached = self.parsing_data_cache.lock().expect("cache lock").clone();
        match cached {
            Some(r) => Ok(r),
            None => {
                let mut sink = CurrencySink::new(!self.fallback, EntrypointTable::Top);
                sink.parsing_data = Some(ParsingData::default());
                self.rb.get_all_items_with_fallback("", &mut sink)?;
                let pd = sink.parsing_data.take().expect("set above");
                let result = Arc::new((
                    Arc::new(pd.symbol_to_iso_code),
                    Arc::new(pd.name_to_iso_code),
                ));
                *self.parsing_data_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }

    fn fetch_unit_patterns(&self) -> Result<Arc<JavaHashMap<String>>> {
        let cached = self.unit_patterns_cache.lock().expect("cache lock").clone();
        match cached {
            Some(r) => Ok(r),
            None => {
                let mut sink = CurrencySink::new(false, EntrypointTable::CurrencyUnitPatterns);
                sink.unit_patterns = Some(JavaHashMap::new());
                self.rb
                    .get_all_items_with_fallback("CurrencyUnitPatterns", &mut sink)?;
                let result = Arc::new(sink.unit_patterns.take().expect("set above"));
                *self.unit_patterns_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }

    fn fetch_spacing_info(&self) -> Result<Arc<CurrencySpacingInfo>> {
        let cached = self.spacing_info_cache.lock().expect("cache lock").clone();
        match cached {
            Some(r) => Ok(r),
            None => {
                let mut sink = CurrencySink::new(!self.fallback, EntrypointTable::CurrencySpacing);
                sink.spacing_info = Some(CurrencySpacingInfo::new());
                self.rb
                    .get_all_items_with_fallback("currencySpacing", &mut sink)?;
                let result = Arc::new(sink.spacing_info.take().expect("set above"));
                *self.spacing_info_cache.lock().expect("cache lock") = Some(result.clone());
                Ok(result)
            }
        }
    }
}

// =====================
// START PUBLIC API
// =====================

impl CurrencyDisplayNames for ICUCurrencyDisplayInfo {
    fn get_u_locale(&self) -> ULocale {
        self.rb.get_u_locale()
    }

    fn get_name(&self, iso_code: &str) -> Result<Option<String>> {
        let formatting_data = self.fetch_formatting_data(iso_code)?;

        // Fall back to ISO Code
        if formatting_data.display_name.is_none() && self.fallback {
            return Ok(Some(iso_code.to_string()));
        }
        Ok(formatting_data.display_name.clone())
    }

    fn get_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        let formatting_data = self.fetch_formatting_data(iso_code)?;

        // Fall back to ISO Code
        if formatting_data.symbol.is_none() && self.fallback {
            return Ok(Some(iso_code.to_string()));
        }
        Ok(formatting_data.symbol.clone())
    }

    fn get_narrow_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        let variant_symbol = self.fetch_variant_symbol(iso_code, "narrow")?;

        // Fall back to regular symbol
        if variant_symbol.symbol.is_none() && self.fallback {
            return self.get_symbol(iso_code);
        }
        Ok(variant_symbol.symbol.clone())
    }

    fn get_formal_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        let variant_symbol = self.fetch_variant_symbol(iso_code, "formal")?;

        // Fall back to regular symbol
        if variant_symbol.symbol.is_none() && self.fallback {
            return self.get_symbol(iso_code);
        }
        Ok(variant_symbol.symbol.clone())
    }

    fn get_variant_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        let variant_symbol = self.fetch_variant_symbol(iso_code, "variant")?;

        // Fall back to regular symbol
        if variant_symbol.symbol.is_none() && self.fallback {
            return self.get_symbol(iso_code);
        }
        Ok(variant_symbol.symbol.clone())
    }

    fn get_plural_name(&self, iso_code: &str, plural_key: &str) -> Result<Option<String>> {
        let plural = StandardPlural::or_null_from_string(plural_key);
        let plurals_data = self.fetch_plurals_data(iso_code)?;
        let plural_keys = self.fetch_unit_patterns()?;

        // See http://unicode.org/reports/tr35/#Currencies, especially the fallback rule.
        let mut result: Option<String> = None;
        if let Some(p) = plural {
            result = plurals_data[1 + p.ordinal() as usize].clone();
        }
        if result.is_none() && (self.fallback || plural_keys.contains_key(plural_key)) {
            // First fall back to the "other" plural variant
            // Note: If plural is already "other", this fallback is benign
            result = plurals_data[1 + StandardPlural::Other.ordinal() as usize].clone();
        }
        if result.is_none() && (self.fallback || plural_keys.contains_key(plural_key)) {
            // If that fails, fall back to the display name
            let formatting_data = self.fetch_formatting_data(iso_code)?;
            result = formatting_data.display_name.clone();
        }
        if result.is_none() && self.fallback {
            // If all else fails, return the ISO code
            result = Some(iso_code.to_string());
        }
        Ok(result)
    }

    fn symbol_map(&self) -> Result<Arc<JavaHashMap<String>>> {
        let parsing_data = self.fetch_parsing_data()?;
        Ok(parsing_data.0.clone())
    }

    fn name_map(&self) -> Result<Arc<JavaHashMap<String>>> {
        let parsing_data = self.fetch_parsing_data()?;
        Ok(parsing_data.1.clone())
    }
}

impl CurrencyDisplayInfo for ICUCurrencyDisplayInfo {
    fn get_unit_patterns(&self) -> Result<Option<Arc<JavaHashMap<String>>>> {
        // Default result is the empty map. Callers who require a pattern will have to
        // supply a default.
        Ok(Some(self.fetch_unit_patterns()?))
    }

    fn get_format_info(&self, iso_code: &str) -> Result<Option<Arc<CurrencyFormatInfo>>> {
        let formatting_data = self.fetch_formatting_data(iso_code)?;
        Ok(formatting_data.format_info.clone())
    }

    fn get_spacing_info(&self) -> Result<Option<Arc<CurrencySpacingInfo>>> {
        let spacing_info = self.fetch_spacing_info()?;

        // Fall back to DEFAULT
        if (!spacing_info.has_before_currency || !spacing_info.has_after_currency) && self.fallback
        {
            return Ok(Some(CurrencySpacingInfo::default_info()));
        }
        Ok(Some(spacing_info))
    }
}

// =========================================
// END DATA FRONTEND -- START DATA SINK
// =========================================

/// `CurrencySink.EntrypointTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntrypointTable {
    // For Parsing:
    Top,

    // For Formatting:
    Currencies,
    CurrencyPlurals,
    CurrencyVariant,
    CurrencySpacing,
    CurrencyUnitPatterns,
}

/// `ICUCurrencyDisplayInfo.CurrencySink`: fills the field of this run
/// (the one that is `Some`).
struct CurrencySink {
    no_root: bool,
    entrypoint_table: EntrypointTable,

    // The fields to be populated on this run of the data sink will be non-null.
    formatting_data: Option<FormattingData>,
    plurals_data: Option<Vec<Option<String>>>,
    parsing_data: Option<ParsingData>,
    unit_patterns: Option<JavaHashMap<String>>,
    spacing_info: Option<CurrencySpacingInfo>,
    variant_symbol: Option<VariantSymbol>,
}

fn string_of(value: &dyn Value) -> Result<String> {
    Ok(to_rust_string(&value.get_string()?))
}

impl CurrencySink {
    fn new(no_root: bool, entrypoint_table: EntrypointTable) -> Self {
        Self {
            no_root,
            entrypoint_table,
            formatting_data: None,
            plurals_data: None,
            parsing_data: None,
            unit_patterns: None,
            spacing_info: None,
            variant_symbol: None,
        }
    }

    fn consume_top_table(&mut self, key: &mut Key, value: &mut dyn Value) -> Result<()> {
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, &mut *value) {
            if key.content_equals("Currencies") {
                self.consume_currencies_table(key, value)?;
            } else if key.content_equals("Currencies%variant") {
                self.consume_currencies_variant_table(key, value)?;
            } else if key.content_equals("CurrencyPlurals") {
                self.consume_currency_plurals_table(key, value)?;
            }
            i += 1;
        }
        Ok(())
    }

    /*
     *  Currencies{
     *      ...
     *      USD{
     *          "US$",        => symbol
     *          "US Dollar",  => display name
     *      }
     *      ...
     *      ESP{
     *          "₧",                  => symbol
     *          "pesseta espanyola",  => display name
     *          {
     *              "¤ #,##0.00",     => currency-specific pattern
     *              ",",              => currency-specific grouping separator
     *              ".",              => currency-specific decimal separator
     *          }
     *      }
     *      ...
     *  }
     */
    fn consume_currencies_table(&mut self, key: &mut Key, value: &mut dyn Value) -> Result<()> {
        // The full Currencies table is consumed for parsing only.
        let parsing_data = self.parsing_data.as_mut().expect("parsing run");
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, &mut *value) {
            let iso_code = key.as_string();
            if value.get_type() != UResourceBundle::ARRAY {
                return Err(ICUException::with_message(format!(
                    "Unexpected data type in Currencies table for {iso_code}"
                ))
                .error());
            }
            let array = value.get_array()?;

            parsing_data
                .symbol_to_iso_code
                .insert(iso_code.clone(), iso_code.clone()); // Add the ISO code itself as a symbol
            array.get_value(0, &mut *value);
            parsing_data
                .symbol_to_iso_code
                .insert(string_of(value)?, iso_code.clone());
            array.get_value(1, &mut *value);
            parsing_data
                .name_to_iso_code
                .insert(string_of(value)?, iso_code);
            i += 1;
        }
        Ok(())
    }

    fn consume_currencies_entry(&mut self, key: &mut Key, value: &mut dyn Value) -> Result<()> {
        let formatting_data = self.formatting_data.as_mut().expect("formatting run");

        let iso_code = key.as_string();
        if value.get_type() != UResourceBundle::ARRAY {
            return Err(ICUException::with_message(format!(
                "Unexpected data type in Currencies table for {iso_code}"
            ))
            .error());
        }
        let array = value.get_array()?;

        if formatting_data.symbol.is_none() {
            array.get_value(0, &mut *value);
            formatting_data.symbol = Some(string_of(value)?);
        }
        if formatting_data.display_name.is_none() {
            array.get_value(1, &mut *value);
            formatting_data.display_name = Some(string_of(value)?);
        }

        // If present, the third element is the currency format info.
        // TODO: Write unit test to ensure that this data is being used by number formatting.
        if array.get_size() > 2 && formatting_data.format_info.is_none() {
            array.get_value(2, &mut *value);
            let format_array = value.get_array()?;
            format_array.get_value(0, &mut *value);
            let format_pattern = string_of(value)?;
            format_array.get_value(1, &mut *value);
            let decimal_separator = string_of(value)?;
            format_array.get_value(2, &mut *value);
            let grouping_separator = string_of(value)?;
            formatting_data.format_info = Some(Arc::new(CurrencyFormatInfo::new(
                &iso_code,
                &format_pattern,
                &decimal_separator,
                &grouping_separator,
            )));
        }
        Ok(())
    }

    /*
     *  Currencies%narrow{
     *      AOA{"Kz"}
     *      ARS{"$"}
     *      ...
     *  }
     */
    fn consume_currencies_variant_entry(
        &mut self,
        _key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        let variant_symbol = self.variant_symbol.as_mut().expect("variant run");

        // No extra structure to traverse.
        if variant_symbol.symbol.is_none() {
            variant_symbol.symbol = Some(string_of(value)?);
        }
        Ok(())
    }

    /*
     *  Currencies%variant{
     *      TRY{"TL"}
     *  }
     */
    fn consume_currencies_variant_table(
        &mut self,
        key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        // Note: This data is used for parsing but not formatting.
        let parsing_data = self.parsing_data.as_mut().expect("parsing run");
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, &mut *value) {
            let iso_code = key.as_string();
            parsing_data
                .symbol_to_iso_code
                .insert(string_of(value)?, iso_code);
            i += 1;
        }
        Ok(())
    }

    /*
     *  CurrencyPlurals{
     *      BYB{
     *          one{"Belarusian new rouble (1994–1999)"}
     *          other{"Belarusian new roubles (1994–1999)"}
     *      }
     *      ...
     *  }
     */
    fn consume_currency_plurals_table(
        &mut self,
        key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        // The full CurrencyPlurals table is consumed for parsing only.
        let parsing_data = self.parsing_data.as_mut().expect("parsing run");
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, &mut *value) {
            let iso_code = key.as_string();
            let plurals_table = value.get_table()?;
            let mut j = 0;
            while plurals_table.get_key_and_value(j, key, &mut *value) {
                let plural = StandardPlural::or_null_from_string(&key.as_string());
                if plural.is_none() {
                    return Err(ICUException::with_message(format!(
                        "Could not make StandardPlural from keyword {}",
                        key.as_string()
                    ))
                    .error());
                }

                parsing_data
                    .name_to_iso_code
                    .insert(string_of(value)?, iso_code.clone());
                j += 1;
            }
            i += 1;
        }
        Ok(())
    }

    fn consume_currency_plurals_entry(
        &mut self,
        key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        let plurals_data = self.plurals_data.as_mut().expect("plurals run");
        let plurals_table = value.get_table()?;
        let mut j = 0;
        while plurals_table.get_key_and_value(j, key, &mut *value) {
            let plural = match StandardPlural::or_null_from_string(&key.as_string()) {
                Some(p) => p,
                None => {
                    return Err(ICUException::with_message(format!(
                        "Could not make StandardPlural from keyword {}",
                        key.as_string()
                    ))
                    .error())
                }
            };

            let slot = &mut plurals_data[1 + plural.ordinal() as usize];
            if slot.is_none() {
                *slot = Some(string_of(value)?);
            }
            j += 1;
        }
        Ok(())
    }

    /*
     *  currencySpacing{
     *      afterCurrency{
     *          currencyMatch{"[:^S:]"}
     *          insertBetween{" "}
     *          surroundingMatch{"[:digit:]"}
     *      }
     *      beforeCurrency{
     *          currencyMatch{"[:^S:]"}
     *          insertBetween{" "}
     *          surroundingMatch{"[:digit:]"}
     *      }
     *  }
     */
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn consume_currency_spacing_table(
        &mut self,
        key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        let spacing_info = self.spacing_info.as_mut().expect("spacing run");
        let spacing_types_table = value.get_table()?;
        let mut i = 0;
        while spacing_types_table.get_key_and_value(i, key, &mut *value) {
            let ty;
            if key.content_equals("beforeCurrency") {
                ty = SpacingType::Before;
                spacing_info.has_before_currency = true;
            } else if key.content_equals("afterCurrency") {
                ty = SpacingType::After;
                spacing_info.has_after_currency = true;
            } else {
                i += 1;
                continue;
            }

            let patterns_table = value.get_table()?;
            let mut j = 0;
            while patterns_table.get_key_and_value(j, key, &mut *value) {
                let pattern;
                if key.content_equals("currencyMatch") {
                    pattern = SpacingPattern::CurrencyMatch;
                } else if key.content_equals("surroundingMatch") {
                    pattern = SpacingPattern::SurroundingMatch;
                } else if key.content_equals("insertBetween") {
                    pattern = SpacingPattern::InsertBetween;
                } else {
                    j += 1;
                    continue;
                }

                spacing_info.set_symbol_if_null(ty, pattern, string_of(value)?);
                j += 1;
            }
            i += 1;
        }
        Ok(())
    }

    /*
     *  CurrencyUnitPatterns{
     *      other{"{0} {1}"}
     *      ...
     *  }
     */
    fn consume_currency_unit_patterns_table(
        &mut self,
        key: &mut Key,
        value: &mut dyn Value,
    ) -> Result<()> {
        let unit_patterns = self.unit_patterns.as_mut().expect("unit patterns run");
        let table = value.get_table()?;
        let mut i = 0;
        while table.get_key_and_value(i, key, &mut *value) {
            let plural_keyword = key.as_string();
            if unit_patterns.get(&plural_keyword).is_none() {
                unit_patterns.insert(plural_keyword, string_of(value)?);
            }
            i += 1;
        }
        Ok(())
    }
}

impl Sink for CurrencySink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, is_root: bool) -> Result<()> {
        if self.no_root && is_root {
            // Don't consume the root bundle
            return Ok(());
        }

        match self.entrypoint_table {
            EntrypointTable::Top => self.consume_top_table(key, value),
            EntrypointTable::Currencies => self.consume_currencies_entry(key, value),
            EntrypointTable::CurrencyPlurals => self.consume_currency_plurals_entry(key, value),
            EntrypointTable::CurrencyVariant => self.consume_currencies_variant_entry(key, value),
            EntrypointTable::CurrencySpacing => self.consume_currency_spacing_table(key, value),
            EntrypointTable::CurrencyUnitPatterns => {
                self.consume_currency_unit_patterns_table(key, value)
            }
        }
    }
}
