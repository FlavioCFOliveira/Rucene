// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MicroProps`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::number::integer_width::IntegerWidth;
use crate::number::number_formatter::{DecimalSeparatorDisplay, SignDisplay};
use crate::number::precision::Precision;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::grouper::Grouper;
use crate::r#impl::number::micro_props_generator::MicroPropsGenerator;
use crate::r#impl::number::modifier::Modifier;
use crate::r#impl::number::padder::Padder;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::measure::Measure;
use crate::util::measure_unit::MeasureUnit;

/// The quantity-dependent and quantity-independent settings of one
/// formatting pass.
///
/// Equivalent to `com.ibm.icu.impl.number.MicroProps` (public mutable
/// fields; `null` is `None`). `clone()` is `Clone` (Java's shallow copy:
/// shared objects are `Arc` handles). Its role as the head of a
/// `MicroPropsGenerator` chain is [`MicroPropsHead`].
#[derive(Debug, Clone)]
pub struct MicroProps {
    // Populated globally:
    /// `sign`.
    pub sign: Option<SignDisplay>,
    /// `symbols`.
    pub symbols: Option<Arc<DecimalFormatSymbols>>,
    /// `nsName`.
    pub ns_name: Option<String>,
    /// `padding`.
    pub padding: Option<Padder>,
    /// `decimal`.
    pub decimal: Option<DecimalSeparatorDisplay>,
    /// `integerWidth`.
    pub integer_width: Option<IntegerWidth>,

    // Populated by notation/unit:
    /// `modOuter`.
    pub mod_outer: Option<Arc<dyn Modifier>>,
    /// `modMiddle`.
    pub mod_middle: Option<Arc<dyn Modifier>>,
    /// `modInner`.
    pub mod_inner: Option<Arc<dyn Modifier>>,
    /// `rounder`.
    pub rounder: Option<Precision>,
    /// `grouping`.
    pub grouping: Option<Grouper>,
    /// `useCurrency`.
    pub use_currency: bool,
    /// `gender`.
    pub gender: Option<String>,

    // Currency symbol to be used as the decimal separator
    /// `currencyAsDecimal`.
    pub currency_as_decimal: Option<Vec<u16>>,

    // Internal fields:
    /// `immutable`.
    immutable: bool,

    /// The output unit, in case the unit changed during the pipeline
    /// (`outputUnit`).
    pub output_unit: Option<MeasureUnit>,

    /// Contains all the measures (`mixedMeasures`).
    pub mixed_measures: Option<Vec<Measure>>,

    /// Points to quantity position, -1 if the position is not set yet
    /// (`indexOfQuantity`).
    pub index_of_quantity: i32,

    /// `exhausted` (`volatile`).
    exhausted: bool,
}

impl MicroProps {
    /// Equivalent to `MicroProps(boolean immutable)`: whether this object
    /// should behave as an immutable after construction with respect to the
    /// quantity chain.
    pub fn new(immutable: bool) -> Self {
        MicroProps {
            sign: None,
            symbols: None,
            ns_name: None,
            padding: None,
            decimal: None,
            integer_width: None,
            mod_outer: None,
            mod_middle: None,
            mod_inner: None,
            rounder: None,
            grouping: None,
            use_currency: false,
            gender: None,
            currency_as_decimal: None,
            immutable,
            output_unit: None,
            mixed_measures: None,
            index_of_quantity: -1,
            exhausted: false,
        }
    }

    /// The symbols, set by `macrosToMicroGenerator` before any use.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` when they were never set.
    pub fn symbols(&self) -> &Arc<DecimalFormatSymbols> {
        self.symbols.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException: micros.symbols"))
    }
}

/// A [`MicroProps`] at the head of a `MicroPropsGenerator` chain.
///
/// Java's `MicroProps` implements `MicroPropsGenerator`:
/// `processQuantity` answers a clone when the object is immutable, and
/// otherwise answers the object itself, once (`AssertionError("Cannot
/// re-use a mutable MicroProps in the quantity chain")` on a second call).
/// A Rust method cannot answer its own receiver by value, so the head keeps
/// the object in a cell: an immutable head answers clones, a mutable one
/// hands its object out on the first call. The object is configured
/// through [`lock`](Self::lock) while the chain is built, as Java writes
/// the fields of the head after linking it.
#[derive(Debug)]
pub struct MicroPropsHead {
    /// The object.
    micros: Mutex<MicroProps>,
}

impl MicroPropsHead {
    /// The head over `micros`.
    pub fn new(micros: MicroProps) -> Arc<MicroPropsHead> {
        Arc::new(MicroPropsHead { micros: Mutex::new(micros) })
    }

    /// The object, to configure it.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, MicroProps> {
        self.micros.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl MicroPropsGenerator for MicroPropsHead {
    /// Equivalent to `MicroProps.processQuantity(DecimalQuantity)`.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError("Cannot re-use a mutable MicroProps in the
    /// quantity chain")` on a second call on a mutable object.
    fn process_quantity(&self, _quantity: &mut dyn DecimalQuantity) -> Result<MicroProps> {
        let mut m = self.lock();
        if m.immutable {
            Ok(m.clone())
        } else if m.exhausted {
            // Safety check
            panic!("java.lang.AssertionError: Cannot re-use a mutable MicroProps in the quantity chain");
        } else {
            m.exhausted = true;
            let mut taken = std::mem::replace(&mut *m, MicroProps::new(false));
            taken.exhausted = true;
            m.exhausted = true;
            Ok(taken)
        }
    }
}
