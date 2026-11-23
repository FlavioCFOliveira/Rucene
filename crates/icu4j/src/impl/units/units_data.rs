// © 2020 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.units.UnitsData`.

use std::collections::HashMap;
use std::sync::LazyLock;

use rucene::error::Result;

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::illegal_icu_argument_exception::IllegalIcuArgumentException;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::r#impl::units::conversion_rates::ConversionRates;
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::unit_preferences::{UnitPreference, UnitPreferences};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// The static fields `simpleUnits` and `simpleUnitCategories`, loaded by the
/// static initializer.
struct SimpleUnits {
    /// `simpleUnits`.
    simple_units: Vec<String>,
    /// `simpleUnitCategories`.
    simple_unit_categories: Vec<i32>,
}

/// The static initializer of `UnitsData`.
static SIMPLE_UNITS: LazyLock<SimpleUnits> = LazyLock::new(|| {
    let load = || -> Result<SimpleUnits> {
        let resource = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "units")?;
        let mut sink = SimpleUnitIdentifiersSink::default();
        resource.as_icu()?.get_all_items_with_fallback("convertUnits", &mut sink)?;
        Ok(SimpleUnits { simple_units: sink.simple_units, simple_unit_categories: sink.simple_unit_categories })
    };
    match load() {
        Ok(s) => s,
        Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
    }
});

/// The units data: conversion rates and unit preferences.
///
/// Equivalent to `com.ibm.icu.impl.units.UnitsData`.
#[derive(Debug)]
pub struct UnitsData {
    /// `conversionRates`.
    conversion_rates: ConversionRates,
    /// `unitPreferences`.
    unit_preferences: UnitPreferences,
}

impl UnitsData {
    /// Equivalent to `UnitsData()`.
    ///
    /// # Errors
    ///
    /// The errors of the data loading.
    pub fn new() -> Result<Self> {
        Ok(UnitsData { conversion_rates: ConversionRates::new()?, unit_preferences: UnitPreferences::new()? })
    }

    /// Returns a list of all simple units. Equivalent to
    /// `getSimpleUnits()`.
    pub fn get_simple_units() -> &'static [String] {
        &SIMPLE_UNITS.simple_units
    }

    /// Equivalent to `getConversionRates()`.
    pub fn get_conversion_rates(&self) -> &ConversionRates {
        &self.conversion_rates
    }

    /// Equivalent to `getUnitPreferences()`.
    pub fn get_unit_preferences(&self) -> &UnitPreferences {
        &self.unit_preferences
    }

    /// Equivalent to `getCategoryIndexOfSimpleUnit(int)`.
    ///
    /// # Panics
    ///
    /// Java's `ArrayIndexOutOfBoundsException` for an index outside the
    /// simple units.
    pub fn get_category_index_of_simple_unit(simple_unit_index: i32) -> i32 {
        let cats = &SIMPLE_UNITS.simple_unit_categories;
        if simple_unit_index < 0 || simple_unit_index as usize >= cats.len() {
            crate::jdk::array_index_out_of_bounds(simple_unit_index);
        }
        cats[simple_unit_index as usize]
    }

    /// Equivalent to `getCategory(MeasureUnitImpl)`: the category of the
    /// unit, for example "length".
    ///
    /// # Errors
    ///
    /// `IllegalIcuArgumentException("This unit does not has a category" +
    /// identifier)`, and the errors of the serialization.
    pub fn get_category(&self, measure_unit: &MeasureUnitImpl) -> Result<String> {
        let mut base_measure_unit_impl = self.get_conversion_rates().extract_compound_base_unit(measure_unit)?;
        base_measure_unit_impl.serialize()?;
        let mut identifier = base_measure_unit_impl.get_identifier();
        let base_unit_to_index = &CATEGORIES.base_unit_to_index;
        let lookup = |id: &Option<String>| id.as_ref().and_then(|i| base_unit_to_index.get(i).copied());
        let mut index = lookup(&identifier);

        // In case the base unit identifier did not match any entry.
        if index.is_none() {
            base_measure_unit_impl.take_reciprocal();
            base_measure_unit_impl.serialize()?;
            identifier = base_measure_unit_impl.get_identifier();
            index = lookup(&identifier);
        }

        // In case the reciprocal of the base unit identifier did not match
        // any entry.
        base_measure_unit_impl.take_reciprocal(); // return to original form
        let mut simplified_unit = base_measure_unit_impl.copy_and_simplify();
        if index.is_none() {
            simplified_unit.serialize()?;
            identifier = simplified_unit.get_identifier();
            index = lookup(&identifier);
        }

        // In case the simplified base unit identifier did not match any
        // entry.
        if index.is_none() {
            simplified_unit.take_reciprocal();
            simplified_unit.serialize()?;
            identifier = simplified_unit.get_identifier();
            index = lookup(&identifier);
        }

        // If there is no match at all, throw an exception.
        match index {
            Some(index) => Ok(CATEGORIES.index_to_category[index as usize].clone()),
            None => Err(IllegalIcuArgumentException::with_message(format!(
                "This unit does not has a category{}",
                measure_unit.get_identifier().as_deref().unwrap_or("null")
            ))
            .error()),
        }
    }

    /// Equivalent to `getPreferencesFor(String, String, ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the preference lookup.
    pub fn get_preferences_for(&self, category: &str, usage: &str, locale: &ULocale) -> Result<Vec<UnitPreference>> {
        self.unit_preferences.get_preferences_for(category, usage, locale, self)
    }
}

