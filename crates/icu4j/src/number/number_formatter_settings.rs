// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberFormatterSettings`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::jdk::math::RoundingMode;
use crate::number::integer_width::IntegerWidth;
use crate::number::notation::Notation;
use crate::number::number_formatter::{
    DecimalSeparatorDisplay, GroupingStrategy, SignDisplay, UnitWidth,
};
use crate::number::number_skeleton_impl::NumberSkeletonImpl;
use crate::number::precision::Precision;
use crate::number::scale::Scale;
use crate::r#impl::number::macro_props::{GroupingObject, MacroProps, SymbolsObject};
use crate::r#impl::number::padder::Padder;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::display_options::{DisplayOptions, GrammaticalCase};
use crate::text::numbering_system::NumberingSystem;
use crate::util::measure_unit::MeasureUnit;
use crate::util::u_locale::ULocale;

/// `KEY_MACROS`.
pub(crate) const KEY_MACROS: i32 = 0;
/// `KEY_LOCALE`.
pub(crate) const KEY_LOCALE: i32 = 1;
/// `KEY_NOTATION`.
pub(crate) const KEY_NOTATION: i32 = 2;
/// `KEY_UNIT`.
pub(crate) const KEY_UNIT: i32 = 3;
/// `KEY_PRECISION`.
pub(crate) const KEY_PRECISION: i32 = 4;
/// `KEY_ROUNDING_MODE`.
pub(crate) const KEY_ROUNDING_MODE: i32 = 5;
/// `KEY_GROUPING`.
pub(crate) const KEY_GROUPING: i32 = 6;
/// `KEY_PADDER`.
pub(crate) const KEY_PADDER: i32 = 7;
/// `KEY_INTEGER`.
pub(crate) const KEY_INTEGER: i32 = 8;
/// `KEY_SYMBOLS`.
pub(crate) const KEY_SYMBOLS: i32 = 9;
/// `KEY_UNIT_WIDTH`.
pub(crate) const KEY_UNIT_WIDTH: i32 = 10;
/// `KEY_SIGN`.
pub(crate) const KEY_SIGN: i32 = 11;
/// `KEY_DECIMAL`.
pub(crate) const KEY_DECIMAL: i32 = 12;
/// `KEY_SCALE`.
pub(crate) const KEY_SCALE: i32 = 13;
/// `KEY_THRESHOLD`.
pub(crate) const KEY_THRESHOLD: i32 = 14;
/// `KEY_PER_UNIT`.
pub(crate) const KEY_PER_UNIT: i32 = 15;
/// `KEY_USAGE`.
pub(crate) const KEY_USAGE: i32 = 16;
/// `KEY_UNIT_DISPLAY_CASE`.
pub(crate) const KEY_UNIT_DISPLAY_CASE: i32 = 17;
/// `KEY_MAX` (unused in Java too).
#[allow(dead_code)]
pub(crate) const KEY_MAX: i32 = 18;

/// The `Object value` of a settings link (`None` is Java's `null`); the
/// variant is the class `resolve()` casts the value to for the link's key.
/// Internal: Java's `create(int, Object)` is package-private.
#[derive(Debug, Clone)]
pub enum SettingValue {
    /// `KEY_MACROS`: a `MacroProps`.
    Macros(Arc<MacroProps>),
    /// `KEY_LOCALE`: a `ULocale`.
    Locale(ULocale),
    /// `KEY_NOTATION`.
    Notation(Notation),
    /// `KEY_UNIT` and `KEY_PER_UNIT`: a `MeasureUnit`.
    Unit(MeasureUnit),
    /// `KEY_PRECISION`.
    Precision(Precision),
    /// `KEY_ROUNDING_MODE`.
    RoundingMode(RoundingMode),
    /// `KEY_GROUPING`: a `GroupingStrategy` (or a `Grouper`).
    Grouping(GroupingObject),
    /// `KEY_PADDER`.
    Padder(Padder),
    /// `KEY_INTEGER`.
    IntegerWidth(IntegerWidth),
    /// `KEY_SYMBOLS`: a `DecimalFormatSymbols` or a `NumberingSystem`.
    Symbols(SymbolsObject),
    /// `KEY_UNIT_WIDTH`.
    UnitWidth(UnitWidth),
    /// `KEY_SIGN`.
    Sign(SignDisplay),
    /// `KEY_DECIMAL`.
    Decimal(DecimalSeparatorDisplay),
    /// `KEY_SCALE`.
    Scale(Scale),
    /// `KEY_THRESHOLD`: a `Long`.
    Threshold(i64),
    /// `KEY_USAGE` and `KEY_UNIT_DISPLAY_CASE`: a `String`.
    String(String),
}

