// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.ConstrainedFieldPosition`.

use std::fmt;

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{same_field, values_equal, AttributeValue, FieldRef};

/// `ConstrainedFieldPosition.ConstraintType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintType {
    /// Represents the lack of a constraint.
    None,
    /// Represents that the field class is constrained.
    Class,
    /// Represents that the field is constrained.
    Field,
    /// Represents that the field value is constrained.
    Value,
}

/// Represents a span of a string containing a given field, optionally
/// constrained to a field, a field class or a field value; used to iterate
/// over the fields of a `FormattedValue`.
///
/// Equivalent to `com.ibm.icu.text.ConstrainedFieldPosition`. Java's
/// `Class<?>` constraint is the Java class name (`Object.class` is
/// `"java.lang.Object"`), tested with [`Attribute::is_instance_of`](crate::jdk::text::Attribute::is_instance_of);
/// `Object` field values are [`AttributeValue`]s, Java's `null` `None`.
#[derive(Debug, Clone)]
pub struct ConstrainedFieldPosition {
    f_constraint: ConstraintType,
    f_class_constraint: &'static str,
    f_field: Option<FieldRef>,
    f_value: Option<AttributeValue>,
    f_start: i32,
    f_limit: i32,
    f_context: i64,
}

impl Default for ConstrainedFieldPosition {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstrainedFieldPosition {
    /// `new ConstrainedFieldPosition()`: no constraint, empty span.
    pub fn new() -> Self {
        let mut c = Self {
            f_constraint: ConstraintType::None,
            f_class_constraint: "java.lang.Object",
            f_field: None,
            f_value: None,
            f_start: 0,
            f_limit: 0,
            f_context: 0,
        };
        c.reset();
        c
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.f_constraint = ConstraintType::None;
        self.f_class_constraint = "java.lang.Object";
        self.f_field = None;
        self.f_value = None;
        self.f_start = 0;
        self.f_limit = 0;
        self.f_context = 0;
    }

    /// `constrainField(Field field)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Cannot constrain on null field")` for
    /// Java's `null`.
    pub fn constrain_field(&mut self, field: Option<FieldRef>) -> Result<()> {
        let Some(field) = field else {
            return Err(LuceneError::illegal_argument("Cannot constrain on null field"));
        };
        self.f_constraint = ConstraintType::Field;
        self.f_class_constraint = "java.lang.Object";
        self.f_field = Some(field);
        self.f_value = None;
        Ok(())
    }

    /// `constrainClass(Class<?> classConstraint)`: the Java class name of
    /// the constraint.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Cannot constrain on null field class")`
    /// for Java's `null`.
    pub fn constrain_class(&mut self, class_constraint: Option<&'static str>) -> Result<()> {
        let Some(class_constraint) = class_constraint else {
            return Err(LuceneError::illegal_argument("Cannot constrain on null field class"));
        };
        self.f_constraint = ConstraintType::Class;
        self.f_class_constraint = class_constraint;
        self.f_field = None;
        self.f_value = None;
        Ok(())
    }

    /// `constrainFieldAndValue(Field field, Object fieldValue)`
    /// (deprecated, internal).
    pub fn constrain_field_and_value(&mut self, field: Option<FieldRef>, field_value: Option<AttributeValue>) {
        self.f_constraint = ConstraintType::Value;
        self.f_class_constraint = "java.lang.Object";
        self.f_field = field;
        self.f_value = field_value;
    }

    /// `getField()`.
    pub fn get_field(&self) -> Option<FieldRef> {
        self.f_field
    }

    /// `getStart()`.
    pub fn get_start(&self) -> i32 {
        self.f_start
    }

    /// `getLimit()`.
    pub fn get_limit(&self) -> i32 {
        self.f_limit
    }

    /// `getFieldValue()`.
    pub fn get_field_value(&self) -> Option<AttributeValue> {
        self.f_value
    }

    /// `getInt64IterationContext()`.
    pub fn get_int64_iteration_context(&self) -> i64 {
        self.f_context
    }

    /// `setInt64IterationContext(long context)`.
    pub fn set_int64_iteration_context(&mut self, context: i64) {
        self.f_context = context;
    }

    /// `setState(Field field, Object value, int start, int limit)`.
    pub fn set_state(&mut self, field: Option<FieldRef>, value: Option<AttributeValue>, start: i32, limit: i32) {
        // Clients of ConstrainedFieldPosition should not call setState with a field that does
        // not match the constraint.
        if field.is_some() {
            debug_assert!(self.matches_field(field, value.as_ref()).unwrap_or(false));
        }
        self.f_field = field;
        self.f_value = value;
        self.f_start = start;
        self.f_limit = limit;
    }

    /// `matchesField(Field field, Object fieldValue)`: whether the
    /// constraint admits `field` with `field_value`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("field must not be null")` for Java's
    /// `null` field.
    pub fn matches_field(&self, field: Option<FieldRef>, field_value: Option<&AttributeValue>) -> Result<bool> {
        let Some(f) = field else {
            return Err(LuceneError::illegal_argument("field must not be null"));
        };
        Ok(match self.f_constraint {
            ConstraintType::None => true,
            ConstraintType::Class => f.is_instance_of(self.f_class_constraint),
            ConstraintType::Field => same_field(self.f_field, field),
            ConstraintType::Value => same_field(self.f_field, field) && values_equal(self.f_value.as_ref(), field_value),
        })
    }
}

impl fmt::Display for ConstrainedFieldPosition {
    /// `toString()`: `CFPos[<start>-<limit> <field>]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = self.f_field.map_or_else(|| "null".to_string(), |x| x.to_java_string());
        write!(f, "CFPos[{}-{} {}]", self.f_start, self.f_limit, field)
    }
}
