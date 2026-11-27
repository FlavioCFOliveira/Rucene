// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.LongNameHandler`.

use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::jdk::text::FieldRef;
use crate::jdk::{index_of, is_space_char, to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::number::number_formatter::UnitWidth;
use crate::r#impl::currency_data;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::long_name_multiplexer::ParentlessMicroPropsGenerator;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::modifier::{Modifier, Parameters, Signum};
use crate::r#impl::number::modifier_store::ModifierStore;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::number::simple_modifier::SimpleModifier;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::simple_formatter_impl::{Arg, SimpleFormatterImpl};
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::u_resource::{Key, Sink, Table, Value};
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::text::number_format::field as nf_field;
use crate::text::plural_rules::PluralRules;
use crate::util::currency::{self, Currency};
use crate::util::icu_exception::ICUException;
use crate::util::measure_unit::{self, Complexity, MeasurePrefix, MeasureUnit};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// `DNAM_INDEX`: the index of the unit display name.
const DNAM_INDEX: usize = StandardPlural::COUNT as usize;
/// `PER_INDEX`: the index of the "per" form.
const PER_INDEX: usize = StandardPlural::COUNT as usize + 1;
/// `GENDER_INDEX`: the index of the gender.
const GENDER_INDEX: usize = StandardPlural::COUNT as usize + 2;
/// `CONSTANT_DENOMINATOR_INDEX`.
const CONSTANT_DENOMINATOR_INDEX: usize = StandardPlural::COUNT as usize + 3;
/// `ARRAY_LENGTH`.
pub(crate) const ARRAY_LENGTH: usize = StandardPlural::COUNT as usize + 4;

/// A `String[ARRAY_LENGTH]` of unit data (`None` for Java's `null`).
pub(crate) type UnitData = Vec<Option<Vec<u16>>>;

/// `new String[ARRAY_LENGTH]`.
pub(crate) fn new_unit_data() -> UnitData {
    vec![None; ARRAY_LENGTH]
}

/// `getIndex(String)`: returns the plural index for the given string, or
/// one of the extra indices.
fn get_index(plural_keyword: &str) -> Result<usize> {
    // pluralKeyword can also be "dnam", "per" or "gender"
    match plural_keyword {
        "dnam" => Ok(DNAM_INDEX),
        "per" => Ok(PER_INDEX),
        "gender" => Ok(GENDER_INDEX),
        _ => Ok(StandardPlural::from_string(plural_keyword)?.ordinal() as usize),
    }
}

/// Gets the plural form, falling back to "other". Equivalent to
/// `getWithPlural(String[], StandardPlural)` (package-private).
///
/// # Errors
///
/// `ICUException("Could not find data in 'other' plural variant")`.
pub(crate) fn get_with_plural(strings: &UnitData, plural: StandardPlural) -> Result<Vec<u16>> {
    let result = strings[plural.ordinal() as usize]
        .as_ref()
        .or(strings[StandardPlural::Other.ordinal() as usize].as_ref());
    match result {
        Some(r) => Ok(r.clone()),
        // There should always be data in the "other" plural variant.
        None => {
            Err(ICUException::with_message("Could not find data in 'other' plural variant").error())
        }
    }
}

/// Equivalent to `LongNameHandler.PlaceholderPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaceholderPosition {
    None,
    Beginning,
    Middle,
    End,
}

/// Equivalent to `LongNameHandler.ExtractCorePatternResult`.
struct ExtractCorePatternResult {
    /// `coreUnit`.
    core_unit: Vec<u16>,
    /// `placeholderPosition`.
    placeholder_position: PlaceholderPosition,
    /// `joinerChar`.
    joiner_char: u16,
}

/// Returns three outputs extracted from pattern. Equivalent to
/// `extractCorePattern(String)`.
fn extract_core_pattern(pattern: &[u16]) -> ExtractCorePatternResult {
    let placeholder = units("{0}");
    let len = pattern.len();
    if pattern.starts_with(&placeholder) {
        if len > 3 && is_space_char(pattern[3]) {
            ExtractCorePatternResult {
                core_unit: pattern[4..].to_vec(),
                placeholder_position: PlaceholderPosition::Beginning,
                joiner_char: pattern[3],
            }
        } else {
            ExtractCorePatternResult {
                core_unit: pattern[3..].to_vec(),
                placeholder_position: PlaceholderPosition::Beginning,
                joiner_char: 0,
            }
        }
    } else if pattern.ends_with(&placeholder) {
        // Java indexes len - 4, which throws for the pattern "{0}" only
        // reached through startsWith above.
        if is_space_char(pattern[len - 4]) {
            ExtractCorePatternResult {
                core_unit: pattern[..len - 4].to_vec(),
                placeholder_position: PlaceholderPosition::End,
                joiner_char: pattern[len - 4],
            }
        } else {
            ExtractCorePatternResult {
                core_unit: pattern[..len - 3].to_vec(),
                placeholder_position: PlaceholderPosition::End,
                joiner_char: 0,
            }
        }
    } else if index_of(pattern, &placeholder, 1) == -1 {
        ExtractCorePatternResult {
            core_unit: pattern.to_vec(),
            placeholder_position: PlaceholderPosition::None,
            joiner_char: 0,
        }
    } else {
        ExtractCorePatternResult {
            core_unit: pattern.to_vec(),
            placeholder_position: PlaceholderPosition::Middle,
            joiner_char: 0,
        }
    }
}

/// The `ICUResourceBundle` of a base name and locale.
fn bundle(base_name: &str, locale: &ULocale) -> Result<Arc<ICUResourceBundle>> {
    Ok(
        UResourceBundle::get_bundle_instance_for(Some(base_name), Some(locale))?
            .as_icu()?
            .clone(),
    )
}

