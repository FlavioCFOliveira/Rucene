// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MicroPropsGenerator`.

use std::fmt;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::micro_props::MicroProps;

/// This interface is used when all number formatting settings, including
/// the locale, are known, except for the quantity itself. The
/// `process_quantity` method performs the final step in the number
/// processing pipeline: it uses the quantity to generate a finalized
/// [`MicroProps`], which can be used to render the number to output.
///
/// In other words, this interface is used for the parts of number
/// processing that are *quantity-dependent*.
///
/// In order to allow for multiple different objects to all mutate the same
/// MicroProps, a "chain" of MicroPropsGenerators are linked together, and
/// each one is responsible for manipulating a certain quantity-dependent
/// part of the MicroProps. At the top of the linked list is a base instance
/// of `MicroProps` with properties that are not quantity-dependent. Each
/// element in the linked list calls `process_quantity` on its "parent",
/// then does its work, and then returns the result.
///
/// Equivalent to `com.ibm.icu.impl.number.MicroPropsGenerator` (an
/// interface). Generators are shared
/// objects (`Arc<dyn MicroPropsGenerator>`); the `MicroProps` they produce
/// is an owned value (Java's object passed down the chain).
pub trait MicroPropsGenerator: fmt::Debug + Send + Sync {
    /// Considers the given [`DecimalQuantity`], optionally mutates it, and
    /// populates a [`MicroProps`] instance. Equivalent to
    /// `processQuantity(DecimalQuantity)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of rounding (`RoundingMode.UNNECESSARY`)
    /// and the errors of the data loading some generators perform.
    fn process_quantity(&self, quantity: &mut dyn DecimalQuantity) -> Result<MicroProps>;
}
