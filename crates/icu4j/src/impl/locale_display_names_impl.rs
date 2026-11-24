// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2009-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.LocaleDisplayNamesImpl`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{code_point_at, null_pointer, to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::lang::u_script::UScript;
use crate::r#impl::currency_data::{self, CurrencyDisplayInfo};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_lang_data_tables::ICULangDataTables;
use crate::r#impl::icu_region_data_tables::ICURegionDataTables;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::icu_resource_table_access::ICUResourceTableAccess;
use crate::r#impl::locale::ascii_util::AsciiUtil;
use crate::r#impl::simple_formatter_impl::{Arg, SimpleFormatterImpl};
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::break_iterator::{self, BreakIterator};
use crate::text::case_map::{self, Title};
use crate::text::display_context::{DisplayContext, Type};
use crate::text::locale_display_names::{
    sort_ui_list, DialectHandling, LocaleDisplayNames, UiListItem,
};
use crate::util::u_locale::{Builder as ULocaleBuilder, Minimize, ULocale};
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// `CapitalizationContextUsage` (private enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CapitalizationContextUsage {
    Language,
    Script,
    Territory,
    Variant,
    Key,
    Keyvalue,
}

/// `CapitalizationContextUsage.values().length`.
const CAPITALIZATION_CONTEXT_USAGE_COUNT: usize = 6;

/// `contextUsageTypeMap`.
fn context_usage_type(key: &str) -> Option<CapitalizationContextUsage> {
    match key {
        "languages" => Some(CapitalizationContextUsage::Language),
        "script" => Some(CapitalizationContextUsage::Script),
        "territory" => Some(CapitalizationContextUsage::Territory),
        "variant" => Some(CapitalizationContextUsage::Variant),
        "key" => Some(CapitalizationContextUsage::Key),
        "keyValue" => Some(CapitalizationContextUsage::Keyvalue),
        _ => None,
    }
}

/// `TO_TITLE_WHOLE_STRING_NO_LOWERCASE`.
static TO_TITLE_WHOLE_STRING_NO_LOWERCASE: LazyLock<Title> = LazyLock::new(|| {
    case_map::to_title()
        .whole_string()
        .expect("no other iterator option is set")
        .no_lowercase()
});

/// `toTitleWholeStringNoLowercase(ULocale locale, String s)`.
fn to_title_whole_string_no_lowercase(locale: &ULocale, s: &str) -> Result<String> {
    let l: Locale = locale.to_locale();
    Ok(to_rust_string(&TO_TITLE_WHOLE_STRING_NO_LOWERCASE.apply(
        Some(&l),
        None,
        &units(s),
    )?))
}

/// The ICU implementation of [`LocaleDisplayNames`].
///
/// Equivalent to `com.ibm.icu.impl.LocaleDisplayNamesImpl`. The sentence
/// break iterator that `adjustForUsageAndContext` uses under
/// `synchronized (this)` is held behind a `Mutex`.
pub struct LocaleDisplayNamesImpl {
    locale: ULocale,
    dialect_handling: DialectHandling,
    capitalization: DisplayContext,
    name_length: DisplayContext,
    substitute_handling: DisplayContext,
    lang_data: DataTable,
    region_data: DataTable,
    // Compiled SimpleFormatter patterns.
    separator_format: Vec<u16>,
    format: Vec<u16>,
    key_type_format: Vec<u16>,
    format_open_paren: char,
    format_replace_open_paren: char,
    format_close_paren: char,
    format_replace_close_paren: char,
    currency_display_info: Option<Arc<dyn CurrencyDisplayInfo>>,
    /// `capitalizationUsage`: `None` = Java's `null`.
    capitalization_usage: Option<[bool; CAPITALIZATION_CONTEXT_USAGE_COUNT]>,
    /// `capitalizationBrkIter` (transient).
    capitalization_brk_iter: Mutex<Option<Box<dyn BreakIterator>>>,
}

impl std::fmt::Debug for LocaleDisplayNamesImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocaleDisplayNamesImpl")
            .field("locale", &self.locale)
            .field("dialect_handling", &self.dialect_handling)
            .field("capitalization", &self.capitalization)
            .field("name_length", &self.name_length)
            .field("substitute_handling", &self.substitute_handling)
            .finish_non_exhaustive()
    }
}

/// `cache`: the single-entry `Cache` (private static class), used under
/// `synchronized (cache)`.
struct Cache {
    locale: Option<ULocale>,
    dialect_handling: Option<DialectHandling>,
    capitalization: Option<DisplayContext>,
    name_length: Option<DisplayContext>,
    substitute_handling: Option<DisplayContext>,
    cache: Option<Arc<LocaleDisplayNamesImpl>>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    locale: None,
    dialect_handling: None,
    capitalization: None,
    name_length: None,
    substitute_handling: None,
    cache: None,
});

