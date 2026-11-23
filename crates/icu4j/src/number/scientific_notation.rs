// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.ScientificNotation`.

use std::any::Any;
use std::ops::Deref;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::jdk::text::FieldRef;
use crate::number::notation::{Notation, NotationKind};
use crate::number::number_formatter::SignDisplay;
use crate::number::precision::PrecisionKind;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::number::constant_affix_modifier;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::modifier::{Modifier, Parameters};
use crate::r#impl::number::multiplier_producer::MultiplierProducer;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::number_format::field as nf_field;

/// The fields of a `ScientificNotation` object.
#[derive(Debug, Clone)]
pub(crate) struct ScientificNotationData {
    /// `engineeringInterval`.
    pub(crate) engineering_interval: i32,
    /// `requireMinInt`.
    pub(crate) require_min_int: bool,
    /// `minExponentDigits`.
    pub(crate) min_exponent_digits: i32,
    /// `exponentSignDisplay`.
    pub(crate) exponent_sign_display: SignDisplay,
}

/// A class that defines the scientific notation style to be used when
/// formatting numbers in NumberFormatter.
///
/// To create a ScientificNotation, use one of the factory methods in
/// [`Notation`].
///
/// Equivalent to `com.ibm.icu.number.ScientificNotation`: a view over a
/// [`Notation`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct ScientificNotation(Notation);

impl ScientificNotation {
    /// `ScientificNotation(int, boolean, int, SignDisplay)`
    /// (package-private).
    pub(crate) fn new(
        engineering_interval: i32,
        require_min_int: bool,
        min_exponent_digits: i32,
        exponent_sign_display: SignDisplay,
    ) -> Self {
        ScientificNotation(Notation(Arc::new(NotationKind::Scientific(ScientificNotationData {
            engineering_interval,
            require_min_int,
            min_exponent_digits,
            exponent_sign_display,
        }))))
    }

    /// `notation instanceof ScientificNotation ? (ScientificNotation)
    /// notation : null`.
    pub fn try_cast(notation: &Notation) -> Option<ScientificNotation> {
        match notation.kind() {
            NotationKind::Scientific(_) => Some(ScientificNotation(notation.clone())),
            _ => None,
        }
    }

    /// `(ScientificNotation) notation`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` for another class.
    pub fn cast(notation: Notation) -> ScientificNotation {
        assert!(
            matches!(notation.kind(), NotationKind::Scientific(_)),
            "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.number.ScientificNotation",
            notation.java_class_name()
        );
        ScientificNotation(notation)
    }

    /// The `Notation` this view is.
    pub fn into_notation(self) -> Notation {
        self.0
    }

    /// The fields.
    pub(crate) fn data(&self) -> &ScientificNotationData {
        match self.0.kind() {
            NotationKind::Scientific(d) => d,
            _ => unreachable!("a ScientificNotation view holds a ScientificNotation"),
        }
    }

    /// Sets the minimum number of digits to show in the exponent of
    /// scientific notation, padding with zeros if necessary. Equivalent to
    /// `withMinExponentDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn with_min_exponent_digits(&self, min_exponent_digits: i32) -> Result<ScientificNotation> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_exponent_digits) {
            let mut other = self.data().clone();
            other.min_exponent_digits = min_exponent_digits;
            Ok(ScientificNotation(Notation(Arc::new(NotationKind::Scientific(other)))))
        } else {
            Err(LuceneError::illegal_argument(format!(
                "Integer digits must be between 1 and {} (inclusive)",
                RoundingUtils::MAX_INT_FRAC_SIG
            )))
        }
    }

    /// Sets whether to show the sign on positive and negative exponents in
    /// scientific notation. Equivalent to
    /// `withExponentSignDisplay(SignDisplay)`.
    pub fn with_exponent_sign_display(&self, exponent_sign_display: SignDisplay) -> ScientificNotation {
        let mut other = self.data().clone();
        other.exponent_sign_display = exponent_sign_display;
        ScientificNotation(Notation(Arc::new(NotationKind::Scientific(other))))
    }

    /// `withLocaleData(DecimalFormatSymbols, boolean, MicroPropsGenerator)`
    /// (package-private).
    pub(crate) fn with_locale_data(
        &self,
        symbols: Arc<DecimalFormatSymbols>,
        build: bool,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Arc<dyn MicroPropsGenerator> {
        ScientificHandler::new(self.data().clone(), symbols, build, parent)
    }
}

impl Deref for ScientificNotation {
    type Target = Notation;

    fn deref(&self) -> &Notation {
        &self.0
    }
}

