// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2004-2016, Google Inc, International Business Machines
 * Corporation and others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.util.MeasureUnit`.

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::{units, DataOutputStream, InputStream};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::icu_data_class_loader;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::r#impl::units::measure_unit_impl::MeasureUnitImpl;
use crate::r#impl::units::single_unit_impl::SingleUnitImpl;
use crate::r#impl::utility::Utility;
use crate::text::unicode_set::UnicodeSet;
use crate::util::currency::Currency;
use crate::util::no_unit::NoUnit;
use crate::util::time_unit::TimeUnit;
use crate::util::u_resource_bundle::UResourceBundle;

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = -1839973855554750484;

/// Enumeration for unit complexity. Equivalent to `MeasureUnit.Complexity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Complexity {
    /// A single unit, like kilojoule.
    Single,
    /// A compound unit, like meter-per-second.
    Compound,
    /// A mixed unit, like hour+minute.
    Mixed,
}

/// Enumeration for SI and binary prefixes, e.g. "kilo-", "nano-", "mebi-".
/// Equivalent to `MeasureUnit.MeasurePrefix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasurePrefix {
    /// SI prefix: quetta, 10^30.
    Quetta,
    /// SI prefix: ronna, 10^27.
    Ronna,
    /// SI prefix: yotta, 10^24.
    Yotta,
    /// SI prefix: zetta, 10^21.
    Zetta,
    /// SI prefix: exa, 10^18.
    Exa,
    /// SI prefix: peta, 10^15.
    Peta,
    /// SI prefix: tera, 10^12.
    Tera,
    /// SI prefix: giga, 10^9.
    Giga,
    /// SI prefix: mega, 10^6.
    Mega,
    /// SI prefix: kilo, 10^3.
    Kilo,
    /// SI prefix: hecto, 10^2.
    Hecto,
    /// SI prefix: deka, 10^1.
    Deka,
    /// The absence of an SI or binary prefix.
    One,
    /// SI prefix: deci, 10^-1.
    Deci,
    /// SI prefix: centi, 10^-2.
    Centi,
    /// SI prefix: milli, 10^-3.
    Milli,
    /// SI prefix: micro, 10^-6.
    Micro,
    /// SI prefix: nano, 10^-9.
    Nano,
    /// SI prefix: pico, 10^-12.
    Pico,
    /// SI prefix: femto, 10^-15.
    Femto,
    /// SI prefix: atto, 10^-18.
    Atto,
    /// SI prefix: zepto, 10^-21.
    Zepto,
    /// SI prefix: yocto, 10^-24.
    Yocto,
    /// SI prefix: ronto, 10^-27.
    Ronto,
    /// SI prefix: quecto, 10^-30.
    Quecto,
    /// IEC binary prefix: kibi, 1024^1.
    Kibi,
    /// IEC binary prefix: mebi, 1024^2.
    Mebi,
    /// IEC binary prefix: gibi, 1024^3.
    Gibi,
    /// IEC binary prefix: tebi, 1024^4.
    Tebi,
    /// IEC binary prefix: pebi, 1024^5.
    Pebi,
    /// IEC binary prefix: exbi, 1024^6.
    Exbi,
    /// IEC binary prefix: zebi, 1024^7.
    Zebi,
    /// IEC binary prefix: yobi, 1024^8.
    Yobi,
}

impl MeasurePrefix {
    /// The constants in declaration order (`values()`).
    pub const VALUES: [MeasurePrefix; 33] = [
        MeasurePrefix::Quetta,
        MeasurePrefix::Ronna,
        MeasurePrefix::Yotta,
        MeasurePrefix::Zetta,
        MeasurePrefix::Exa,
        MeasurePrefix::Peta,
        MeasurePrefix::Tera,
        MeasurePrefix::Giga,
        MeasurePrefix::Mega,
        MeasurePrefix::Kilo,
        MeasurePrefix::Hecto,
        MeasurePrefix::Deka,
        MeasurePrefix::One,
        MeasurePrefix::Deci,
        MeasurePrefix::Centi,
        MeasurePrefix::Milli,
        MeasurePrefix::Micro,
        MeasurePrefix::Nano,
        MeasurePrefix::Pico,
        MeasurePrefix::Femto,
        MeasurePrefix::Atto,
        MeasurePrefix::Zepto,
        MeasurePrefix::Yocto,
        MeasurePrefix::Ronto,
        MeasurePrefix::Quecto,
        MeasurePrefix::Kibi,
        MeasurePrefix::Mebi,
        MeasurePrefix::Gibi,
        MeasurePrefix::Tebi,
        MeasurePrefix::Pebi,
        MeasurePrefix::Exbi,
        MeasurePrefix::Zebi,
        MeasurePrefix::Yobi,
    ];

    /// `(power, identifier, base)` of the constant.
    fn data(self) -> (i32, &'static str, i32) {
        match self {
            MeasurePrefix::Quetta => (30, "quetta", 10),
            MeasurePrefix::Ronna => (27, "ronna", 10),
            MeasurePrefix::Yotta => (24, "yotta", 10),
            MeasurePrefix::Zetta => (21, "zetta", 10),
            MeasurePrefix::Exa => (18, "exa", 10),
            MeasurePrefix::Peta => (15, "peta", 10),
            MeasurePrefix::Tera => (12, "tera", 10),
            MeasurePrefix::Giga => (9, "giga", 10),
            MeasurePrefix::Mega => (6, "mega", 10),
            MeasurePrefix::Kilo => (3, "kilo", 10),
            MeasurePrefix::Hecto => (2, "hecto", 10),
            MeasurePrefix::Deka => (1, "deka", 10),
            MeasurePrefix::One => (0, "", 10),
            MeasurePrefix::Deci => (-1, "deci", 10),
            MeasurePrefix::Centi => (-2, "centi", 10),
            MeasurePrefix::Milli => (-3, "milli", 10),
            MeasurePrefix::Micro => (-6, "micro", 10),
            MeasurePrefix::Nano => (-9, "nano", 10),
            MeasurePrefix::Pico => (-12, "pico", 10),
            MeasurePrefix::Femto => (-15, "femto", 10),
            MeasurePrefix::Atto => (-18, "atto", 10),
            MeasurePrefix::Zepto => (-21, "zepto", 10),
            MeasurePrefix::Yocto => (-24, "yocto", 10),
            MeasurePrefix::Ronto => (-27, "ronto", 10),
            MeasurePrefix::Quecto => (-30, "quecto", 10),
            MeasurePrefix::Kibi => (1, "kibi", 1024),
            MeasurePrefix::Mebi => (2, "mebi", 1024),
            MeasurePrefix::Gibi => (3, "gibi", 1024),
            MeasurePrefix::Tebi => (4, "tebi", 1024),
            MeasurePrefix::Pebi => (5, "pebi", 1024),
            MeasurePrefix::Exbi => (6, "exbi", 1024),
            MeasurePrefix::Zebi => (7, "zebi", 1024),
            MeasurePrefix::Yobi => (8, "yobi", 1024),
        }
    }

    /// Returns the identifier of the prefix, e.g. "kilo". Equivalent to
    /// `getIdentifier()` (internal).
    pub fn get_identifier(self) -> &'static str {
        self.data().1
    }

    /// Returns the base of the prefix (10 or 1024). Equivalent to
    /// `getBase()`.
    pub fn get_base(self) -> i32 {
        self.data().2
    }

    /// Returns the exponent of the prefix. Equivalent to `getPower()`.
    pub fn get_power(self) -> i32 {
        self.data().0
    }

    /// Equivalent to `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }
}