impl Cache {
    fn get_dialect(
        &mut self,
        locale: &ULocale,
        dialect_handling: DialectHandling,
    ) -> Result<Arc<LocaleDisplayNamesImpl>> {
        if !(Some(dialect_handling) == self.dialect_handling
            && Some(DisplayContext::CapitalizationNone) == self.capitalization
            && Some(DisplayContext::LengthFull) == self.name_length
            && Some(DisplayContext::Substitute) == self.substitute_handling
            && Some(locale) == self.locale.as_ref())
        {
            self.locale = Some(locale.clone());
            self.dialect_handling = Some(dialect_handling);
            self.capitalization = Some(DisplayContext::CapitalizationNone);
            self.name_length = Some(DisplayContext::LengthFull);
            self.substitute_handling = Some(DisplayContext::Substitute);
            // (Java assigns the fields first: a throwing constructor leaves them updated.)
            self.cache = Some(Arc::new(LocaleDisplayNamesImpl::new_dialect(
                locale,
                dialect_handling,
            )?));
        }
        Ok(self
            .cache
            .clone()
            .expect("set above or by an earlier call with the same key"))
    }

    fn get_contexts(
        &mut self,
        locale: &ULocale,
        contexts: &[DisplayContext],
    ) -> Result<Arc<LocaleDisplayNamesImpl>> {
        let mut dialect_handling_in = DialectHandling::StandardNames;
        let mut capitalization_in = DisplayContext::CapitalizationNone;
        let mut name_length_in = DisplayContext::LengthFull;
        let mut substitute_handling = DisplayContext::Substitute;
        for &context_item in contexts {
            match context_item.r#type() {
                Type::DialectHandling => {
                    dialect_handling_in =
                        if context_item.value() == DisplayContext::StandardNames.value() {
                            DialectHandling::StandardNames
                        } else {
                            DialectHandling::DialectNames
                        };
                }
                Type::Capitalization => capitalization_in = context_item,
                Type::DisplayLength => name_length_in = context_item,
                Type::SubstituteHandling => substitute_handling = context_item,
            }
        }
        if !(Some(dialect_handling_in) == self.dialect_handling
            && Some(capitalization_in) == self.capitalization
            && Some(name_length_in) == self.name_length
            && Some(substitute_handling) == self.substitute_handling
            && Some(locale) == self.locale.as_ref())
        {
            self.locale = Some(locale.clone());
            self.dialect_handling = Some(dialect_handling_in);
            self.capitalization = Some(capitalization_in);
            self.name_length = Some(name_length_in);
            self.substitute_handling = Some(substitute_handling);
            self.cache = Some(Arc::new(LocaleDisplayNamesImpl::new_contexts(
                locale, contexts,
            )?));
        }
        Ok(self
            .cache
            .clone()
            .expect("set above or by an earlier call with the same key"))
    }
}

/// `CapitalizationContextSink` (private final inner class).
struct CapitalizationContextSink<'a> {
    capitalization: DisplayContext,
    capitalization_usage: &'a mut [bool; CAPITALIZATION_CONTEXT_USAGE_COUNT],
    has_capitalization_usage: bool,
}

impl Sink for CapitalizationContextSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let contexts_table = value.get_table()?;
        let mut i = 0;
        while contexts_table.get_key_and_value(i, key, &mut *value) {
            i += 1;
            let usage = match context_usage_type(&key.as_string()) {
                None => continue,
                Some(u) => u,
            };

            let int_vector = value.get_int_vector()?;
            if int_vector.len() < 2 {
                continue;
            }

            let titlecase_int =
                if self.capitalization == DisplayContext::CapitalizationForUiListOrMenu {
                    int_vector[0]
                } else {
                    int_vector[1]
                };
            if titlecase_int == 0 {
                continue;
            }

            self.capitalization_usage[usage as usize] = true;
            self.has_capitalization_usage = true;
        }
        Ok(())
    }
}

/// `String.replace(char, char)` twice: the parentheses of the format
/// pattern replaced by their substitutes.
fn replace_parens(
    s: &str,
    open: char,
    replace_open: char,
    close: char,
    replace_close: char,
) -> String {
    s.replace(open, &replace_open.to_string())
        .replace(close, &replace_close.to_string())
}

impl LocaleDisplayNamesImpl {
    /// `getInstance(ULocale locale, DialectHandling dialectHandling)`: the
    /// cached instance (a new one when the arguments differ from the last
    /// call).
    ///
    /// # Errors
    ///
    /// The errors of the constructor.
    pub fn get_instance_dialect(
        locale: &ULocale,
        dialect_handling: DialectHandling,
    ) -> Result<Arc<dyn LocaleDisplayNames>> {
        let mut cache = CACHE.lock().expect("cache lock");
        Ok(cache.get_dialect(locale, dialect_handling)? as Arc<dyn LocaleDisplayNames>)
    }

    /// `getInstance(ULocale locale, DisplayContext... contexts)`.
    ///
    /// # Errors
    ///
    /// The errors of the constructor.
    pub fn get_instance_contexts(
        locale: &ULocale,
        contexts: &[DisplayContext],
    ) -> Result<Arc<dyn LocaleDisplayNames>> {
        let mut cache = CACHE.lock().expect("cache lock");
        Ok(cache.get_contexts(locale, contexts)? as Arc<dyn LocaleDisplayNames>)
    }

