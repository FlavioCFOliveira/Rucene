// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.CurrencyData`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use crate::r#impl::icu_currency_display_info_provider::ICUCurrencyDisplayInfoProvider;
use crate::text::currency_display_names::CurrencyDisplayNames;
use crate::util::u_locale::ULocale;

/// `CurrencyData.CurrencyDisplayInfoProvider`.
pub trait CurrencyDisplayInfoProvider: Send + Sync {
    /// `getInstance(ULocale locale, boolean withFallback)` (`None` locale =
    /// `null`). `None` result = Java's `null` (no data for the locale when
    /// fallback is off).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data cannot be loaded.
    fn get_instance(
        &self,
        locale: Option<&ULocale>,
        with_fallback: bool,
    ) -> Result<Option<Arc<dyn CurrencyDisplayInfo>>>;

    /// `hasData()`.
    fn has_data(&self) -> bool;
}

/// `CurrencyData.CurrencyDisplayInfo` (abstract class extending
/// `CurrencyDisplayNames`).
pub trait CurrencyDisplayInfo: CurrencyDisplayNames {
    /// `getUnitPatterns()`: the currency unit patterns by plural keyword
    /// (`None` = `null`).
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_unit_patterns(&self) -> Result<Option<Arc<JavaHashMap<String>>>>;

    /// `getFormatInfo(String isoCode)`.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_format_info(&self, iso_code: &str) -> Result<Option<Arc<CurrencyFormatInfo>>>;

    /// `getSpacingInfo()`.
    ///
    /// # Errors
    ///
    /// `ICUException` for malformed data.
    fn get_spacing_info(&self) -> Result<Option<Arc<CurrencySpacingInfo>>>;
}

/// Currency-specific format data.
///
/// Equivalent to `CurrencyData.CurrencyFormatInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyFormatInfo {
    /// `isoCode`.
    pub iso_code: String,
    /// `currencyPattern`.
    pub currency_pattern: String,
    /// `monetaryDecimalSeparator`.
    pub monetary_decimal_separator: String,
    /// `monetaryGroupingSeparator`.
    pub monetary_grouping_separator: String,
}

impl CurrencyFormatInfo {
    /// `new CurrencyFormatInfo(String isoCode, String currencyPattern,
    /// String monetarySeparator, String monetaryGroupingSeparator)`.
    pub fn new(
        iso_code: &str,
        currency_pattern: &str,
        monetary_separator: &str,
        monetary_grouping_separator: &str,
    ) -> Self {
        Self {
            iso_code: iso_code.to_string(),
            currency_pattern: currency_pattern.to_string(),
            monetary_decimal_separator: monetary_separator.to_string(),
            monetary_grouping_separator: monetary_grouping_separator.to_string(),
        }
    }
}

/// `CurrencySpacingInfo.SpacingType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingType {
    /// `BEFORE`.
    Before,
    /// `AFTER`.
    After,
    /// `COUNT`.
    Count,
}

/// `CurrencySpacingInfo.SpacingPattern`. Each constant's value is the
/// matching `DecimalFormatSymbols.CURRENCY_SPC_*` constant, which equals
/// its ordinal (`assert value == ordinal()`): `CURRENCY_SPC_CURRENCY_MATCH =
/// 0`, `CURRENCY_SPC_SURROUNDING_MATCH = 1`, `CURRENCY_SPC_INSERT = 2`
/// (`DecimalFormatSymbols.java:1134-1147`, compile-time constants javac
/// inlines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingPattern {
    /// `CURRENCY_MATCH`.
    CurrencyMatch = 0,
    /// `SURROUNDING_MATCH`.
    SurroundingMatch = 1,
    /// `INSERT_BETWEEN`.
    InsertBetween = 2,
    /// `COUNT`.
    Count = 3,
}

const SPACING_TYPE_COUNT: usize = SpacingType::Count as usize;
const SPACING_PATTERN_COUNT: usize = SpacingPattern::Count as usize;

/// Currency spacing data.
///
/// Equivalent to `CurrencyData.CurrencySpacingInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CurrencySpacingInfo {
    symbols: [[Option<String>; SPACING_PATTERN_COUNT]; SPACING_TYPE_COUNT],
    /// `hasBeforeCurrency`.
    pub has_before_currency: bool,
    /// `hasAfterCurrency`.
    pub has_after_currency: bool,
}

const DEFAULT_CUR_MATCH: &str = "[:letter:]";
const DEFAULT_CTX_MATCH: &str = "[:digit:]";
const DEFAULT_INSERT: &str = " ";

/// `CurrencySpacingInfo.DEFAULT`.
static DEFAULT_SPACING_INFO: LazyLock<Arc<CurrencySpacingInfo>> = LazyLock::new(|| {
    Arc::new(CurrencySpacingInfo::from_strings(&[
        DEFAULT_CUR_MATCH,
        DEFAULT_CTX_MATCH,
        DEFAULT_INSERT,
        DEFAULT_CUR_MATCH,
        DEFAULT_CTX_MATCH,
        DEFAULT_INSERT,
    ]))
});