/// Custom symbols of a `CustomSymbolCurrency`: the overriding state of that
/// subclass, kept in the base object so that every `Currency` method sees it
/// (Java's virtual dispatch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomSymbols {
    /// `symbol1`: the currency symbol.
    pub symbol1: Vec<u16>,
    /// `symbol2`: the international currency symbol (the `getCurrencyCode()`
    /// override).
    pub symbol2: Vec<u16>,
}

/// The runtime class of a `MeasureUnit` object (Java's subclasses).
#[derive(Debug, Clone)]
pub enum UnitClass {
    /// `com.ibm.icu.util.MeasureUnit`.
    MeasureUnit,
    /// `com.ibm.icu.util.Currency` (with its `isoCode` field), or its
    /// subclass `com.ibm.icu.impl.number.CustomSymbolCurrency` when `custom`
    /// is set.
    Currency {
        /// `isoCode`.
        iso_code: String,
        /// The `CustomSymbolCurrency` fields.
        custom: Option<CustomSymbols>,
    },
    /// `com.ibm.icu.util.TimeUnit` (with its `index` field).
    TimeUnit {
        /// `index`.
        index: i32,
    },
}

/// The fields of a `MeasureUnit` object.
#[derive(Debug)]
pub struct MeasureUnitData {
    /// `type` (protected; `null` for units built from a `MeasureUnitImpl`).
    pub(crate) type_: Option<String>,
    /// `subType` (protected).
    pub(crate) sub_type: Option<String>,
    /// `measureUnitImpl`.
    pub(crate) measure_unit_impl: Option<MeasureUnitImpl>,
    /// The runtime class.
    pub(crate) class: UnitClass,
}

/// A unit such as length, mass, volume, currency, etc. A unit is coupled
/// with a numeric amount to produce a Measure. MeasureUnit objects are
/// immutable. All subclasses must guarantee that.
///
/// Equivalent to `com.ibm.icu.util.MeasureUnit`. Units are shared objects
/// (Java's cache hands out the same instance); its subclasses `Currency`
/// (and `CustomSymbolCurrency`) and `TimeUnit` are views over a unit whose
/// [`UnitClass`] records the runtime class (see [`Currency`],
/// [`TimeUnit`]). Java's `null` unit (`NoUnit.BASE`) is `None` of an
/// `Option<MeasureUnit>`. The `Object` members: `equals(Object)` is
/// [`java_equals`](Self::java_equals) (and `PartialEq`), `hashCode()`
/// [`hash_code`](Self::hash_code), `toString()` [`Display`](fmt::Display).
/// Java serialization replaces a unit by a [`MeasureUnitProxy`]
/// ([`write_replace`](Self::write_replace)).
#[derive(Debug, Clone)]
pub struct MeasureUnit(pub(crate) Arc<MeasureUnitData>);

/// `cache`: type to (subtype to unit), Java `HashMap` iteration order.
static CACHE: LazyLock<Mutex<JavaHashMap<JavaHashMap<MeasureUnit>>>> = LazyLock::new(|| Mutex::new(JavaHashMap::new()));

/// `cacheIsPopulated`.
static CACHE_IS_POPULATED: Mutex<bool> = Mutex::new(false);

/// `ASCII`.
static ASCII: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut s = UnicodeSet::new_range('a' as i32, 'z' as i32).expect("valid range");
    s.freeze();
    s
});

/// `ASCII_HYPHEN_DIGITS`.
static ASCII_HYPHEN_DIGITS: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut s = UnicodeSet::from_pairs(&['-' as i32, '-' as i32, '0' as i32, '9' as i32, 'a' as i32, 'z' as i32])
        .expect("valid ranges");
    s.freeze();
    s
});

/// For ICU use only. Equivalent to the protected interface
/// `MeasureUnit.Factory`.
pub trait Factory: Send + Sync {
    /// Equivalent to `create(String, String)`.
    fn create(&self, type_: &str, sub_type: &str) -> MeasureUnit;
}

/// `UNIT_FACTORY`.
struct UnitFactory;

impl Factory for UnitFactory {
    fn create(&self, type_: &str, sub_type: &str) -> MeasureUnit {
        MeasureUnit::new(type_, sub_type)
    }
}

/// `CURRENCY_FACTORY`.
pub(crate) struct CurrencyFactory;

impl Factory for CurrencyFactory {
    fn create(&self, _unused_type: &str, sub_type: &str) -> MeasureUnit {
        Currency::new(sub_type).into_measure_unit()
    }
}

/// `TIMEUNIT_FACTORY`.
pub(crate) struct TimeUnitFactory;

impl Factory for TimeUnitFactory {
    fn create(&self, type_: &str, sub_type: &str) -> MeasureUnit {
        TimeUnit::new(type_, sub_type).into_measure_unit()
    }
}

/// Sink for enumerating the available measure units. Equivalent to
/// `MeasureUnit.MeasureUnitSink`.
struct MeasureUnitSink;

impl Sink for MeasureUnitSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let unit_types_table = value.get_table()?;
        let mut i2 = 0;
        while unit_types_table.get_key_and_value(i2, key, value) {
            // Skip "compound" and "coordinate" since they are treated
            // differently from the other units
            if key.content_equals("compound") || key.content_equals("coordinate") {
                i2 += 1;
                continue;
            }

            let unit_type = key.to_java_string();
            let unit_names_table = value.get_table()?;
            let mut i3 = 0;
            while unit_names_table.get_key_and_value(i3, key, value) {
                let unit_name = key.to_java_string();
                MeasureUnit::internal_get_instance(&unit_type, &unit_name)?;
                i3 += 1;
            }
            i2 += 1;
        }
        Ok(())
    }
}

/// Sink for loading the currency numeric codes. Equivalent to
/// `MeasureUnit.CurrencyNumericCodeSink`.
struct CurrencyNumericCodeSink;

impl Sink for CurrencyNumericCodeSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let codes_table = value.get_table()?;
        let mut i1 = 0;
        while codes_table.get_key_and_value(i1, key, value) {
            MeasureUnit::internal_get_instance("currency", &key.to_java_string())?;
            i1 += 1;
        }
        Ok(())
    }
}

impl MeasureUnit {
    /// The unit object behind a static constant (the constants name valid
    /// type/subtype pairs).
    pub(crate) fn constant(type_: &str, sub_type: &str) -> MeasureUnit {
        MeasureUnit::internal_get_instance(type_, sub_type).expect("MeasureUnit constants name valid units")
    }

    /// Equivalent to the protected `MeasureUnit(String type, String
    /// subType)`.
    pub(crate) fn new(type_: &str, sub_type: &str) -> MeasureUnit {
        Self::with_class(Some(type_.to_string()), Some(sub_type.to_string()), UnitClass::MeasureUnit)
    }

    /// A unit of the given runtime class (the subclass constructors).
    pub(crate) fn with_class(type_: Option<String>, sub_type: Option<String>, class: UnitClass) -> MeasureUnit {
        MeasureUnit(Arc::new(MeasureUnitData { type_, sub_type, measure_unit_impl: None, class }))
    }

