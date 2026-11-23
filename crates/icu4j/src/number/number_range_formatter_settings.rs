// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberRangeFormatterSettings`.

use std::sync::{Arc, OnceLock};

use crate::number::number_range_formatter::{RangeCollapse, RangeIdentityFallback};
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::r#impl::number::range::range_macro_props::RangeMacroProps;
use crate::util::u_locale::ULocale;

/// `KEY_MACROS` (not used).
pub(crate) const KEY_MACROS: i32 = 0;
/// `KEY_LOCALE`.
pub(crate) const KEY_LOCALE: i32 = 1;
/// `KEY_FORMATTER_1`.
pub(crate) const KEY_FORMATTER_1: i32 = 2;
/// `KEY_FORMATTER_2`.
pub(crate) const KEY_FORMATTER_2: i32 = 3;
/// `KEY_SAME_FORMATTERS`.
pub(crate) const KEY_SAME_FORMATTERS: i32 = 4;
/// `KEY_COLLAPSE`.
pub(crate) const KEY_COLLAPSE: i32 = 5;
/// `KEY_IDENTITY_FALLBACK`.
pub(crate) const KEY_IDENTITY_FALLBACK: i32 = 6;
/// `KEY_MAX`.
pub(crate) const KEY_MAX: i32 = 7;

/// The `Object value` of a settings link, typed by its key.
#[derive(Debug, Clone)]
pub enum RangeSettingValue {
    /// `KEY_LOCALE`: a `ULocale`.
    Locale(ULocale),
    /// `KEY_FORMATTER_1`/`KEY_FORMATTER_2`: an `UnlocalizedNumberFormatter`.
    Formatter(UnlocalizedNumberFormatter),
    /// `KEY_SAME_FORMATTERS`: a `Boolean`.
    SameFormatters(bool),
    /// `KEY_COLLAPSE`: a `RangeCollapse`.
    Collapse(RangeCollapse),
    /// `KEY_IDENTITY_FALLBACK`: a `RangeIdentityFallback`.
    IdentityFallback(RangeIdentityFallback),
}

/// One link of a range settings chain: the fields `parent`, `key`, `value`
/// and `resolvedMacros` of a `NumberRangeFormatterSettings` object.
#[derive(Debug)]
pub struct RangeSettingsNode {
    parent: Option<Arc<RangeSettingsNode>>,
    key: i32,
    value: Option<RangeSettingValue>,
    resolved_macros: OnceLock<Arc<RangeMacroProps>>,
}

impl RangeSettingsNode {
    /// `NumberRangeFormatterSettings(NumberRangeFormatterSettings<?>, int,
    /// Object)`.
    pub(crate) fn new(parent: Option<Arc<RangeSettingsNode>>, key: i32, value: Option<RangeSettingValue>) -> Arc<Self> {
        Arc::new(RangeSettingsNode { parent, key, value, resolved_macros: OnceLock::new() })
    }

    /// Equivalent to `resolve()` (package-private).
    ///
    /// # Panics
    ///
    /// Java's `AssertionError("Unknown key: <key>")` for an undefined key,
    /// and the `ClassCastException` of a value whose type does not match its
    /// key (unreachable through the public setters).
    pub(crate) fn resolve(&self) -> Arc<RangeMacroProps> {
        self.resolved_macros.get_or_init(|| Arc::new(self.compute())).clone()
    }

    fn compute(&self) -> RangeMacroProps {
        let mut macros = RangeMacroProps::default();
        // Bitmap: 1 if seen; 0 if unseen
        let mut seen: i64 = 0;
        let mut current: Option<&RangeSettingsNode> = Some(self);
        while let Some(node) = current {
            let key_bitmask = 1i64 << node.key;
            if 0 != (seen & key_bitmask) {
                current = node.parent.as_deref();
                continue;
            }
            seen |= key_bitmask;
            match (node.key, node.value.as_ref()) {
                (KEY_MACROS, _) => {
                    // ignored for now
                }
                (KEY_LOCALE, Some(RangeSettingValue::Locale(l))) => macros.loc = Some(l.clone()),
                (KEY_LOCALE, None) => macros.loc = None,
                (KEY_FORMATTER_1, Some(RangeSettingValue::Formatter(f))) => macros.formatter1 = Some(f.clone()),
                (KEY_FORMATTER_1, None) => macros.formatter1 = None,
                (KEY_FORMATTER_2, Some(RangeSettingValue::Formatter(f))) => macros.formatter2 = Some(f.clone()),
                (KEY_FORMATTER_2, None) => macros.formatter2 = None,
                (KEY_SAME_FORMATTERS, Some(RangeSettingValue::SameFormatters(b))) => {
                    macros.same_formatters = i32::from(*b);
                }
                (KEY_COLLAPSE, Some(RangeSettingValue::Collapse(c))) => macros.collapse = Some(*c),
                (KEY_COLLAPSE, None) => macros.collapse = None,
                (KEY_IDENTITY_FALLBACK, Some(RangeSettingValue::IdentityFallback(i))) => {
                    macros.identity_fallback = Some(*i);
                }
                (KEY_IDENTITY_FALLBACK, None) => macros.identity_fallback = None,
                (KEY_SAME_FORMATTERS, None) => panic!("java.lang.NullPointerException"),
                (KEY_LOCALE..=KEY_IDENTITY_FALLBACK, Some(_)) => panic!("java.lang.ClassCastException"),
                (key, _) => panic!("java.lang.AssertionError: Unknown key: {key}"),
            }
            current = node.parent.as_deref();
        }
        // Copy the locale into the children (see also NumberFormatter.locale)
        if let Some(f) = macros.formatter1.as_ref() {
            f.set_resolved_loc(macros.loc.clone());
        }
        if let Some(f) = macros.formatter2.as_ref() {
            f.set_resolved_loc(macros.loc.clone());
        }
        macros
    }
}