/// One link of a settings chain: the fields `parent`, `key`, `value` and
/// `resolvedMacros` of a `NumberFormatterSettings` object.
#[derive(Debug)]
pub struct SettingsNode {
    /// `parent`.
    parent: Option<Arc<SettingsNode>>,
    /// `key`.
    key: i32,
    /// `value`.
    value: Option<SettingValue>,
    /// `resolvedMacros` (`volatile`, computed once).
    resolved_macros: Mutex<Option<Arc<MacroProps>>>,
}

impl SettingsNode {
    /// `NumberFormatterSettings(NumberFormatterSettings<?>, int, Object)`.
    pub(crate) fn new(
        parent: Option<Arc<SettingsNode>>,
        key: i32,
        value: Option<SettingValue>,
    ) -> Arc<SettingsNode> {
        Arc::new(SettingsNode {
            parent,
            key,
            value,
            resolved_macros: Mutex::new(None),
        })
    }

    /// Equivalent to `resolve()` (package-private): the macros of the chain,
    /// the latest setting of each key winning.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError("Unknown key: <key>")` for a key outside the
    /// defined ones.
    pub(crate) fn resolve(&self) -> Arc<MacroProps> {
        let mut cache = self
            .resolved_macros
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache
            .get_or_insert_with(|| Arc::new(self.compute_macros()))
            .clone()
    }

    /// `resolve().loc = loc` (`NumberRangeFormatterSettings.resolve` copies
    /// the range locale into the formatter's resolved macros).
    ///
    /// **Divergence.** Java assigns the field of the cached `MacroProps`
    /// object, visible to every holder of it; the port replaces the cached
    /// value, which later `resolve()` calls answer (the only readers).
    pub(crate) fn set_resolved_loc(&self, loc: Option<ULocale>) {
        let mut cache = self
            .resolved_macros
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut macros = (**cache.get_or_insert_with(|| Arc::new(self.compute_macros()))).clone();
        macros.loc = loc;
        *cache = Some(Arc::new(macros));
    }