    /// The runtime class of this unit.
    pub fn unit_class(&self) -> &UnitClass {
        &self.0.class
    }

    /// Whether `self` and `other` are the same object (Java `==`).
    pub fn ptr_eq(&self, other: &MeasureUnit) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Construct a MeasureUnit from a CLDR Core Unit Identifier, as defined
    /// in UTS 35. An empty identifier is `NoUnit.BASE` (`None`). Equivalent
    /// to `forIdentifier(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the identifier is invalid.
    pub fn for_identifier(identifier: Option<&str>) -> Result<Option<MeasureUnit>> {
        match identifier {
            None | Some("") => Ok(NoUnit::BASE),
            Some(identifier) => Ok(Some(MeasureUnitImpl::for_identifier(identifier)?.build()?)),
        }
    }

    /// Equivalent to `fromMeasureUnitImpl(MeasureUnitImpl)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the data loading.
    pub fn from_measure_unit_impl(measure_unit_impl: &mut MeasureUnitImpl) -> Result<MeasureUnit> {
        measure_unit_impl.serialize()?;
        let identifier = measure_unit_impl.get_identifier().unwrap_or_default();
        if let Some(result) = MeasureUnit::find_by_sub_type(&identifier)? {
            return Ok(result);
        }

        Ok(MeasureUnit::from_impl(measure_unit_impl))
    }

    /// `MeasureUnit(MeasureUnitImpl)`.
    fn from_impl(measure_unit_impl: &MeasureUnitImpl) -> MeasureUnit {
        MeasureUnit(Arc::new(MeasureUnitData {
            type_: None,
            sub_type: None,
            measure_unit_impl: Some(measure_unit_impl.copy()),
            class: UnitClass::MeasureUnit,
        }))
    }

    /// Get the type, such as "length". May return `None`. Equivalent to
    /// `getType()`.
    pub fn get_type(&self) -> Option<&str> {
        self.0.type_.as_deref()
    }

    /// Get the subType, such as "foot". May return `None`. Equivalent to
    /// `getSubtype()`.
    pub fn get_subtype(&self) -> Option<&str> {
        self.0.sub_type.as_deref()
    }

    /// Gets the CLDR Unit Identifier for this MeasureUnit, as defined in UTS
    /// 35. Equivalent to `getIdentifier()`.
    pub fn get_identifier(&self) -> String {
        let result = match &self.0.measure_unit_impl {
            None => self.get_subtype().map(str::to_string),
            Some(i) => i.get_identifier(),
        };
        result.unwrap_or_default()
    }

    /// Compute the complexity of the unit. Equivalent to `getComplexity()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an unparsable identifier.
    pub fn get_complexity(&self) -> Result<Complexity> {
        match &self.0.measure_unit_impl {
            None => Ok(MeasureUnitImpl::for_identifier(&self.get_identifier())?.get_complexity()),
            Some(i) => Ok(i.get_complexity()),
        }
    }

    /// Creates a MeasureUnit which is this SINGLE unit augmented with the
    /// specified prefix. Equivalent to `withPrefix(MeasurePrefix)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException`/`IllegalArgumentException` if this is not a
    /// SINGLE unit.
    pub fn with_prefix(&self, prefix: MeasurePrefix) -> Result<MeasureUnit> {
        let mut single_unit = self.get_single_unit_impl()?;
        single_unit.set_prefix(prefix);
        single_unit.build()
    }

    /// Returns the current SI or binary prefix of this SINGLE unit.
    /// Equivalent to `getPrefix()`.
    ///
    /// # Errors
    ///
    /// As [`with_prefix`](Self::with_prefix).
    pub fn get_prefix(&self) -> Result<MeasurePrefix> {
        Ok(self.get_single_unit_impl()?.get_prefix())
    }

    /// Creates a new MeasureUnit with a specified constant denominator.
    /// Equivalent to `withConstantDenominator(long)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Denominator cannot be negative")`,
    /// `UnsupportedOperationException("Constant denominator can only be
    /// applied to COMPOUND & SINGLE units")`.
    pub fn with_constant_denominator(&self, denominator: i64) -> Result<MeasureUnit> {
        if denominator < 0 {
            return Err(LuceneError::illegal_argument("Denominator cannot be negative"));
        }

        if self.get_complexity()? != Complexity::Compound && self.get_complexity()? != Complexity::Single {
            return Err(LuceneError::unsupported_operation(
                "Constant denominator can only be applied to COMPOUND & SINGLE units",
            ));
        }

        let mut measure_unit_impl = self.get_copy_of_measure_unit_impl()?;
        measure_unit_impl.set_constant_denominator(denominator);

        measure_unit_impl.set_complexity(if denominator == 0 && measure_unit_impl.get_single_units().len() < 2 {
            Complexity::Single
        } else {
            Complexity::Compound
        });

        measure_unit_impl.build()
    }

    /// Get the constant denominator of the unit (0 when it has none).
    /// Equivalent to `getConstantDenominator()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException("Constant denominator is only supported
    /// for COMPOUND & SINGLE units")`.
    pub fn get_constant_denominator(&self) -> Result<i64> {
        if self.get_complexity()? != Complexity::Compound && self.get_complexity()? != Complexity::Single {
            return Err(LuceneError::unsupported_operation(
                "Constant denominator is only supported for COMPOUND & SINGLE units",
            ));
        }

        match &self.0.measure_unit_impl {
            None => Ok(0),
            Some(i) => Ok(i.get_constant_denominator()),
        }
    }

    /// Returns the dimensionality (power) of this MeasureUnit. Equivalent to
    /// `getDimensionality()`.
    ///
    /// # Errors
    ///
    /// As [`with_prefix`](Self::with_prefix).
    pub fn get_dimensionality(&self) -> Result<i32> {
        Ok(self.get_single_unit_impl()?.get_dimensionality())
    }

    /// Creates a MeasureUnit which is this SINGLE unit augmented with the
    /// specified dimensionality (power). Equivalent to
    /// `withDimensionality(int)`.
    ///
    /// # Errors
    ///
    /// As [`with_prefix`](Self::with_prefix).
    pub fn with_dimensionality(&self, dimensionality: i32) -> Result<MeasureUnit> {
        let mut single_unit = self.get_single_unit_impl()?;
        single_unit.set_dimensionality(dimensionality);
        single_unit.build()
    }

    /// Computes the reciprocal of this MeasureUnit, with the numerator and
    /// denominator flipped. Equivalent to `reciprocal()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException("Cannot take reciprocal of a unit with
    /// a constant denominator")`, and the errors of MIXED units.
    pub fn reciprocal(&self) -> Result<MeasureUnit> {
        if self.get_complexity()? == Complexity::Compound && self.get_constant_denominator()? != 0 {
            return Err(LuceneError::unsupported_operation(
                "Cannot take reciprocal of a unit with a constant denominator",
            ));
        }

        let mut measure_unit = self.get_copy_of_measure_unit_impl()?;
        measure_unit.take_reciprocal();
        measure_unit.build()
    }

