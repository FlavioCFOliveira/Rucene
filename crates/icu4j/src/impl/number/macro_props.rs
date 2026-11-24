// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MacroProps`.

use std::sync::Arc;

use rucene::util::java_collections::{
    java_boolean_hash_code, java_identity_hash_code_stand_in, java_string_hash_code,
};

use crate::jdk::math::RoundingMode;
use crate::number::integer_width::IntegerWidth;
use crate::number::notation::Notation;
use crate::number::number_formatter::{
    DecimalSeparatorDisplay, GroupingStrategy, SignDisplay, UnitWidth,
};
use crate::number::precision::Precision;
use crate::number::scale::Scale;
use crate::r#impl::number::affix_pattern_provider::AffixPatternProvider;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::padder::Padder;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::numbering_system::NumberingSystem;
use crate::text::plural_rules::PluralRules;
use crate::util::measure_unit::MeasureUnit;
use crate::util::u_locale::ULocale;

/// The `Object` value of `MacroProps.grouping`: a `GroupingStrategy` or a
/// `Grouper`.
#[derive(Debug, Clone)]
pub enum GroupingObject {
    /// A `NumberFormatter.GroupingStrategy`.
    Strategy(GroupingStrategy),
    /// A `Grouper`.
    Grouper(Grouper),
}

impl GroupingObject {
    /// `Objects.equals(a, b)`: enum identity, `Grouper` identity.
    fn java_equals(&self, other: &GroupingObject) -> bool {
        match (self, other) {
            (GroupingObject::Strategy(a), GroupingObject::Strategy(b)) => a == b,
            (GroupingObject::Grouper(a), GroupingObject::Grouper(b)) => a.ptr_eq(b),
            _ => false,
        }
    }

    /// `hashCode()`.
    fn hash_code(&self) -> i32 {
        match self {
            GroupingObject::Strategy(s) => s.hash_code(),
            GroupingObject::Grouper(g) => g.hash_code(),
        }
    }
}

/// The `Object` value of `MacroProps.symbols`: a `DecimalFormatSymbols` or a
/// `NumberingSystem`.
#[derive(Debug, Clone)]
pub enum SymbolsObject {
    /// A `DecimalFormatSymbols`.
    Symbols(Arc<DecimalFormatSymbols>),
    /// A `NumberingSystem`.
    NumberingSystem(Arc<NumberingSystem>),
}

impl SymbolsObject {
    /// `Objects.equals(a, b)`: `DecimalFormatSymbols.equals`, and the
    /// identity of `NumberingSystem` (which does not override `equals`).
    fn java_equals(&self, other: &SymbolsObject) -> bool {
        match (self, other) {
            (SymbolsObject::Symbols(a), SymbolsObject::Symbols(b)) => a.java_equals(b),
            (SymbolsObject::NumberingSystem(a), SymbolsObject::NumberingSystem(b)) => {
                Arc::ptr_eq(a, b)
            }
            _ => false,
        }
    }

    /// `hashCode()`.
    fn hash_code(&self) -> i32 {
        match self {
            SymbolsObject::Symbols(s) => s.hash_code(),
            SymbolsObject::NumberingSystem(_) => {
                java_identity_hash_code_stand_in("com.ibm.icu.text.NumberingSystem")
            }
        }
    }
}

/// The settings of a number formatter, as resolved from the
/// `NumberFormatterSettings` chain.
///
/// Equivalent to `com.ibm.icu.impl.number.MacroProps` (public mutable
/// fields; `null` is `None`). `clone()` is `Clone`; `equals(Object)` is
/// [`java_equals`](Self::java_equals) and `hashCode()`
/// [`hash_code`](Self::hash_code). The identity-compared field objects
/// (`Notation`, `Precision`, `Padder`, `IntegerWidth`, `Scale`, `Grouper`,
/// the affix provider) compare by identity, as `Objects.equals` does on
/// them in Java.
#[derive(Debug, Clone, Default)]
pub struct MacroProps {
    /// `notation`.
    pub notation: Option<Notation>,
    /// `unit` (`NoUnit.BASE` is `None`).
    pub unit: Option<MeasureUnit>,
    /// `perUnit`.
    pub per_unit: Option<MeasureUnit>,
    /// `precision`.
    pub precision: Option<Precision>,
    /// `roundingMode`.
    pub rounding_mode: Option<RoundingMode>,
    /// `grouping`.
    pub grouping: Option<GroupingObject>,
    /// `padder`.
    pub padder: Option<Padder>,
    /// `integerWidth`.
    pub integer_width: Option<IntegerWidth>,
    /// `symbols`.
    pub symbols: Option<SymbolsObject>,
    /// `unitWidth`.
    pub unit_width: Option<UnitWidth>,
    /// `unitDisplayCase`.
    pub unit_display_case: Option<String>,
    /// `sign`.
    pub sign: Option<SignDisplay>,
    /// `approximately`.
    pub approximately: Option<bool>,
    /// `decimal`.
    pub decimal: Option<DecimalSeparatorDisplay>,
    /// `scale`.
    pub scale: Option<Scale>,
    /// `usage`.
    pub usage: Option<String>,
    /// `affixProvider`.
    pub affix_provider: Option<Arc<dyn AffixPatternProvider>>,
    /// `rules`.
    pub rules: Option<Arc<PluralRules>>,
    /// `threshold`.
    pub threshold: Option<i64>,

    /// `loc`.
    pub loc: Option<ULocale>,
}

/// `Objects.equals` for two optional values with an equality.
fn eq_opt<T>(a: &Option<T>, b: &Option<T>, eq: impl Fn(&T, &T) -> bool) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => eq(a, b),
        _ => false,
    }
}