    /// `new LocaleDisplayNamesImpl(ULocale locale, DialectHandling
    /// dialectHandling)`.
    ///
    /// # Errors
    ///
    /// As [`new_contexts`](Self::new_contexts).
    pub fn new_dialect(locale: &ULocale, dialect_handling: DialectHandling) -> Result<Self> {
        Self::new_contexts(
            locale,
            &[
                if dialect_handling == DialectHandling::StandardNames {
                    DisplayContext::StandardNames
                } else {
                    DisplayContext::DialectNames
                },
                DisplayContext::CapitalizationNone,
            ],
        )
    }

    /// `new LocaleDisplayNamesImpl(ULocale locale, DisplayContext...
    /// contexts)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the display data cannot be loaded;
    /// the errors of compiling the display patterns and of creating the
    /// sentence break iterator.
    pub fn new_contexts(locale: &ULocale, contexts: &[DisplayContext]) -> Result<Self> {
        let mut dialect_handling = DialectHandling::StandardNames;
        let mut capitalization = DisplayContext::CapitalizationNone;
        let mut name_length = DisplayContext::LengthFull;
        let mut substitute_handling = DisplayContext::Substitute;
        for &context_item in contexts {
            match context_item.r#type() {
                Type::DialectHandling => {
                    dialect_handling =
                        if context_item.value() == DisplayContext::StandardNames.value() {
                            DialectHandling::StandardNames
                        } else {
                            DialectHandling::DialectNames
                        };
                }
                Type::Capitalization => capitalization = context_item,
                Type::DisplayLength => name_length = context_item,
                Type::SubstituteHandling => substitute_handling = context_item,
            }
        }

        let lang_data =
            lang_data_tables().get(locale, substitute_handling == DisplayContext::NoSubstitute)?;
        let region_data = region_data_tables()
            .get(locale, substitute_handling == DisplayContext::NoSubstitute)?;
        let this_locale = if ULocale::root() == lang_data.get_locale() {
            region_data.get_locale()
        } else {
            lang_data.get_locale()
        };

        // Note, by going through DataTable, this uses table lookup rather than straight lookup.
        // That should get us the same data, I think.  This way we don't have to explicitly
        // load the bundle again.  Using direct lookup didn't seem to make an appreciable
        // difference in performance.
        let mut sep = lang_data.get_sub("localeDisplayPattern", None, "separator");
        if sep.as_deref().is_none_or(|s| s == "separator") {
            sep = Some("{0}, {1}".to_string());
        }
        let mut sb = Vec::new();
        let separator_format = SimpleFormatterImpl::compile_to_string_min_max_arguments(
            &units(sep.as_deref().expect("set")),
            &mut sb,
            2,
            2,
        )?;

        let mut pattern = lang_data.get_sub("localeDisplayPattern", None, "pattern");
        if pattern.as_deref().is_none_or(|p| p == "pattern") {
            pattern = Some("{0} ({1})".to_string());
        }
        let pattern = pattern.expect("set");
        let format = SimpleFormatterImpl::compile_to_string_min_max_arguments(
            &units(&pattern),
            &mut sb,
            2,
            2,
        )?;
        let (
            format_open_paren,
            format_close_paren,
            format_replace_open_paren,
            format_replace_close_paren,
        ) = if pattern.contains('（') {
            ('（', '）', '［', '］')
        } else {
            ('(', ')', '[', ']')
        };

        let mut key_type_pattern =
            lang_data.get_sub("localeDisplayPattern", None, "keyTypePattern");
        if key_type_pattern
            .as_deref()
            .is_none_or(|p| p == "keyTypePattern")
        {
            key_type_pattern = Some("{0}={1}".to_string());
        }
        let key_type_format = SimpleFormatterImpl::compile_to_string_min_max_arguments(
            &units(key_type_pattern.as_deref().expect("set")),
            &mut sb,
            2,
            2,
        )?;

        // Get values from the contextTransforms data if we need them
        // Also check whether we will need a break iterator (depends on the data)
        let mut need_brk_iter = false;
        let mut capitalization_usage = None;
        if capitalization == DisplayContext::CapitalizationForUiListOrMenu
            || capitalization == DisplayContext::CapitalizationForStandalone
        {
            let mut usage = [false; CAPITALIZATION_CONTEXT_USAGE_COUNT]; // initialized to all false
            let rb = UResourceBundle::get_bundle_instance_for(
                Some(ICUData::ICU_BASE_NAME),
                Some(locale),
            )?;
            let rb: Arc<ICUResourceBundle> = rb.as_icu()?.clone();
            let mut sink = CapitalizationContextSink {
                capitalization,
                capitalization_usage: &mut usage,
                has_capitalization_usage: false,
            };
            match rb.get_all_items_with_fallback("contextTransforms", &mut sink) {
                Ok(()) => {}
                // Silently ignore.  Not every locale has contextTransforms.
                Err(e) if is_missing_resource(&e) => {}
                Err(e) => return Err(e),
            }
            need_brk_iter = sink.has_capitalization_usage;
            capitalization_usage = Some(usage);
        }
        // Get a sentence break iterator if we will need it
        let mut capitalization_brk_iter = None;
        if need_brk_iter || capitalization == DisplayContext::CapitalizationForBeginningOfSentence {
            capitalization_brk_iter = Some(break_iterator::get_sentence_instance_ulocale(locale)?);
        }

