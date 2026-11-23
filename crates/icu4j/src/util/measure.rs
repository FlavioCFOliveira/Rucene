// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 **********************************************************************
 * Copyright (c) 2004-2013, International Business Machines
 * Corporation and others.  All Rights Reserved.
 **********************************************************************
 * Author: Alan Liu
 * Created: April 20, 2004
 * Since: ICU 3.0
 **********************************************************************
 */

//! `com.ibm.icu.util.Measure`.

use std::fmt;

use rucene::error::Result;
use rucene::util::java_collections::java_double_hash_code;

use crate::jdk::null_pointer;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::util::measure_unit::MeasureUnit;

/// An amount of a specified unit, consisting of a Number and a Unit. For
/// example, a length measure consists of a Number and a length unit, such
/// as feet or meters.
///
/// Equivalent to `com.ibm.icu.util.Measure`; its subclass
/// `CurrencyAmount` is [`CurrencyAmount`](crate::util::currency_amount::CurrencyAmount),
/// which holds one.
#[derive(Debug, Clone)]
pub struct Measure {
    number: JavaNumber,
    unit: MeasureUnit,
}

impl Measure {
    /// Constructs a new object given a number and a unit. Equivalent to
    /// `Measure(Number, MeasureUnit)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("Number and MeasureUnit must not be null")`.
    pub fn new(number: Option<JavaNumber>, unit: Option<MeasureUnit>) -> Result<Self> {
        match (number, unit) {
            (Some(number), Some(unit)) => Ok(Self { number, unit }),
            _ => Err(null_pointer("Number and MeasureUnit must not be null")),
        }
    }

    /// Returns true if the given object is equal to this object. Equivalent
    /// to `equals(Object)`.
    pub fn java_equals(&self, m: &Measure) -> bool {
        if std::ptr::eq(self, m) {
            return true;
        }
        self.unit.java_equals(&m.unit) && Self::numbers_equal(&self.number, &m.number)
    }

    /// See if two numbers are identical or have the same double value.
    /// Equivalent to `numbersEqual(Number, Number)`.
    fn numbers_equal(a: &JavaNumber, b: &JavaNumber) -> bool {
        if a.java_equals(b) {
            return true;
        }
        a.double_value() == b.double_value()
    }

    /// Returns a hashcode for this object. Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        31i32.wrapping_mul(java_double_hash_code(self.number.double_value())).wrapping_add(self.unit.hash_code())
    }

    /// Returns a string representation of this object. Equivalent to
    /// `toString()`.
    pub fn to_java_string(&self) -> String {
        format!("{} {}", self.number.to_java_string(), self.unit.to_java_string())
    }

    /// Returns the numeric value of this object. Equivalent to
    /// `getNumber()`.
    pub fn get_number(&self) -> &JavaNumber {
        &self.number
    }

    /// Returns the unit of this object. Equivalent to `getUnit()`.
    pub fn get_unit(&self) -> &MeasureUnit {
        &self.unit
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