/// An abstract base class for specifying settings related to number
/// formatting. This class is implemented by
/// `UnlocalizedNumberRangeFormatter` and `LocalizedNumberRangeFormatter`.
/// This class is not intended for public subclassing.
///
/// Equivalent to `com.ibm.icu.number.NumberRangeFormatterSettings<T>`:
/// the implementor holds its [`RangeSettingsNode`] and `create`.
pub trait NumberRangeFormatterSettings: Sized {
    /// The settings link of this object.
    fn node(&self) -> &Arc<RangeSettingsNode>;

    /// Equivalent to the package-private `create(int, Object)`.
    fn create(&self, key: i32, value: Option<RangeSettingValue>) -> Self;

    /// Sets the NumberFormatter instance to use for the numbers in the
    /// range. The same formatter is applied to both sides of the range.
    /// Equivalent to `numberFormatterBoth(UnlocalizedNumberFormatter)`.
    fn number_formatter_both(&self, formatter: Option<&UnlocalizedNumberFormatter>) -> Self {
        self.create(KEY_SAME_FORMATTERS, Some(RangeSettingValue::SameFormatters(true)))
            .create(KEY_FORMATTER_1, formatter.cloned().map(RangeSettingValue::Formatter))
    }

    /// Sets the NumberFormatter instance to use for the first number in the
    /// range. Equivalent to `numberFormatterFirst(UnlocalizedNumberFormatter)`.
    fn number_formatter_first(&self, formatter_first: Option<&UnlocalizedNumberFormatter>) -> Self {
        self.create(KEY_SAME_FORMATTERS, Some(RangeSettingValue::SameFormatters(false)))
            .create(KEY_FORMATTER_1, formatter_first.cloned().map(RangeSettingValue::Formatter))
    }

    /// Sets the NumberFormatter instances to use for the second number in
    /// the range. Equivalent to
    /// `numberFormatterSecond(UnlocalizedNumberFormatter)`.
    fn number_formatter_second(&self, formatter_second: Option<&UnlocalizedNumberFormatter>) -> Self {
        self.create(KEY_SAME_FORMATTERS, Some(RangeSettingValue::SameFormatters(false)))
            .create(KEY_FORMATTER_2, formatter_second.cloned().map(RangeSettingValue::Formatter))
    }

    /// Sets the aggressiveness of "collapsing" fields across the range
    /// separator. Equivalent to `collapse(RangeCollapse)`.
    fn collapse(&self, collapse: Option<RangeCollapse>) -> Self {
        self.create(KEY_COLLAPSE, collapse.map(RangeSettingValue::Collapse))
    }

    /// Sets the behavior when the two sides of the range are the same.
    /// Equivalent to `identityFallback(RangeIdentityFallback)`.
    fn identity_fallback(&self, identity_fallback: Option<RangeIdentityFallback>) -> Self {
        self.create(KEY_IDENTITY_FALLBACK, identity_fallback.map(RangeSettingValue::IdentityFallback))
    }

    /// Equivalent to `resolve()` (package-private).
    fn resolve(&self) -> Arc<RangeMacroProps> {
        self.node().resolve()
    }

    /// Equivalent to `hashCode()`.
    fn hash_code(&self) -> i32 {
        self.resolve().hash_code()
    }

    /// Equivalent to `equals(Object)`.
    fn java_equals<O: NumberRangeFormatterSettings>(&self, other: &O) -> bool {
        if Arc::ptr_eq(self.node(), other.node()) {
            return true;
        }
        self.resolve().java_equals(&other.resolve())
    }
}