    fn compute_macros(&self) -> MacroProps {
        // Although the linked-list fluent storage approach requires this
        // method, my benchmarks show that linked-list is still faster than a
        // full object copy whenever a setter is called. Therefore, the
        // fluent setter methods are quicker than they would be otherwise.
        let mut macros = MacroProps::default();
        // Bitmap: 1 if seen; 0 if unseen
        let mut seen: i64 = 0;
        let mut current: Option<&SettingsNode> = Some(self);
        while let Some(node) = current {
            let key_bitmask = 1i64 << node.key;
            if 0 != (seen & key_bitmask) {
                current = node.parent.as_deref();
                continue;
            }
            seen |= key_bitmask;
            let v = node.value.clone();
            match node.key {
                KEY_MACROS => {
                    // Java NPE for a null MacroProps.
                    let m = match v {
                        Some(SettingValue::Macros(m)) => m,
                        _ => panic!("java.lang.NullPointerException: macros"),
                    };
                    macros.fallback(&m);
                }
                KEY_LOCALE => {
                    macros.loc = match v {
                        Some(SettingValue::Locale(l)) => Some(l),
                        _ => None,
                    }
                }
                KEY_NOTATION => {
                    macros.notation = match v {
                        Some(SettingValue::Notation(n)) => Some(n),
                        _ => None,
                    }
                }
                KEY_UNIT => {
                    macros.unit = match v {
                        Some(SettingValue::Unit(u)) => Some(u),
                        _ => None,
                    }
                }
                KEY_PRECISION => {
                    macros.precision = match v {
                        Some(SettingValue::Precision(p)) => Some(p),
                        _ => None,
                    }
                }
                KEY_ROUNDING_MODE => {
                    macros.rounding_mode = match v {
                        Some(SettingValue::RoundingMode(r)) => Some(r),
                        _ => None,
                    }
                }
                KEY_GROUPING => {
                    macros.grouping = match v {
                        Some(SettingValue::Grouping(g)) => Some(g),
                        _ => None,
                    }
                }
                KEY_PADDER => {
                    macros.padder = match v {
                        Some(SettingValue::Padder(p)) => Some(p),
                        _ => None,
                    }
                }
                KEY_INTEGER => {
                    macros.integer_width = match v {
                        Some(SettingValue::IntegerWidth(i)) => Some(i),
                        _ => None,
                    }
                }
                KEY_SYMBOLS => {
                    macros.symbols = match v {
                        Some(SettingValue::Symbols(s)) => Some(s),
                        _ => None,
                    }
                }
                KEY_UNIT_WIDTH => {
                    macros.unit_width = match v {
                        Some(SettingValue::UnitWidth(u)) => Some(u),
                        _ => None,
                    }
                }
                KEY_SIGN => {
                    macros.sign = match v {
                        Some(SettingValue::Sign(s)) => Some(s),
                        _ => None,
                    }
                }
                KEY_DECIMAL => {
                    macros.decimal = match v {
                        Some(SettingValue::Decimal(d)) => Some(d),
                        _ => None,
                    }
                }
                KEY_SCALE => {
                    macros.scale = match v {
                        Some(SettingValue::Scale(s)) => Some(s),
                        _ => None,
                    }
                }
                KEY_THRESHOLD => {
                    macros.threshold = match v {
                        Some(SettingValue::Threshold(t)) => Some(t),
                        _ => None,
                    }
                }
                KEY_PER_UNIT => {
                    macros.per_unit = match v {
                        Some(SettingValue::Unit(u)) => Some(u),
                        _ => None,
                    }
                }
                KEY_USAGE => {
                    macros.usage = match v {
                        Some(SettingValue::String(s)) => Some(s),
                        _ => None,
                    }
                }
                KEY_UNIT_DISPLAY_CASE => {
                    macros.unit_display_case = match v {
                        Some(SettingValue::String(s)) => Some(s),
                        _ => None,
                    }
                }
                key => panic!("java.lang.AssertionError: Unknown key: {key}"),
            }
            current = node.parent.as_deref();
        }
        macros
    }
}

/// An abstract base class for specifying settings related to number
/// formatting. This class is implemented by
/// `UnlocalizedNumberFormatter` and `LocalizedNumberFormatter`. This class
/// is not intended for public subclassing.
///
/// Equivalent to `com.ibm.icu.number.NumberFormatterSettings<T>`: the
/// implementing types hold a [`SettingsNode`] (`node`) and create their own
/// type from a new link (`create`); every fluent setter answers a new
/// object and leaves this one unchanged. The setters take
/// `impl Into<Option<_>>` so that Java's `null` arguments (`None`) remain
/// expressible. `equals(Object)` is
/// [`java_equals`](Self::java_equals), `hashCode()`
/// [`hash_code`](Self::hash_code).
pub trait NumberFormatterSettings: Sized {
    /// The link of this object.
    fn node(&self) -> &Arc<SettingsNode>;

    /// Equivalent to `create(int, Object)` (package-private, abstract).
    fn create(&self, key: i32, value: Option<SettingValue>) -> Self;

