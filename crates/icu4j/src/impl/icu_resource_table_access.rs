// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUResourceTableAccess`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use rucene::error::Result;

use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::locale_ids::LocaleIDs;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// Static utility functions for probing resource tables, used by `ULocale`
/// and `LocaleDisplayNames`.
///
/// Equivalent to `com.ibm.icu.impl.ICUResourceTableAccess`.
pub struct ICUResourceTableAccess;

impl ICUResourceTableAccess {
    /// `getTableString(String path, ULocale locale, String tableName,
    /// String itemName, String defaultValue)`: a string from a table in a
    /// locale bundle.
    ///
    /// # Errors
    ///
    /// The errors of opening the bundle (`MissingResourceException`).
    pub fn get_table_string(
        path: &str,
        locale: &ULocale,
        table_name: &str,
        item_name: &str,
        default_value: Option<&str>,
    ) -> Result<Option<String>> {
        let bundle = UResourceBundle::get_bundle_instance(Some(path), &locale.get_base_name())?;
        let bundle = bundle.as_icu()?.clone();
        Ok(Self::get_table_string_of(
            bundle,
            table_name,
            None,
            item_name,
            default_value,
        ))
    }

    /// `getTableString(ICUResourceBundle bundle, String tableName, String
    /// subtableName, String item, String defaultValue)`: a string from a
    /// table, following deprecated codes and the `Fallback` locale; any
    /// exception (Java's `catch (Exception e)`) gives the default value.
    pub fn get_table_string_of(
        mut bundle: Arc<ICUResourceBundle>,
        table_name: &str,
        subtable_name: Option<&str>,
        item: &str,
        default_value: Option<&str>,
    ) -> Option<String> {
        let mut result: Option<String> = None;
        let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<Option<Option<String>>> {
            loop {
                let table = match bundle.find_with_fallback(table_name)? {
                    None => return Ok(Some(default_value.map(str::to_string))),
                    Some(t) => t,
                };
                let mut stable = Some(table.clone());
                if let Some(sub) = subtable_name {
                    stable = table.find_with_fallback(sub)?;
                }
                if let Some(stable) = stable {
                    result = stable
                        .find_string_with_fallback(item)?
                        .map(|s| String::from_utf16_lossy(&s));
                    if result.is_some() {
                        break; // possible real exception
                    }
                }

                // if we get here, stable was null, or there was no string for the item
                if subtable_name.is_none() {
                    // may be a deprecated code
                    let current_name = if table_name == "Countries" {
                        Some(LocaleIDs::get_current_country_id(item))
                    } else if table_name == "Languages" {
                        Some(LocaleIDs::get_current_language_id(item))
                    } else {
                        None
                    };
                    if let Some(current_name) = current_name {
                        result = table
                            .find_string_with_fallback(&current_name)?
                            .map(|s| String::from_utf16_lossy(&s));
                        if result.is_some() {
                            break; // possible real exception
                        }
                    }
                }

                // still can't figure it out? try the fallback mechanism
                let fallback_locale = table.find_string_with_fallback("Fallback")?; // again, possible exception
                let mut fallback_locale = match fallback_locale {
                    None => return Ok(Some(default_value.map(str::to_string))),
                    Some(f) => String::from_utf16_lossy(&f),
                };
                if fallback_locale.is_empty() {
                    fallback_locale = "root".to_string();
                }

                if fallback_locale == table.get_u_locale().get_name() {
                    return Ok(Some(default_value.map(str::to_string)));
                }

                let base_name = bundle.get_base_name().to_string();
                bundle = UResourceBundle::get_bundle_instance(Some(&base_name), &fallback_locale)?
                    .as_icu()?
                    .clone();
            }
            Ok(None)
        }));
        if let Ok(Ok(Some(early))) = outcome {
            return early;
        }
        // If something is seriously wrong, we might call getString on a resource that is
        // not a string.  That will throw an exception, which we catch and ignore here.

        // If the result is empty return item instead
        match result {
            Some(r) if !r.is_empty() => Some(r),
            _ => default_value.map(str::to_string),
        }
    }
}