/// `getGenderForBuiltin(ULocale, MeasureUnit)`: loads and returns the
/// gender of the specified builtin unit.
fn get_gender_for_builtin(
    locale: &ULocale,
    builtin_unit: Option<&MeasureUnit>,
) -> Result<Vec<u16>> {
    let builtin_unit = builtin_unit.ok_or_else(|| crate::jdk::null_pointer("builtinUnit"))?;
    let units_bundle = bundle(ICUData::ICU_UNIT_BASE_NAME, locale)?;

    let mut key = String::new();
    key.push_str("units/");
    key.push_str(builtin_unit.get_type().unwrap_or("null"));
    key.push('/');

    // Map duration-year-person, duration-week-person, etc. to
    // duration-year, duration-week, ...
    // TODO(ICU-20400): Get duration-*-person data properly with
    // aliases.
    match builtin_unit.get_subtype() {
        Some(sub) if sub.ends_with("-person") => key.push_str(&sub[..sub.len() - 7]),
        sub => key.push_str(sub.unwrap_or("null")),
    }
    key.push_str("/gender");

    match units_bundle.get_with_fallback(&key) {
        Ok(b) => UResourceBundle::Icu(b).get_string(),
        Err(e) if is_missing_resource(&e) => {
            // TODO(icu-units#28): "$unitRes/gender" does not exist. Do we
            // want to check whether the parent "$unitRes" exists? Then we
            // could return U_MISSING_RESOURCE_ERROR for incorrect usage
            // (e.g. builtinUnit not found).
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

/// Sink for patterns for inflected units. Equivalent to the private class
/// `LongNameHandler.InflectedPluralSink`.
struct InflectedPluralSink<'a> {
    /// `gender`.
    gender: Option<Vec<u16>>,
    /// `caseVariant`.
    case_variant: Option<String>,
    /// `outArray`.
    out_array: &'a mut UnitData,
}

impl<'a> InflectedPluralSink<'a> {
    fn new(
        gender: Option<Vec<u16>>,
        case_variant: Option<String>,
        out_array: &'a mut UnitData,
    ) -> Self {
        for slot in out_array.iter_mut().take(ARRAY_LENGTH) {
            *slot = None;
        }
        InflectedPluralSink {
            gender,
            case_variant,
            out_array,
        }
    }

    /// Tries to load data for the configured gender from `genderTable`.
    /// Returns true if found, returning the data in `value`. The returned
    /// data will be for the configured gender if found, falling back to
    /// "neuter" and no-gender if not. The caseVariant property is used to
    /// look up the appropriate case.
    fn load_for_plural_form(
        &self,
        gender_table: &dyn Table,
        value: &mut dyn Value,
    ) -> Result<bool> {
        if let Some(gender) = self.gender.as_ref().filter(|g| !g.is_empty()) {
            if self.load_for_gender(gender_table, gender, value)? {
                return Ok(true);
            }
            // Java compares with the literal by identity (`gender !=
            // "neuter"`); a value equal to "neuter" was looked up above
            // already, so comparing values gives the same lookups.
            if *gender != units("neuter")
                && self.load_for_gender(gender_table, &units("neuter"), value)?
            {
                return Ok(true);
            }
        }
        self.load_for_gender(gender_table, &units("_"), value)
    }

    /// Tries to load data for the given gender from `genderTable`.
    /// Returns true if found, returning the data in `value`.
    fn load_for_gender(
        &self,
        gender_table: &dyn Table,
        gender_val: &[u16],
        value: &mut dyn Value,
    ) -> Result<bool> {
        if !gender_table.find_value(gender_val, value) {
            return Ok(false);
        }
        let case_table = value.get_table()?;
        if let Some(case_variant) = self.case_variant.as_ref().filter(|c| !c.is_empty()) {
            if Self::load_for_case(case_table.as_ref(), &units(case_variant), value) {
                return Ok(true);
            }
            // As above: identity in Java, an equal value repeats the
            // lookup above.
            if case_variant != "nominative"
                && Self::load_for_case(case_table.as_ref(), &units("nominative"), value)
            {
                return Ok(true);
            }
        }
        Ok(Self::load_for_case(case_table.as_ref(), &units("_"), value))
    }

    /// Tries to load data for the given case from `caseTable`. Returns true
    /// if found, returning the data in `value`.
    fn load_for_case(case_table: &dyn Table, case_value: &[u16], value: &mut dyn Value) -> bool {
        case_table.find_value(case_value, value)
    }
}

impl Sink for InflectedPluralSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let plurals_table = value.get_table()?;
        let mut i = 0;
        while plurals_table.get_key_and_value(i, key, value) {
            i += 1;
            let key_string = key.as_string();
            let plural_index = get_index(&key_string)?;
            if self.out_array[plural_index].is_some() {
                // We already have a pattern
                continue;
            }
            let gender_table = value.get_table()?;
            if self.load_for_plural_form(gender_table.as_ref(), value)? {
                self.out_array[plural_index] = Some(value.get_string()?);
            }
        }
        Ok(())
    }
}

/// Fills `out_array` with the inflected patterns of a unit (part).
/// Equivalent to `getInflectedMeasureData(String, ULocale, UnitWidth,
/// String, String, String[])` (package-private).
///
/// # Errors
///
/// `MissingResourceException` when the key is found nowhere.
pub(crate) fn get_inflected_measure_data(
    sub_key: &str,
    locale: &ULocale,
    width: UnitWidth,
    gender: Option<Vec<u16>>,
    case_variant: Option<String>,
    out_array: &mut UnitData,
) -> Result<()> {
    let mut sink = InflectedPluralSink::new(gender, case_variant, out_array);
    let units_bundle = bundle(ICUData::ICU_UNIT_BASE_NAME, locale)?;

    let mut key = String::from("units");
    if width == UnitWidth::Narrow {
        key.push_str("Narrow");
    } else if width == UnitWidth::Short {
        key.push_str("Short");
    }
    key.push('/');
    key.push_str(sub_key);

    match units_bundle.get_all_items_with_fallback(&key, &mut sink) {
        Ok(()) => {
            if width == UnitWidth::Short {
                return Ok(());
            }
        }
        Err(e) if is_missing_resource(&e) => {
            // Continue: fall back to short
        }
        Err(e) => return Err(e),
    }

    // NOTE: the Java code repeats the same key (the C++ code falls back to
    // "unitsShort").
    units_bundle.get_all_items_with_fallback(&key, &mut sink)
}

/// Equivalent to the private class `LongNameHandler.PluralTableSink`.
struct PluralTableSink<'a> {
    /// `outArray`.
    out_array: &'a mut UnitData,
}

impl Sink for PluralTableSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let plurals_table = value.get_table()?;
        let mut i = 0;
        while plurals_table.get_key_and_value(i, key, value) {
            i += 1;
            let key_string = key.as_string();

            if key_string == "case" {
                continue;
            }

            let index = get_index(&key_string)?;

            if self.out_array[index].is_some() {
                continue;
            }

            let format_string = value.get_string()?;
            self.out_array[index] = Some(format_string);
        }
        Ok(())
    }
}

/// Equivalent to the private class `LongNameHandler.AliasSink`.
#[derive(Default)]
struct AliasSink {
    /// `replacement`.
    replacement: Option<String>,
}

impl Sink for AliasSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let alias_table = value.get_table()?;
        let mut i = 0;
        while alias_table.get_key_and_value(i, key, value) {
            i += 1;
            let key_string = key.as_string();
            if key_string == "replacement" {
                self.replacement = Some(value.to_java_string());
            }
        }
        Ok(())
    }
}

/// Gets the data of a unit, populating `out_array` (which must be
/// `ARRAY_LENGTH` long). Equivalent to `getMeasureData(ULocale,
/// MeasureUnit, UnitWidth, String, String[])` (package-private).
///
/// # Errors
///
/// `IllegalArgumentException("No data for unit <unit>, width <width>")`
/// when no data is found, and the errors of the data loading.
pub(crate) fn get_measure_data(
    locale: &ULocale,
    unit: &MeasureUnit,
    width: UnitWidth,
    unit_display_case: Option<&str>,
    out_array: &mut UnitData,
) -> Result<()> {
    let resource = bundle(ICUData::ICU_UNIT_BASE_NAME, locale)?;

    let mut sub_key = String::from("/");
    sub_key.push_str(unit.get_type().unwrap_or("null"));
    sub_key.push('/');

    // If the unit is an alias, replace it with the replacement.
    let mut unit_sub_type: Option<String> = unit.get_subtype().map(str::to_string);
    let metadata_resource =
        UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "metadata")?;
    let mut alias_sink = AliasSink::default();
    metadata_resource
        .as_icu()?
        .get_all_items_with_fallback_no_fail(
            &format!("alias/unit/{}", unit_sub_type.as_deref().unwrap_or("null")),
            &mut alias_sink,
        )?;
    if alias_sink.replacement.is_some() {
        unit_sub_type = alias_sink.replacement.clone();
    }

    // Map duration-year-person, duration-week-person, etc. to
    // duration-year, duration-week, ...
    // TODO(ICU-20400): Get duration-*-person data properly with aliases.
    match unit_sub_type.as_deref() {
        Some(sub) if sub.ends_with("-person") => sub_key.push_str(&sub[..sub.len() - 7]),
        sub => sub_key.push_str(sub.unwrap_or("null")),
    }

    if width != UnitWidth::FullName {
        let gender_key = format!("units{sub_key}/gender");
        match resource.get_with_fallback(&gender_key) {
            Ok(b) => out_array[GENDER_INDEX] = Some(UResourceBundle::Icu(b).get_string()?),
            Err(e) if is_missing_resource(&e) => {
                // continue
            }
            Err(e) => return Err(e),
        }
    }

    let mut key = String::from("units");
    if width == UnitWidth::Narrow {
        key.push_str("Narrow");
    } else if width == UnitWidth::Short {
        key.push_str("Short");
    }
    key.push_str(&sub_key);

    // Grab desired case first, if available. Then grab nominative case to
    // fill in the gaps.
    if width == UnitWidth::FullName {
        if let Some(case) = unit_display_case.filter(|c| !c.is_empty()) {
            let case_key = format!("{key}/case/{case}");
            match resource
                .get_all_items_with_fallback(&case_key, &mut PluralTableSink { out_array })
            {
                Ok(()) => {
                    // TODO(icu-units#138): our fallback logic is not
                    // spec-compliant: we might have found "other" data in
                    // the case table without finding data for the other
                    // plural forms.
                }
                Err(e) if is_missing_resource(&e) => {
                    // continue.
                }
                Err(e) => return Err(e),
            }
        }
    }

    match resource.get_all_items_with_fallback(&key, &mut PluralTableSink { out_array }) {
        Ok(()) => Ok(()),
        Err(e) if is_missing_resource(&e) => Err(LuceneError::illegal_argument_with_cause(
            format!("No data for unit {}, width {}", unit, width),
            Arc::new(e),
        )),
        Err(e) => Err(e),
    }
}