        let currency_display_info = currency_data::provider().get_instance(Some(locale), false)?;

        Ok(Self {
            locale: this_locale,
            dialect_handling,
            capitalization,
            name_length,
            substitute_handling,
            lang_data,
            region_data,
            separator_format,
            format,
            key_type_format,
            format_open_paren,
            format_replace_open_paren,
            format_close_paren,
            format_replace_close_paren,
            currency_display_info,
            capitalization_usage,
            capitalization_brk_iter: Mutex::new(capitalization_brk_iter),
        })
    }

    fn rp(&self, s: &str) -> String {
        replace_parens(
            s,
            self.format_open_paren,
            self.format_replace_open_paren,
            self.format_close_paren,
            self.format_replace_close_paren,
        )
    }

    fn adjust_for_usage_and_context(
        &self,
        usage: CapitalizationContextUsage,
        name: Option<String>,
    ) -> Result<Option<String>> {
        if let Some(n) = &name {
            let u = units(n);
            if !u.is_empty()
                && UCharacter::is_lower_case(code_point_at(&u, 0))
                && (self.capitalization == DisplayContext::CapitalizationForBeginningOfSentence
                    || self
                        .capitalization_usage
                        .is_some_and(|cu| cu[usage as usize]))
            {
                // Note, won't have capitalizationUsage != null && capitalizationUsage[usage.ordinal()]
                // unless capitalization is CAPITALIZATION_FOR_UI_LIST_OR_MENU or CAPITALIZATION_FOR_STANDALONE
                let mut brk = self
                    .capitalization_brk_iter
                    .lock()
                    .expect("break iterator lock");
                if brk.is_none() {
                    // should only happen when deserializing, etc.
                    *brk = Some(break_iterator::get_sentence_instance_ulocale(&self.locale)?);
                }
                let it = brk.as_mut().expect("set above").as_mut();
                let titled = UCharacter::to_title_case_ulocale_options(
                    Some(&self.locale),
                    &u,
                    Some(it),
                    UCharacter::TITLECASE_NO_LOWERCASE | UCharacter::TITLECASE_NO_BREAK_ADJUSTMENT,
                )?;
                return Ok(Some(to_rust_string(&titled)));
            }
        }
        Ok(name)
    }

    // TODO: implement use of capitalization
    fn locale_display_name_internal(&self, locale: &ULocale) -> Result<Option<String>> {
        // lang
        // lang (script, country, variant, keyword=value, ...)
        // script, country, variant, keyword=value, ...

        let mut result_name: Option<String> = None;

        let mut lang = locale.get_language();

        // Empty basename indicates root locale (keywords are ignored for this).
        // For the display name, we treat this as unknown language (ICU-20273).
        if lang.is_empty() {
            lang = "und".to_string();
        }
        let script = locale.get_script();
        let country = locale.get_country();
        let variant = locale.get_variant();

        let mut has_script = !script.is_empty();
        let mut has_country = !country.is_empty();
        let has_variant = !variant.is_empty();

        // always have a value for lang
        if self.dialect_handling == DialectHandling::DialectNames {
            // loop construct is so we can break early out of search
            #[allow(clippy::never_loop)]
            loop {
                if has_script && has_country {
                    let lang_script_country = format!("{lang}_{script}_{country}");
                    let result = self.locale_id_name(&lang_script_country)?;
                    if let Some(r) = result.filter(|r| *r != lang_script_country) {
                        result_name = Some(r);
                        has_script = false;
                        has_country = false;
                        break;
                    }
                }
                if has_script {
                    let lang_script = format!("{lang}_{script}");
                    let result = self.locale_id_name(&lang_script)?;
                    if let Some(r) = result.filter(|r| *r != lang_script) {
                        result_name = Some(r);
                        has_script = false;
                        break;
                    }
                }
                if has_country {
                    let lang_country = format!("{lang}_{country}");
                    let result = self.locale_id_name(&lang_country)?;
                    if let Some(r) = result.filter(|r| *r != lang_country) {
                        result_name = Some(r);
                        has_country = false;
                        break;
                    }
                }
                break;
            }
        }

        let mut result_name = match result_name {
            Some(r) => r,
            None => {
                let result = match self.locale_id_name(&lang)? {
                    None => return Ok(None),
                    Some(r) => r,
                };
                self.rp(&result)
            }
        };

        let mut buf: Vec<u16> = Vec::new();
        if has_script {
            // first element, don't need appendWithSep
            let result = match self.script_display_name_in_context_skip(&script, true)? {
                None => return Ok(None),
                Some(r) => r,
            };
            buf.extend_from_slice(&units(&self.rp(&result)));
        }
        if has_country {
            let result = match self.region_display_name_skip(&country, true)? {
                None => return Ok(None),
                Some(r) => r,
            };
            self.append_with_sep(&self.rp(&result), &mut buf)?;
        }
        if has_variant {
            let result = match self.variant_display_name_skip(&variant, true)? {
                None => return Ok(None),
                Some(r) => r,
            };
            self.append_with_sep(&self.rp(&result), &mut buf)?;
        }

        if let Some(keys) = locale.get_keywords() {
            for key in keys {
                let value = locale.get_keyword_value(&key);
                let key_display_name = match self.key_display_name_skip(&key, true)? {
                    None => return Ok(None),
                    Some(k) => k,
                };
                let key_display_name = self.rp(&key_display_name);
                let value_display_name =
                    match self.key_value_display_name_skip(&key, value.as_deref(), true)? {
                        None => return Ok(None),
                        Some(v) => v,
                    };
                let value_display_name = self.rp(&value_display_name);
                if value.as_deref() != Some(value_display_name.as_str()) {
                    self.append_with_sep(&value_display_name, &mut buf)?;
                } else if key != key_display_name {
                    let key_value = SimpleFormatterImpl::format_compiled_pattern(
                        &self.key_type_format,
                        &[
                            Arg::Text(&units(&key_display_name)),
                            Arg::Text(&units(&value_display_name)),
                        ],
                    )?;
                    self.append_with_sep(&to_rust_string(&key_value), &mut buf)?;
                } else {
                    self.append_with_sep(&key_display_name, &mut buf)?;
                    buf.extend_from_slice(&units("="));
                    buf.extend_from_slice(&units(&value_display_name));
                }
            }
        }

        let mut result_remainder: Option<Vec<u16>> = None;
        if !buf.is_empty() {
            result_remainder = Some(buf);
        }

        if let Some(rem) = result_remainder {
            result_name = to_rust_string(&SimpleFormatterImpl::format_compiled_pattern(
                &self.format,
                &[Arg::Text(&units(&result_name)), Arg::Text(&rem)],
            )?);
        }

        self.adjust_for_usage_and_context(CapitalizationContextUsage::Language, Some(result_name))
    }

    fn locale_id_name(&self, locale_id: &str) -> Result<Option<String>> {
        let mut loc_id_name;
        if self.name_length == DisplayContext::LengthShort {
            loc_id_name = self.lang_data.get("Languages%short", locale_id);
            if loc_id_name.as_deref().is_some_and(|n| n != locale_id) {
                return Ok(loc_id_name);
            }
        }
        loc_id_name = self.lang_data.get("Languages", locale_id);
        if loc_id_name.as_deref().is_none_or(|n| n == locale_id) && !locale_id.contains('_') {
            // Canonicalize lang and try again, ICU-20870
            // (only for language codes without script or region)
            let canon_locale = ULocale::create_canonical(locale_id)?;
            let canon_loc_id = canon_locale.get_name().to_string();
            if self.name_length == DisplayContext::LengthShort {
                loc_id_name = self.lang_data.get("Languages%short", &canon_loc_id);
                if loc_id_name.as_deref().is_some_and(|n| n != canon_loc_id) {
                    return Ok(loc_id_name);
                }
            }
            loc_id_name = self.lang_data.get("Languages", &canon_loc_id);
        }
        Ok(loc_id_name)
    }

    fn script_display_name_in_context_skip(
        &self,
        script: &str,
        skip_adjust: bool,
    ) -> Result<Option<String>> {
        if self.name_length == DisplayContext::LengthShort {
            let script_name = self.lang_data.get("Scripts%short", script);
            if script_name.as_deref().is_some_and(|n| n != script) {
                return if skip_adjust {
                    Ok(script_name)
                } else {
                    self.adjust_for_usage_and_context(
                        CapitalizationContextUsage::Script,
                        script_name,
                    )
                };
            }
        }
        let script_name = self.lang_data.get("Scripts", script);
        if skip_adjust {
            Ok(script_name)
        } else {
            self.adjust_for_usage_and_context(CapitalizationContextUsage::Script, script_name)
        }
    }

    fn region_display_name_skip(&self, region: &str, skip_adjust: bool) -> Result<Option<String>> {
        if self.name_length == DisplayContext::LengthShort {
            let region_name = self.region_data.get("Countries%short", region);
            if region_name.as_deref().is_some_and(|n| n != region) {
                return if skip_adjust {
                    Ok(region_name)
                } else {
                    self.adjust_for_usage_and_context(
                        CapitalizationContextUsage::Territory,
                        region_name,
                    )
                };
            }
        }
        let region_name = self.region_data.get("Countries", region);
        if skip_adjust {
            Ok(region_name)
        } else {
            self.adjust_for_usage_and_context(CapitalizationContextUsage::Territory, region_name)
        }
    }

    fn variant_display_name_skip(
        &self,
        variant: &str,
        skip_adjust: bool,
    ) -> Result<Option<String>> {
        // don't have a resource for short variant names
        let variant_name = self.lang_data.get("Variants", variant);
        if skip_adjust {
            Ok(variant_name)
        } else {
            self.adjust_for_usage_and_context(CapitalizationContextUsage::Variant, variant_name)
        }
    }

    fn key_display_name_skip(&self, key: &str, skip_adjust: bool) -> Result<Option<String>> {
        // don't have a resource for short key names
        let key_name = self.lang_data.get("Keys", key);
        if skip_adjust {
            Ok(key_name)
        } else {
            self.adjust_for_usage_and_context(CapitalizationContextUsage::Key, key_name)
        }
    }

    fn key_value_display_name_skip(
        &self,
        key: &str,
        value: Option<&str>,
        skip_adjust: bool,
    ) -> Result<Option<String>> {
        let mut key_value_name: Option<String> = None;

        if key == "currency" {
            // (Java dereferences `currencyDisplayInfo` and `value` here: NullPointerException when null.)
            let info = self
                .currency_display_info
                .as_ref()
                .ok_or_else(|| null_pointer("Cannot invoke \"com.ibm.icu.impl.CurrencyData$CurrencyDisplayInfo.getName(String)\" because \"this.currencyDisplayInfo\" is null"))?;
            let v = value.ok_or_else(|| {
                null_pointer("Cannot invoke \"String.length()\" because \"s\" is null")
            })?;
            key_value_name = info.get_name(&AsciiUtil::to_upper_string(v))?;
            if key_value_name.is_none() {
                key_value_name = Some(v.to_string());
            }
        } else {
            if self.name_length == DisplayContext::LengthShort {
                let tmp = self.lang_data.get_sub_value("Types%short", key, value);
                if tmp.is_some() && tmp.as_deref() != value {
                    key_value_name = tmp;
                }
            }
            if key_value_name.is_none() {
                key_value_name = self.lang_data.get_sub_value("Types", key, value);
            }
        }

        if skip_adjust {
            Ok(key_value_name)
        } else {
            self.adjust_for_usage_and_context(CapitalizationContextUsage::Keyvalue, key_value_name)
        }
    }

    fn new_row(&self, modified: ULocale, cap_context: DisplayContext) -> Result<UiListItem> {
        let minimized = ULocale::minimize_subtags_favor(&modified, Minimize::FavorScript)?;
        // (`getDisplayName` uses the standard, substituting instance: never null.)
        let temp_name = modified.get_display_name_in(&self.locale)?.ok_or_else(|| {
            null_pointer("Cannot invoke \"String.length()\" because \"s\" is null")
        })?;
        let titlecase = cap_context == DisplayContext::CapitalizationForUiListOrMenu;
        let name_in_display_locale = if titlecase {
            to_title_whole_string_no_lowercase(&self.locale, &temp_name)?
        } else {
            temp_name
        };
        let temp_name = modified.get_display_name_in(&modified)?.ok_or_else(|| {
            null_pointer("Cannot invoke \"String.length()\" because \"s\" is null")
        })?;
        let name_in_self = if cap_context == DisplayContext::CapitalizationForUiListOrMenu {
            to_title_whole_string_no_lowercase(&modified, &temp_name)?
        } else {
            temp_name
        };
        Ok(UiListItem::new(
            minimized,
            modified,
            name_in_display_locale,
            name_in_self,
        ))
    }

    fn append_with_sep(&self, s: &str, b: &mut Vec<u16>) -> Result<()> {
        if b.is_empty() {
            b.extend_from_slice(&units(s));
        } else {
            let su = units(s);
            SimpleFormatterImpl::format_and_replace(
                &self.separator_format,
                b,
                None,
                &[Arg::Result, Arg::Text(&su)],
            )?;
        }
        Ok(())
    }

    /// `haveData(DataTableType type)`: whether the lang or region display
    /// data tables are the ICU ones.
    pub fn have_data(ty: DataTableType) -> bool {
        match ty {
            DataTableType::Lang => matches!(lang_data_tables(), DataTables::Icu(_)),
            DataTableType::Region => matches!(region_data_tables(), DataTables::Icu(_)),
        }
    }
}

