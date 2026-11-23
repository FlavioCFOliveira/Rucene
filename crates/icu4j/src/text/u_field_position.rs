// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UFieldPosition`.

use std::any::Any;

use crate::jdk::text::{FieldPosition, FieldPositionSubclass, FieldRef};

/// The state `UFieldPosition` adds to a `FieldPosition`: the number of
/// visible fraction digits and the fraction digits as a `long`.
///
/// Equivalent to the deprecated class `com.ibm.icu.text.UFieldPosition`,
/// which extends `java.text.FieldPosition`. A `UFieldPosition` object is
/// a [`FieldPosition`] carrying this state as its subclass extension
/// (see [`FieldPosition`]); the constructors return that position,
/// [`of`](Self::of)/[`of_mut`](Self::of_mut) are `instanceof` plus the
/// cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UFieldPosition {
    count_visible_fraction_digits: i32,
    fraction_digits: i64,
}

impl FieldPositionSubclass for UFieldPosition {
    fn java_class(&self) -> &'static str {
        "com.ibm.icu.text.UFieldPosition"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FieldPositionSubclass> {
        Box::new(self.clone())
    }
}

impl UFieldPosition {
    fn state() -> Box<dyn FieldPositionSubclass> {
        Box::new(UFieldPosition { count_visible_fraction_digits: -1, fraction_digits: 0 })
    }

    /// `new UFieldPosition()`: field id -1.
    pub fn new() -> FieldPosition {
        FieldPosition::new(-1).with_subclass(Self::state())
    }

    /// `new UFieldPosition(int field)`.
    pub fn with_field(field: i32) -> FieldPosition {
        FieldPosition::new(field).with_subclass(Self::state())
    }

    /// `new UFieldPosition(Field attribute, int fieldID)`.
    pub fn with_attribute_and_field(attribute: Option<FieldRef>, field_id: i32) -> FieldPosition {
        FieldPosition::with_attribute_and_field(attribute, field_id).with_subclass(Self::state())
    }

    /// `new UFieldPosition(Field attribute)`.
    pub fn with_attribute(attribute: Option<FieldRef>) -> FieldPosition {
        FieldPosition::with_attribute(attribute).with_subclass(Self::state())
    }

    /// `fp instanceof UFieldPosition ? (UFieldPosition) fp : null`.
    pub fn of(fp: &FieldPosition) -> Option<&UFieldPosition> {
        fp.subclass()?.as_any().downcast_ref()
    }

    /// The mutable form of [`of`](Self::of).
    pub fn of_mut(fp: &mut FieldPosition) -> Option<&mut UFieldPosition> {
        fp.subclass_mut()?.as_any_mut().downcast_mut()
    }

    /// `setFractionDigits(int countVisibleFractionDigits, long
    /// fractionDigits)`.
    pub fn set_fraction_digits(&mut self, count_visible_fraction_digits: i32, fraction_digits: i64) {
        self.count_visible_fraction_digits = count_visible_fraction_digits;
        self.fraction_digits = fraction_digits;
    }

    /// `getCountVisibleFractionDigits()`.
    pub fn get_count_visible_fraction_digits(&self) -> i32 {
        self.count_visible_fraction_digits
    }

    /// `getFractionDigits()`.
    pub fn get_fraction_digits(&self) -> i64 {
        self.fraction_digits
    }
}
