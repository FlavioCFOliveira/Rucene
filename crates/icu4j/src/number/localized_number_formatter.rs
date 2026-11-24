// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.LocalizedNumberFormatter`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::number::formatted_number::FormattedNumber;
use crate::number::number_formatter_impl::NumberFormatterImpl;
use crate::number::number_formatter_settings::{
    NumberFormatterSettings, SettingValue, SettingsNode, KEY_LOCALE, KEY_UNIT,
};
use crate::number::unlocalized_number_formatter::UnlocalizedNumberFormatter;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{
    DecimalQuantity_DualStorageBCD, JavaNumber,
};
use crate::r#impl::number::localized_number_formatter_as_format::LocalizedNumberFormatterAsFormat;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::standard_plural::StandardPlural;
use crate::util::measure::Measure;
use crate::util::measure_unit::MeasureUnit;

/// The fields of a `LocalizedNumberFormatter` object.
#[derive(Debug)]
struct Inner {
    /// The `NumberFormatterSettings` fields.
    node: Arc<SettingsNode>,
    /// `callCountInternal` (updated through `callCount`, an
    /// `AtomicLongFieldUpdater`).
    call_count_internal: AtomicI64,
    /// `savedWithUnit` (`volatile`).
    saved_with_unit: Mutex<Option<LocalizedNumberFormatter>>,
    /// `compiled` (`volatile`).
    compiled: Mutex<Option<Arc<NumberFormatterImpl>>>,
}

/// A NumberFormatter that has a locale associated with it; this means
/// .format() methods are available.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.LocalizedNumberFormatter`. Clones
/// share the same object (its call count and compiled formatter), as Java
/// references do.
#[derive(Debug, Clone)]
pub struct LocalizedNumberFormatter(Arc<Inner>);

impl LocalizedNumberFormatter {
    /// `LocalizedNumberFormatter(NumberFormatterSettings<?>, int, Object)`.
    pub(crate) fn with_parent(
        parent: &Arc<SettingsNode>,
        key: i32,
        value: Option<SettingValue>,
    ) -> Self {
        LocalizedNumberFormatter(Arc::new(Inner {
            node: SettingsNode::new(Some(parent.clone()), key, value),
            call_count_internal: AtomicI64::new(0),
            saved_with_unit: Mutex::new(None),
            compiled: Mutex::new(None),
        }))
    }

    /// Format the given long to a string using the settings specified in
    /// the NumberFormatter fluent setting chain. Equivalent to
    /// `format(long)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipeline.
    pub fn format_long(&self, input: i64) -> Result<FormattedNumber> {
        self.format_quantity(Box::new(DecimalQuantity_DualStorageBCD::from_long(input)))
    }

    /// Format the given float or double to a string. Equivalent to
    /// `format(double)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipeline.
    pub fn format_double(&self, input: f64) -> Result<FormattedNumber> {
        self.format_quantity(Box::new(DecimalQuantity_DualStorageBCD::from_double(input)))
    }

    /// Format the given BigInteger, BigDecimal, or other Number to a string.
    /// Equivalent to `format(Number)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion and of the formatting pipeline.
    pub fn format_number(&self, input: &JavaNumber) -> Result<FormattedNumber> {
        self.format_quantity(Box::new(DecimalQuantity_DualStorageBCD::from_number(
            input,
        )?))
    }

    /// Format the given Measure, using the unit of the measure if it
    /// differs from the formatter's unit. Equivalent to `format(Measure)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion and of the formatting pipeline.
    pub fn format_measure(&self, input: &Measure) -> Result<FormattedNumber> {
        let mut fq: Box<dyn DecimalQuantity> = Box::new(
            DecimalQuantity_DualStorageBCD::from_number(input.get_number())?,
        );
        let unit = input.get_unit().clone();
        let mut string = FormattedStringBuilder::new();
        let micros = self.format_impl_with_unit(fq.as_mut(), Some(&unit), &mut string)?;
        Ok(FormattedNumber::new(
            string,
            fq,
            micros.output_unit,
            micros.gender,
        ))
    }

    /// Creates a representation of this LocalizedNumberFormat as a
    /// `java.text.Format`, enabling the use of this number formatter with
    /// APIs that need an object of that type. Equivalent to `toFormat()`.
    pub fn to_format(&self) -> LocalizedNumberFormatterAsFormat {
        LocalizedNumberFormatterAsFormat::new(self.clone(), self.resolve().loc.clone())
    }

    /// Disassociate the locale from this formatter. Equivalent to
    /// `withoutLocale()`.
    pub fn without_locale(&self) -> UnlocalizedNumberFormatter {
        UnlocalizedNumberFormatter::with_parent(&self.0.node, KEY_LOCALE, None)
    }