/// The hash code of an enum constant (`Object.hashCode`, identity): see
/// `java_identity_hash_code_stand_in`.
pub(crate) fn enum_hash_code(class: &str, constant: &str) -> i32 {
    java_identity_hash_code_stand_in(&format!("{class}${constant}"))
}

impl MacroProps {
    /// Copies values from fallback into this instance if they are null in
    /// this instance. Equivalent to `fallback(MacroProps)`.
    pub fn fallback(&mut self, fallback: &MacroProps) {
        if self.notation.is_none() {
            self.notation = fallback.notation.clone();
        }
        if self.unit.is_none() {
            self.unit = fallback.unit.clone();
        }
        if self.per_unit.is_none() {
            self.per_unit = fallback.per_unit.clone();
        }
        if self.precision.is_none() {
            self.precision = fallback.precision.clone();
        }
        if self.rounding_mode.is_none() {
            self.rounding_mode = fallback.rounding_mode;
        }
        if self.grouping.is_none() {
            self.grouping = fallback.grouping.clone();
        }
        if self.padder.is_none() {
            self.padder = fallback.padder.clone();
        }
        if self.integer_width.is_none() {
            self.integer_width = fallback.integer_width.clone();
        }
        if self.symbols.is_none() {
            self.symbols = fallback.symbols.clone();
        }
        if self.unit_width.is_none() {
            self.unit_width = fallback.unit_width;
        }
        if self.unit_display_case.is_none() {
            self.unit_display_case = fallback.unit_display_case.clone();
        }
        if self.sign.is_none() {
            self.sign = fallback.sign;
        }
        if self.approximately.is_none() {
            self.approximately = fallback.approximately;
        }
        if self.decimal.is_none() {
            self.decimal = fallback.decimal;
        }
        if self.affix_provider.is_none() {
            self.affix_provider = fallback.affix_provider.clone();
        }
        if self.scale.is_none() {
            self.scale = fallback.scale.clone();
        }
        if self.usage.is_none() {
            self.usage = fallback.usage.clone();
        }
        if self.rules.is_none() {
            self.rules = fallback.rules.clone();
        }
        if self.loc.is_none() {
            self.loc = fallback.loc.clone();
        }
    }

    /// Equivalent to `hashCode()`: `Objects.hash` over the fields
    /// (`threshold` excluded).
    pub fn hash_code(&self) -> i32 {
        let hashes = [
            self.notation.as_ref().map_or(0, Notation::hash_code),
            self.unit.as_ref().map_or(0, MeasureUnit::hash_code),
            self.per_unit.as_ref().map_or(0, MeasureUnit::hash_code),
            self.precision.as_ref().map_or(0, Precision::hash_code),
            self.rounding_mode
                .map_or(0, |r| enum_hash_code("java.math.RoundingMode", r.name())),
            self.grouping.as_ref().map_or(0, GroupingObject::hash_code),
            self.padder.as_ref().map_or(0, Padder::hash_code),
            self.integer_width
                .as_ref()
                .map_or(0, IntegerWidth::hash_code),
            self.symbols.as_ref().map_or(0, SymbolsObject::hash_code),
            self.unit_width.map_or(0, UnitWidth::hash_code),
            self.unit_display_case
                .as_deref()
                .map_or(0, java_string_hash_code),
            self.sign.map_or(0, SignDisplay::hash_code),
            self.approximately.map_or(0, java_boolean_hash_code),
            self.decimal.map_or(0, DecimalSeparatorDisplay::hash_code),
            // The providers do not override hashCode (identity).
            self.affix_provider.as_ref().map_or(0, |_| {
                java_identity_hash_code_stand_in("com.ibm.icu.impl.number.AffixPatternProvider")
            }),
            self.scale.as_ref().map_or(0, Scale::hash_code),
            self.usage.as_deref().map_or(0, java_string_hash_code),
            self.rules.as_ref().map_or(0, |r| r.hash_code()),
            self.loc.as_ref().map_or(0, ULocale::hash_code),
        ];
        let mut result: i32 = 1;
        for h in hashes {
            result = result.wrapping_mul(31).wrapping_add(h);
        }
        result
    }

    /// Equivalent to `equals(Object)`.
    pub fn java_equals(&self, other: &MacroProps) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        eq_opt(&self.notation, &other.notation, Notation::ptr_eq)
            && eq_opt(&self.unit, &other.unit, MeasureUnit::java_equals)
            && eq_opt(&self.per_unit, &other.per_unit, MeasureUnit::java_equals)
            && eq_opt(&self.precision, &other.precision, Precision::ptr_eq)
            && self.rounding_mode == other.rounding_mode
            && eq_opt(&self.grouping, &other.grouping, GroupingObject::java_equals)
            && eq_opt(&self.padder, &other.padder, Padder::ptr_eq)
            && eq_opt(
                &self.integer_width,
                &other.integer_width,
                IntegerWidth::ptr_eq,
            )
            && eq_opt(&self.symbols, &other.symbols, SymbolsObject::java_equals)
            && self.unit_width == other.unit_width
            && self.unit_display_case == other.unit_display_case
            && self.sign == other.sign
            && self.approximately == other.approximately
            && self.decimal == other.decimal
            && eq_opt(&self.affix_provider, &other.affix_provider, |a, b| {
                std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
            })
            && eq_opt(&self.scale, &other.scale, Scale::ptr_eq)
            && self.usage == other.usage
            && eq_opt(&self.rules, &other.rules, |a, b| a.equals(Some(b)))
            && eq_opt(&self.loc, &other.loc, |a, b| a == b)
    }
}