/// `getCurrencyLongNameData(ULocale, Currency, String[])`.
fn get_currency_long_name_data(
    locale: &ULocale,
    currency: &Currency,
    out_array: &mut UnitData,
) -> Result<()> {
    // In ICU4J, this method gets a CurrencyData from CurrencyData.provider.
    // TODO(ICU4J): Implement this without going through CurrencyDisplayNames?
    let info = currency_data::provider()
        .get_instance(Some(locale), true)?
        .ok_or_else(|| crate::jdk::null_pointer("currency display info"))?;
    let mut data = (*info
        .get_unit_patterns()?
        .ok_or_else(|| crate::jdk::null_pointer("unit patterns"))?)
    .clone();
    // Entries in the data map are filled in with any CurrencyUnitPatterns
    // data for locale, or if there is no CurrencyUnitPatterns data for
    // locale since the patterns all inherited from the "other" pattern in
    // root (which is true for many locales in CLDR 46), then only the
    // "other" entry has a currency pattern. So now what we do is: For all
    // valid plural keywords for the locale, if the corresponding outArray[]
    // entry is bogus/empty, fill it in from the outArray[] entry for
    // "other".
    let prules = PluralRules::for_locale(locale)?;
    let keywords = prules.get_keywords();
    if let Some(other_pattern) = data.get("other").cloned() {
        for keyword in keywords {
            if keyword != "other" && !data.contains_key(keyword) {
                data.insert(keyword.clone(), other_pattern.clone());
            }
        }
    }
    for (plural_keyword, simple_format) in data.iter() {
        let index = get_index(plural_keyword)?;
        let long_name = currency
            .get_name_plural(locale, currency::PLURAL_LONG_NAME, plural_keyword, None)?
            .ok_or_else(|| crate::jdk::null_pointer("currency long name"))?;
        // TODO(ICU4J): Get a plural keyword based on long name (and
        // reject {1}?)
        let simple_format = replace_all(&units(simple_format), &units("{1}"), &units(&long_name));
        out_array[index] = Some(simple_format);
    }
    Ok(())
}