impl CurrencySpacingInfo {
    /// `new CurrencySpacingInfo()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `new CurrencySpacingInfo(String... strings)`: six strings, before
    /// then after, each in `SpacingPattern` order.
    #[allow(clippy::needless_range_loop)] // indexed loop mirrors the Java for-loop
    pub fn from_strings(strings: &[&str]) -> Self {
        debug_assert!(strings.len() == 6);
        let mut s = Self::default();
        let mut k = 0;
        for i in 0..SPACING_TYPE_COUNT {
            for j in 0..SPACING_PATTERN_COUNT {
                s.symbols[i][j] = Some(strings[k].to_string());
                k += 1;
            }
        }
        s
    }

    /// `DEFAULT`.
    pub fn default_info() -> Arc<CurrencySpacingInfo> {
        DEFAULT_SPACING_INFO.clone()
    }

    /// `setSymbolIfNull(SpacingType type, SpacingPattern pattern, String
    /// value)`.
    pub fn set_symbol_if_null(&mut self, ty: SpacingType, pattern: SpacingPattern, value: String) {
        let i = ty as usize;
        let j = pattern as usize;
        if self.symbols[i][j].is_none() {
            self.symbols[i][j] = Some(value);
        }
    }

    /// `getBeforeSymbols()`.
    pub fn get_before_symbols(&self) -> &[Option<String>] {
        &self.symbols[SpacingType::Before as usize]
    }

    /// `getAfterSymbols()`.
    pub fn get_after_symbols(&self) -> &[Option<String>] {
        &self.symbols[SpacingType::After as usize]
    }
}

/// `provider`: `Class.forName("com.ibm.icu.impl.ICUCurrencyDisplayInfoProvider")`
/// finds the class (it is part of icu4j), so the provider is an
/// `ICUCurrencyDisplayInfoProvider`, constructed directly (no reflection in
/// Rust; precedent of the crate's other `Class.forName` sites).
static PROVIDER: LazyLock<Arc<dyn CurrencyDisplayInfoProvider>> =
    LazyLock::new(|| Arc::new(ICUCurrencyDisplayInfoProvider::new()));

/// `CurrencyData.provider`.
pub fn provider() -> Arc<dyn CurrencyDisplayInfoProvider> {
    PROVIDER.clone()
}

/// The provider of the static initializer's `catch (Throwable t)` branch
/// (Java's anonymous `CurrencyDisplayInfoProvider`, used when the ICU
/// provider class cannot be instantiated): `DefaultInfo` instances, no data.
#[derive(Debug, Default)]
pub struct FallbackCurrencyDisplayInfoProvider;

impl CurrencyDisplayInfoProvider for FallbackCurrencyDisplayInfoProvider {
    fn get_instance(
        &self,
        _locale: Option<&ULocale>,
        with_fallback: bool,
    ) -> Result<Option<Arc<dyn CurrencyDisplayInfo>>> {
        Ok(Some(DefaultInfo::get_with_fallback(with_fallback)))
    }

    fn has_data(&self) -> bool {
        false
    }
}

/// Display info without data: the ISO code itself with fallback, `None`
/// without.
///
/// Equivalent to `CurrencyData.DefaultInfo`.
#[derive(Debug)]
pub struct DefaultInfo {
    fallback: bool,
}

static FALLBACK_INSTANCE: LazyLock<Arc<DefaultInfo>> =
    LazyLock::new(|| Arc::new(DefaultInfo { fallback: true }));
static NO_FALLBACK_INSTANCE: LazyLock<Arc<DefaultInfo>> =
    LazyLock::new(|| Arc::new(DefaultInfo { fallback: false }));
static EMPTY_MAP: LazyLock<Arc<JavaHashMap<String>>> =
    LazyLock::new(|| Arc::new(JavaHashMap::new()));

impl DefaultInfo {
    /// `getWithFallback(boolean fallback)`.
    pub fn get_with_fallback(fallback: bool) -> Arc<dyn CurrencyDisplayInfo> {
        if fallback {
            FALLBACK_INSTANCE.clone()
        } else {
            NO_FALLBACK_INSTANCE.clone()
        }
    }

    fn code_or_none(&self, iso_code: &str) -> Option<String> {
        if self.fallback {
            Some(iso_code.to_string())
        } else {
            None
        }
    }
}

impl CurrencyDisplayNames for DefaultInfo {
    fn get_u_locale(&self) -> ULocale {
        ULocale::root()
    }

    fn get_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn get_narrow_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn get_formal_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn get_variant_symbol(&self, iso_code: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn get_name(&self, iso_code: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn get_plural_name(&self, iso_code: &str, _plural_type: &str) -> Result<Option<String>> {
        Ok(self.code_or_none(iso_code))
    }

    fn symbol_map(&self) -> Result<Arc<JavaHashMap<String>>> {
        Ok(EMPTY_MAP.clone())
    }

    fn name_map(&self) -> Result<Arc<JavaHashMap<String>>> {
        Ok(EMPTY_MAP.clone())
    }
}

impl CurrencyDisplayInfo for DefaultInfo {
    fn get_unit_patterns(&self) -> Result<Option<Arc<JavaHashMap<String>>>> {
        if self.fallback {
            return Ok(Some(EMPTY_MAP.clone()));
        }
        Ok(None)
    }

    fn get_format_info(&self, _iso_code: &str) -> Result<Option<Arc<CurrencyFormatInfo>>> {
        Ok(None)
    }

    fn get_spacing_info(&self) -> Result<Option<Arc<CurrencySpacingInfo>>> {
        Ok(if self.fallback {
            Some(CurrencySpacingInfo::default_info())
        } else {
            None
        })
    }
}
