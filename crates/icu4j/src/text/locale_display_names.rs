// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and    *
 * others. All Rights Reserved.                                                *
 *******************************************************************************
 */

//! `com.ibm.icu.text.LocaleDisplayNames`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::lang::u_script::UScript;
use crate::r#impl::icu_config::ICUConfig;
use crate::r#impl::locale_display_names_impl::LocaleDisplayNamesImpl;
use crate::text::display_context::{DisplayContext, Type};
use crate::util::u_locale::ULocale;

/// Enum used in [`get_instance_dialect`].
///
/// Equivalent to `LocaleDisplayNames.DialectHandling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialectHandling {
    /// `STANDARD_NAMES`: use standard names when generating a locale name,
    /// e.g. en_GB displays as 'English (United Kingdom)'.
    StandardNames,
    /// `DIALECT_NAMES`: use dialect names when generating a locale name,
    /// e.g. en_GB displays as 'British English'.
    DialectNames,
}

/// Returns display names of ULocales and components of ULocales.
///
/// Equivalent to `com.ibm.icu.text.LocaleDisplayNames` (abstract class).
/// Locale codes are Rust strings; display names are Rust strings, as the
/// display names of the crate's ICU services (`ICUService.getDisplayName`)
/// are: ICU's display data is well-formed UTF-16, so no text is lost. A
/// Java `null` result is `None`. Every method returns `Result` for the
/// exceptions of locale parsing and data access.
pub trait LocaleDisplayNames: Send + Sync {
    /// `getLocale()`: the locale used to determine the display names.
    fn get_locale(&self) -> ULocale;

    /// `getDialectHandling()`.
    fn get_dialect_handling(&self) -> DialectHandling;

    /// `getContext(DisplayContext.Type type)`: the current value for a
    /// specified DisplayContext.Type.
    fn get_context(&self, ty: Type) -> DisplayContext;

    /// `localeDisplayName(ULocale locale)`: the display name of the
    /// provided ulocale.
    ///
    /// # Errors
    ///
    /// The errors of locale canonicalization and data access.
    fn locale_display_name(&self, locale: &ULocale) -> Result<Option<String>>;

    /// `localeDisplayName(Locale locale)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn locale_display_name_locale(&self, locale: &Locale) -> Result<Option<String>>;

    /// `localeDisplayName(String localeId)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name); the errors
    /// of `new ULocale`.
    fn locale_display_name_id(&self, locale_id: &str) -> Result<Option<String>>;

    /// `languageDisplayName(String lang)`: the display name of the provided
    /// language code.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn language_display_name(&self, lang: &str) -> Result<Option<String>>;

    /// `scriptDisplayName(String script)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn script_display_name(&self, script: &str) -> Result<Option<String>>;

    /// `scriptDisplayNameInContext(String script)` (internal, deprecated):
    /// the display name of a script code for use in a locale display name.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn script_display_name_in_context(&self, script: &str) -> Result<Option<String>> {
        self.script_display_name(script)
    }

    /// `scriptDisplayName(int scriptCode)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn script_display_name_code(&self, script_code: i32) -> Result<Option<String>>;

    /// `regionDisplayName(String region)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn region_display_name(&self, region: &str) -> Result<Option<String>>;

    /// `variantDisplayName(String variant)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn variant_display_name(&self, variant: &str) -> Result<Option<String>>;

    /// `keyDisplayName(String key)`.
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn key_display_name(&self, key: &str) -> Result<Option<String>>;

    /// `keyValueDisplayName(String key, String value)` (`None` value =
    /// `null`).
    ///
    /// # Errors
    ///
    /// As [`locale_display_name`](Self::locale_display_name).
    fn key_value_display_name(&self, key: &str, value: Option<&str>) -> Result<Option<String>>;

    /// `getUiList(Set<ULocale> localeSet, boolean inSelf,
    /// Comparator<Object> collator)`: a list of information used to
    /// construct a UI list/menu of locales, sorted by the collator over the
    /// names in the display locale (or in self).
    ///
    /// # Errors
    ///
    /// As [`get_ui_list_compare_whole_items`](Self::get_ui_list_compare_whole_items).
    fn get_ui_list(
        &self,
        locale_set: &[ULocale],
        in_self: bool,
        collator: &dyn Fn(&str, &str) -> i32,
    ) -> Result<Vec<UiListItem>> {
        let comparator = UiListItem::get_comparator(collator, in_self);
        self.get_ui_list_compare_whole_items(locale_set, &comparator)
    }

    /// `getUiListCompareWholeItems(Set<ULocale> localeSet,
    /// Comparator<UiListItem> comparator)`.
    ///
    /// # Errors
    ///
    /// `IllformedLocaleException` for an ill-formed locale; the errors of
    /// the likely-subtags and display-name lookups.
    fn get_ui_list_compare_whole_items(
        &self,
        locale_set: &[ULocale],
        comparator: &dyn Fn(&UiListItem, &UiListItem) -> i32,
    ) -> Result<Vec<UiListItem>>;
}