    /// Computes the product of this unit with another unit. Equivalent to
    /// `product(MeasureUnit)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for MIXED units and for two constant
    /// denominators.
    pub fn product(&self, other: Option<&MeasureUnit>) -> Result<MeasureUnit> {
        let mut impl_copy = self.get_copy_of_measure_unit_impl()?;

        let other = match other {
            None => return impl_copy.build(),
            Some(o) => o,
        };

        let other_impl_ref = other.get_maybe_reference_of_measure_unit_impl()?;
        if impl_copy.get_complexity() == Complexity::Mixed || other_impl_ref.get_complexity() == Complexity::Mixed {
            return Err(crate::jdk::unchecked_no_message("java.lang.UnsupportedOperationException"));
        }

        for single_unit in other_impl_ref.get_single_units() {
            impl_copy.append_single_unit(Some(single_unit));
        }

        let this_constant_denominator = self.get_constant_denominator()?;
        let other_constant_denominator = other.get_constant_denominator()?;

        // TODO: we can also multiply the constant denominators instead of
        // throwing an exception.
        if this_constant_denominator != 0 && other_constant_denominator != 0 {
            // There is only `one` constant denominator in a compound unit.
            // Therefore, we cannot multiply units that both of them have a
            // constant denominator
            return Err(LuceneError::unsupported_operation(
                "Cannot multiply units that both of them have a constant denominator",
            ));
        }

        // Because either one of the constant denominators is zero, we can
        // use the maximum of them.
        impl_copy.set_constant_denominator(this_constant_denominator.max(other_constant_denominator));

        impl_copy.build()
    }

    /// Returns the list of SINGLE units contained within a sequence of
    /// COMPOUND units. Equivalent to `splitToSingleUnits()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an unparsable identifier.
    pub fn split_to_single_units(&self) -> Result<Vec<MeasureUnit>> {
        let impl_ref = self.get_maybe_reference_of_measure_unit_impl()?;
        let single_units = impl_ref.get_single_units();
        let mut result = Vec::with_capacity(single_units.len());
        for single_unit in single_units {
            result.push(single_unit.build()?);
        }

        Ok(result)
    }

    /// Equivalent to `hashCode()` (overridden by `CustomSymbolCurrency`).
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` for a unit without type or subtype
    /// (one built from a `MeasureUnitImpl`).
    pub fn hash_code(&self) -> i32 {
        let type_ = self.0.type_.as_deref().expect("java.lang.NullPointerException: type");
        let sub_type = self.0.sub_type.as_deref().expect("java.lang.NullPointerException: subType");
        let base = 31i32
            .wrapping_mul(Utility::char_sequence_hash_code(&units(type_)))
            .wrapping_add(Utility::char_sequence_hash_code(&units(sub_type)));
        match &self.0.class {
            UnitClass::Currency { custom: Some(c), .. } => {
                base ^ Utility::char_sequence_hash_code(&c.symbol1) ^ Utility::char_sequence_hash_code(&c.symbol2)
            }
            _ => base,
        }
    }

    /// Equivalent to `equals(Object)`: same object, or same identifier; a
    /// `CustomSymbolCurrency` also requires equal symbols of the other
    /// object, which must be a `CustomSymbolCurrency` too.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` when this is a `CustomSymbolCurrency`
    /// and `rhs` an equal unit of another class.
    pub fn java_equals(&self, rhs: &MeasureUnit) -> bool {
        let base = self.ptr_eq(rhs) || self.get_identifier() == rhs.get_identifier();
        match &self.0.class {
            UnitClass::Currency { custom: Some(c), .. } => {
                if !base {
                    return false;
                }
                match &rhs.0.class {
                    UnitClass::Currency { custom: Some(o), .. } => o.symbol1 == c.symbol1 && o.symbol2 == c.symbol2,
                    _ => panic!(
                        "java.lang.ClassCastException: class {} cannot be cast to class \
                         com.ibm.icu.impl.number.CustomSymbolCurrency",
                        rhs.java_class_name()
                    ),
                }
            }
            _ => base,
        }
    }

