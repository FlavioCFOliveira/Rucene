// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.LocalizedNumberRangeFormatter`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::number::formatted_number_range::FormattedNumberRange;
use crate::number::number_range_formatter_impl::NumberRangeFormatterImpl;
use crate::number::number_range_formatter_settings::{
    NumberRangeFormatterSettings, RangeSettingValue, RangeSettingsNode, KEY_LOCALE,
};
use crate::number::unlocalized_number_range_formatter::UnlocalizedNumberRangeFormatter;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{
    DecimalQuantity_DualStorageBCD, JavaNumber,
};

#[derive(Debug)]
struct Inner {
    node: Arc<RangeSettingsNode>,
    /// `fImpl` (`volatile`, created on first use).
    f_impl: Mutex<Option<Arc<NumberRangeFormatterImpl>>>,
}

/// A NumberRangeFormatter that has a locale associated with it; this means
/// .formatRange() methods are available.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.LocalizedNumberRangeFormatter`.
#[derive(Debug, Clone)]
pub struct LocalizedNumberRangeFormatter(Arc<Inner>);

impl LocalizedNumberRangeFormatter {
    /// Equivalent to the package-private
    /// `LocalizedNumberRangeFormatter(NumberRangeFormatterSettings<?>, int,
    /// Object)`.
    pub(crate) fn with_parent(
        parent: &Arc<RangeSettingsNode>,
        key: i32,
        value: Option<RangeSettingValue>,
    ) -> Self {
        LocalizedNumberRangeFormatter(Arc::new(Inner {
            node: RangeSettingsNode::new(Some(parent.clone()), key, value),
            f_impl: Mutex::new(None),
        }))
    }

    /// Format the given integers to a string using the settings specified in
    /// the NumberRangeFormatter fluent setting chain. Equivalent to
    /// `formatRange(int, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter construction and of the formatting.
    pub fn format_range_int(&self, first: i32, second: i32) -> Result<FormattedNumberRange> {
        let dq1 = DecimalQuantity_DualStorageBCD::from_int(first);
        let dq2 = DecimalQuantity_DualStorageBCD::from_int(second);
        self.format_impl(Box::new(dq1), Box::new(dq2), first == second)
    }

    /// Format the given doubles to a string. Equivalent to
    /// `formatRange(double, double)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter construction and of the formatting.
    pub fn format_range_double(&self, first: f64, second: f64) -> Result<FormattedNumberRange> {
        let dq1 = DecimalQuantity_DualStorageBCD::from_double(first);
        let dq2 = DecimalQuantity_DualStorageBCD::from_double(second);
        // Note: double equality could be changed to epsilon equality later if there is demand.
        // The epsilon should be set via an API method.
        #[allow(clippy::float_cmp)]
        let equal = first == second;
        self.format_impl(Box::new(dq1), Box::new(dq2), equal)
    }

    /// Format the given Numbers to a string. Equivalent to
    /// `formatRange(Number, Number)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Cannot format null values in range")` for
    /// a `None`; the errors of the conversion and of the formatting.
    pub fn format_range(
        &self,
        first: Option<&JavaNumber>,
        second: Option<&JavaNumber>,
    ) -> Result<FormattedNumberRange> {
        let (Some(first), Some(second)) = (first, second) else {
            return Err(LuceneError::illegal_argument(
                "Cannot format null values in range",
            ));
        };
        let dq1 = DecimalQuantity_DualStorageBCD::from_number(first)?;
        let dq2 = DecimalQuantity_DualStorageBCD::from_number(second)?;
        self.format_impl(Box::new(dq1), Box::new(dq2), first.java_equals(second))
    }

    /// Disassociate the locale from this formatter. Equivalent to
    /// `withoutLocale()`.
    pub fn without_locale(&self) -> UnlocalizedNumberRangeFormatter {
        UnlocalizedNumberRangeFormatter::with_parent(&self.0.node, KEY_LOCALE, None)
    }

    /// Equivalent to the package-private `formatImpl(DecimalQuantity,
    /// DecimalQuantity, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter construction and of the formatting.
    pub(crate) fn format_impl(
        &self,
        first: Box<dyn DecimalQuantity>,
        second: Box<dyn DecimalQuantity>,
        equal_before_rounding: bool,
    ) -> Result<FormattedNumberRange> {
        let f_impl = {
            let mut guard = self.0.f_impl.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(f) => f.clone(),
                None => {
                    let f = Arc::new(NumberRangeFormatterImpl::new(&self.resolve())?);
                    *guard = Some(f.clone());
                    f
                }
            }
        };
        f_impl.format(first, second, equal_before_rounding)
    }
}

impl NumberRangeFormatterSettings for LocalizedNumberRangeFormatter {
    fn node(&self) -> &Arc<RangeSettingsNode> {
        &self.0.node
    }

    fn create(&self, key: i32, value: Option<RangeSettingValue>) -> Self {
        LocalizedNumberRangeFormatter::with_parent(&self.0.node, key, value)
    }
}