impl From<ScientificNotation> for Notation {
    fn from(n: ScientificNotation) -> Notation {
        n.0
    }
}

/// The state `ScientificHandler.doApply` reads: the notation and the
/// symbols.
///
/// Java's `ScientificModifier` keeps a reference to its `ScientificHandler`
/// and calls `handler.doApply`; the handler in turn owns the precomputed
/// modifiers. The port shares this part of the handler between both, which
/// keeps the same behavior without a reference cycle.
#[derive(Debug)]
struct ScientificApplier {
    /// `notation`.
    notation: ScientificNotationData,
    /// `symbols`.
    symbols: Arc<DecimalFormatSymbols>,
}

impl ScientificApplier {
    /// `ScientificHandler.doApply(int, FormattedStringBuilder, int)`.
    fn do_apply(&self, exponent: i32, output: &mut FormattedStringBuilder, right_index: i32) -> i32 {
        // FIXME: Localized exponent separator location.
        let mut i = right_index;
        // Append the exponent separator and sign
        i += output.insert(
            i,
            self.symbols.get_exponent_separator(),
            Some(FieldObject::Field(&nf_field::EXPONENT_SYMBOL as FieldRef)),
        );
        if exponent < 0 && self.notation.exponent_sign_display != SignDisplay::Never {
            i += output.insert(
                i,
                self.symbols.get_minus_sign_string(),
                Some(FieldObject::Field(&nf_field::EXPONENT_SIGN as FieldRef)),
            );
        } else if exponent >= 0 && self.notation.exponent_sign_display == SignDisplay::Always {
            i += output.insert(
                i,
                self.symbols.get_plus_sign_string(),
                Some(FieldObject::Field(&nf_field::EXPONENT_SIGN as FieldRef)),
            );
        }
        // Append the exponent digits (using a simple inline algorithm)
        let mut disp = exponent.wrapping_abs();
        let mut j = 0;
        while j < self.notation.min_exponent_digits || disp > 0 {
            let d = disp % 10;
            let digit_string = &self.symbols.get_digit_strings_local()[d as usize];
            i += output.insert(i - j, digit_string, Some(FieldObject::Field(&nf_field::EXPONENT as FieldRef)));
            j += 1;
            disp /= 10;
        }
        i - right_index
    }
}

/// Equivalent to the private class `ScientificNotation.ScientificHandler`
/// (a `MicroPropsGenerator`, `MultiplierProducer` and `Modifier`).
#[derive(Debug)]
struct ScientificHandler {
    /// `notation` and `symbols`.
    applier: Arc<ScientificApplier>,
    /// `precomputedMods` (`None` for Java's `null`).
    precomputed_mods: Option<Vec<Arc<dyn Modifier>>>,
    /// `parent`.
    parent: Arc<dyn MicroPropsGenerator>,
    /// `exponent`: the exponent of the last quantity (unsafe code path).
    exponent: AtomicI32,
    /// This object, for `micros.modInner = this`.
    self_ref: Weak<ScientificHandler>,
}

impl ScientificHandler {
    /// `ScientificHandler(ScientificNotation, DecimalFormatSymbols, boolean,
    /// MicroPropsGenerator)`.
    fn new(
        notation: ScientificNotationData,
        symbols: Arc<DecimalFormatSymbols>,
        safe: bool,
        parent: Arc<dyn MicroPropsGenerator>,
    ) -> Arc<ScientificHandler> {
        let applier = Arc::new(ScientificApplier { notation, symbols });
        let precomputed_mods = if safe {
            // Pre-build the modifiers for exponents -12 through 12
            let mut mods: Vec<Arc<dyn Modifier>> = Vec::with_capacity(25);
            for i in -12..=12 {
                mods.push(Arc::new(ScientificModifier { exponent: i, handler: applier.clone() }));
            }
            Some(mods)
        } else {
            None
        };
        Arc::new_cyclic(|self_ref| ScientificHandler {
            applier,
            precomputed_mods,
            parent,
            exponent: AtomicI32::new(0),
            self_ref: self_ref.clone(),
        })
    }
}

impl MicroPropsGenerator for ScientificHandler {
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let mut micros = self.parent.process_quantity(quantity)?;
        debug_assert!(micros.rounder.is_some());

        // Do not apply scientific notation to special doubles
        if quantity.is_infinite() || quantity.is_nan() {
            micros.mod_inner = Some(constant_affix_modifier::EMPTY.clone());
            return Ok(micros);
        }