impl LocaleDisplayNames for LocaleDisplayNamesImpl {
    fn get_locale(&self) -> ULocale {
        self.locale.clone()
    }

    fn get_dialect_handling(&self) -> DialectHandling {
        self.dialect_handling
    }

    fn get_context(&self, ty: Type) -> DisplayContext {
        match ty {
            Type::DialectHandling => {
                if self.dialect_handling == DialectHandling::StandardNames {
                    DisplayContext::StandardNames
                } else {
                    DisplayContext::DialectNames
                }
            }
            Type::Capitalization => self.capitalization,
            Type::DisplayLength => self.name_length,
            Type::SubstituteHandling => self.substitute_handling,
        }
    }

    fn locale_display_name(&self, locale: &ULocale) -> Result<Option<String>> {
        self.locale_display_name_internal(locale)
    }

    fn locale_display_name_locale(&self, locale: &Locale) -> Result<Option<String>> {
        let u = ULocale::for_locale(Some(locale))?.expect("non-null locale");
        self.locale_display_name_internal(&u)
    }

    fn locale_display_name_id(&self, locale_id: &str) -> Result<Option<String>> {
        self.locale_display_name_internal(&ULocale::new(locale_id)?)
    }

    fn language_display_name(&self, lang: &str) -> Result<Option<String>> {
        // Special case to eliminate non-languages, which pollute our data.
        if lang == "root" || lang.contains('_') {
            return Ok(if self.substitute_handling == DisplayContext::Substitute {
                Some(lang.to_string())
            } else {
                None
            });
        }
        let mut lang_name;
        if self.name_length == DisplayContext::LengthShort {
            lang_name = self.lang_data.get("Languages%short", lang);
            if lang_name.as_deref().is_some_and(|n| n != lang) {
                return self
                    .adjust_for_usage_and_context(CapitalizationContextUsage::Language, lang_name);
            }
        }
        lang_name = self.lang_data.get("Languages", lang);
        if lang_name.as_deref().is_none_or(|n| n == lang) {
            // Canonicalize lang and try again, ICU-20870
            let canon_locale = ULocale::create_canonical(lang)?;
            let canon_loc_id = canon_locale.get_name().to_string();
            if self.name_length == DisplayContext::LengthShort {
                lang_name = self.lang_data.get("Languages%short", &canon_loc_id);
                if lang_name.as_deref().is_some_and(|n| n != canon_loc_id) {
                    return self.adjust_for_usage_and_context(
                        CapitalizationContextUsage::Language,
                        lang_name,
                    );
                }
            }
            lang_name = self.lang_data.get("Languages", &canon_loc_id);
        }
        self.adjust_for_usage_and_context(CapitalizationContextUsage::Language, lang_name)
    }

