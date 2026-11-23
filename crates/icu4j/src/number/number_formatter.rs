// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberFormatter`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_property_mapper::NumberPropertyMapper;
use crate::number::number_skeleton_impl::NumberSkeletonImpl;
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::macro_props::enum_hash_code;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::u_locale::ULocale;

/// Declares a Java enum nested in `NumberFormatter`: its constants, `name()`,
/// `ordinal()`, `values()` and the identity `hashCode()` stand-in.
macro_rules! number_formatter_enum {
    ($(#[$doc:meta])* $name:ident, $class:literal, [$($(#[$cdoc:meta])* $variant:ident = $jname:literal),* $(,)?]) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$cdoc])* $variant,)*
        }

        impl $name {
            /// `values()`.
            pub const VALUES: &'static [$name] = &[$($name::$variant,)*];

            /// `name()`.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $jname,)*
                }
            }

            /// `ordinal()`.
            pub fn ordinal(self) -> i32 {
                self as i32
            }

            /// `hashCode()` (identity): see `java_identity_hash_code_stand_in`.
            pub fn hash_code(self) -> i32 {
                enum_hash_code($class, self.name())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }
    };
}

number_formatter_enum!(
    /// An enum declaring how to resolve conflicts between maximum fraction
    /// digits and maximum significant digits. Equivalent to
    /// `NumberFormatter.RoundingPriority`.
    RoundingPriority, "com.ibm.icu.number.NumberFormatter$RoundingPriority",
    [
        /// Favor greater precision by relaxing one of the rounding
        /// constraints.
        Relaxed = "RELAXED",
        /// Favor adherence to all rounding constraints by producing lower
        /// precision.
        Strict = "STRICT",
    ]
);

number_formatter_enum!(
    /// An enum declaring how to render units, including currencies.
    /// Equivalent to `NumberFormatter.UnitWidth`.
    UnitWidth, "com.ibm.icu.number.NumberFormatter$UnitWidth",
    [
        /// Use the most abbreviated form of the unit.
        Narrow = "NARROW",
        /// Use the short form of the unit (default).
        Short = "SHORT",
        /// Use the full name of the unit.
        FullName = "FULL_NAME",
        /// Use the three-digit ISO XXX code in place of the symbol.
        IsoCode = "ISO_CODE",
        /// Use the formal variant of the currency symbol.
        Formal = "FORMAL",
        /// Use the alternate variant of the currency symbol.
        Variant = "VARIANT",
        /// Format the number according to the specified unit, but do not
        /// display the unit.
        Hidden = "HIDDEN",
    ]
);

number_formatter_enum!(
    /// An enum declaring the strategy for when and how to display grouping
    /// separators. Equivalent to `NumberFormatter.GroupingStrategy`.
    GroupingStrategy, "com.ibm.icu.number.NumberFormatter$GroupingStrategy",
    [
        /// Do not display grouping separators in any locale.
        Off = "OFF",
        /// Display grouping using locale defaults, except do not show
        /// grouping on values smaller than 10000.
        Min2 = "MIN2",
        /// Display grouping using the default strategy for all locales.
        Auto = "AUTO",
        /// Always display the grouping separator on values of at least 1000.
        OnAligned = "ON_ALIGNED",
        /// Use the Western defaults: groups of 3 and enabled for all numbers
        /// 1000 or greater.
        Thousands = "THOUSANDS",
    ]
);

number_formatter_enum!(
    /// An enum declaring how to denote positive and negative numbers.
    /// Equivalent to `NumberFormatter.SignDisplay`.
    SignDisplay, "com.ibm.icu.number.NumberFormatter$SignDisplay",
    [
        /// Show the minus sign on negative numbers, and do not show the sign
        /// on positive numbers (default).
        Auto = "AUTO",
        /// Show the minus sign on negative numbers and the plus sign on
        /// positive numbers, including zero.
        Always = "ALWAYS",
        /// Do not show the sign on positive or negative numbers.
        Never = "NEVER",
        /// Use the locale-dependent accounting format on negative numbers.
        Accounting = "ACCOUNTING",
        /// Use the locale-dependent accounting format on negative numbers,
        /// and show the plus sign on positive numbers, including zero.
        AccountingAlways = "ACCOUNTING_ALWAYS",
        /// Show the minus sign on negative numbers and the plus sign on
        /// positive numbers. Do not show a sign on zero, NaN, or positive
        /// zero.
        ExceptZero = "EXCEPT_ZERO",
        /// Use the locale-dependent accounting format on negative numbers,
        /// and show the plus sign on positive numbers. Do not show a sign on
        /// zero, NaN, or positive zero.
        AccountingExceptZero = "ACCOUNTING_EXCEPT_ZERO",
        /// Same as AUTO, but do not show the sign on negative zero.
        Negative = "NEGATIVE",
        /// Same as ACCOUNTING, but do not show the sign on negative zero.
        AccountingNegative = "ACCOUNTING_NEGATIVE",
    ]
);