/// `getInstance(ULocale locale)`: an instance using the given locale to
/// generate names.
pub fn get_instance(locale: &ULocale) -> Arc<dyn LocaleDisplayNames> {
    get_instance_dialect(locale, DialectHandling::StandardNames)
}

/// `getInstance(Locale locale)`.
///
/// # Errors
///
/// The errors of `ULocale.forLocale`.
pub fn get_instance_locale(locale: &Locale) -> Result<Arc<dyn LocaleDisplayNames>> {
    let u = ULocale::for_locale(Some(locale))?.expect("non-null locale");
    Ok(get_instance(&u))
}

/// Whether `FACTORY_DIALECTHANDLING`/`FACTORY_DISPLAYCONTEXT` exist: the
/// static initializer resolves the class named by the ICU configuration
/// (`com.ibm.icu.text.LocaleDisplayNames.impl`, default
/// `com.ibm.icu.impl.LocaleDisplayNamesImpl`) with `Class.forName`; the
/// only class of that name in icu4j is `LocaleDisplayNamesImpl`, which has
/// both factory methods. Any other name meets Java's
/// `ClassNotFoundException` branch (no factories: last-resort names).
static HAS_FACTORIES: LazyLock<bool> = LazyLock::new(|| {
    let impl_class_name = ICUConfig::get("com.ibm.icu.text.LocaleDisplayNames.impl")
        .unwrap_or_else(|| "com.ibm.icu.impl.LocaleDisplayNamesImpl".to_string());
    impl_class_name == "com.ibm.icu.impl.LocaleDisplayNamesImpl"
});

/// `getInstance(ULocale locale, DialectHandling dialectHandling)`: the
/// `LocaleDisplayNamesImpl` instance, or the last-resort implementation
/// when the factory throws (Java's `InvocationTargetException` branch) or
/// does not exist.
pub fn get_instance_dialect(
    locale: &ULocale,
    dialect_handling: DialectHandling,
) -> Arc<dyn LocaleDisplayNames> {
    let mut result: Option<Arc<dyn LocaleDisplayNames>> = None;
    if *HAS_FACTORIES {
        if let Ok(r) = LocaleDisplayNamesImpl::get_instance_dialect(locale, dialect_handling) {
            result = Some(r);
        }
        // InvocationTargetException: fall through
    }
    match result {
        Some(r) => r,
        None => Arc::new(LastResortLocaleDisplayNames::new_dialect(
            locale,
            dialect_handling,
        )),
    }
}

/// `getInstance(ULocale locale, DisplayContext... contexts)`.
pub fn get_instance_contexts(
    locale: &ULocale,
    contexts: &[DisplayContext],
) -> Arc<dyn LocaleDisplayNames> {
    let mut result: Option<Arc<dyn LocaleDisplayNames>> = None;
    if *HAS_FACTORIES {
        if let Ok(r) = LocaleDisplayNamesImpl::get_instance_contexts(locale, contexts) {
            result = Some(r);
        }
        // InvocationTargetException: fall through
    }
    match result {
        Some(r) => r,
        None => Arc::new(LastResortLocaleDisplayNames::new_contexts(locale, contexts)),
    }
}

/// `getInstance(Locale locale, DisplayContext... contexts)`.
///
/// # Errors
///
/// The errors of `ULocale.forLocale`.
pub fn get_instance_locale_contexts(
    locale: &Locale,
    contexts: &[DisplayContext],
) -> Result<Arc<dyn LocaleDisplayNames>> {
    let u = ULocale::for_locale(Some(locale))?.expect("non-null locale");
    Ok(get_instance_contexts(&u, contexts))
}

/// Struct-like class used to return information for constructing a UI
/// list, each corresponding to a locale.
///
/// Equivalent to `LocaleDisplayNames.UiListItem`.
#[derive(Debug, Clone)]
pub struct UiListItem {
    /// `minimized`: the minimized locale.
    pub minimized: ULocale,
    /// `modified`: the modified locale (script or region removed when not
    /// needed to distinguish).
    pub modified: ULocale,
    /// `nameInDisplayLocale`: the name of the modified locale in the display
    /// locale.
    pub name_in_display_locale: String,
    /// `nameInSelf`: the name of the modified locale in itself.
    pub name_in_self: String,
}

impl UiListItem {
    /// `new UiListItem(ULocale minimized, ULocale modified, String
    /// nameInDisplayLocale, String nameInSelf)`.
    pub fn new(
        minimized: ULocale,
        modified: ULocale,
        name_in_display_locale: String,
        name_in_self: String,
    ) -> Self {
        Self {
            minimized,
            modified,
            name_in_display_locale,
            name_in_self,
        }
    }

    /// `hashCode()`: `modified.hashCode() ^ nameInDisplayLocale.hashCode()`.
    pub fn hash_code(&self) -> i32 {
        self.modified.hash_code()
            ^ rucene::util::java_collections::java_string_hash_code(&self.name_in_display_locale)
    }

