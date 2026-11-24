// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.MultiplierProducer`.

/// A producer of the power-of-ten multiplier for a magnitude (compact and
/// scientific notation).
///
/// Equivalent to the interface
/// `com.ibm.icu.impl.number.MultiplierProducer`.
pub trait MultiplierProducer {
    /// `getMultiplier(int magnitude)`.
    fn get_multiplier(&self, magnitude: i32) -> i32;
}
