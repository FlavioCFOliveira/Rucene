// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 **************************************************************************
 * Copyright (C) 2008-2014, Google, International Business Machines
 * Corporation and others. All Rights Reserved.
 **************************************************************************
 */

//! `com.ibm.icu.util.TimeUnit`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use crate::util::measure_unit::{self, MeasureUnit, MeasureUnitProxy, UnitClass};

/// Measurement unit for time units.
///
/// Equivalent to `com.ibm.icu.util.TimeUnit` (a `MeasureUnit` subclass): a
/// view over a [`MeasureUnit`] whose runtime class is `TimeUnit`; it
/// dereferences to the unit for the inherited members.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeUnit(MeasureUnit);

impl TimeUnit {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = -2839973855554750484;

    /// Equivalent to the package-private `TimeUnit(String type, String
    /// code)` (`index` is 0).
    pub(crate) fn new(type_: &str, code: &str) -> Self {
        TimeUnit(MeasureUnit::with_class(
            Some(type_.to_string()),
            Some(code.to_string()),
            UnitClass::TimeUnit { index: 0 },
        ))
    }

    /// The `TimeUnit` behind a static constant (Java's `(TimeUnit)` cast of
    /// `internalGetInstance("duration", ...)`).
    pub(crate) fn constant(type_: &str, sub_type: &str) -> Self {
        TimeUnit::cast(MeasureUnit::constant(type_, sub_type))
    }

    /// Java's `(TimeUnit) unit` cast.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` when `unit` is not a `TimeUnit`.
    pub fn cast(unit: MeasureUnit) -> TimeUnit {
        Self::try_cast(&unit).unwrap_or_else(|| {
            panic!(
                "java.lang.ClassCastException: class {} cannot be cast to class com.ibm.icu.util.TimeUnit",
                unit.java_class_name()
            )
        })
    }

    /// `unit instanceof TimeUnit ? (TimeUnit) unit : null`.
    pub fn try_cast(unit: &MeasureUnit) -> Option<TimeUnit> {
        match unit.unit_class() {
            UnitClass::TimeUnit { .. } => Some(TimeUnit(unit.clone())),
            _ => None,
        }
    }

    /// The unit this view stands for.
    pub fn into_measure_unit(self) -> MeasureUnit {
        self.0
    }

    /// The unit this view stands for.
    pub fn as_measure_unit(&self) -> &MeasureUnit {
        &self.0
    }

    /// Supplies the list of available time units: SECOND, MINUTE, HOUR, DAY,
    /// WEEK, MONTH, YEAR. Equivalent to `values()`.
    pub fn values() -> Vec<TimeUnit> {
        vec![
            measure_unit::SECOND.clone(),
            measure_unit::MINUTE.clone(),
            measure_unit::HOUR.clone(),
            measure_unit::DAY.clone(),
            measure_unit::WEEK.clone(),
            measure_unit::MONTH.clone(),
            measure_unit::YEAR.clone(),
        ]
    }

    /// Equivalent to `writeReplace()`.
    pub fn write_replace(&self) -> MeasureUnitProxy {
        self.0.write_replace()
    }

    /// For backward compatibility only: the constant of the deserialized
    /// `index`. Equivalent to `readResolve()`.
    ///
    /// # Errors
    ///
    /// `InvalidObjectException("Bad index: <index>")`.
    pub fn read_resolve(&self) -> Result<TimeUnit> {
        let index = match self.0.unit_class() {
            UnitClass::TimeUnit { index } => *index,
            _ => 0,
        };
        // The old index field used to be used for this
        match index {
            6 => Ok(measure_unit::SECOND.clone()),
            5 => Ok(measure_unit::MINUTE.clone()),
            4 => Ok(measure_unit::HOUR.clone()),
            3 => Ok(measure_unit::DAY.clone()),
            2 => Ok(measure_unit::WEEK.clone()),
            1 => Ok(measure_unit::MONTH.clone()),
            0 => Ok(measure_unit::YEAR.clone()),
            _ => Err(LuceneError::unchecked(
                "java.io.InvalidObjectException",
                format!("Bad index: {index}"),
            )),
        }
    }
}

impl Deref for TimeUnit {
    type Target = MeasureUnit;

    fn deref(&self) -> &MeasureUnit {
        &self.0
    }
}
