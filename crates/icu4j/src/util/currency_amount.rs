// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 **********************************************************************
 * Copyright (c) 2004-2016, International Business Machines
 * Corporation and others.  All Rights Reserved.
 **********************************************************************
 * Author: Alan Liu
 * Created: April 12, 2004
 * Since: ICU 3.0
 **********************************************************************
 */

//! `com.ibm.icu.util.CurrencyAmount`.

use std::ops::Deref;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::jdk::currency::Currency as JavaCurrency;
use crate::util::currency::Currency;
use crate::util::measure::Measure;

/// An amount of currency, consisting of a Number and a Currency.
/// CurrencyAmount objects are immutable.
///
/// Equivalent to `com.ibm.icu.util.CurrencyAmount` (a `Measure`
/// subclass): it dereferences to its [`Measure`]. A `java.util.Currency`
/// argument is the emulation [`JavaCurrency`].
#[derive(Debug, Clone)]
pub struct CurrencyAmount(Measure);

impl CurrencyAmount {
    /// Constructs a new object given a number and a currency. Equivalent to
    /// `CurrencyAmount(Number, Currency)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("Number and MeasureUnit must not be null")`.
    pub fn new(number: Option<JavaNumber>, currency: Option<Currency>) -> Result<Self> {
        Ok(CurrencyAmount(Measure::new(number, currency.map(Currency::into_measure_unit))?))
    }

    /// Constructs a new object given a double value and a currency.
    /// Equivalent to `CurrencyAmount(double, Currency)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_double(number: f64, currency: Option<Currency>) -> Result<Self> {
        Self::new(Some(JavaNumber::Double(number)), currency)
    }

    /// Constructs a new object given a number and a Java currency.
    /// Equivalent to `CurrencyAmount(Number, java.util.Currency)`.
    ///
    /// # Errors
    ///
    /// The errors of `Currency.getInstance(String)` and of
    /// [`new`](Self::new).
    pub fn with_java_currency(number: Option<JavaNumber>, currency: &JavaCurrency) -> Result<Self> {
        Self::new(number, Some(Currency::from_java_currency(currency)?))
    }

    /// Constructs a new object given a double value and a Java currency.
    /// Equivalent to `CurrencyAmount(double, java.util.Currency)`.
    ///
    /// # Errors
    ///
    /// As [`with_java_currency`](Self::with_java_currency).
    pub fn from_double_with_java_currency(number: f64, currency: &JavaCurrency) -> Result<Self> {
        Self::with_java_currency(Some(JavaNumber::Double(number)), currency)
    }

    /// Returns the currency of this object. Equivalent to `getCurrency()`.
    pub fn get_currency(&self) -> Currency {
        Currency::cast(self.0.get_unit().clone())
    }

    /// The measure this amount is.
    pub fn as_measure(&self) -> &Measure {
        &self.0
    }
}

impl Deref for CurrencyAmount {
    type Target = Measure;

    fn deref(&self) -> &Measure {
        &self.0
    }
}
