// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationLoader`.

use std::sync::{Arc, Mutex, OnceLock};

use rucene::error::Result;

use crate::r#impl::coll::collation_data_reader::CollationDataReader;
use crate::r#impl::coll::collation_root::CollationRoot;
use crate::r#impl::coll::collation_tailoring::CollationTailoring;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{ICUResourceBundle, OpenType};
use crate::jdk::to_rust_string;
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;
use crate::util::output::Output;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// Convenience string denoting the Collation data tree.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationLoader` (all methods
/// static).
#[derive(Debug)]
pub struct CollationLoader;

/// `rootRules` (volatile, set once under the class monitor).
static ROOT_RULES: OnceLock<Vec<u16>> = OnceLock::new();
static ROOT_RULES_LOCK: Mutex<()> = Mutex::new(());

/// `ICUResourceBundle` cast of a bundle; Java's `(ICUResourceBundle)`.
fn as_icu(bundle: &UResourceBundle) -> &Arc<ICUResourceBundle> {
    match bundle {
        UResourceBundle::Icu(b) => b,
        UResourceBundle::Wrapper(_) => panic!(
            "java.lang.ClassCastException: class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle"
        ),
    }
}

/// `ASCII.toLowerCase(String)`.
fn ascii_to_lower_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push((c as u8 + 0x20) as char);
        } else {
            out.push(c);
        }
    }
    out
}