    /// Specifies the notation style (simple, scientific, or compact) for
    /// rendering numbers. Equivalent to `notation(Notation)`.
    fn notation(&self, notation: impl Into<Option<Notation>>) -> Self {
        self.create(KEY_NOTATION, notation.into().map(SettingValue::Notation))
    }

    /// Specifies the unit (unit of measure, currency, or percent) to
    /// associate with rendered numbers. Equivalent to `unit(MeasureUnit)`
    /// (`None` is `NoUnit.BASE`).
    fn unit(&self, unit: Option<MeasureUnit>) -> Self {
        self.create(KEY_UNIT, unit.map(SettingValue::Unit))
    }

    /// Sets a unit to be used in the denominator. Equivalent to
    /// `perUnit(MeasureUnit)`.
    fn per_unit(&self, per_unit: Option<MeasureUnit>) -> Self {
        self.create(KEY_PER_UNIT, per_unit.map(SettingValue::Unit))
    }

    /// Specifies the rounding precision to use when formatting numbers.
    /// Equivalent to `precision(Precision)`.
    fn precision(&self, precision: impl Into<Option<Precision>>) -> Self {
        self.create(KEY_PRECISION, precision.into().map(SettingValue::Precision))
    }

    /// Specifies how to determine the direction to round a number when it
    /// has more digits than fit in the desired precision. Equivalent to
    /// `roundingMode(RoundingMode)`.
    fn rounding_mode(&self, rounding_mode: impl Into<Option<RoundingMode>>) -> Self {
        self.create(
            KEY_ROUNDING_MODE,
            rounding_mode.into().map(SettingValue::RoundingMode),
        )
    }

    /// Specifies the grouping strategy to use when formatting numbers.
    /// Equivalent to `grouping(GroupingStrategy)`.
    fn grouping(&self, strategy: impl Into<Option<GroupingStrategy>>) -> Self {
        self.create(
            KEY_GROUPING,
            strategy
                .into()
                .map(|s| SettingValue::Grouping(GroupingObject::Strategy(s))),
        )
    }

    /// Specifies the minimum and maximum number of digits to render before
    /// the decimal mark. Equivalent to `integerWidth(IntegerWidth)`.
    fn integer_width(&self, style: impl Into<Option<IntegerWidth>>) -> Self {
        self.create(KEY_INTEGER, style.into().map(SettingValue::IntegerWidth))
    }

    /// Specifies the symbols (decimal separator, grouping separator,
    /// percent sign, numerals, etc.) to use when rendering numbers.
    /// Equivalent to `symbols(DecimalFormatSymbols)`: the symbols are
    /// cloned (Java NPE for `null`).
    fn symbols(&self, symbols: &DecimalFormatSymbols) -> Self {
        let symbols = symbols.clone();
        self.create(
            KEY_SYMBOLS,
            Some(SettingValue::Symbols(SymbolsObject::Symbols(Arc::new(
                symbols,
            )))),
        )
    }

    /// Specifies that the given numbering system should be used when
    /// fetching symbols. Equivalent to `symbols(NumberingSystem)`.
    fn symbols_numbering_system(&self, ns: Option<Arc<NumberingSystem>>) -> Self {
        self.create(
            KEY_SYMBOLS,
            ns.map(|ns| SettingValue::Symbols(SymbolsObject::NumberingSystem(ns))),
        )
    }

    /// Sets the width of the unit (measure unit or currency). Equivalent to
    /// `unitWidth(UnitWidth)`.
    fn unit_width(&self, style: impl Into<Option<UnitWidth>>) -> Self {
        self.create(KEY_UNIT_WIDTH, style.into().map(SettingValue::UnitWidth))
    }

    /// Sets the plus/minus sign display strategy. Equivalent to
    /// `sign(SignDisplay)`.
    fn sign(&self, style: impl Into<Option<SignDisplay>>) -> Self {
        self.create(KEY_SIGN, style.into().map(SettingValue::Sign))
    }

