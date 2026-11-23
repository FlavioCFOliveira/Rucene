// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.UnitPreferences`.

use std::collections::HashMap;

use rucene::error::Result;

use crate::jdk::math::BigDecimal;
use crate::jdk::{to_rust_string, units};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::units_data::{Constants, UnitsData};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// `measurementSystem`: the measurement systems a locale's `measure`
/// keyword may name, with the region whose preferences they use.
fn measurement_system(system: &str) -> Option<&'static str> {
    match system {
        "metric" => Some("001"),
        "ussystem" => Some("US"),
        "uksystem" => Some("GB"),
        _ => None,
    }
}

/// The unit preferences of the regions, from the `units` resource.
///
/// Equivalent to `com.ibm.icu.impl.units.UnitPreferences`.
#[derive(Debug, Clone)]
pub struct UnitPreferences {
    /// `mapToUnitPreferences`: "category++usage" to region to preferences.
    map_to_unit_preferences: HashMap<String, HashMap<String, Vec<UnitPreference>>>,
}

impl UnitPreferences {
    /// Equivalent to `UnitPreferences()`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading.
    pub fn new() -> Result<Self> {
        let resource = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "units")?;
        let mut sink = UnitPreferencesSink::new();
        resource.as_icu()?.get_all_items_with_fallback(Constants::UNIT_PREFERENCE_TABLE_NAME, &mut sink)?;
        Ok(UnitPreferences { map_to_unit_preferences: sink.get_map_to_unit_preferences() })
    }

    /// Equivalent to `formMapKey(String, String)`.
    pub fn form_map_key(category: &str, usage: &str) -> String {
        format!("{category}++{usage}")
    }

    /// Extracts all the sub-usages from a usage including the default one
    /// in the end. The usages will be in order starting with the longest
    /// matching one. Equivalent to `getAllUsages(String)`.
    fn get_all_usages(usage: &str) -> Vec<String> {
        let mut result = vec![usage.to_string()];
        let u = units(usage);
        for i in (0..u.len()).rev() {
            if u[i] == '-' as u16 {
                result.push(to_rust_string(&u[..i]));
            }
        }

        if usage != Constants::DEFAULT_USAGE {
            // Do not add default usage twice.
            result.push(Constants::DEFAULT_USAGE.to_string());
        }
        result
    }

    /// Equivalent to `getPreferencesFor(String, String, ULocale,
    /// UnitsData)`.
    ///
    /// # Errors
    ///
    /// The errors of the region lookup and of the system check.
    pub fn get_preferences_for(
        &self,
        category: &str,
        usage: &str,
        locale: &ULocale,
        data: &UnitsData,
    ) -> Result<Vec<UnitPreference>> {
        // TODO: remove this condition when all the categories are allowed.
        // WARNING: when this is removed please make sure to keep the
        // "fahrenhe" => "fahrenheit" mapping
        if category == "temperature" {
            let mut locale_unit = locale.get_keyword_value("mu");
            // The value for -u-mu- is `fahrenhe`, but CLDR and everything
            // else uses `fahrenheit`
            if locale_unit.as_deref() == Some("fahrenhe") {
                locale_unit = Some("fahrenheit".to_string());
            }
            // catch (Exception e): any failure of the lookup is a null
            // category.
            let locale_unit_category = match &locale_unit {
                None => None,
                Some(u) => MeasureUnitImpl::for_identifier(u).and_then(|m| data.get_category(&m)).ok(),
            };
            if locale_unit_category.as_deref() == Some(category) {
                return Ok(vec![UnitPreference::new(locale_unit.as_deref().unwrap_or(""), None, None)?]);
            }
        }

        let region = ULocale::get_region_for_supplemental_data(locale, true)?;

        // Check the locale system as well, e.g. `ussystem` or `metric`
        let locale_system = locale.get_keyword_value("measure");
        let is_locale_system = locale_system.as_deref().and_then(measurement_system).is_some();

        let sub_usages = Self::get_all_usages(usage);
        let mut result: Option<Vec<UnitPreference>> = None;
        for sub_usage in &sub_usages {
            result = self.get_unit_preferences(category, sub_usage, &region);

            if result.is_some() && is_locale_system {
                let locale_system = locale_system.as_deref().unwrap_or("");
                let rates = ConversionRates::new()?;
                let mut units_match_system = true;
                for unit_pref in result.as_deref().unwrap_or(&[]) {
                    let measure_unit = MeasureUnitImpl::for_identifier(unit_pref.get_unit())?;
                    let single_units = measure_unit.get_single_units().clone();
                    for single_unit in &single_units {
                        let systems = rates
                            .extract_systems(single_unit)?
                            .ok_or_else(|| crate::jdk::null_pointer("systems"))?;
                        if !systems.contains("metric_adjacent") && !systems.contains(locale_system) {
                            units_match_system = false;
                        }
                    }
                }
                if !units_match_system {
                    let new_region = measurement_system(locale_system).unwrap_or("");
                    result = self.get_unit_preferences(category, sub_usage, new_region);
                }
            }

            if result.is_some() {
                break;
            }
        }

        debug_assert!(result.is_some(), "At least the category must be exist");
        result.ok_or_else(|| crate::jdk::null_pointer("no unit preferences for the category"))
    }

    /// Equivalent to `getUnitPreferences(String, String, String)`: `None`
    /// for Java's `null`.
    fn get_unit_preferences(&self, category: &str, usage: &str, region: &str) -> Option<Vec<UnitPreference>> {
        let key = Self::form_map_key(category, usage);
        match self.map_to_unit_preferences.get(&key) {
            Some(unit_preferences_map) => {
                let result = unit_preferences_map
                    .get(region)
                    .or_else(|| unit_preferences_map.get(Constants::DEFAULT_REGION))
                    .cloned();

                debug_assert!(result.is_some());
                result
            }
            None => None,
        }
    }
}