/// `String.replace(CharSequence, CharSequence)`.
fn replace_all(s: &[u16], target: &[u16], replacement: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if !target.is_empty() && s[i..].starts_with(target) {
            out.extend_from_slice(replacement);
            i += target.len();
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// `getCompoundValue(String, ULocale, UnitWidth)`.
fn get_compound_value(compound_key: &str, locale: &ULocale, width: UnitWidth) -> Result<Vec<u16>> {
    let resource = bundle(ICUData::ICU_UNIT_BASE_NAME, locale)?;

    let mut key = String::from("units");
    if width == UnitWidth::Narrow {
        key.push_str("Narrow");
    } else if width == UnitWidth::Short {
        key.push_str("Short");
    }
    key.push_str("/compound/");
    key.push_str(compound_key);

    match resource.get_string_with_fallback(&key) {
        Ok(s) => return Ok(s),
        Err(e) if is_missing_resource(&e) => {
            if width == UnitWidth::Short {
                return Ok(Vec::new());
            }
        }
        Err(e) => return Err(e),
    }

    // NOTE: the Java code repeats the same key.
    match resource.get_string_with_fallback(&key) {
        Ok(s) => Ok(s),
        Err(e) if is_missing_resource(&e) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Loads and applies deriveComponent rules from CLDR's
/// grammaticalFeatures.xml. Equivalent to the private class
/// `LongNameHandler.DerivedComponents`.
struct DerivedComponents {
    /// `value0` (`None` for Java's `null`).
    value0: Option<String>,
    /// `value1` (`None` for Java's `null`).
    value1: Option<String>,
}

impl DerivedComponents {
    /// Constructor. Equivalent to `DerivedComponents(ULocale, String,
    /// String)`.
    fn new(locale: &ULocale, feature: &str, structure: &str) -> Result<Self> {
        let mut result = DerivedComponents {
            value0: Some(String::new()),
            value1: Some(String::new()),
        };
        let load = |result: &mut DerivedComponents| -> Result<()> {
            let derivations_bundle = UResourceBundle::get_bundle_instance(
                Some(ICUData::ICU_BASE_NAME),
                "grammaticalFeatures",
            )?;
            let derivations_bundle = derivations_bundle.get("grammaticalData")?;
            let derivations_bundle = derivations_bundle.get("derivations")?;

            let stack_bundle = match derivations_bundle.get(&locale.get_language()) {
                Ok(b) => b,
                // TODO(CLDR-14582): check that the true parent locale is
                // "root" after the language part of locale.
                Err(e) if is_missing_resource(&e) => derivations_bundle.get("root")?,
                Err(e) => return Err(e),
            };

            let stack_bundle = stack_bundle.get("component")?;
            let stack_bundle = stack_bundle.get(feature)?;
            let stack_bundle = stack_bundle.get(structure)?;
            let value = to_rust_string(&stack_bundle.get_string_at(0)?);
            result.value0 = if value == "compound" {
                None
            } else {
                Some(value)
            };
            let value = to_rust_string(&stack_bundle.get_string_at(1)?);
            result.value1 = if value == "compound" {
                None
            } else {
                Some(value)
            };
            Ok(())
        };
        match load(&mut result) {
            Ok(()) => {}
            Err(e) if is_missing_resource(&e) => {
                // Fall back to uninflected.
            }
            Err(e) => return Err(e),
        }
        Ok(result)
    }

    /// `value0(String)`.
    fn value0(&self, compound_value: Option<&str>) -> Option<String> {
        self.value0
            .clone()
            .or_else(|| compound_value.map(str::to_string))
    }

    /// `value1(String)`.
    fn value1(&self, compound_value: Option<&str>) -> Option<String> {
        self.value1
            .clone()
            .or_else(|| compound_value.map(str::to_string))
    }
}

/// `getDeriveCompoundRule(ULocale, String, String)`.
fn get_derive_compound_rule(locale: &ULocale, feature: &str, structure: &str) -> Result<Vec<u16>> {
    let derivations_bundle =
        UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BASE_NAME), "grammaticalFeatures")?;

    let derivations_bundle = derivations_bundle.get("grammaticalData")?;
    let derivations_bundle = derivations_bundle.get("derivations")?;

    let stack_bundle = match derivations_bundle.get(&locale.get_language()) {
        Ok(b) => b,
        // TODO(CLDR-14582): check that the true parent locale is "root"
        // after the language part of locale.
        Err(e) if is_missing_resource(&e) => derivations_bundle.get("root")?,
        Err(e) => return Err(e),
    };

    let stack_bundle = stack_bundle.get("compound")?;
    let stack_bundle = stack_bundle.get(feature)?;

    stack_bundle.get_string_by_key(structure)
}

/// `getDerivedGender(ULocale, String, String[], String[])`: returns the
/// gender string for structures following these rules.
fn get_derived_gender(
    locale: &ULocale,
    structure: &str,
    data0: &UnitData,
    data1: Option<&UnitData>,
) -> Result<Option<Vec<u16>>> {
    let val = get_derive_compound_rule(locale, "gender", structure)?;
    if val.len() == 1 {
        match val[0] {
            c if c == '0' as u16 => return Ok(data0[GENDER_INDEX].clone()),
            c if c == '1' as u16 => {
                return match data1 {
                    None => Ok(None),
                    Some(d1) => Ok(d1[GENDER_INDEX].clone()),
                };
            }
            _ => {}
        }
    }
    Ok(Some(val))
}

/// `calculateGenderForUnit(ULocale, MeasureUnit)`.
fn calculate_gender_for_unit(locale: &ULocale, unit: &MeasureUnit) -> Result<Vec<u16>> {
    let mui = unit.get_copy_of_measure_unit_impl()?;
    let single_units = mui.get_single_units();
    let mut single_unit_index = 0usize;
    if mui.get_complexity() == Complexity::Compound {
        let mut start_slice = 0i32;
        let mut end_slice = single_units.len() as i32 - 1;
        debug_assert!(
            end_slice > 0,
            "COMPOUND units have more than one single unit"
        );
        if single_units[end_slice as usize].get_dimensionality() < 0 {
            // We have a -per- construct
            let per_rule = get_derive_compound_rule(locale, "gender", "per")?;
            if per_rule.len() != 1 {
                // Fixed gender for -per- units
                return Ok(per_rule);
            }
            if per_rule[0] == '1' as u16 {
                // Find the start of the denominator. We already know there
                // is one.
                while single_units[start_slice as usize].get_dimensionality() >= 0 {
                    start_slice += 1;
                }
            } else {
                // Find the end of the numerator
                while end_slice >= 0 && single_units[end_slice as usize].get_dimensionality() < 0 {
                    end_slice -= 1;
                }
                if end_slice < 0 {
                    // We have only a denominator, e.g. "per-second".
                    // TODO(icu-units#28): find out what gender to use in
                    // the absence of a first value - mentioned in CLDR-14253.
                    return Ok(Vec::new());
                }
            }
        }
        if end_slice > start_slice {
            // We have a -times- construct
            let times_rule = get_derive_compound_rule(locale, "gender", "times")?;
            if times_rule.len() != 1 {
                // Fixed gender for -times- units
                return Ok(times_rule);
            }
            if times_rule[0] == '0' as u16 {
                end_slice = start_slice;
            } else {
                // We assume timesRule[0] == u'1'
                start_slice = end_slice;
            }
        }
        debug_assert!(start_slice == end_slice);
        single_unit_index = start_slice as usize;
    } else if mui.get_complexity() == Complexity::Mixed {
        return Err(ICUException::with_message(
            "calculateGenderForUnit does not support MIXED units",
        )
        .error());
    } else {
        debug_assert!(mui.get_complexity() == Complexity::Single);
        debug_assert!(single_units.len() == 1);
    }

    // Now we know which singleUnit's gender we want
    let single_unit = &single_units[single_unit_index];
    // Check for any power-prefix gender override:
    if single_unit.get_dimensionality().wrapping_abs() != 1 {
        let power_rule = get_derive_compound_rule(locale, "gender", "power")?;
        if power_rule.len() != 1 {
            // Fixed gender for -powN- units
            return Ok(power_rule);
        }
        // Otherwise we use the gender of the simple unit (no prefix)
    }
    // Java tests the dimensionality here too (not the prefix), as the
    // ICU 77.1 source does.
    if single_unit.get_dimensionality().wrapping_abs() != 1 {
        let prefix_rule = get_derive_compound_rule(locale, "gender", "prefix")?;
        if prefix_rule.len() != 1 {
            // Fixed gender for -powN- units
            return Ok(prefix_rule);
        }
        // Otherwise we use the gender of the simple unit (no prefix)
    }
    get_gender_for_builtin(
        locale,
        MeasureUnit::for_identifier(Some(single_unit.get_simple_unit_id()))?.as_ref(),
    )
}

/// `maybeCalculateGender(ULocale, MeasureUnit, String[])`.
fn maybe_calculate_gender(
    locale: &ULocale,
    unit: &MeasureUnit,
    out_array: &mut UnitData,
) -> Result<()> {
    if out_array[GENDER_INDEX].is_none() {
        let meter_gender = get_gender_for_builtin(locale, Some(&measure_unit::METER))?;
        if meter_gender.is_empty() {
            // No gender for meter: assume ungendered language
            return Ok(());
        }
        // We have a gendered language, but are lacking gender for unit:
        // calculate it.
        out_array[GENDER_INDEX] = Some(calculate_gender_for_unit(locale, unit)?);
    }
    Ok(())
}

/// A `ModifierStore` that forwards to a handler through a weak reference.
///
/// **Divergence.** Java's `LongNameHandler` stores modifiers whose
/// `Parameters.obj` is the handler itself (`LongNameHandler.java:1257,
/// 1288`), a reference cycle the garbage collector reclaims. With `Arc`
/// that cycle would never be freed, so `Parameters.obj` is this proxy,
/// which answers `getModifier` from the handler while it is alive (the
/// only time a modifier of the handler is compared) and `None` afterwards.
#[derive(Debug)]
struct HandlerModifierStore(Weak<LongNameHandler>);

impl ModifierStore for HandlerModifierStore {
    fn get_modifier(&self, signum: Signum, plural: StandardPlural) -> Option<Arc<dyn Modifier>> {
        self.0
            .upgrade()
            .and_then(|h| h.get_modifier(signum, plural))
    }
}

/// The generator of the long names of units and currencies (for example
/// "3 meters", "3 US dollars").
///
/// Equivalent to `com.ibm.icu.impl.number.LongNameHandler` (a
/// `MicroPropsGenerator`, `ModifierStore` and
/// `LongNameMultiplexer.ParentlessMicroPropsGenerator`). Handlers are
/// shared (`Arc`): their modifiers refer back to them.
#[derive(Debug)]
pub struct LongNameHandler {
    /// `modifiers`: one per plural form, by ordinal.
    modifiers: Vec<Option<Arc<dyn Modifier>>>,
    /// `rules`.
    rules: Arc<PluralRules>,
    /// `parent` (`None` for Java's `null`).
    parent: Option<Arc<dyn MicroPropsGenerator>>,
    /// Grammatical gender of the formatted result. Not applicable to
    /// currency long names (`gender`).
    gender: String,
}

/// The compiled patterns of `simpleFormatsToModifiers` or
/// `multiSimpleFormatsToModifiers`, one per plural form, and their field.
struct CompiledModifiers {
    compiled: Vec<Vec<u16>>,
    field: FieldRef,
}

impl LongNameHandler {
    /// `LongNameHandler(Map, PluralRules, MicroPropsGenerator)` followed by
    /// the population of the modifiers and the gender.
    fn build(
        compiled: CompiledModifiers,
        rules: &Arc<PluralRules>,
        parent: Option<Arc<dyn MicroPropsGenerator>>,
        gender: String,
    ) -> Arc<LongNameHandler> {
        Arc::new_cyclic(|weak: &Weak<LongNameHandler>| {
            let store: Arc<dyn ModifierStore> = Arc::new(HandlerModifierStore(weak.clone()));
            let mut modifiers: Vec<Option<Arc<dyn Modifier>>> =
                vec![None; StandardPlural::COUNT as usize];
            for (plural, compiled_pattern) in
                StandardPlural::VALUES.iter().zip(compiled.compiled.iter())
            {
                let parameters = Parameters {
                    obj: Some(store.clone()),
                    signum: None,
                    plural: Some(*plural),
                };
                modifiers[plural.ordinal() as usize] =
                    Some(Arc::new(SimpleModifier::with_parameters(
                        compiled_pattern,
                        Some(compiled.field),
                        false,
                        Some(parameters),
                    )));
            }
            LongNameHandler {
                modifiers,
                rules: rules.clone(),
                parent,
                gender,
            }
        })
    }

    /// Equivalent to `getUnitDisplayName(ULocale, MeasureUnit, UnitWidth)`.
    ///
    /// # Errors
    ///
    /// The errors of [`get_measure_data`].
    pub fn get_unit_display_name(
        locale: &ULocale,
        unit: &MeasureUnit,
        width: UnitWidth,
    ) -> Result<Option<Vec<u16>>> {
        let mut measure_data = new_unit_data();
        get_measure_data(locale, unit, width, Some(""), &mut measure_data)?;
        Ok(measure_data[DNAM_INDEX].clone())
    }

    /// Equivalent to `forCurrencyLongNames(ULocale, Currency, PluralRules,
    /// MicroPropsGenerator)`.
    ///
    /// # Errors
    ///
    /// The errors of the currency data and of the pattern compilation.
    pub fn for_currency_long_names(
        locale: &ULocale,
        currency: &Currency,
        rules: &Arc<PluralRules>,
        parent: Option<Arc<dyn MicroPropsGenerator>>,
    ) -> Result<Arc<LongNameHandler>> {
        let mut simple_formats = new_unit_data();
        get_currency_long_name_data(locale, currency, &mut simple_formats)?;
        // TODO(ICU4J): Reduce the number of object creations here?
        let compiled = Self::simple_formats_to_modifiers(&simple_formats, &nf_field::CURRENCY)?;
        // `gender` not set: currency long names have no gender.
        Ok(Self::build(compiled, rules, parent, String::new()))
    }

    /// Construct a localized LongNameHandler for the specified MeasureUnit.
    /// Equivalent to `forMeasureUnit(ULocale, MeasureUnit, UnitWidth,
    /// String, PluralRules, MicroPropsGenerator)`.
    ///
    /// Mixed units are not supported, use MixedUnitLongNameHandler.
    ///
    /// # Errors
    ///
    /// The errors of the unit data and of the pattern compilation.
    pub fn for_measure_unit(
        locale: &ULocale,
        unit: &MeasureUnit,
        width: UnitWidth,
        unit_display_case: Option<&str>,
        rules: &Arc<PluralRules>,
        parent: Option<Arc<dyn MicroPropsGenerator>>,
    ) -> Result<Arc<LongNameHandler>> {
        // From https://unicode-org.github.io/cldr/ldml/tr35-general.html#compound-units -
        // Note that there is no unit identifier for the 'per' form.
        if unit.get_type().is_some() {
            let mut simple_formats = new_unit_data();
            get_measure_data(locale, unit, width, unit_display_case, &mut simple_formats)?;
            maybe_calculate_gender(locale, unit, &mut simple_formats)?;
            // TODO(ICU4J): Reduce the number of object creations here?
            let compiled =
                Self::simple_formats_to_modifiers(&simple_formats, &nf_field::MEASURE_UNIT)?;
            let gender = match &simple_formats[GENDER_INDEX] {
                Some(g) => to_rust_string(g),
                None => String::new(),
            };
            Ok(Self::build(compiled, rules, parent, gender))
        } else {
            debug_assert!(
                unit.get_complexity()? != Complexity::Mixed,
                "Mixed units not supported by LongNameHandler: use MixedUnitLongNameHandler"
            );
            Self::for_arbitrary_unit(locale, unit, width, unit_display_case, rules, parent)
        }
    }

    /// `forArbitraryUnit(ULocale, MeasureUnit, UnitWidth, String,
    /// PluralRules, MicroPropsGenerator)`.
    fn for_arbitrary_unit(
        loc: &ULocale,
        unit: &MeasureUnit,
        width: UnitWidth,
        unit_display_case: Option<&str>,
        rules: &Arc<PluralRules>,
        parent: Option<Arc<dyn MicroPropsGenerator>>,
    ) -> Result<Arc<LongNameHandler>> {
        // Numbered list items are from the algorithms at
        // https://unicode.org/reports/tr35/tr35-general.html#compound-units:
        //
        // 4. Divide the identifier into numerator and denominator parts
        let full_unit = unit.get_copy_of_measure_unit_impl()?;
        let mut unit: Option<MeasureUnit> = None;
        let mut per_unit: Option<MeasureUnit> = None;

        // Initialize the perUnit with the constant denominator if it
        // exists.
        if full_unit.get_constant_denominator() != 0 {
            let mut per_unit_impl = MeasureUnitImpl::new();
            per_unit_impl.set_constant_denominator(full_unit.get_constant_denominator());
            per_unit = Some(per_unit_impl.build()?);
        }

        for sub_unit in full_unit.get_single_units() {
            let mut sub_unit = sub_unit.clone();
            if sub_unit.get_dimensionality() > 0 {
                unit = Some(match unit {
                    None => sub_unit.build()?,
                    Some(u) => u.product(Some(&sub_unit.build()?))?,
                });
            } else {
                // It's okay to mutate fullUnit, we made a temporary copy:
                sub_unit.set_dimensionality(-sub_unit.get_dimensionality());
                per_unit = Some(match per_unit {
                    None => sub_unit.build()?,
                    Some(p) => p.product(Some(&sub_unit.build()?))?,
                });
            }
        }
        let unit_impl = match &unit {
            None => None,
            Some(u) => Some(u.get_copy_of_measure_unit_impl()?),
        };
        let per_unit_impl = match &per_unit {
            None => None,
            Some(p) => Some(p.get_copy_of_measure_unit_impl()?),
        };

        // TODO(icu-units#28): check placeholder logic, see if it needs to
        // be present here instead of only in processPatternTimes:
        //
        // 5. Set both globalPlaceholder and globalPlaceholderPosition to be
        // empty

        let derived_per_cases = DerivedComponents::new(loc, "case", "per")?;

        // 6. numeratorUnitString
        let mut numerator_unit_data = new_unit_data();
        process_pattern_times(
            unit_impl,
            loc,
            width,
            derived_per_cases.value0(unit_display_case),
            &mut numerator_unit_data,
        )?;

        // 7. denominatorUnitString
        let mut denominator_unit_data = new_unit_data();
        process_pattern_times(
            per_unit_impl,
            loc,
            width,
            derived_per_cases.value1(unit_display_case),
            &mut denominator_unit_data,
        )?;

        // TODO(icu-units#139):
        // 8. Set perPattern to be getValue([per], locale, length)
        let per_unit_pattern = match &denominator_unit_data[PER_INDEX] {
            Some(p) => p.clone(),
            None => {
                let mut sb = Vec::new();
                let raw_per_unit_format = get_compound_value("per", loc, width)?;
                // rawPerUnitFormat is something like "{0} per {1}"; we need
                // to substitute in the plural form.
                let per_pattern_formatter =
                    SimpleFormatterImpl::compile_to_string_min_max_arguments(
                        &raw_per_unit_format,
                        &mut sb,
                        2,
                        2,
                    )?;
                // Plural and placeholder handling for 7. denominatorUnitString:
                // TODO(icu-units#139): hardcoded:
                // <deriveComponent feature="plural" structure="per" value0="compound" value1="one"/>
                let raw_denominator_format =
                    get_with_plural(&denominator_unit_data, StandardPlural::One)?;
                // Some "one" pattern may not contain "{0}". For example in
                // "ar" or "ne" locale.
                let denominator_formatter =
                    SimpleFormatterImpl::compile_to_string_min_max_arguments(
                        &raw_denominator_format,
                        &mut sb,
                        0,
                        1,
                    )?;
                let denominator_string = PatternProps::trim_space_char(
                    &SimpleFormatterImpl::get_text_with_no_arguments(&denominator_formatter),
                );
                // TODO(icu-units#28): Why does UnicodeString need to be
                // explicit in the following line?
                SimpleFormatterImpl::format_compiled_pattern(
                    &per_pattern_formatter,
                    &[Arg::Text(&units("{0}")), Arg::Text(&denominator_string)],
                )?
            }
        };

        let compiled = if per_unit_pattern.is_empty() {
            Self::simple_formats_to_modifiers(&numerator_unit_data, &nf_field::MEASURE_UNIT)?
        } else {
            Self::multi_simple_formats_to_modifiers(
                &numerator_unit_data,
                &per_unit_pattern,
                &nf_field::MEASURE_UNIT,
            )?
        };

        // Gender
        //
        // TODO(icu-units#28): find out what gender to use in the absence of
        // a first value - e.g. what's the gender of "per-second"? Mentioned
        // in CLDR-14253.
        //
        // gender/per deriveCompound rules don't say:
        // <deriveCompound feature="gender" structure="per" value="0"/> <!-- gender(gram-per-meter) ←  gender(gram) -->
        let gender = get_derived_gender(
            loc,
            "per",
            &numerator_unit_data,
            Some(&denominator_unit_data),
        )?;
        let gender = gender.map_or_else(String::new, |g| to_rust_string(&g));

        Ok(Self::build(compiled, rules, parent, gender))
    }

    /// `simpleFormatsToModifiers(String[], NumberFormat.Field)`: the
    /// compiled pattern of each plural form.
    fn simple_formats_to_modifiers(
        simple_formats: &UnitData,
        field: FieldRef,
    ) -> Result<CompiledModifiers> {
        let mut sb = Vec::new();
        let mut compiled = Vec::with_capacity(StandardPlural::COUNT as usize);
        for plural in StandardPlural::VALUES {
            let simple_format = get_with_plural(simple_formats, plural)?;
            compiled.push(SimpleFormatterImpl::compile_to_string_min_max_arguments(
                &simple_format,
                &mut sb,
                0,
                1,
            )?);
        }
        Ok(CompiledModifiers { compiled, field })
    }

    /// `multiSimpleFormatsToModifiers(String[], String,
    /// NumberFormat.Field)`: the compiled compound pattern of each plural
    /// form.
    fn multi_simple_formats_to_modifiers(
        lead_formats: &UnitData,
        trail_format: &[u16],
        field: FieldRef,
    ) -> Result<CompiledModifiers> {
        let mut sb = Vec::new();
        let trail_compiled =
            SimpleFormatterImpl::compile_to_string_min_max_arguments(trail_format, &mut sb, 1, 1)?;
        let mut compiled = Vec::with_capacity(StandardPlural::COUNT as usize);
        for plural in StandardPlural::VALUES {
            let lead_format = get_with_plural(lead_formats, plural)?;
            let compound_format = if lead_format.is_empty() {
                trail_format.to_vec()
            } else {
                SimpleFormatterImpl::format_compiled_pattern(
                    &trail_compiled,
                    &[Arg::Text(&lead_format)],
                )?
            };
            compiled.push(SimpleFormatterImpl::compile_to_string_min_max_arguments(
                &compound_format,
                &mut sb,
                0,
                1,
            )?);
        }
        Ok(CompiledModifiers { compiled, field })
    }

    /// Equivalent to `getModifier(Signum, StandardPlural)` (the
    /// `ModifierStore` method).
    pub fn get_modifier(
        &self,
        _signum: Signum,
        plural: StandardPlural,
    ) -> Option<Arc<dyn Modifier>> {
        // Signum ignored
        self.modifiers[plural.ordinal() as usize].clone()
    }
}

impl ModifierStore for LongNameHandler {
    fn get_modifier(&self, signum: Signum, plural: StandardPlural) -> Option<Arc<dyn Modifier>> {
        LongNameHandler::get_modifier(self, signum, plural)
    }
}

impl MicroPropsGenerator for LongNameHandler {
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let parent = self
            .parent
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: parent"));
        let mut micros = parent.process_quantity(quantity)?;
        let plural_form =
            RoundingUtils::get_plural_safe(micros.rounder.as_ref(), Some(&self.rules), quantity)?;
        micros.mod_outer = self.modifiers[plural_form.ordinal() as usize].clone();
        micros.gender = Some(self.gender.clone());
        Ok(micros)
    }
}

impl ParentlessMicroPropsGenerator for LongNameHandler {
    /// Produces a plural-appropriate Modifier for a unit: `quantity` is
    /// taken as the final smallest unit, while the larger unit values must
    /// be provided via `micros.mixedMeasures`.
    fn process_quantity_with_micros(
        &self,
        quantity: &mut dyn DecimalQuantity,
        mut micros: MicroProps,
    ) -> Result<MicroProps> {
        // TODO(units): Get the right plural form?
        let plural_form =
            RoundingUtils::get_plural_safe(micros.rounder.as_ref(), Some(&self.rules), quantity)?;
        micros.mod_outer = self.modifiers[plural_form.ordinal() as usize].clone();
        Ok(micros)
    }
}

/// `processPatternTimes(MeasureUnitImpl, ULocale, UnitWidth, String,
/// String[])`: roughly corresponds to patternTimes(...) in the spec.
///
/// If productUnit is `None`, outArray's "other" and "per" entries are set
/// to the empty string.
fn process_pattern_times(
    product_unit: Option<MeasureUnitImpl>,
    loc: &ULocale,
    width: UnitWidth,
    mut case_variant: Option<String>,
    out_array: &mut UnitData,
) -> Result<()> {
    debug_assert!(
        out_array[StandardPlural::Other.ordinal() as usize].is_none(),
        "outArray must have only null values!"
    );
    debug_assert!(
        out_array[PER_INDEX].is_none(),
        "outArray must have only null values!"
    );

    let mut product_unit = match product_unit {
        None => {
            out_array[StandardPlural::Other.ordinal() as usize] = Some(Vec::new());
            out_array[PER_INDEX] = Some(Vec::new());
            return Ok(());
        }
        Some(p) => p,
    };
    if product_unit.get_complexity() == Complexity::Mixed {
        // TODO(icu-units#28): consider when serialize should be called.
        // (Java: Mixed units not supported.)
        return Err(LuceneError::unsupported_operation(
            "Mixed units not supported by LongNameHandler",
        ));
    }

    // 1. If the unitId is empty or invalid, fail
    if product_unit.get_identifier().is_none() {
        // TODO(icu-units#28): consider when serialize should be called.
        // identifier might also be empty for MeasureUnit().
        product_unit.serialize()?;
    }
    let product_identifier = product_unit.get_identifier().unwrap_or_default();
    if product_identifier.is_empty() {
        // MeasureUnit(): no units: return empty strings.
        return Ok(());
    }

    let simple_unit = MeasureUnit::find_by_sub_type(&product_identifier)?;
    if let Some(simple_unit) = simple_unit {
        // TODO(ICU-21900): consider checking MeasureUnit::findBySubType
        // first. Instead of assuming that the input is a built-in unit
        // that we can use, we could check this.
        get_measure_data(loc, &simple_unit, width, case_variant.as_deref(), out_array)?;
        maybe_calculate_gender(loc, &simple_unit, out_array)?;
        return Ok(());
    }

    // 2. Set timesPattern to be getValue(times, locale, length)
    let times_pattern = get_compound_value("times", loc, width)?;
    let mut sb = Vec::new();
    let times_pattern_formatter =
        SimpleFormatterImpl::compile_to_string_min_max_arguments(&times_pattern, &mut sb, 2, 2)?;

    let mut global_placeholder: Vec<Option<PlaceholderPosition>> = vec![None; ARRAY_LENGTH];
    let mut global_joiner_char: u16 = 0;
    // Numbered list items are from the algorithms at
    // https://unicode.org/reports/tr35/tr35-general.html#compound-units:
    //
    // pattern(...) point 5:
    // - Set both globalPlaceholder and globalPlaceholderPosition to be empty
    //
    // 3. Set result to be empty
    for plural in StandardPlural::VALUES {
        let plural_index = plural.ordinal() as usize;
        // Initial state: empty string pattern, via all falling back to
        // OTHER:
        if plural == StandardPlural::Other {
            out_array[plural_index] = Some(Vec::new());
        } else {
            out_array[plural_index] = None;
        }
        global_placeholder[plural_index] = None;
    }

    // null represents "compound" (propagate the plural form).
    let mut plural_category: Option<String> = None;
    let derived_times_plurals = DerivedComponents::new(loc, "plural", "times")?;
    let derived_times_cases = DerivedComponents::new(loc, "case", "times")?;
    let derived_power_cases = DerivedComponents::new(loc, "case", "power")?;

    // 4. For each single_unit in product_unit
    let single_units_len = product_unit.get_single_units().len();
    for single_unit_index in 0..single_units_len {
        // Java mutates the element of the list in place.
        let single_unit = &mut product_unit.get_single_units_mut()[single_unit_index];
        let single_plural_category;
        let mut single_case_variant;
        // TODO(icu-units#28): ensure we have unit tests that change/fail if
        // we assign incorrect case variants here:
        if single_unit_index < single_units_len - 1 {
            // 4.1. If hasMultiple
            single_plural_category = derived_times_plurals.value0(plural_category.as_deref());
            single_case_variant = derived_times_cases.value0(case_variant.as_deref());
            plural_category = derived_times_plurals.value1(plural_category.as_deref());
            case_variant = derived_times_cases.value1(case_variant.as_deref());
        } else {
            single_plural_category = derived_times_plurals.value1(plural_category.as_deref());
            single_case_variant = derived_times_cases.value1(case_variant.as_deref());
        }

        // 4.2. Get the gender of that single_unit
        let simple_unit = MeasureUnit::find_by_sub_type(single_unit.get_simple_unit_id())?;
        let simple_unit = match simple_unit {
            // Ideally all simple units should be known, but they're not:
            // 100-kilometer is internally treated as a simple unit, but it
            // is not a built-in unit and does not have formatting data in
            // CLDR 39.
            //
            // TODO(icu-units#28): test (desirable) invariants in unit
            // tests.
            None => {
                return Err(LuceneError::unsupported_operation(format!(
                    "Unsupported sinlgeUnit: {}",
                    single_unit.get_simple_unit_id()
                )))
            }
            Some(u) => u,
        };
        let gender = get_gender_for_builtin(loc, Some(&simple_unit))?;

        // 4.3. If singleUnit starts with a dimensionality_prefix, such as
        // 'square-'
        debug_assert!(single_unit.get_dimensionality() > 0);
        let dimensionality = single_unit.get_dimensionality();
        let mut dimensionality_prefix_patterns = new_unit_data();
        if dimensionality != 1 {
            // 4.3.1. set dimensionalityPrefixPattern to be
            //   getValue(that dimensionality_prefix, locale, length,
            //   singlePluralCategory, singleCaseVariant, gender), such as
            //   "{0} kwadratowym"
            let dimensionality_key = format!("compound/power{dimensionality}");
            match get_inflected_measure_data(
                &dimensionality_key,
                loc,
                width,
                Some(gender.clone()),
                single_case_variant.clone(),
                &mut dimensionality_prefix_patterns,
            ) {
                Ok(()) => {}
                Err(e) if is_missing_resource(&e) => {
                    // At the time of writing, only pow2 and pow3 are
                    // supported. Attempting to format other powers results
                    // in a MissingResourceException.
                    if dimensionality > 3 {
                        return Err(LuceneError::unsupported_operation(format!(
                            "powerN not supported for N > 3: {product_identifier}"
                        )));
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }

            // TODO(icu-units#139):
            // 4.3.2. set singlePluralCategory to be power0(singlePluralCategory)

            // 4.3.3. set singleCaseVariant to be power0(singleCaseVariant)
            single_case_variant = derived_power_cases.value0(single_case_variant.as_deref());
            // 4.3.4. remove the dimensionality_prefix from singleUnit
            single_unit.set_dimensionality(1);
        }

        // 4.4. if singleUnit starts with an si_prefix, such as 'centi'
        let prefix = single_unit.get_prefix();
        let mut prefix_pattern = Vec::new();
        if prefix != MeasurePrefix::One {
            // 4.4.1. set siPrefixPattern to be getValue(that si_prefix,
            //   locale, length), such as "centy{0}"
            let prefix_key = format!("{}p{}", prefix.get_base(), prefix.get_power());
            // TODO(icu-units#28): this is inefficient, it calls
            // getAllItemsWithFallback for each.
            prefix_pattern = get_compound_value(&prefix_key, loc, width)?;

            // TODO(icu-units#139):
            // 4.4.2. set singlePluralCategory to be prefix0(singlePluralCategory)

            // TODO(icu-units#139):
            // 4.4.3. set singleCaseVariant to be prefix0(singleCaseVariant)

            // 4.4.4. remove the si_prefix from singleUnit
            single_unit.set_prefix(MeasurePrefix::One);
        }

        // 4.5. Set corePattern to be the getValue(singleUnit, locale,
        // length, singlePluralCategory, singleCaseVariant), such as
        // "{0} metrem"
        let mut single_unit_array = new_unit_data();
        // At this point we are left with a Simple Unit:
        debug_assert!(
            single_unit
                .build()
                .map(|u| u.get_identifier())
                .ok()
                .as_deref()
                == Some(single_unit.get_simple_unit_id()),
            "Should be equal: singleUnit.build().getIdentifier() and singleUnit.getSimpleUnitID()"
        );
        let built = single_unit.build()?;
        get_measure_data(
            loc,
            &built,
            width,
            single_case_variant.as_deref(),
            &mut single_unit_array,
        )?;
        // TODO(icu-units#139): ...
        if single_unit_array[GENDER_INDEX].is_some() {
            debug_assert!(!single_unit_array[GENDER_INDEX]
                .as_ref()
                .is_some_and(Vec::is_empty));

            if prefix != MeasurePrefix::One {
                single_unit_array[GENDER_INDEX] =
                    get_derived_gender(loc, "prefix", &single_unit_array, None)?;
            }

            // Powers use compoundUnitPattern1, dimensionalityPrefixPatterns
            // may have a "gender" entry, but they're ignored for
            // determining the gender:
            if dimensionality != 1 {
                single_unit_array[GENDER_INDEX] =
                    get_derived_gender(loc, "power", &single_unit_array, None)?;
            }

            let times_gender_rule = get_derive_compound_rule(loc, "gender", "times")?;
            if times_gender_rule.len() == 1 {
                if times_gender_rule[0] == '0' as u16 {
                    if single_unit_index == 0 {
                        debug_assert!(out_array[GENDER_INDEX].is_none());
                        out_array[GENDER_INDEX] = single_unit_array[GENDER_INDEX].clone();
                    }
                } else if times_gender_rule[0] == '1' as u16
                    && single_unit_index == single_units_len - 1
                {
                    debug_assert!(out_array[GENDER_INDEX].is_none());
                    out_array[GENDER_INDEX] = single_unit_array[GENDER_INDEX].clone();
                }
            } else if out_array[GENDER_INDEX].is_none() {
                out_array[GENDER_INDEX] = Some(times_gender_rule);
            }
        }

        // Calculate output gender
        for plural_ in StandardPlural::VALUES {
            let mut plural = plural_;
            let plural_index = plural.ordinal() as usize;

            if out_array[plural_index].is_none() {
                if single_unit_array[plural_index].is_none() {
                    // Let the usual plural fallback mechanism take care of
                    // this plural form
                    continue;
                } else {
                    // Since our singleUnit can have a plural form that
                    // outArray doesn't yet have (relying on fallback to
                    // OTHER), we start by grabbing it with the normal
                    // plural fallback mechanism
                    out_array[plural_index] = Some(get_with_plural(out_array, plural)?);
                }
            }

            if let Some(category) = &single_plural_category {
                plural = StandardPlural::from_string(category)?;
            }

            // 4.6. Extract(corePattern, coreUnit, placeholder,
            // placeholderPosition) from that pattern.
            let mut r = extract_core_pattern(&get_with_plural(&single_unit_array, plural)?);

            // 4.7 If the position is middle, then fail
            if r.placeholder_position == PlaceholderPosition::Middle {
                return Err(crate::jdk::unchecked_no_message(
                    "java.lang.UnsupportedOperationException",
                ));
            }

            // 4.8. If globalPlaceholder is empty
            if global_placeholder[plural_index].is_none() {
                global_placeholder[plural_index] = Some(r.placeholder_position);
                global_joiner_char = r.joiner_char;
            } else {
                // Expect all units involved to have the same placeholder
                // position
                debug_assert!(global_placeholder[plural_index] == Some(r.placeholder_position));
                // TODO(icu-units#28): Do we want to add an assert that
                // the joiner characters are the same?
            }

            // Not mentioned in spec: whether to apply the SI prefix
            // pattern first or the dimensionality prefix pattern first.
            if prefix != MeasurePrefix::One {
                let prefix_compiled = SimpleFormatterImpl::compile_to_string_min_max_arguments(
                    &prefix_pattern,
                    &mut sb,
                    1,
                    1,
                )?;

                // 4.9.1. If siPrefixPattern is not empty, then set
                // coreUnit to be the combination of siPrefixPattern and
                // coreUnit
                // TODO(icu-units#28): unit test under which this is needed.
                if width == UnitWidth::FullName {
                    r.core_unit = UCharacter::to_lower_case_ulocale(loc, &r.core_unit);
                }
                r.core_unit = SimpleFormatterImpl::format_compiled_pattern(
                    &prefix_compiled,
                    &[Arg::Text(&r.core_unit)],
                )?;
            }

            // 4.9.2. If dimensionalityPrefixPattern is not empty, then set
            // coreUnit to be the combination of dimensionalityPrefixPattern
            // and coreUnit
            if dimensionality != 1 {
                let dimensionality_compiled =
                    SimpleFormatterImpl::compile_to_string_min_max_arguments(
                        &get_with_plural(&dimensionality_prefix_patterns, plural)?,
                        &mut sb,
                        1,
                        1,
                    )?;
                if width == UnitWidth::FullName {
                    r.core_unit = UCharacter::to_lower_case_ulocale(loc, &r.core_unit);
                }
                r.core_unit = SimpleFormatterImpl::format_compiled_pattern(
                    &dimensionality_compiled,
                    &[Arg::Text(&r.core_unit)],
                )?;
            }

            let current = out_array[plural_index].clone().unwrap_or_default();
            if current.is_empty() {
                // 4.11. If the result is empty, set result to be coreUnit
                out_array[plural_index] = Some(r.core_unit);
            } else {
                // 4.12. Otherwise set result to be format(timesPattern,
                // result, coreUnit)
                out_array[plural_index] = Some(SimpleFormatterImpl::format_compiled_pattern(
                    &times_pattern_formatter,
                    &[Arg::Text(&current), Arg::Text(&r.core_unit)],
                )?);
            }
        }
    }

    // 5. Handling constant denominator if it exists.
    if product_unit.get_constant_denominator() != 0 {
        out_array[CONSTANT_DENOMINATOR_INDEX] =
            Some(units(&product_unit.get_constant_denominator().to_string()));
        let mut plural_index: Option<usize> = None;
        for plural_ in StandardPlural::VALUES {
            if out_array[plural_.ordinal() as usize].is_some() {
                plural_index = Some(plural_.ordinal() as usize);
                break;
            }
        }

        debug_assert!(
            plural_index.is_some(),
            "No plural form found for constant denominator"
        );
        let plural_index = plural_index.unwrap_or_else(|| panic!("java.lang.NullPointerException"));

        // TODO(ICU-23039):
        // Improve the handling of constant_denominator representation.
        // For instance, a constant_denominator of 1000000 should be adaptable
        // to formats like 1,000,000, 1e6, or 1 million. Furthermore,
        // ensure consistent pluralization rules for units. For example,
        // "meter per 100 seconds" should be evaluated for correct
        // singular/plural usage: "second" or "seconds"? Similarly, "kilogram
        // per 1000 meters" should be checked for "meter" or "meters"?
        let current = out_array[plural_index].clone().unwrap_or_default();
        let constant = out_array[CONSTANT_DENOMINATOR_INDEX]
            .clone()
            .unwrap_or_default();
        if current.is_empty() {
            out_array[plural_index] = Some(constant);
        } else {
            out_array[plural_index] = Some(SimpleFormatterImpl::format_compiled_pattern(
                &times_pattern_formatter,
                &[Arg::Text(&constant), Arg::Text(&current)],
            )?);
        }
    }

    // 6. Handling globalPlaceholderPosition.
    for plural in StandardPlural::VALUES {
        let plural_index = plural.ordinal() as usize;
        match global_placeholder[plural_index] {
            Some(PlaceholderPosition::Beginning) => {
                let mut tmp = units("{0}");
                if global_joiner_char != 0 {
                    tmp.push(global_joiner_char);
                }
                append_java_string(&mut tmp, out_array[plural_index].as_deref());
                out_array[plural_index] = Some(tmp);
            }
            Some(PlaceholderPosition::End) => {
                let mut s = Vec::new();
                append_java_string(&mut s, out_array[plural_index].as_deref());
                if global_joiner_char != 0 {
                    s.push(global_joiner_char);
                }
                s.extend_from_slice(&units("{0}"));
                out_array[plural_index] = Some(s);
            }
            _ => {}
        }
    }
    Ok(())
}

/// `StringBuilder.append(String)` / string concatenation of a nullable
/// string (`null` appends `"null"`).
fn append_java_string(sb: &mut Vec<u16>, s: Option<&[u16]>) {
    match s {
        Some(s) => sb.extend_from_slice(s),
        None => sb.extend_from_slice(&units("null")),
    }
}