    /// The Java binary class name of this unit's runtime class.
    pub fn java_class_name(&self) -> &'static str {
        match &self.0.class {
            UnitClass::MeasureUnit => "com.ibm.icu.util.MeasureUnit",
            UnitClass::Currency { custom: None, .. } => "com.ibm.icu.util.Currency",
            UnitClass::Currency { custom: Some(_), .. } => "com.ibm.icu.impl.number.CustomSymbolCurrency",
            UnitClass::TimeUnit { .. } => "com.ibm.icu.util.TimeUnit",
        }
    }

    /// Equivalent to `toString()` (`Currency` overrides it with the
    /// subtype).
    pub fn to_java_string(&self) -> String {
        if let UnitClass::Currency { .. } = self.0.class {
            return self.0.sub_type.clone().unwrap_or_else(|| "null".to_string());
        }
        let result = match &self.0.measure_unit_impl {
            None => Some(format!(
                "{}-{}",
                self.0.type_.as_deref().unwrap_or("null"),
                self.0.sub_type.as_deref().unwrap_or("null")
            )),
            Some(i) => i.get_identifier(),
        };
        result.unwrap_or_default()
    }

    /// Get all of the available units' types. Equivalent to
    /// `getAvailableTypes()`.
    ///
    /// # Errors
    ///
    /// The errors of the unit data loading.
    pub fn get_available_types() -> Result<Vec<String>> {
        Self::populate_cache()?;
        Ok(lock_cache().keys().cloned().collect())
    }

    /// For the given type, return the available units. Equivalent to
    /// `getAvailable(String)`.
    ///
    /// # Errors
    ///
    /// The errors of the unit data loading.
    pub fn get_available_of_type(type_: &str) -> Result<Vec<MeasureUnit>> {
        Self::populate_cache()?;
        let cache = lock_cache();
        // Train users not to modify returned set from the start giving us
        // more flexibility for implementation.
        // Use CollectionSet instead of HashSet for better performance.
        Ok(match cache.get(type_) {
            None => Vec::new(),
            Some(units) => units.values().cloned().collect(),
        })
    }

    /// Get all of the available units. Returned set is unmodifiable.
    /// Equivalent to `getAvailable()` (`synchronized`): a `HashSet` of
    /// units, whose `hashCode` is `31 * type + subType`.
    ///
    /// # Errors
    ///
    /// The errors of the unit data loading.
    pub fn get_available() -> Result<Vec<MeasureUnit>> {
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut result: Vec<MeasureUnit> = Vec::new();
        for type_ in MeasureUnit::get_available_types()? {
            for unit in MeasureUnit::get_available_of_type(&type_)? {
                if !result.iter().any(|u| u.java_equals(&unit)) {
                    result.push(unit);
                }
            }
        }
        // Train users not to modify returned set from the start giving us
        // more flexibility for implementation.
        Ok(result)
    }

    /// Creates a MeasureUnit instance (creates a singleton instance) or
    /// returns one from the cache. Equivalent to `internalGetInstance(String,
    /// String)` (internal).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("The type or subType are invalid.")`.
    pub fn internal_get_instance(type_: &str, sub_type: &str) -> Result<MeasureUnit> {
        if "currency" != type_
            && (!ASCII.contains_all_str(&units(type_)) || !ASCII_HYPHEN_DIGITS.contains_all_str(&units(sub_type)))
        {
            return Err(LuceneError::illegal_argument("The type or subType are invalid."));
        }
        let factory: &dyn Factory = if "currency" == type_ {
            &CurrencyFactory
        } else if "duration" == type_ {
            &TimeUnitFactory
        } else {
            &UnitFactory
        };
        Ok(MeasureUnit::add_unit(type_, sub_type, factory))
    }

    /// Equivalent to `findBySubType(String)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the unit data loading.
    pub fn find_by_sub_type(sub_type: &str) -> Result<Option<MeasureUnit>> {
        Self::populate_cache()?;
        let cache = lock_cache();
        for units_for_type in cache.values() {
            if let Some(u) = units_for_type.get(sub_type) {
                return Ok(Some(u.clone()));
            }
        }
        Ok(None)
    }

    /// Populate the MeasureUnit cache with all types from the data. Only
    /// built-in types are added; the constructed MeasureUnits are reused.
    /// Equivalent to `populateCache()` (`static synchronized`).
    fn populate_cache() -> Result<()> {
        let mut populated = CACHE_IS_POPULATED.lock().unwrap_or_else(|e| e.into_inner());
        if *populated {
            return Ok(());
        }
        *populated = true;

        // Schema:
        //
        // units{
        //   duration{
        //     day{
        //       one{"{0} ден"}
        //       other{"{0} дена"}
        //     }

        // Load the unit types.  Use English, since we know that that is a
        // superset.
        let rb1 = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_UNIT_BASE_NAME), "en")?;
        rb1.as_icu()?.get_all_items_with_fallback("units", &mut MeasureUnitSink)?;

        // Load the currencies
        let rb2 = UResourceBundle::get_bundle_instance_loader(
            Some(ICUData::ICU_BASE_NAME),
            "currencyNumericCodes",
            icu_data_class_loader(),
        )?;
        rb2.as_icu()?.get_all_items_with_fallback("codeMap", &mut CurrencyNumericCodeSink)?;
        Ok(())
    }

    /// Equivalent to `addUnit(String, String, Factory)` (internal,
    /// `static synchronized`).
    pub fn add_unit(type_: &str, unit_name: &str, factory: &dyn Factory) -> MeasureUnit {
        let mut cache = lock_cache();
        let key = type_.to_string();
        let mut type_ = key.clone();
        match cache.get(&key) {
            None => {
                cache.insert(key.clone(), JavaHashMap::new());
            }
            Some(tmp) => {
                // "intern" the type by setting to first item's type.
                if let Some(first) = tmp.values().next() {
                    type_ = first.0.type_.clone().unwrap_or_default();
                }
            }
        }
        let tmp = cache.get_mut(&key).expect("the type key is present");
        if let Some(unit) = tmp.get(unit_name) {
            return unit.clone();
        }
        let unit = factory.create(&type_, unit_name);
        tmp.insert(unit_name.to_string(), unit.clone());
        unit
    }

    /// Equivalent to `writeReplace()` (`Currency` and `TimeUnit` replace
    /// themselves the same way).
    pub fn write_replace(&self) -> MeasureUnitProxy {
        MeasureUnitProxy::new(self.0.type_.clone(), self.0.sub_type.clone())
    }

    /// `getSingleUnitImpl()`.
    fn get_single_unit_impl(&self) -> Result<SingleUnitImpl> {
        match &self.0.measure_unit_impl {
            None => MeasureUnitImpl::for_identifier(&self.get_identifier())?.get_single_unit_impl(),
            Some(i) => i.get_single_unit_impl(),
        }
    }

    /// Equivalent to `getCopyOfMeasureUnitImpl()` (internal).
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an unparsable identifier.
    pub fn get_copy_of_measure_unit_impl(&self) -> Result<MeasureUnitImpl> {
        match &self.0.measure_unit_impl {
            None => MeasureUnitImpl::for_identifier(&self.get_identifier()),
            Some(i) => Ok(i.copy()),
        }
    }

    /// `getMaybeReferenceOfMeasureUnitImpl()`: the impl itself or a freshly
    /// parsed one.
    fn get_maybe_reference_of_measure_unit_impl(&self) -> Result<std::borrow::Cow<'_, MeasureUnitImpl>> {
        match &self.0.measure_unit_impl {
            None => Ok(std::borrow::Cow::Owned(MeasureUnitImpl::for_identifier(&self.get_identifier())?)),
            Some(i) => Ok(std::borrow::Cow::Borrowed(i)),
        }
    }
}

impl PartialEq for MeasureUnit {
    fn eq(&self, other: &Self) -> bool {
        self.java_equals(other)
    }
}

impl fmt::Display for MeasureUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

fn lock_cache() -> std::sync::MutexGuard<'static, JavaHashMap<JavaHashMap<MeasureUnit>>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The serialization proxy of a unit. Equivalent to
/// `MeasureUnit.MeasureUnitProxy` (`Externalizable`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeasureUnitProxy {
    type_: Option<String>,
    sub_type: Option<String>,
}

impl MeasureUnitProxy {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = -3910681415330989598;

    /// Equivalent to `MeasureUnitProxy(String, String)`.
    pub fn new(type_: Option<String>, sub_type: Option<String>) -> Self {
        Self { type_, sub_type }
    }

    /// Equivalent to `writeExternal(ObjectOutput)`: a version byte, the type
    /// and subtype as modified UTF-8, and a zero extra-length short.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream; `NullPointerException` for a missing
    /// type or subtype (`writeUTF(null)`); `UTFDataFormatException` for a
    /// string longer than 65535 encoded bytes.
    pub fn write_external(&self, out: &mut DataOutputStream<'_>) -> Result<()> {
        out.write_byte(0)?;
        write_utf(out, self.type_.as_deref())?;
        write_utf(out, self.sub_type.as_deref())?;
        out.write_short(0)?; // allow for more data.
        Ok(())
    }

    /// Equivalent to `readExternal(ObjectInput)`.
    ///
    /// # Errors
    ///
    /// The `IOException`/`EOFException`/`UTFDataFormatException` of the
    /// stream.
    pub fn read_external(&mut self, input: &mut dyn InputStream) -> Result<()> {
        read_u8(input)?; // version
        self.type_ = Some(read_utf(input)?);
        self.sub_type = Some(read_utf(input)?);
        // allow for more data from future version
        let extra = ((read_u8(input)? << 8) | read_u8(input)?) as i16 as i32;
        if extra > 0 {
            let mut extra_bytes = vec![0u8; extra as usize];
            input.read(&mut extra_bytes, 0, extra as usize).map_err(LuceneError::from)?;
        }
        Ok(())
    }

    /// Equivalent to `readResolve()`.
    ///
    /// # Errors
    ///
    /// The errors of `internalGetInstance` (`NullPointerException("Type and
    /// subType must be non-null")` for a proxy never read).
    pub fn read_resolve(&self) -> Result<MeasureUnit> {
        match (&self.type_, &self.sub_type) {
            (Some(t), Some(s)) => MeasureUnit::internal_get_instance(t, s),
            _ => Err(crate::jdk::null_pointer("Type and subType must be non-null")),
        }
    }
}