    fn script_display_name(&self, script: &str) -> Result<Option<String>> {
        let mut str = self.lang_data.get("Scripts%stand-alone", script);
        if str.as_deref().is_none_or(|s| s == script) {
            if self.name_length == DisplayContext::LengthShort {
                str = self.lang_data.get("Scripts%short", script);
                if str.as_deref().is_some_and(|s| s != script) {
                    return self
                        .adjust_for_usage_and_context(CapitalizationContextUsage::Script, str);
                }
            }
            str = self.lang_data.get("Scripts", script);
        }
        self.adjust_for_usage_and_context(CapitalizationContextUsage::Script, str)
    }

    fn script_display_name_in_context(&self, script: &str) -> Result<Option<String>> {
        self.script_display_name_in_context_skip(script, false)
    }

    fn script_display_name_code(&self, script_code: i32) -> Result<Option<String>> {
        // (Java passes UScript.getShortName's result, never null for a valid code.)
        let short_name = UScript::get_short_name(script_code)?;
        match short_name {
            Some(s) => self.script_display_name(&s),
            None => Err(null_pointer(
                "Cannot invoke \"String.equals(Object)\" because \"script\" is null",
            )),
        }
    }

    fn region_display_name(&self, region: &str) -> Result<Option<String>> {
        self.region_display_name_skip(region, false)
    }