/// Equivalent to `UnitsData.SimpleUnitIdentifiersSink`.
#[derive(Debug, Default)]
pub struct SimpleUnitIdentifiersSink {
    /// `simpleUnits`.
    simple_units: Vec<String>,
    /// `simpleUnitCategories`.
    simple_unit_categories: Vec<i32>,
}

impl Sink for SimpleUnitIdentifiersSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        debug_assert!(key.as_string() == Constants::CONVERSION_UNIT_TABLE_NAME);
        debug_assert!(value.get_type() == UResourceBundle::TABLE);

        let simple_units_table = value.get_table()?;
        let mut simple_units = Vec::new();
        let mut simple_unit_categories = Vec::new();
        let mut i = 0;
        while simple_units_table.get_key_and_value(i, key, value) {
            i += 1;
            if key.as_string() == "kilogram" {
                // For parsing, we use "gram", the prefixless metric mass
                // unit. We thus ignore the SI Base Unit of Mass: it exists
                // due to being the mass conversion target unit, but not
                // needed for MeasureUnit parsing.
                continue;
            }

            // Find the base target unit for this simple unit
            let table = value.get_table()?;
            if !table.find_value(&crate::jdk::units("target"), value) {
                // TODO: is there a more idiomatic way to deal with Data
                // errors?
                debug_assert!(false, "Could not find \"target\" for simple unit: {}", key.as_string());
                continue;
            }
            let target = crate::jdk::to_rust_string(&value.get_string()?);

            simple_units.push(key.as_string());
            // Java auto-unboxes the Integer: a missing category is a
            // NullPointerException.
            let category = CATEGORIES
                .base_unit_to_index
                .get(&target)
                .copied()
                .ok_or_else(|| crate::jdk::null_pointer("unitQuantities has no entry for the target"))?;
            simple_unit_categories.push(category);
        }

        self.simple_units = simple_units;
        self.simple_unit_categories = simple_unit_categories;
        Ok(())
    }
}

/// Equivalent to `UnitsData.Constants`.
pub struct Constants;

impl Constants {
    /// Trie value offset for simple units, e.g. "gram", "nautical-mile",
    /// "fluid-ounce-imperial".
    pub const K_SIMPLE_UNIT_OFFSET: i32 = 512;

    /// Trie value offset for powers like "square-", "cubic-", "pow2-" etc.
    pub const K_POWER_PART_OFFSET: i32 = 256;

    /// Trie value offset for "per-".
    pub const K_INITIAL_COMPOUND_PART_OFFSET: i32 = 192;

    /// Trie value offset for compound parts, e.g. "-per-", "-", "-and-".
    pub const K_COMPOUND_PART_OFFSET: i32 = 128;

    /// Trie value offset for SI or binary prefixes. This is big enough to
    /// ensure we only insert positive integers into the trie.
    pub const K_PREFIX_OFFSET: i32 = 64;

    /// Tables Keys.
    pub const CONVERSION_UNIT_TABLE_NAME: &'static str = "convertUnits";
    /// `UNIT_PREFERENCE_TABLE_NAME`.
    pub const UNIT_PREFERENCE_TABLE_NAME: &'static str = "unitPreferenceData";
    /// `CATEGORY_TABLE_NAME`.
    pub const CATEGORY_TABLE_NAME: &'static str = "unitQuantities";
    /// `DEFAULT_REGION`.
    pub const DEFAULT_REGION: &'static str = "001";
    /// `DEFAULT_USAGE`.
    pub const DEFAULT_USAGE: &'static str = "default";
}

/// The static fields of `UnitsData.Categories`.
pub struct Categories {
    /// `baseUnitToIndex`: contains the map between units in their base
    /// units into their category, e.g. meter --> length.
    base_unit_to_index: HashMap<String, i32>,
    /// `indexToCategory`.
    index_to_category: Vec<String>,
}

/// The static initializer of `UnitsData.Categories`.
static CATEGORIES: LazyLock<Categories> = LazyLock::new(|| {
    let load = || -> Result<Categories> {
        let resource = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "units")?;
        let mut sink = CategoriesSink::new();
        resource.as_icu()?.get_all_items_with_fallback(Constants::CATEGORY_TABLE_NAME, &mut sink)?;
        Ok(Categories { base_unit_to_index: sink.map_from_unit_to_index, index_to_category: sink.categories })
    };
    match load() {
        Ok(c) => c,
        Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
    }
});

/// A Resource Sink that collects information from `unitQuantities` in the
/// `units` resource to provide key->value lookups from base unit to
/// category, as well as preserving ordering information for these
/// categories.
///
/// Equivalent to `UnitsData.CategoriesSink`.
#[derive(Debug, Default)]
pub struct CategoriesSink {
    /// `mapFromUnitToIndex`.
    map_from_unit_to_index: HashMap<String, i32>,
    /// `categories`.
    categories: Vec<String>,
}

impl CategoriesSink {
    /// Equivalent to `CategoriesSink()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Sink for CategoriesSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        debug_assert!(key.as_string() == Constants::CATEGORY_TABLE_NAME);
        debug_assert!(value.get_type() == UResourceBundle::ARRAY);

        let category_array = value.get_array()?;
        let mut i = 0;
        while category_array.get_value(i, value) {
            i += 1;
            debug_assert!(value.get_type() == UResourceBundle::TABLE);
            let table = value.get_table()?;
            debug_assert!(table.get_size() == 1, "expecting single-entry table, got size: {}", table.get_size());
            table.get_key_and_value(0, key, value);
            debug_assert!(value.get_type() == UResourceBundle::STRING, "expecting category string");
            self.map_from_unit_to_index.insert(key.as_string(), self.categories.len() as i32);
            self.categories.push(value.to_java_string());
        }
        Ok(())
    }
}