impl CollationLoader {
    fn load_root_rules() -> Result<()> {
        if ROOT_RULES.get().is_some() {
            return Ok(());
        }
        let _guard = ROOT_RULES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if ROOT_RULES.get().is_none() {
            let root_bundle =
                UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_COLLATION_BASE_NAME), Some(&ULocale::root()))?;
            let _ = ROOT_RULES.set(root_bundle.get_string_by_key("UCARules")?);
        }
        Ok(())
    }

    /// C++: static void appendRootRules(UnicodeString &s)
    ///
    /// Equivalent to `getRootRules()`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the root rules are missing.
    pub fn get_root_rules() -> Result<Vec<u16>> {
        Self::load_root_rules()?;
        Ok(ROOT_RULES.get().expect("loaded").clone())
    }

    /// `loadRules(ULocale, String)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the rules are missing.
    pub fn load_rules(locale: &ULocale, collation_type: &str) -> Result<Vec<u16>> {
        let bundle = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_COLLATION_BASE_NAME), Some(locale))?;
        let data = as_icu(&bundle).get_with_fallback(&format!("collations/{}", ascii_to_lower_case(collation_type)))?;
        UResourceBundle::Icu(data).get_string_by_key("Sequence")
    }

    fn find_with_fallback(table: &UResourceBundle, entry_name: &str) -> Result<Option<UResourceBundle>> {
        Ok(as_icu(table).find_with_fallback(entry_name)?.map(UResourceBundle::Icu))
    }

    /// `loadTailoring(ULocale, Output<ULocale>)`.
    ///
    /// # Errors
    ///
    /// The exceptions of loading the root and the tailoring data.
    pub fn load_tailoring(locale: &ULocale, out_valid_locale: &mut Output<ULocale>) -> Result<Arc<CollationTailoring>> {
        // Java porting note: ICU4J getWithFallback/getStringWithFallback currently does not
        // work well when alias table is involved in a resource path, unless full path is specified.
        // For now, collation resources does not contain such data, so the code below should work fine.

        let root = CollationRoot::get_root()?;
        let locale_name = locale.get_name();
        if locale_name.is_empty() || locale_name == "root" {
            out_valid_locale.value = Some(ULocale::root());
            return Ok(root);
        }

        let bundle = match ICUResourceBundle::get_bundle_instance_ulocale(
            Some(ICUData::ICU_COLLATION_BASE_NAME),
            Some(locale),
            OpenType::LocaleRoot,
        ) {
            Ok(b) => UResourceBundle::Icu(b),
            Err(e) if is_missing_resource(&e) => {
                out_valid_locale.value = Some(ULocale::root());
                return Ok(root);
            }
            Err(e) => return Err(e),
        };

        let mut valid_locale = bundle.get_u_locale()?;
        // Normalize the root locale. See
        // https://unicode-org.atlassian.net/browse/ICU-10715
        let valid_locale_name = valid_locale.get_name().to_string();
        if valid_locale_name.is_empty() || valid_locale_name == "root" {
            valid_locale = ULocale::root();
        }
        out_valid_locale.value = Some(valid_locale.clone());

        // There are zero or more tailorings in the collations table.
        let collations = match bundle.get("collations") {
            Ok(c) => c,
            Err(e) if is_missing_resource(&e) => return Ok(root),
            Err(e) => return Err(e),
        };

        // Fetch the collation type from the locale ID and the default type from the data.
        let mut type_ = locale.get_keyword_value("collation");
        let mut default_type = "standard".to_string();

        let def_t = as_icu(&collations).find_string_with_fallback("default")?;
        if let Some(def_t) = def_t {
            default_type = to_rust_string(&def_t);
        }

        let mut type_ = match type_.take() {
            None => default_type.clone(),
            Some(t) if t == "default" => default_type.clone(),
            Some(t) => ascii_to_lower_case(&t),
        };

        // Load the collations/type tailoring, with type fallback.

        // Java porting note: typeFallback is used for setting U_USING_DEFAULT_WARNING in
        // ICU4C, but not used by ICU4J

        // boolean typeFallback = false;
        let mut data = Self::find_with_fallback(&collations, &type_)?;
        if data.is_none() && type_.encode_utf16().count() > 6 && type_.starts_with("search") {
            // fall back from something like "searchjl" to "search"
            // typeFallback = true;
            type_ = "search".to_string();
            data = Self::find_with_fallback(&collations, &type_)?;
        }

        if data.is_none() && type_ != default_type {
            // fall back to the default type
            // typeFallback = true;
            type_ = default_type.clone();
            data = Self::find_with_fallback(&collations, &type_)?;
        }

        if data.is_none() && type_ != "standard" {
            // fall back to the "standard" type
            // typeFallback = true;
            type_ = "standard".to_string();
            data = Self::find_with_fallback(&collations, &type_)?;
        }

        let Some(data) = data else {
            return Ok(root);
        };

        // Is this the same as the root collator? If so, then use that instead.
        let mut actual_locale = data.get_u_locale()?;
        // https://unicode-org.atlassian.net/browse/ICU-10715 ICUResourceBundle(root).getULocale() != ULocale.ROOT
        // Therefore not just if (actualLocale.equals(ULocale.ROOT) && type.equals("standard")) {
        let actual_locale_name = actual_locale.get_name().to_string();
        if actual_locale_name.is_empty() || actual_locale_name == "root" {
            actual_locale = ULocale::root();
            if type_ == "standard" {
                return Ok(root);
            }
        }

        let mut t = CollationTailoring::new(Some(&root.settings));
        t.actual_locale = Some(actual_locale.clone());

        // deserialize
        let binary = data.get("%%CollationBin")?;
        let mut in_bytes = binary.get_binary()?;
        if let Err(e) = CollationDataReader::read(Some(&root), &mut in_bytes, &mut t) {
            if e.is_io_exception() {
                return Err(ICUUncheckedIOException::with_message_and_cause(
                    format!("Failed to load collation tailoring data for locale:{actual_locale} type:{type_}"),
                    e,
                )
                .error());
            }
            return Err(e);
        }

        // Try to fetch the optional rules string.
        match data.get("Sequence") {
            Ok(r) => t.set_rules_resource(r),
            Err(e) if is_missing_resource(&e) => {}
            Err(e) => return Err(e),
        }

        // Set the collation types on the informational locales,
        // except when they match the default types (for brevity and backwards compatibility).
        // For the valid locale, suppress the default type.
        if type_ != default_type {
            out_valid_locale.value = Some(valid_locale.set_keyword_value(Some("collation"), Some(&type_))?);
        }

        // For the actual locale, suppress the default type *according to the actual locale*.
        // For example, zh has default=pinyin and contains all of the Chinese tailorings.
        // zh_Hant has default=stroke but has no other data.
        // For the valid locale "zh_Hant" we need to suppress stroke.
        // For the actual locale "zh" we need to suppress pinyin instead.
        if actual_locale != valid_locale {
            // Opening a bundle for the actual locale should always succeed.
            let actual_bundle =
                UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_COLLATION_BASE_NAME), Some(&actual_locale))?;
            let def_t = as_icu(&actual_bundle).find_string_with_fallback("collations/default")?;
            if let Some(def_t) = def_t {
                default_type = to_rust_string(&def_t);
            }
        }

        if type_ != default_type {
            let current = t.actual_locale.clone().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            t.actual_locale = Some(current.set_keyword_value(Some("collation"), Some(&type_))?);
        }

        // if (typeFallback) {
        //     ICU4C implementation sets U_USING_DEFAULT_WARNING here
        // }
        Ok(Arc::new(t))
    }
}
