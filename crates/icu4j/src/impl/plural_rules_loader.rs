// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2008-2015, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.PluralRulesLoader`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;
use rucene::util::extra::ParseException;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::to_rust_string;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{icu_data_class_loader, ICUResourceBundle};
use crate::r#impl::number::range::standard_plural_ranges::StandardPluralRanges;
use crate::text::plural_rules::{self, Factory, PluralRules, PluralType};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// `loader`: the default `PluralRulesLoader`.
pub static LOADER: LazyLock<PluralRulesLoader> = LazyLock::new(PluralRulesLoader::new);

/// The three maps `checkBuildRulesIdMaps` builds once.
#[derive(Debug, Default)]
struct RulesIdMaps {
    /// `localeIdToCardinalRulesId` (a `TreeMap`).
    locale_id_to_cardinal_rules_id: BTreeMap<String, String>,
    /// `localeIdToOrdinalRulesId` (a `TreeMap`).
    locale_id_to_ordinal_rules_id: BTreeMap<String, String>,
    /// `rulesIdToEquivalentULocale` (a `HashMap`).
    rules_id_to_equivalent_u_locale: HashMap<String, ULocale>,
}

/// Loader for plural rules data.
///
/// Equivalent to `com.ibm.icu.impl.PluralRulesLoader` (a
/// `PluralRules.Factory`). Its three lazily built maps are set once under
/// the instance lock (Java's `synchronized (this)`); the rules cache maps a
/// key to Java's possibly-`null` rules (`None`).
#[derive(Debug)]
pub struct PluralRulesLoader {
    /// `pluralRulesCache`.
    plural_rules_cache: Mutex<HashMap<String, Option<Arc<PluralRules>>>>,
    /// The three id maps (`null` until built).
    maps: Mutex<Option<Arc<RulesIdMaps>>>,
}

impl PluralRulesLoader {
    /// Access through singleton.
    fn new() -> Self {
        Self { plural_rules_cache: Mutex::new(HashMap::new()), maps: Mutex::new(None) }
    }

    /// Returns the lazily-constructed map: `getLocaleIdToRulesIdMap(PluralType)`.
    fn get_locale_id_to_rules_id_map(&self, plural_type: PluralType) -> Result<(Arc<RulesIdMaps>, PluralType)> {
        Ok((self.check_build_rules_id_maps()?, plural_type))
    }