/// `DataOutput.writeUTF(String)`.
fn write_utf(out: &mut DataOutputStream<'_>, s: Option<&str>) -> Result<()> {
    let s = s.ok_or_else(|| LuceneError::null_pointer())?;
    let mut bytes: Vec<u8> = Vec::new();
    for c in s.encode_utf16() {
        let c = c as u32;
        if (0x0001..=0x007f).contains(&c) {
            bytes.push(c as u8);
        } else if c <= 0x07ff {
            bytes.push((0xc0 | ((c >> 6) & 0x1f)) as u8);
            bytes.push((0x80 | (c & 0x3f)) as u8);
        } else {
            bytes.push((0xe0 | ((c >> 12) & 0x0f)) as u8);
            bytes.push((0x80 | ((c >> 6) & 0x3f)) as u8);
            bytes.push((0x80 | (c & 0x3f)) as u8);
        }
    }
    if bytes.len() > 65535 {
        return Err(LuceneError::unchecked(
            "java.io.UTFDataFormatException",
            format!("encoded string ({}...{}) too long: {} bytes", truncate8(s, true), truncate8(s, false), bytes.len()),
        ));
    }
    out.write_short(bytes.len() as i32)?;
    for b in bytes {
        out.write_byte(b as i32)?;
    }
    Ok(())
}

/// The 8-char head or tail `DataOutputStream.writeUTF` quotes in its
/// too-long message.
fn truncate8(s: &str, head: bool) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    let part = if head { &u[..u.len().min(8)] } else { &u[u.len().saturating_sub(8)..] };
    String::from_utf16_lossy(part)
}

fn read_u8(input: &mut dyn InputStream) -> Result<i32> {
    let b = input.read_byte().map_err(LuceneError::from)?;
    if b < 0 {
        return Err(LuceneError::unchecked_with_message("java.io.EOFException", None));
    }
    Ok(b)
}

/// `DataInput.readUTF()`.
fn read_utf(input: &mut dyn InputStream) -> Result<String> {
    let utflen = ((read_u8(input)? << 8) | read_u8(input)?) as usize;
    let mut bytes = vec![0u8; utflen];
    for b in bytes.iter_mut() {
        *b = read_u8(input)? as u8;
    }
    let mut chars: Vec<u16> = Vec::with_capacity(utflen);
    let mut count = 0;
    let malformed = |at: usize| {
        LuceneError::unchecked("java.io.UTFDataFormatException", format!("malformed input around byte {at}"))
    };
    while count < utflen {
        let c = bytes[count] as u32;
        match c >> 4 {
            0..=7 => {
                count += 1;
                chars.push(c as u16);
            }
            12 | 13 => {
                count += 2;
                if count > utflen {
                    return Err(LuceneError::unchecked(
                        "java.io.UTFDataFormatException",
                        "malformed input: partial character at end",
                    ));
                }
                let c2 = bytes[count - 1] as u32;
                if (c2 & 0xc0) != 0x80 {
                    return Err(malformed(count));
                }
                chars.push((((c & 0x1f) << 6) | (c2 & 0x3f)) as u16);
            }
            14 => {
                count += 3;
                if count > utflen {
                    return Err(LuceneError::unchecked(
                        "java.io.UTFDataFormatException",
                        "malformed input: partial character at end",
                    ));
                }
                let c2 = bytes[count - 2] as u32;
                let c3 = bytes[count - 1] as u32;
                if (c2 & 0xc0) != 0x80 || (c3 & 0xc0) != 0x80 {
                    return Err(malformed(count - 1));
                }
                chars.push((((c & 0x0f) << 12) | ((c2 & 0x3f) << 6) | (c3 & 0x3f)) as u16);
            }
            _ => return Err(malformed(count)),
        }
    }
    Ok(String::from_utf16_lossy(&chars))
}

// Start generated MeasureUnit constants (Java: "All code between the
// "Start generated MeasureUnit constants" comment and the "End generated
// MeasureUnit constants" comment is auto generated code").

/// Constant for unit of acceleration: g-force
pub static G_FORCE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("acceleration", "g-force"));

/// Constant for unit of acceleration: meter-per-square-second
pub static METER_PER_SECOND_SQUARED: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("acceleration", "meter-per-square-second"));

/// Constant for unit of angle: arc-minute
pub static ARC_MINUTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("angle", "arc-minute"));

/// Constant for unit of angle: arc-second
pub static ARC_SECOND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("angle", "arc-second"));

/// Constant for unit of angle: degree
pub static DEGREE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("angle", "degree"));

/// Constant for unit of angle: radian
pub static RADIAN: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("angle", "radian"));

/// Constant for unit of angle: revolution
pub static REVOLUTION_ANGLE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("angle", "revolution"));

/// Constant for unit of area: acre
pub static ACRE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "acre"));

/// Constant for unit of area: dunam
pub static DUNAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "dunam"));

/// Constant for unit of area: hectare
pub static HECTARE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "hectare"));

/// Constant for unit of area: square-centimeter
pub static SQUARE_CENTIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-centimeter"));

/// Constant for unit of area: square-foot
pub static SQUARE_FOOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-foot"));

/// Constant for unit of area: square-inch
pub static SQUARE_INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-inch"));

/// Constant for unit of area: square-kilometer
pub static SQUARE_KILOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-kilometer"));

/// Constant for unit of area: square-meter
pub static SQUARE_METER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-meter"));

/// Constant for unit of area: square-mile
pub static SQUARE_MILE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-mile"));

/// Constant for unit of area: square-yard
pub static SQUARE_YARD: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("area", "square-yard"));

/// Constant for unit of concentr: item
pub static ITEM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "item"));

/// Constant for unit of concentr: karat
pub static KARAT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "karat"));

/// Constant for unit of concentr: milligram-ofglucose-per-deciliter
pub static MILLIGRAM_OFGLUCOSE_PER_DECILITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "milligram-ofglucose-per-deciliter"));

/// Constant for unit of concentr: milligram-per-deciliter
pub static MILLIGRAM_PER_DECILITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "milligram-per-deciliter"));

/// Constant for unit of concentr: millimole-per-liter
pub static MILLIMOLE_PER_LITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "millimole-per-liter"));

/// Constant for unit of concentr: mole
pub static MOLE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "mole"));

/// Constant for unit of concentr: percent
pub static PERCENT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "percent"));

/// Constant for unit of concentr: permille
pub static PERMILLE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "permille"));

/// Constant for unit of concentr: permillion
pub static PART_PER_MILLION: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "permillion"));

/// Constant for unit of concentr: permyriad
pub static PERMYRIAD: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "permyriad"));

/// Constant for unit of concentr: portion-per-1e9
pub static PORTION_PER_1E9: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("concentr", "portion-per-1e9"));

/// Constant for unit of consumption: liter-per-100-kilometer
pub static LITER_PER_100KILOMETERS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("consumption", "liter-per-100-kilometer"));

/// Constant for unit of consumption: liter-per-kilometer
pub static LITER_PER_KILOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("consumption", "liter-per-kilometer"));

/// Constant for unit of consumption: mile-per-gallon
pub static MILE_PER_GALLON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("consumption", "mile-per-gallon"));

/// Constant for unit of consumption: mile-per-gallon-imperial
pub static MILE_PER_GALLON_IMPERIAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("consumption", "mile-per-gallon-imperial"));

/// Constant for unit of digital: bit
pub static BIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "bit"));

/// Constant for unit of digital: byte
pub static BYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "byte"));

/// Constant for unit of digital: gigabit
pub static GIGABIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "gigabit"));

/// Constant for unit of digital: gigabyte
pub static GIGABYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "gigabyte"));

/// Constant for unit of digital: kilobit
pub static KILOBIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "kilobit"));

/// Constant for unit of digital: kilobyte
pub static KILOBYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "kilobyte"));

/// Constant for unit of digital: megabit
pub static MEGABIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "megabit"));