        // Treat zero as if it had magnitude 0
        let rounder = micros.rounder.clone().expect("micros.rounder is set by the pipeline");
        let exponent = if quantity.is_zeroish() {
            if self.applier.notation.require_min_int && matches!(rounder.kind(), PrecisionKind::Significant { .. }) {
                // Show "00.000E0" on pattern "00.000E0"
                rounder.apply_significant_min_int(quantity, self.applier.notation.engineering_interval);
            } else {
                rounder.apply(quantity)?;
            }
            0
        } else {
            -rounder.choose_multiplier_and_apply(quantity, self)?
        };

        // Add the Modifier for the scientific format.
        if let Some(mods) = &self.precomputed_mods {
            if (-12..=12).contains(&exponent) {
                micros.mod_inner = Some(mods[(exponent + 12) as usize].clone());
            } else {
                micros.mod_inner = Some(Arc::new(ScientificModifier { exponent, handler: self.applier.clone() }));
            }
        } else {
            // Unsafe code path: mutates the object and re-uses it as a
            // Modifier!
            self.exponent.store(exponent, Ordering::Relaxed);
            let this: Arc<dyn Modifier> = self.self_ref.upgrade().expect("the handler is alive while it runs");
            micros.mod_inner = Some(this);
        }

        // Change the exponent only after we select appropriate plural form
        // for formatting purposes so that we preserve expected formatted
        // string behavior.
        quantity.adjust_exponent(exponent);

        // We already performed rounding. Do not perform it again.
        micros.rounder = None;

        Ok(micros)
    }
}

impl MultiplierProducer for ScientificHandler {
    fn get_multiplier(&self, magnitude: i32) -> i32 {
        let interval = self.applier.notation.engineering_interval;
        let digits_shown = if self.applier.notation.require_min_int {
            // For patterns like "000.00E0" and ".00E0"
            interval
        } else if interval <= 1 {
            // For patterns like "0.00E0" and "@@@E0"
            1
        } else {
            // For patterns like "##0.00"
            ((magnitude % interval + interval) % interval) + 1
        };
        digits_shown - magnitude - 1
    }
}

impl Modifier for ScientificHandler {
    fn apply(&self, output: &mut FormattedStringBuilder, _left_index: i32, right_index: i32) -> Result<i32> {
        Ok(self.applier.do_apply(self.exponent.load(Ordering::Relaxed), output, right_index))
    }

    fn get_prefix_length(&self) -> Result<i32> {
        // TODO: Localized exponent separator location.
        Ok(0)
    }

    fn get_code_point_count(&self) -> Result<i32> {
        // NOTE: This method is only called one place, NumberRangeFormatterImpl.
        // The call site only cares about != 0 and != 1.
        // Return a very large value so that if this method is used
        // elsewhere, we should notice.
        Ok(999)
    }

    fn is_strong(&self) -> bool {
        // Scientific is always strong
        true
    }

    fn contains_field(&self, _field: FieldRef) -> bool {
        // This method is not currently used. (unsafe path not used in range
        // formatting)
        debug_assert!(false);
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        // This method is not currently used.
        debug_assert!(false);
        None
    }

    fn strict_equals(&self, _other: &dyn Modifier) -> bool {
        // This method is not currently used. (unsafe path not used in range
        // formatting)
        debug_assert!(false);
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to the private class `ScientificNotation.ScientificModifier`.
#[derive(Debug)]
struct ScientificModifier {
    /// `exponent`.
    exponent: i32,
    /// `handler` (the part of it `doApply` reads).
    handler: Arc<ScientificApplier>,
}

impl Modifier for ScientificModifier {
    fn apply(&self, output: &mut FormattedStringBuilder, _left_index: i32, right_index: i32) -> Result<i32> {
        Ok(self.handler.do_apply(self.exponent, output, right_index))
    }

    fn get_prefix_length(&self) -> Result<i32> {
        // TODO: Localized exponent separator location.
        Ok(0)
    }

    fn get_code_point_count(&self) -> Result<i32> {
        // NOTE: This method is only called one place, NumberRangeFormatterImpl.
        // The call site only cares about != 0 and != 1.
        // Return a very large value so that if this method is used
        // elsewhere, we should notice.
        Ok(999)
    }

    fn is_strong(&self) -> bool {
        // Scientific is always strong
        true
    }

    fn contains_field(&self, _field: FieldRef) -> bool {
        // This method is not used for inner modifiers.
        debug_assert!(false);
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        None
    }

    fn strict_equals(&self, other: &dyn Modifier) -> bool {
        match other.as_any().downcast_ref::<ScientificModifier>() {
            Some(other) => self.exponent == other.exponent,
            None => false,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
