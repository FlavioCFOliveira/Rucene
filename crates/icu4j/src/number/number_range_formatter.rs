// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.NumberRangeFormatter`.

use std::sync::LazyLock;

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::number::localized_number_range_formatter::LocalizedNumberRangeFormatter;
use crate::number::unlocalized_number_range_formatter::UnlocalizedNumberRangeFormatter;
use crate::r#impl::number::macro_props::enum_hash_code;
use crate::util::u_locale::ULocale;

/// Declares a Java enum nested in `NumberRangeFormatter`: `name()`,
/// `ordinal()`, `values()` and the identity `hashCode()` stand-in.
macro_rules! range_enum {
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

range_enum!(
    /// Defines how to merge fields that are identical across the range
    /// sides. Equivalent to `NumberRangeFormatter.RangeCollapse`.
    RangeCollapse, "com.ibm.icu.number.NumberRangeFormatter$RangeCollapse",
    [
        /// Use locale data and heuristics to determine how much of the string
        /// to collapse. Could end up collapsing none, some, or all repeated
        /// pieces in a locale-sensitive way.
        Auto = "AUTO",
        /// Do not collapse any part of the number. Example: "3.2 thousand
        /// kilograms – 5.3 thousand kilograms".
        None = "NONE",
        /// Collapse the unit part of the number, but not the notation, if
        /// present. Example: "3.2 thousand – 5.3 thousand kilograms".
        Unit = "UNIT",
        /// Collapse any field that is equal across the range sides. Example:
        /// "3.2 – 5.3 thousand kilograms".
        All = "ALL",
    ]
);

range_enum!(
    /// Defines the behavior when the two numbers in the range are identical
    /// after rounding. Equivalent to
    /// `NumberRangeFormatter.RangeIdentityFallback`.
    RangeIdentityFallback, "com.ibm.icu.number.NumberRangeFormatter$RangeIdentityFallback",
    [
        /// Show the number as a single value rather than a range. Example:
        /// "$5".
        SingleValue = "SINGLE_VALUE",
        /// Show the number using a locale-sensitive approximation pattern.
        /// If the numbers were the same before rounding, show the single
        /// value. Example: "~$5" or "$5".
        ApproximatelyOrSingleValue = "APPROXIMATELY_OR_SINGLE_VALUE",
        /// Show the number using a locale-sensitive approximation pattern.
        /// Use the range pattern always, even if the inputs are the same.
        /// Example: "~$5".
        Approximately = "APPROXIMATELY",
        /// Show the number as the range of two equal values. Use the range
        /// pattern always, even if the inputs are the same. Example (with
        /// RangeCollapse.NONE): "$5 – $5".
        Range = "RANGE",
    ]
);

range_enum!(
    /// Used in the result class FormattedNumberRange to indicate to the user
    /// whether the numbers formatted in the range were equal or not, and
    /// whether or not the identity fallback was applied. Equivalent to
    /// `NumberRangeFormatter.RangeIdentityResult`.
    RangeIdentityResult, "com.ibm.icu.number.NumberRangeFormatter$RangeIdentityResult",
    [
        /// Used to indicate that the two numbers in the range were equal,
        /// even before any rounding rules were applied.
        EqualBeforeRounding = "EQUAL_BEFORE_ROUNDING",
        /// Used to indicate that the two numbers in the range were equal, but
        /// only after rounding rules were applied.
        EqualAfterRounding = "EQUAL_AFTER_ROUNDING",
        /// Used to indicate that the two numbers in the range were not equal,
        /// even after rounding rules were applied.
        NotEqual = "NOT_EQUAL",
    ]
);

/// `NumberRangeFormatter.SpanField` and its constant.
pub mod span_field {
    use rucene::error::{LuceneError, Result};

    use crate::jdk::text::{Attribute, FieldRef};

    crate::java_format_field_class!(
        /// Class for span fields in NumberRangeFormatter. Equivalent to
        /// `NumberRangeFormatter.SpanField` (a `UFormat.SpanField`).
        pub SpanField, "com.ibm.icu.number.NumberRangeFormatter$SpanField", ["com.ibm.icu.text.UFormat$SpanField"]
    );

    /// The concrete field used for spans in FormattedNumberRange. Instances
    /// of NUMBER_RANGE_SPAN should have an associated value, the index
    /// within the input list of the corresponding number. Equivalent to
    /// `NUMBER_RANGE_SPAN`.
    pub static NUMBER_RANGE_SPAN: SpanField = SpanField::new("number-range-span");

    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 8750397196515368729;

    impl SpanField {
        /// Serialization method resolve instances to the constant
        /// NumberRangeFormatter.SpanField values. Equivalent to
        /// `readResolve()`.
        ///
        /// # Errors
        ///
        /// `InvalidObjectException("An invalid object.")`.
        pub fn read_resolve(&self) -> Result<FieldRef> {
            if self.get_name() == NUMBER_RANGE_SPAN.get_name() {
                return Ok(&NUMBER_RANGE_SPAN);
            }
            Err(LuceneError::unchecked("java.io.InvalidObjectException", "An invalid object."))
        }
    }
}

static BASE: LazyLock<UnlocalizedNumberRangeFormatter> = LazyLock::new(UnlocalizedNumberRangeFormatter::new);

/// The main entrypoint to the formatting of ranges of numbers, including
/// currencies and other units of measurement.
///
/// Equivalent to `com.ibm.icu.number.NumberRangeFormatter` (static
/// members).
pub struct NumberRangeFormatter;

impl NumberRangeFormatter {
    /// Call this method at the beginning of a NumberRangeFormatter fluent
    /// chain in which the locale is not currently known at the call site.
    /// Equivalent to `with()`.
    pub fn with() -> UnlocalizedNumberRangeFormatter {
        BASE.clone()
    }

    /// Call this method at the beginning of a NumberRangeFormatter fluent
    /// chain in which the locale is known at the call site. Equivalent to
    /// `withLocale(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn with_locale_java(locale: &Locale) -> Result<LocalizedNumberRangeFormatter> {
        BASE.locale_java(locale)
    }

    /// Equivalent to `withLocale(ULocale)`.
    pub fn with_locale(locale: &ULocale) -> LocalizedNumberRangeFormatter {
        BASE.locale(locale)
    }
}