/// Constant for unit of digital: megabyte
pub static MEGABYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "megabyte"));

/// Constant for unit of digital: petabyte
pub static PETABYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "petabyte"));

/// Constant for unit of digital: terabit
pub static TERABIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "terabit"));

/// Constant for unit of digital: terabyte
pub static TERABYTE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("digital", "terabyte"));

/// Constant for unit of duration: century
pub static CENTURY: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "century"));

/// Constant for unit of duration: day
pub static DAY: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "day"));

/// Constant for unit of duration: day-person
pub static DAY_PERSON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "day-person"));

/// Constant for unit of duration: decade
pub static DECADE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "decade"));

/// Constant for unit of duration: hour
pub static HOUR: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "hour"));

/// Constant for unit of duration: microsecond
pub static MICROSECOND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "microsecond"));

/// Constant for unit of duration: millisecond
pub static MILLISECOND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "millisecond"));

/// Constant for unit of duration: minute
pub static MINUTE: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "minute"));

/// Constant for unit of duration: month
pub static MONTH: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "month"));

/// Constant for unit of duration: month-person
pub static MONTH_PERSON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "month-person"));

/// Constant for unit of duration: nanosecond
pub static NANOSECOND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "nanosecond"));

/// Constant for unit of duration: night
pub static NIGHT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "night"));

/// Constant for unit of duration: quarter
pub static QUARTER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "quarter"));

/// Constant for unit of duration: second
pub static SECOND: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "second"));

/// Constant for unit of duration: week
pub static WEEK: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "week"));

/// Constant for unit of duration: week-person
pub static WEEK_PERSON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "week-person"));

/// Constant for unit of duration: year
pub static YEAR: LazyLock<TimeUnit> = LazyLock::new(|| TimeUnit::constant("duration", "year"));

/// Constant for unit of duration: year-person
pub static YEAR_PERSON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("duration", "year-person"));

/// Constant for unit of electric: ampere
pub static AMPERE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("electric", "ampere"));

/// Constant for unit of electric: milliampere
pub static MILLIAMPERE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("electric", "milliampere"));

/// Constant for unit of electric: ohm
pub static OHM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("electric", "ohm"));

/// Constant for unit of electric: volt
pub static VOLT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("electric", "volt"));

/// Constant for unit of energy: british-thermal-unit
pub static BRITISH_THERMAL_UNIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "british-thermal-unit"));

/// Constant for unit of energy: calorie
pub static CALORIE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "calorie"));

/// Constant for unit of energy: electronvolt
pub static ELECTRONVOLT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "electronvolt"));

/// Constant for unit of energy: foodcalorie
pub static FOODCALORIE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "foodcalorie"));

/// Constant for unit of energy: joule
pub static JOULE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "joule"));

/// Constant for unit of energy: kilocalorie
pub static KILOCALORIE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "kilocalorie"));

/// Constant for unit of energy: kilojoule
pub static KILOJOULE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "kilojoule"));

/// Constant for unit of energy: kilowatt-hour
pub static KILOWATT_HOUR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "kilowatt-hour"));

/// Constant for unit of energy: therm-us
pub static THERM_US: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("energy", "therm-us"));

/// Constant for unit of force: kilowatt-hour-per-100-kilometer
pub static KILOWATT_HOUR_PER_100_KILOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("force", "kilowatt-hour-per-100-kilometer"));

/// Constant for unit of force: newton
pub static NEWTON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("force", "newton"));

/// Constant for unit of force: pound-force
pub static POUND_FORCE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("force", "pound-force"));

/// Constant for unit of frequency: gigahertz
pub static GIGAHERTZ: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("frequency", "gigahertz"));

/// Constant for unit of frequency: hertz
pub static HERTZ: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("frequency", "hertz"));

/// Constant for unit of frequency: kilohertz
pub static KILOHERTZ: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("frequency", "kilohertz"));

/// Constant for unit of frequency: megahertz
pub static MEGAHERTZ: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("frequency", "megahertz"));

/// Constant for unit of graphics: dot
pub static DOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "dot"));

/// Constant for unit of graphics: dot-per-centimeter
pub static DOT_PER_CENTIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "dot-per-centimeter"));

/// Constant for unit of graphics: dot-per-inch
pub static DOT_PER_INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "dot-per-inch"));

/// Constant for unit of graphics: em
pub static EM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "em"));

/// Constant for unit of graphics: megapixel
pub static MEGAPIXEL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "megapixel"));

/// Constant for unit of graphics: pixel
pub static PIXEL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "pixel"));

/// Constant for unit of graphics: pixel-per-centimeter
pub static PIXEL_PER_CENTIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "pixel-per-centimeter"));

/// Constant for unit of graphics: pixel-per-inch
pub static PIXEL_PER_INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("graphics", "pixel-per-inch"));

/// Constant for unit of length: astronomical-unit
pub static ASTRONOMICAL_UNIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "astronomical-unit"));

/// Constant for unit of length: centimeter
pub static CENTIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "centimeter"));

/// Constant for unit of length: decimeter
pub static DECIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "decimeter"));

/// Constant for unit of length: earth-radius
pub static EARTH_RADIUS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "earth-radius"));

/// Constant for unit of length: fathom
pub static FATHOM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "fathom"));

/// Constant for unit of length: foot
pub static FOOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "foot"));

/// Constant for unit of length: furlong
pub static FURLONG: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "furlong"));

/// Constant for unit of length: inch
pub static INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "inch"));

/// Constant for unit of length: kilometer
pub static KILOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "kilometer"));

/// Constant for unit of length: light-year
pub static LIGHT_YEAR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "light-year"));

/// Constant for unit of length: meter
pub static METER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "meter"));

/// Constant for unit of length: micrometer
pub static MICROMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "micrometer"));

/// Constant for unit of length: mile
pub static MILE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "mile"));

/// Constant for unit of length: mile-scandinavian
pub static MILE_SCANDINAVIAN: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "mile-scandinavian"));

/// Constant for unit of length: millimeter
pub static MILLIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "millimeter"));

/// Constant for unit of length: nanometer
pub static NANOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "nanometer"));

/// Constant for unit of length: nautical-mile
pub static NAUTICAL_MILE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "nautical-mile"));

/// Constant for unit of length: parsec
pub static PARSEC: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "parsec"));

/// Constant for unit of length: picometer
pub static PICOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "picometer"));

/// Constant for unit of length: point
pub static POINT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "point"));

/// Constant for unit of length: solar-radius
pub static SOLAR_RADIUS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "solar-radius"));

/// Constant for unit of length: yard
pub static YARD: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("length", "yard"));

/// Constant for unit of light: candela
pub static CANDELA: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("light", "candela"));

/// Constant for unit of light: lumen
pub static LUMEN: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("light", "lumen"));

/// Constant for unit of light: lux
pub static LUX: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("light", "lux"));

/// Constant for unit of light: solar-luminosity
pub static SOLAR_LUMINOSITY: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("light", "solar-luminosity"));

/// Constant for unit of mass: carat
pub static CARAT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "carat"));

/// Constant for unit of mass: dalton
pub static DALTON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "dalton"));

/// Constant for unit of mass: earth-mass
pub static EARTH_MASS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "earth-mass"));

/// Constant for unit of mass: grain
pub static GRAIN: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "grain"));

/// Constant for unit of mass: gram
pub static GRAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "gram"));