/// Equivalent to `UnitPreferences.UnitPreference`.
#[derive(Debug, Clone)]
pub struct UnitPreference {
    /// `unit`.
    unit: String,
    /// `geq`.
    geq: BigDecimal,
    /// `skeleton`.
    skeleton: String,
}

impl UnitPreference {
    /// Equivalent to `UnitPreference(String, String, String)` (`None` for
    /// Java's `null`).
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of the `geq` parsing.
    pub fn new(unit: &str, geq: Option<&str>, skeleton: Option<&str>) -> Result<Self> {
        Ok(UnitPreference {
            unit: unit.to_string(),
            geq: match geq {
                None => BigDecimal::value_of_double(f64::from_bits(1))?, // Double.MIN_VALUE
                Some(g) => BigDecimal::from_string(&units(g))?,
            },
            skeleton: skeleton.unwrap_or("").to_string(),
        })
    }

    /// Equivalent to `getUnit()`.
    pub fn get_unit(&self) -> &str {
        &self.unit
    }

    /// Equivalent to `getGeq()`.
    pub fn get_geq(&self) -> &BigDecimal {
        &self.geq
    }

    /// Equivalent to `getSkeleton()`.
    pub fn get_skeleton(&self) -> &str {
        &self.skeleton
    }
}

/// Equivalent to `UnitPreferences.UnitPreferencesSink`.
#[derive(Debug, Default)]
pub struct UnitPreferencesSink {
    /// `mapToUnitPreferences`.
    map_to_unit_preferences: HashMap<String, HashMap<String, Vec<UnitPreference>>>,
}

impl UnitPreferencesSink {
    /// Equivalent to `UnitPreferencesSink()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `getMapToUnitPreferences()`.
    pub fn get_map_to_unit_preferences(self) -> HashMap<String, HashMap<String, Vec<UnitPreference>>> {
        self.map_to_unit_preferences
    }

    /// `insertUnitPreferences(String, String, String, UnitPreference[])`.
    fn insert_unit_preferences(&mut self, category: &str, usage: &str, region: &str, unit_preferences: Vec<UnitPreference>) {
        let key = UnitPreferences::form_map_key(category, usage);
        self.map_to_unit_preferences.entry(key).or_default().insert(region.to_string(), unit_preferences);
    }
}

impl Sink for UnitPreferencesSink {
    /// The unitPreferenceData structure (see icu4c/source/data/misc/units.txt):
    /// category { usage { region [ { unit, geq, skeleton } ... ] } }.
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        debug_assert!(Constants::UNIT_PREFERENCE_TABLE_NAME == key.as_string());

        let category_table = value.get_table()?;
        let mut i = 0;
        while category_table.get_key_and_value(i, key, value) {
            i += 1;
            debug_assert!(value.get_type() == UResourceBundle::TABLE);

            let category = key.as_string();
            let usage_table = value.get_table()?;
            let mut j = 0;
            while usage_table.get_key_and_value(j, key, value) {
                j += 1;
                debug_assert!(value.get_type() == UResourceBundle::TABLE);

                let usage = key.as_string();
                let region_table = value.get_table()?;
                let mut k = 0;
                while region_table.get_key_and_value(k, key, value) {
                    k += 1;
                    debug_assert!(value.get_type() == UResourceBundle::ARRAY);

                    let region = key.as_string();
                    let preferences_table = value.get_array()?;
                    let mut unit_preferences = Vec::new();
                    let mut l = 0;
                    while preferences_table.get_value(l, value) {
                        l += 1;
                        debug_assert!(value.get_type() == UResourceBundle::TABLE);

                        let single_pref_table = value.get_table()?;
                        // TODO collect the data
                        let mut unit: Option<String> = None;
                        let mut geq = "1".to_string();
                        let mut skeleton = String::new();
                        let mut m = 0;
                        while single_pref_table.get_key_and_value(m, key, value) {
                            m += 1;
                            debug_assert!(value.get_type() == UResourceBundle::STRING);
                            let key_string = key.as_string();
                            match key_string.as_str() {
                                "unit" => unit = Some(to_rust_string(&value.get_string()?)),
                                "geq" => geq = to_rust_string(&value.get_string()?),
                                "skeleton" => skeleton = to_rust_string(&value.get_string()?),
                                _ => debug_assert!(false, "key must be unit, geq or skeleton"),
                            }
                        }
                        debug_assert!(unit.is_some());
                        unit_preferences.push(UnitPreference::new(
                            unit.as_deref().unwrap_or(""),
                            Some(&geq),
                            Some(&skeleton),
                        )?);
                    }

                    debug_assert!(!unit_preferences.is_empty());
                    self.insert_unit_preferences(&category, &usage, &region, unit_preferences);
                }
            }
        }
        Ok(())
    }
}