    /// `getComparator(Comparator<Object> comparator, boolean inSelf)`: a
    /// comparator over the names in self or in the display locale, then
    /// the modified locales.
    pub fn get_comparator<'c>(
        comparator: &'c dyn Fn(&str, &str) -> i32,
        in_self: bool,
    ) -> impl Fn(&UiListItem, &UiListItem) -> i32 + 'c {
        // UiListItemComparator
        let use_self = in_self;
        move |o1: &UiListItem, o2: &UiListItem| {
            let result = if use_self {
                comparator(&o1.name_in_self, &o2.name_in_self)
            } else {
                comparator(&o1.name_in_display_locale, &o2.name_in_display_locale)
            };
            if result != 0 {
                result
            } else {
                o1.modified.compare_to(&o2.modified) // just in case
            }
        }
    }
}

/// `equals(Object obj)`: equal names and locales.
impl PartialEq for UiListItem {
    fn eq(&self, other: &Self) -> bool {
        self.name_in_display_locale == other.name_in_display_locale
            && self.name_in_self == other.name_in_self
            && self.minimized == other.minimized
            && self.modified == other.modified
    }
}

impl Eq for UiListItem {}

/// `toString()`.
impl fmt::Display for UiListItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}, {}, {}, {}}}",
            self.minimized, self.modified, self.name_in_display_locale, self.name_in_self
        )
    }
}

/// Sorts `items` with a Java comparator (`Collections.sort`, a stable
/// merge sort).
pub(crate) fn sort_ui_list(
    items: &mut [UiListItem],
    comparator: &dyn Fn(&UiListItem, &UiListItem) -> i32,
) {
    items.sort_by(|a, b| comparator(a, b).cmp(&0));
}

/// Minimum implementation of LocaleDisplayNames, used when no
/// implementation is available.
///
/// Equivalent to `LocaleDisplayNames.LastResortLocaleDisplayNames`
/// (private).
#[derive(Debug)]
struct LastResortLocaleDisplayNames {
    locale: ULocale,
    contexts: Vec<DisplayContext>,
}

impl LastResortLocaleDisplayNames {
    fn new_dialect(locale: &ULocale, dialect_handling: DialectHandling) -> Self {
        let context = if dialect_handling == DialectHandling::DialectNames {
            DisplayContext::DialectNames
        } else {
            DisplayContext::StandardNames
        };
        Self {
            locale: locale.clone(),
            contexts: vec![context],
        }
    }

    fn new_contexts(locale: &ULocale, contexts: &[DisplayContext]) -> Self {
        Self {
            locale: locale.clone(),
            contexts: contexts.to_vec(),
        }
    }
}

impl LocaleDisplayNames for LastResortLocaleDisplayNames {
    fn get_locale(&self) -> ULocale {
        self.locale.clone()
    }

    fn get_dialect_handling(&self) -> DialectHandling {
        let mut result = DialectHandling::StandardNames;
        for context in &self.contexts {
            if context.r#type() == Type::DialectHandling
                && context.value() == DisplayContext::DialectNames.ordinal()
            {
                result = DialectHandling::DialectNames;
                break;
            }
        }
        result
    }

    fn get_context(&self, ty: Type) -> DisplayContext {
        let mut result = DisplayContext::StandardNames; // final fallback
        for context in &self.contexts {
            if context.r#type() == ty {
                result = *context;
                break;
            }
        }
        result
    }

    fn locale_display_name(&self, locale: &ULocale) -> Result<Option<String>> {
        Ok(Some(locale.get_name().to_string()))
    }

    fn locale_display_name_locale(&self, locale: &Locale) -> Result<Option<String>> {
        let u = ULocale::for_locale(Some(locale))?.expect("non-null locale");
        Ok(Some(u.get_name().to_string()))
    }

    fn locale_display_name_id(&self, locale_id: &str) -> Result<Option<String>> {
        Ok(Some(ULocale::new(locale_id)?.get_name().to_string()))
    }

    fn language_display_name(&self, lang: &str) -> Result<Option<String>> {
        Ok(Some(lang.to_string()))
    }

    fn script_display_name(&self, script: &str) -> Result<Option<String>> {
        Ok(Some(script.to_string()))
    }

    fn script_display_name_code(&self, script_code: i32) -> Result<Option<String>> {
        UScript::get_short_name(script_code)
    }

    fn region_display_name(&self, region: &str) -> Result<Option<String>> {
        Ok(Some(region.to_string()))
    }

    fn variant_display_name(&self, variant: &str) -> Result<Option<String>> {
        Ok(Some(variant.to_string()))
    }

    fn key_display_name(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(key.to_string()))
    }

    fn key_value_display_name(&self, _key: &str, value: Option<&str>) -> Result<Option<String>> {
        Ok(value.map(str::to_string))
    }

    fn get_ui_list_compare_whole_items(
        &self,
        _locale_set: &[ULocale],
        _comparator: &dyn Fn(&UiListItem, &UiListItem) -> i32,
    ) -> Result<Vec<UiListItem>> {
        Ok(Vec::new())
    }
}