/// Constant for unit of mass: kilogram
pub static KILOGRAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "kilogram"));

/// Constant for unit of mass: metric-ton (renamed to tonne in CLDR 42 / ICU 72).
pub static METRIC_TON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "tonne"));

/// Constant for unit of mass: microgram
pub static MICROGRAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "microgram"));

/// Constant for unit of mass: milligram
pub static MILLIGRAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "milligram"));

/// Constant for unit of mass: ounce
pub static OUNCE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "ounce"));

/// Constant for unit of mass: ounce-troy
pub static OUNCE_TROY: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "ounce-troy"));

/// Constant for unit of mass: pound
pub static POUND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "pound"));

/// Constant for unit of mass: solar-mass
pub static SOLAR_MASS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "solar-mass"));

/// Constant for unit of mass: stone
pub static STONE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "stone"));

/// Constant for unit of mass: ton
pub static TON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "ton"));

/// Constant for unit of mass: tonne
pub static TONNE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("mass", "tonne"));

/// Constant for unit of power: gigawatt
pub static GIGAWATT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "gigawatt"));

/// Constant for unit of power: horsepower
pub static HORSEPOWER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "horsepower"));

/// Constant for unit of power: kilowatt
pub static KILOWATT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "kilowatt"));

/// Constant for unit of power: megawatt
pub static MEGAWATT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "megawatt"));

/// Constant for unit of power: milliwatt
pub static MILLIWATT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "milliwatt"));

/// Constant for unit of power: watt
pub static WATT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("power", "watt"));

/// Constant for unit of pressure: atmosphere
pub static ATMOSPHERE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "atmosphere"));

/// Constant for unit of pressure: bar
pub static BAR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "bar"));

/// Constant for unit of pressure: gasoline-energy-density
pub static GASOLINE_ENERGY_DENSITY: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "gasoline-energy-density"));

/// Constant for unit of pressure: hectopascal
pub static HECTOPASCAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "hectopascal"));

/// Constant for unit of pressure: inch-ofhg
pub static INCH_HG: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "inch-ofhg"));

/// Constant for unit of pressure: kilopascal
pub static KILOPASCAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "kilopascal"));

/// Constant for unit of pressure: megapascal
pub static MEGAPASCAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "megapascal"));

/// Constant for unit of pressure: millibar
pub static MILLIBAR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "millibar"));

/// Constant for unit of pressure: millimeter-ofhg
pub static MILLIMETER_OF_MERCURY: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "millimeter-ofhg"));

/// Constant for unit of pressure: pascal
pub static PASCAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "pascal"));

/// Constant for unit of pressure: pound-force-per-square-inch
pub static POUND_PER_SQUARE_INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("pressure", "pound-force-per-square-inch"));

/// Constant for unit of speed: beaufort
pub static BEAUFORT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "beaufort"));

/// Constant for unit of speed: kilometer-per-hour
pub static KILOMETER_PER_HOUR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "kilometer-per-hour"));

/// Constant for unit of speed: knot
pub static KNOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "knot"));

/// Constant for unit of speed: light-speed
pub static LIGHT_SPEED: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "light-speed"));

/// Constant for unit of speed: meter-per-second
pub static METER_PER_SECOND: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "meter-per-second"));

/// Constant for unit of speed: mile-per-hour
pub static MILE_PER_HOUR: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("speed", "mile-per-hour"));

/// Constant for unit of temperature: celsius
pub static CELSIUS: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("temperature", "celsius"));

/// Constant for unit of temperature: fahrenheit
pub static FAHRENHEIT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("temperature", "fahrenheit"));

/// Constant for unit of temperature: generic
pub static GENERIC_TEMPERATURE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("temperature", "generic"));

/// Constant for unit of temperature: kelvin
pub static KELVIN: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("temperature", "kelvin"));

/// Constant for unit of torque: newton-meter
pub static NEWTON_METER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("torque", "newton-meter"));

/// Constant for unit of torque: pound-force-foot
pub static POUND_FOOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("torque", "pound-force-foot"));

/// Constant for unit of volume: acre-foot
pub static ACRE_FOOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "acre-foot"));

/// Constant for unit of volume: barrel
pub static BARREL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "barrel"));

/// Constant for unit of volume: bushel
pub static BUSHEL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "bushel"));

/// Constant for unit of volume: centiliter
pub static CENTILITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "centiliter"));

/// Constant for unit of volume: cubic-centimeter
pub static CUBIC_CENTIMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-centimeter"));

/// Constant for unit of volume: cubic-foot
pub static CUBIC_FOOT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-foot"));

/// Constant for unit of volume: cubic-inch
pub static CUBIC_INCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-inch"));

/// Constant for unit of volume: cubic-kilometer
pub static CUBIC_KILOMETER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-kilometer"));

/// Constant for unit of volume: cubic-meter
pub static CUBIC_METER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-meter"));

/// Constant for unit of volume: cubic-mile
pub static CUBIC_MILE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-mile"));

/// Constant for unit of volume: cubic-yard
pub static CUBIC_YARD: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cubic-yard"));

/// Constant for unit of volume: cup
pub static CUP: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cup"));

/// Constant for unit of volume: cup-metric
pub static CUP_METRIC: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "cup-metric"));

/// Constant for unit of volume: deciliter
pub static DECILITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "deciliter"));

/// Constant for unit of volume: dessert-spoon
pub static DESSERT_SPOON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "dessert-spoon"));

/// Constant for unit of volume: dessert-spoon-imperial
pub static DESSERT_SPOON_IMPERIAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "dessert-spoon-imperial"));

/// Constant for unit of volume: dram
pub static DRAM: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "dram"));

/// Constant for unit of volume: drop
pub static DROP: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "drop"));

/// Constant for unit of volume: fluid-ounce
pub static FLUID_OUNCE: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "fluid-ounce"));

/// Constant for unit of volume: fluid-ounce-imperial
pub static FLUID_OUNCE_IMPERIAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "fluid-ounce-imperial"));

/// Constant for unit of volume: gallon
pub static GALLON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "gallon"));

/// Constant for unit of volume: gallon-imperial
pub static GALLON_IMPERIAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "gallon-imperial"));

/// Constant for unit of volume: hectoliter
pub static HECTOLITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "hectoliter"));

/// Constant for unit of volume: jigger
pub static JIGGER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "jigger"));

/// Constant for unit of volume: liter
pub static LITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "liter"));

/// Constant for unit of volume: megaliter
pub static MEGALITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "megaliter"));

/// Constant for unit of volume: milliliter
pub static MILLILITER: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "milliliter"));

/// Constant for unit of volume: pinch
pub static PINCH: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "pinch"));

/// Constant for unit of volume: pint
pub static PINT: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "pint"));

/// Constant for unit of volume: pint-metric
pub static PINT_METRIC: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "pint-metric"));

/// Constant for unit of volume: quart
pub static QUART: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "quart"));

/// Constant for unit of volume: quart-imperial
pub static QUART_IMPERIAL: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "quart-imperial"));

/// Constant for unit of volume: tablespoon
pub static TABLESPOON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "tablespoon"));

/// Constant for unit of volume: teaspoon
pub static TEASPOON: LazyLock<MeasureUnit> = LazyLock::new(|| MeasureUnit::constant("volume", "teaspoon"));

// End generated MeasureUnit constants