number_formatter_enum!(
    /// An enum declaring how to render the decimal separator. Equivalent to
    /// `NumberFormatter.DecimalSeparatorDisplay`.
    DecimalSeparatorDisplay, "com.ibm.icu.number.NumberFormatter$DecimalSeparatorDisplay",
    [
        /// Show the decimal separator when there are one or more digits to
        /// display after the separator, and do not show it otherwise.
        Auto = "AUTO",
        /// Always show the decimal separator, even if there are no digits to
        /// display after the separator.
        Always = "ALWAYS",
    ]
);

number_formatter_enum!(
    /// An enum declaring how to render trailing zeros. Equivalent to
    /// `NumberFormatter.TrailingZeroDisplay`.
    TrailingZeroDisplay, "com.ibm.icu.number.NumberFormatter$TrailingZeroDisplay",
    [
        /// Display trailing zeros according to the settings for minimum
        /// fraction and significant digits.
        Auto = "AUTO",
        /// Same as AUTO, but hide trailing zeros after the decimal separator
        /// if they are all zero.
        HideIfWhole = "HIDE_IF_WHOLE",
    ]
);

/// `DEFAULT_THRESHOLD`: use 3 because by convention, the first call is
/// warmup, the second call does the compile, and the third call uses the
/// compiled object.
pub(crate) const DEFAULT_THRESHOLD: i64 = 3;

/// `BASE`.
static BASE: LazyLock<UnlocalizedNumberFormatter> = LazyLock::new(UnlocalizedNumberFormatter::base);

/// The main entrypoint to the formatting of numbers, including currencies
/// and other units of measurement.
///
/// Equivalent to `com.ibm.icu.number.NumberFormatter` (a final class with
/// static members only).
pub struct NumberFormatter;

impl NumberFormatter {
    /// Use a default threshold of 3. Equivalent to `with()`: an
    /// unlocalized formatter, to which a locale must be attached.
    pub fn with() -> UnlocalizedNumberFormatter {
        BASE.clone()
    }

    /// Call this method at the beginning of a NumberFormatter fluent chain
    /// in which the locale is known at the call site. Equivalent to
    /// `withLocale(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn with_locale_java(locale: &Locale) -> Result<LocalizedNumberFormatter> {
        BASE.locale_java(locale)
    }

    /// Call this method at the beginning of a NumberFormatter fluent chain
    /// in which the locale is known at the call site. Equivalent to
    /// `withLocale(ULocale)`.
    pub fn with_locale(locale: &ULocale) -> LocalizedNumberFormatter {
        BASE.locale(locale)
    }

    /// Call this method at the beginning of a NumberFormatter fluent chain
    /// to create an instance based on a given number skeleton string.
    /// Equivalent to `forSkeleton(String)`.
    ///
    /// # Errors
    ///
    /// `SkeletonSyntaxException` for an invalid skeleton.
    pub fn for_skeleton(skeleton: &[u16]) -> Result<UnlocalizedNumberFormatter> {
        NumberSkeletonImpl::get_or_create(skeleton)
    }

    /// Equivalent to `fromDecimalFormat(DecimalFormatProperties,
    /// DecimalFormatSymbols, DecimalFormatProperties)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the property mapping.
    pub fn from_decimal_format(
        properties: &DecimalFormatProperties,
        symbols: Arc<DecimalFormatSymbols>,
        exported_properties: Option<&mut DecimalFormatProperties>,
    ) -> Result<UnlocalizedNumberFormatter> {
        NumberPropertyMapper::create_exported(properties, symbols, exported_properties)
    }
}