    fn variant_display_name(&self, variant: &str) -> Result<Option<String>> {
        self.variant_display_name_skip(variant, false)
    }

    fn key_display_name(&self, key: &str) -> Result<Option<String>> {
        self.key_display_name_skip(key, false)
    }

    fn key_value_display_name(&self, key: &str, value: Option<&str>) -> Result<Option<String>> {
        self.key_value_display_name_skip(key, value, false)
    }

    #[allow(clippy::mutable_key_type)] // ULocale hashes its immutable ID only; its caches do not affect Hash/Eq
    fn get_ui_list_compare_whole_items(
        &self,
        locale_set: &[ULocale],
        comparator: &dyn Fn(&UiListItem, &UiListItem) -> i32,
    ) -> Result<Vec<UiListItem>> {
        let cap_context = self.get_context(Type::Capitalization);

        let mut result: Vec<UiListItem> = Vec::new();
        // (The HashMap/HashSet iteration orders only feed the rows that the stable sort below
        // orders completely: rows that compare equal are identical.)
        let mut base_to_locales: HashMap<ULocale, HashSet<ULocale>> = HashMap::new();
        let mut base_order: Vec<ULocale> = Vec::new();
        let mut builder = ULocaleBuilder::new();
        for loc_original in locale_set {
            builder.set_locale(loc_original)?; // verify well-formed. We do this here so that we consistently throw exception
            let loc = ULocale::add_likely_subtags(loc_original)?;
            let base = ULocale::new(&loc.get_language())?;
            if !base_to_locales.contains_key(&base) {
                base_order.push(base.clone());
            }
            base_to_locales.entry(base).or_default().insert(loc);
        }
        for base in base_order {
            let values = &base_to_locales[&base];
            if values.len() == 1 {
                let locale = values.iter().next().expect("one value").clone();
                result.push(self.new_row(
                    ULocale::minimize_subtags_favor(&locale, Minimize::FavorScript)?,
                    cap_context,
                )?);
            } else {
                let mut scripts: HashSet<String> = HashSet::new();
                let mut regions: HashSet<String> = HashSet::new();
                // need the follow two steps to make sure that unusual scripts or regions are displayed
                let max_base = ULocale::add_likely_subtags(&base)?;
                scripts.insert(max_base.get_script());
                regions.insert(max_base.get_country());
                for locale in values {
                    scripts.insert(locale.get_script());
                    regions.insert(locale.get_country());
                }
                let has_scripts = scripts.len() > 1;
                let has_regions = regions.len() > 1;
                for locale in values {
                    let modified = builder.set_locale(locale)?;
                    if !has_scripts {
                        modified.set_script(Some(""))?;
                    }
                    if !has_regions {
                        modified.set_region(Some(""))?;
                    }
                    let built = modified.build()?;
                    result.push(self.new_row(built, cap_context)?);
                }
            }
        }
        sort_ui_list(&mut result, comparator);
        Ok(result)
    }
}