    /// Sets the decimal separator display strategy. Equivalent to
    /// `decimal(DecimalSeparatorDisplay)`.
    fn decimal(&self, style: impl Into<Option<DecimalSeparatorDisplay>>) -> Self {
        self.create(KEY_DECIMAL, style.into().map(SettingValue::Decimal))
    }

    /// Sets a scale (multiplier) to be used to scale the number by an
    /// arbitrary amount before formatting. Equivalent to `scale(Scale)`.
    fn scale(&self, scale: impl Into<Option<Scale>>) -> Self {
        self.create(KEY_SCALE, scale.into().map(SettingValue::Scale))
    }

    /// Specifies the usage for which numbers will be formatted ("person-height",
    /// "road", "rainfall", etc.). Equivalent to `usage(String)`: an empty
    /// usage is `null`.
    fn usage(&self, usage: Option<&str>) -> Self {
        match usage {
            Some("") => self.create(KEY_USAGE, None),
            u => self.create(KEY_USAGE, u.map(|s| SettingValue::String(s.to_string()))),
        }
    }

    /// Specifies the `DisplayOptions`. Equivalent to
    /// `displayOptions(DisplayOptions)`.
    fn display_options(&self, display_options: &DisplayOptions) -> Self {
        // `displayCase` does not recognise the `undefined`
        if display_options.get_grammatical_case() == GrammaticalCase::Undefined {
            return self.create(KEY_UNIT_DISPLAY_CASE, None);
        }

        self.create(
            KEY_UNIT_DISPLAY_CASE,
            Some(SettingValue::String(
                display_options
                    .get_grammatical_case()
                    .get_identifier()
                    .to_string(),
            )),
        )
    }

    /// Specifies the desired case for a unit formatter's output (e.g.
    /// accusative, dative, genitive). Equivalent to
    /// `unitDisplayCase(String)` (internal).
    fn unit_display_case(&self, unit_display_case: Option<&str>) -> Self {
        self.create(
            KEY_UNIT_DISPLAY_CASE,
            unit_display_case.map(|s| SettingValue::String(s.to_string())),
        )
    }

    /// Internal fluent setter to overwrite the entire macros object.
    /// Equivalent to `macros(MacroProps)` (internal).
    fn macros(&self, macros: MacroProps) -> Self {
        self.create(KEY_MACROS, Some(SettingValue::Macros(Arc::new(macros))))
    }

    /// Set the padding strategy. Equivalent to `padding(Padder)` (internal).
    fn padding(&self, padder: impl Into<Option<Padder>>) -> Self {
        self.create(KEY_PADDER, padder.into().map(SettingValue::Padder))
    }

    /// Internal fluent setter to support a custom regulation threshold. A
    /// threshold of 1 causes the data structures to be built right away.
    /// A threshold of 0 prevents the data structures from being built.
    /// Equivalent to `threshold(Long)` (internal).
    fn threshold(&self, threshold: Option<i64>) -> Self {
        self.create(KEY_THRESHOLD, threshold.map(SettingValue::Threshold))
    }

    /// Creates a skeleton string representation of this number formatter.
    /// Equivalent to `toSkeleton()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if the number formatter has an
    /// option that cannot be represented in a skeleton string.
    fn to_skeleton(&self) -> Result<Vec<u16>> {
        NumberSkeletonImpl::generate(&self.resolve())
    }

    /// Equivalent to `resolve()` (package-private).
    fn resolve(&self) -> Arc<MacroProps> {
        self.node().resolve()
    }

    /// Equivalent to `hashCode()`.
    fn hash_code(&self) -> i32 {
        self.resolve().hash_code()
    }

    /// Equivalent to `equals(Object)` for another settings object (of
    /// either kind).
    fn java_equals<O: NumberFormatterSettings>(&self, other: &O) -> bool {
        if Arc::ptr_eq(self.node(), other.node()) {
            return true;
        }
        self.resolve().java_equals(&other.resolve())
    }
}