    /// Lazily constructs the localeIdToRulesId and rulesIdToEquivalentULocale
    /// maps if necessary. These exactly reflect the contents of the
    /// locales resource in plurals.res. Equivalent to
    /// `checkBuildRulesIdMaps()`.
    fn check_build_rules_id_maps(&self) -> Result<Arc<RulesIdMaps>> {
        let have_map = self.maps.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(maps) = have_map {
            return Ok(maps);
        }
        let temp = match Self::build_maps() {
            Ok(m) => m,
            Err(e) if MissingResourceException::of(&e).is_some() => {
                // dummy so we don't try again
                RulesIdMaps::default()
            }
            Err(e) => return Err(e),
        };
        let mut guard = self.maps.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(Arc::new(temp));
        }
        Ok(guard.clone().expect("set above"))
    }

    /// The `try` body of `checkBuildRulesIdMaps`.
    fn build_maps() -> Result<RulesIdMaps> {
        let pluralb = Self::get_plural_bundle()?;
        let mut maps = RulesIdMaps::default();

        let localeb = pluralb.get("locales")?;

        // sort for convenience of getAvailableULocales
        for i in 0..localeb.get_size() {
            let b = localeb.get_at(i)?;
            let id = b.get_key().unwrap_or_default();
            let value = to_rust_string(&b.get_string()?);
            maps.locale_id_to_cardinal_rules_id.insert(id.clone(), value.clone());

            if !maps.rules_id_to_equivalent_u_locale.contains_key(&value) {
                maps.rules_id_to_equivalent_u_locale.insert(value, ULocale::new(&id)?);
            }
        }

        let localeb = pluralb.get("locales_ordinals")?;
        for i in 0..localeb.get_size() {
            let b = localeb.get_at(i)?;
            let id = b.get_key().unwrap_or_default();
            let value = to_rust_string(&b.get_string()?);
            maps.locale_id_to_ordinal_rules_id.insert(id, value);
        }
        Ok(maps)
    }

    /// Gets the rulesId from the locale, with locale fallback. If there is
    /// no rulesId, return `None`. The rulesId might be the empty string if
    /// the rule is the default rule. Equivalent to
    /// `getRulesIdForLocale(ULocale, PluralType)`.
    ///
    /// # Errors
    ///
    /// The errors of locale canonicalization and of the plural data.
    pub fn get_rules_id_for_locale(&self, locale: &ULocale, plural_type: PluralType) -> Result<Option<String>> {
        let (maps, plural_type) = self.get_locale_id_to_rules_id_map(plural_type)?;
        let id_map = match plural_type {
            PluralType::Cardinal => &maps.locale_id_to_cardinal_rules_id,
            PluralType::Ordinal => &maps.locale_id_to_ordinal_rules_id,
        };
        let mut locale_id = ULocale::canonicalize(&locale.get_base_name())?;
        let mut rules_id: Option<String>;
        loop {
            rules_id = id_map.get(&locale_id).cloned();
            if rules_id.is_some() {
                break;
            }
            let ix = match locale_id.rfind('_') {
                Some(ix) => ix,
                None => break,
            };
            locale_id = locale_id[..ix].to_string();
        }
        Ok(rules_id)
    }

    /// Gets the rule from the rulesId. If there is no rule for this rulesId,
    /// return `None`. Equivalent to `getOrCreateRulesForLocale(ULocale,
    /// PluralType)`.
    ///
    /// # Errors
    ///
    /// The errors of locale canonicalization and of the plural-ranges data
    /// (Java's `getSetForLocale` sits outside the `try`).
    pub fn get_or_create_rules_for_locale(&self, locale: &ULocale, plural_type: PluralType) -> Result<Option<Arc<PluralRules>>> {
        let rules_id = match self.get_rules_id_for_locale(locale, plural_type)? {
            Some(r) if !crate::r#impl::locale_id_parser::java_trim(&r).is_empty() => r,
            _ => return Ok(None),
        };
        let ranges_id = StandardPluralRanges::get_set_for_locale(locale)?;
        let cache_key = format!("{}/{}", rules_id, ranges_id.as_deref().unwrap_or("null"));
        // synchronize on the map.  release the lock temporarily while we
        // build the rules.
        let mut rules: Option<Arc<PluralRules>> = None;
        let cached = {
            let cache = self.plural_rules_cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.get(&cache_key).cloned()
        };
        match cached {
            Some(r) => rules = r,
            None => {
                match Self::build_rules(&rules_id, ranges_id.as_deref()) {
                    Ok(r) => rules = Some(r),
                    // TODO: log?
                    Err(e) if ParseException::of(&e).is_some() => {}
                    // TODO: log?
                    Err(e) if MissingResourceException::of(&e).is_some() => {}
                    Err(e) => return Err(e),
                }
                let mut cache = self.plural_rules_cache.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(r) = cache.get(&cache_key) {
                    // an identical PluralRules object was created in the
                    // meantime
                    rules = r.clone();
                } else {
                    cache.insert(cache_key, rules.clone());
                }
            }
        }
        Ok(rules)
    }

    /// The `try` body of `getOrCreateRulesForLocale`.
    fn build_rules(rules_id: &str, ranges_id: Option<&str>) -> Result<Arc<PluralRules>> {
        let pluralb = Self::get_plural_bundle()?;
        let rulesb = pluralb.get("rules")?;
        let setb = rulesb.get(rules_id)?;

        let mut sb = String::new();
        for i in 0..setb.get_size() {
            let b = setb.get_at(i)?;
            if i > 0 {
                sb.push_str("; ");
            }
            sb.push_str(&b.get_key().unwrap_or_else(|| "null".to_string()));
            sb.push_str(": ");
            sb.push_str(&to_rust_string(&b.get_string()?));
        }
        let ranges = StandardPluralRanges::for_set(ranges_id)?;
        PluralRules::new_internal(&sb, Some(ranges))
    }

    /// Return the plurals resource. Note MissingResourceException is
    /// unchecked, listed here for clarity. Callers should handle this
    /// exception. Equivalent to `getPluralBundle()`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException`.
    pub fn get_plural_bundle() -> Result<UResourceBundle> {
        Ok(UResourceBundle::Icu(ICUResourceBundle::get_bundle_instance(
            Some(ICUData::ICU_BASE_NAME),
            "plurals",
            icu_data_class_loader(),
            true,
        )?))
    }
}

impl Factory for PluralRulesLoader {
    /// Returns the plural rules for the locale. If we don't have plural
    /// rules for the locale, returns the default rules.
    fn for_locale(&self, locale: &ULocale, plural_type: PluralType) -> Result<Arc<PluralRules>> {
        let rules = self.get_or_create_rules_for_locale(locale, plural_type)?;
        Ok(rules.unwrap_or_else(|| plural_rules::DEFAULT.clone()))
    }

    /// Returns the locales for which we have plurals data. Utility for
    /// testing.
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>> {
        let (maps, _) = self.get_locale_id_to_rules_id_map(PluralType::Cardinal)?;
        // LinkedHashSet<ULocale>: insertion order, ULocale.equals.
        let mut locales: Vec<ULocale> = Vec::with_capacity(maps.locale_id_to_cardinal_rules_id.len());
        for key in maps.locale_id_to_cardinal_rules_id.keys() {
            let l = ULocale::create_canonical(key)?;
            if !locales.contains(&l) {
                locales.push(l);
            }
        }
        Ok(locales)
    }

    /// Returns the functionally equivalent locale.
    fn get_functional_equivalent(&self, locale: &ULocale, is_available: Option<&mut [bool]>) -> Result<ULocale> {
        if let Some(is_available) = is_available {
            if !is_available.is_empty() {
                let locale_id = ULocale::canonicalize(&locale.get_base_name())?;
                let (maps, _) = self.get_locale_id_to_rules_id_map(PluralType::Cardinal)?;
                is_available[0] = maps.locale_id_to_cardinal_rules_id.contains_key(&locale_id);
            }
        }

        let rules_id = match self.get_rules_id_for_locale(locale, PluralType::Cardinal)? {
            Some(r) if !crate::r#impl::locale_id_parser::java_trim(&r).is_empty() => r,
            // can't happen
            _ => return Ok(ULocale::root()),
        };

        let maps = self.check_build_rules_id_maps()?;
        match maps.rules_id_to_equivalent_u_locale.get(&rules_id) {
            // can't happen
            None => Ok(ULocale::root()),
            Some(result) => Ok(result.clone()),
        }
    }

    fn has_override(&self, _locale: &ULocale) -> bool {
        false
    }
}