/// `DataTable` (public static class) and its subclass `ICUDataTable`
/// (package-private): display strings by table, subtable and code.
#[derive(Debug, Clone)]
pub enum DataTable {
    /// `DataTable`: no data, the code itself (or `None`).
    Plain {
        /// `nullIfNotFound`.
        null_if_not_found: bool,
    },
    /// `ICUDataTable`: the strings of an ICU display-data bundle.
    Icu {
        /// `nullIfNotFound`.
        null_if_not_found: bool,
        /// `bundle`.
        bundle: Arc<ICUResourceBundle>,
    },
}

impl DataTable {
    /// `new ICUDataTable(String path, ULocale locale, boolean
    /// nullIfNotFound)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the bundle cannot be loaded.
    pub fn new_icu(path: &str, locale: &ULocale, null_if_not_found: bool) -> Result<Self> {
        let bundle = UResourceBundle::get_bundle_instance(Some(path), &locale.get_base_name())?;
        Ok(DataTable::Icu {
            null_if_not_found,
            bundle: bundle.as_icu()?.clone(),
        })
    }

    /// `getLocale()`.
    pub fn get_locale(&self) -> ULocale {
        match self {
            DataTable::Plain { .. } => ULocale::root(),
            DataTable::Icu { bundle, .. } => bundle.get_u_locale(),
        }
    }

    /// `get(String tableName, String code)`.
    pub fn get(&self, table_name: &str, code: &str) -> Option<String> {
        self.get_sub(table_name, None, code)
    }

    /// `get(String tableName, String subTableName, String code)`.
    pub fn get_sub(
        &self,
        table_name: &str,
        sub_table_name: Option<&str>,
        code: &str,
    ) -> Option<String> {
        match self {
            DataTable::Plain { null_if_not_found } => {
                if *null_if_not_found {
                    None
                } else {
                    Some(code.to_string())
                }
            }
            DataTable::Icu {
                null_if_not_found,
                bundle,
            } => ICUResourceTableAccess::get_table_string_of(
                bundle.clone(),
                table_name,
                sub_table_name,
                code,
                if *null_if_not_found { None } else { Some(code) },
            ),
        }
    }

    /// `get(tableName, subTableName, code)` with a nullable code: a `null`
    /// code yields `null` (the plain table returns the code itself;
    /// `ICUResourceTableAccess.getTableString` gets a `NullPointerException`
    /// from the lookup, catches it and returns its default, the code).
    fn get_sub_value(
        &self,
        table_name: &str,
        sub_table_name: &str,
        code: Option<&str>,
    ) -> Option<String> {
        match code {
            Some(c) => self.get_sub(table_name, Some(sub_table_name), c),
            None => None,
        }
    }
}

/// `DataTables` (package-private abstract class) and its subclass
/// `ICUDataTables`: the factory of display-data tables.
#[derive(Debug, Clone, Copy)]
pub enum DataTables {
    /// `ICUDataTables`: ICU display-data bundles under a path.
    Icu(&'static str),
    /// The anonymous `DataTables` of `load`'s `catch (Throwable t)`
    /// branch: plain tables.
    Default,
}

impl DataTables {
    /// `get(ULocale locale, boolean nullIfNotFound)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when an ICU bundle cannot be loaded.
    pub fn get(&self, locale: &ULocale, null_if_not_found: bool) -> Result<DataTable> {
        match self {
            DataTables::Icu(path) => DataTable::new_icu(path, locale, null_if_not_found),
            DataTables::Default => Ok(DataTable::Plain { null_if_not_found }),
        }
    }

    /// `load(String className)`: `Class.forName(className).newInstance()`
    /// over the two classes icu4j has (`ICULangDataTables`,
    /// `ICURegionDataTables`, constructed directly); any other name meets
    /// Java's `catch (Throwable t)` branch.
    pub fn load(class_name: &str) -> DataTables {
        match class_name {
            "com.ibm.icu.impl.ICULangDataTables" => ICULangDataTables::new(),
            "com.ibm.icu.impl.ICURegionDataTables" => ICURegionDataTables::new(),
            _ => DataTables::Default,
        }
    }
}

/// `LangDataTables.impl`.
fn lang_data_tables() -> DataTables {
    static IMPL: LazyLock<DataTables> =
        LazyLock::new(|| DataTables::load("com.ibm.icu.impl.ICULangDataTables"));
    *IMPL
}

/// `RegionDataTables.impl`.
fn region_data_tables() -> DataTables {
    static IMPL: LazyLock<DataTables> =
        LazyLock::new(|| DataTables::load("com.ibm.icu.impl.ICURegionDataTables"));
    *IMPL
}

/// `DataTableType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTableType {
    /// `LANG`.
    Lang,
    /// `REGION`.
    Region,
}