    /// This is the core entrypoint to the number formatting pipeline. It
    /// performs self-regulation: a static code path for the first few calls,
    /// and compiling a more efficient data structure if called repeatedly.
    /// Equivalent to the private `format(DecimalQuantity)`.
    fn format_quantity(&self, mut fq: Box<dyn DecimalQuantity>) -> Result<FormattedNumber> {
        let mut string = FormattedStringBuilder::new();
        let micros = self.format_impl(fq.as_mut(), &mut string)?;
        Ok(FormattedNumber::new(
            string,
            fq,
            micros.output_unit,
            micros.gender,
        ))
    }

    /// Equivalent to `formatImpl(DecimalQuantity, FormattedStringBuilder)`
    /// (internal).
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipeline.
    pub fn format_impl(
        &self,
        fq: &mut dyn DecimalQuantity,
        string: &mut FormattedStringBuilder,
    ) -> Result<MicroProps> {
        if let Some(compiled) = self.compute_compiled()? {
            return compiled.format(fq, string);
        }
        NumberFormatterImpl::format_static(&self.resolve(), fq, string)
    }

    /// Version of above for unit override. Equivalent to
    /// `formatImpl(DecimalQuantity, MeasureUnit, FormattedStringBuilder)`
    /// (internal).
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipeline.
    pub fn format_impl_with_unit(
        &self,
        fq: &mut dyn DecimalQuantity,
        unit: Option<&MeasureUnit>,
        string: &mut FormattedStringBuilder,
    ) -> Result<MicroProps> {
        // Use this formatter if possible
        if opt_unit_equals(self.resolve().unit.as_ref(), unit) {
            return self.format_impl(fq, string);
        }
        // This mechanism saves the previously used unit, so if the user
        // calls this method with the same unit multiple times in a row,
        // they get a more efficient code path.
        let with_unit = {
            let mut saved = self
                .0
                .saved_with_unit
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let reuse = match saved.as_ref() {
                Some(w) => opt_unit_equals(w.resolve().unit.as_ref(), unit),
                None => false,
            };
            if !reuse {
                *saved = Some(LocalizedNumberFormatter::with_parent(
                    &self.0.node,
                    KEY_UNIT,
                    unit.cloned().map(SettingValue::Unit),
                ));
            }
            saved.clone().expect("just set")
        };
        with_unit.format_impl(fq, string)
    }

    /// Equivalent to `getAffixImpl(boolean, boolean)` (internal).
    ///
    /// # Errors
    ///
    /// The errors of the formatting pipeline.
    pub fn get_affix_impl(&self, is_prefix: bool, is_negative: bool) -> Result<Vec<u16>> {
        let mut string = FormattedStringBuilder::new();
        let signum: i8 = if is_negative { -1 } else { 1 };
        // Always return affixes for plural form OTHER.
        let plural = StandardPlural::Other;
        let prefix_length = match self.compute_compiled()? {
            Some(compiled) => compiled.get_prefix_suffix(signum, plural, &mut string)?,
            None => NumberFormatterImpl::get_prefix_suffix_static(
                &self.resolve(),
                signum,
                plural,
                &mut string,
            )?,
        };
        if is_prefix {
            Ok(string.sub_sequence(0, prefix_length).to_java_string())
        } else {
            Ok(string
                .sub_sequence(prefix_length, string.length())
                .to_java_string())
        }
    }

    /// `computeCompiled()`: the compiled formatter once the call count
    /// reaches the threshold, `None` before.
    fn compute_compiled(&self) -> Result<Option<Arc<NumberFormatterImpl>>> {
        let macros = self.resolve();
        // This is the "double-checked locking" pattern, implemented
        // properly for Java 5+.
        let current_count = self.0.call_count_internal.fetch_add(1, Ordering::SeqCst) + 1;
        let threshold = macros
            .threshold
            .ok_or_else(|| crate::jdk::null_pointer("threshold"))?;
        if current_count == threshold {
            let compiled = Arc::new(NumberFormatterImpl::new(&macros)?);
            *self.0.compiled.lock().unwrap_or_else(|e| e.into_inner()) = Some(compiled.clone());
            Ok(Some(compiled))
        } else {
            Ok(self
                .0
                .compiled
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone())
        }
    }
}

/// `Objects.equals(MeasureUnit, MeasureUnit)`.
fn opt_unit_equals(a: Option<&MeasureUnit>, b: Option<&MeasureUnit>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.java_equals(b),
        _ => false,
    }
}

impl NumberFormatterSettings for LocalizedNumberFormatter {
    fn node(&self) -> &Arc<SettingsNode> {
        &self.0.node
    }

    fn create(&self, key: i32, value: Option<SettingValue>) -> Self {
        LocalizedNumberFormatter::with_parent(&self.0.node, key, value)
    }
}
