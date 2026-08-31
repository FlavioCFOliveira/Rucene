//! Sorting on a multi-valued numeric field, ported from
//! `org.apache.lucene.search.SortedNumericSortField`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::search::sort::{MissingValue, SortField, SortFieldKind, SortFieldType};
use crate::search::sorted_numeric_selector::SortedNumericSelectorType;
use crate::store::{DataInput, DataOutput};
use crate::util::NumericUtils;

/// Sorts by a field that indexes several numeric values per document, reducing
/// each document's values to one with a
/// [`SortedNumericSelectorType`](crate::search::SortedNumericSelectorType).
///
/// Equivalent to `org.apache.lucene.search.SortedNumericSortField`, a subclass
/// of `SortField` that declares
/// [`SortFieldType::Custom`](crate::search::SortFieldType::Custom) and
/// overrides the comparator it builds. Rust has no implementation inheritance,
/// so this type builds and reads back a [`SortField`] whose
/// [`SortFieldKind`](crate::search::SortFieldKind) carries the subclass
/// identity, and keeps the accessors and the serialization the Java subclass
/// declares.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SortedNumericSortField {
    sort_field: SortField,
}

impl SortedNumericSortField {
    /// The name this sort field's `SortFieldProvider` is registered under.
    ///
    /// Equivalent to `SortedNumericSortField.Provider.NAME`.
    pub const NAME: &'static str = "SortedNumericSortField";

    /// Creates a sort in the given numeric type, ascending, selecting the
    /// minimum value of each document.
    ///
    /// Equivalent to `new SortedNumericSortField(String, SortField.Type)`.
    ///
    /// # Errors
    ///
    /// As [`with_missing_value`](Self::with_missing_value).
    pub fn new(field: &str, numeric_type: SortFieldType) -> Result<Self> {
        Self::with_reverse(field, numeric_type, false)
    }

    /// Creates a sort in the given numeric type, possibly reversed, selecting
    /// the minimum value of each document.
    ///
    /// Equivalent to
    /// `new SortedNumericSortField(String, SortField.Type, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`with_missing_value`](Self::with_missing_value).
    pub fn with_reverse(field: &str, numeric_type: SortFieldType, reverse: bool) -> Result<Self> {
        Self::with_selector(field, numeric_type, reverse, SortedNumericSelectorType::MIN)
    }

    /// Creates a sort in the given numeric type, possibly reversed, with an
    /// explicit selector.
    ///
    /// Equivalent to
    /// `new SortedNumericSortField(String, SortField.Type, boolean, SortedNumericSelector.Type)`.
    ///
    /// # Errors
    ///
    /// As [`with_missing_value`](Self::with_missing_value).
    pub fn with_selector(
        field: &str,
        numeric_type: SortFieldType,
        reverse: bool,
        selector: SortedNumericSelectorType,
    ) -> Result<Self> {
        Self::with_missing_value(field, numeric_type, reverse, selector, None)
    }

    /// Creates a sort in the given numeric type, possibly reversed, with an
    /// explicit selector and missing value.
    ///
    /// Equivalent to
    /// `new SortedNumericSortField(String, SortField.Type, boolean, SortedNumericSelector.Type, Object)`.
    ///
    /// # Errors
    ///
    /// Propagates the [`SortField`] construction error, which cannot trigger
    /// for a named field.
    pub fn with_missing_value(
        field: &str,
        numeric_type: SortFieldType,
        reverse: bool,
        selector: SortedNumericSelectorType,
        missing_value: Option<MissingValue>,
    ) -> Result<Self> {
        Ok(Self {
            sort_field: SortField::with_kind(
                Some(field.to_string()),
                SortFieldType::Custom,
                reverse,
                missing_value,
                SortFieldKind::SortedNumeric {
                    selector,
                    numeric_type,
                },
            )?,
        })
    }

    /// Returns this sort field as a [`SortField`].
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// Re-wraps a [`SortField`] that carries this subclass's identity.
    ///
    /// Equivalent to the `assert sf instanceof SortedNumericSortField` and the cast
    /// that `Provider.writeSortField` performs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the sort field is not one
    /// of this kind.
    pub fn from_sort_field(sort_field: SortField) -> Result<Self> {
        if !matches!(sort_field.kind(), SortFieldKind::SortedNumeric { .. }) {
            return Err(LuceneError::IllegalArgument(format!(
                "expected a SortedNumericSortField, got {sort_field}"
            )));
        }
        Ok(Self { sort_field })
    }

    /// Unwraps this sort field into a plain [`SortField`], which is what
    /// [`Sort`](crate::search::Sort) holds.
    pub fn into_sort_field(self) -> SortField {
        self.sort_field
    }

    /// Returns the field being sorted on.
    pub fn get_field(&self) -> Option<&str> {
        self.sort_field.field()
    }

    /// Returns the numeric type of the field's values.
    ///
    /// Equivalent to `SortedNumericSortField.getNumericType()`.
    pub fn get_numeric_type(&self) -> SortFieldType {
        match self.sort_field.kind() {
            SortFieldKind::SortedNumeric { numeric_type, .. } => *numeric_type,
            _ => SortFieldType::Custom,
        }
    }

    /// Returns the selector in use.
    ///
    /// Equivalent to `SortedNumericSortField.getSelector()`.
    pub fn get_selector(&self) -> SortedNumericSelectorType {
        match self.sort_field.kind() {
            SortFieldKind::SortedNumeric { selector, .. } => *selector,
            _ => SortedNumericSelectorType::MIN,
        }
    }

    /// Writes this sort field's payload, without the provider name.
    ///
    /// Equivalent to the private `SortedNumericSortField.serialize(DataOutput)`,
    /// which `Provider.writeSortField` calls.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the numeric type or the
    /// missing value cannot be serialised — the cases Java reports with an
    /// `AssertionError` — and propagates any I/O error.
    pub fn serialize(&self, output: &mut dyn DataOutput) -> Result<()> {
        let numeric_type = self.get_numeric_type();
        output.write_string(self.get_field().unwrap_or(""))?;
        output.write_string(numeric_type.java_name())?;
        output.write_int(if self.sort_field.reverse() { 1 } else { 0 })?;
        output.write_int(self.get_selector().ordinal())?;
        match self.sort_field.missing_value() {
            None => output.write_int(0)?,
            Some(missing) => {
                output.write_int(1)?;
                match (numeric_type, missing) {
                    (SortFieldType::Int, MissingValue::Int(value)) => output.write_int(value)?,
                    (SortFieldType::Long, MissingValue::Long(value)) => output.write_long(value)?,
                    (SortFieldType::Float, MissingValue::Float(value)) => {
                        output.write_int(NumericUtils::float_to_sortable_int(value))?
                    }
                    (SortFieldType::Double, MissingValue::Double(value)) => {
                        output.write_long(NumericUtils::double_to_sortable_long(value))?
                    }
                    _ => {
                        return Err(LuceneError::IllegalArgument(format!(
                        "Cannot serialize missing value {missing} for numeric type {numeric_type}"
                    )))
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads a sort field written by [`serialize`](Self::serialize).
    ///
    /// Equivalent to `SortedNumericSortField.Provider.readSortField(DataInput)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an unknown selector or a
    /// numeric type that cannot carry a missing value, and propagates any I/O
    /// error.
    pub fn read_sort_field(input: &mut dyn DataInput) -> Result<Self> {
        let field = input.read_string()?;
        let numeric_type = SortFieldType::parse(&input.read_string()?)?;
        let reverse = input.read_int()? == 1;
        let selector_ordinal = input.read_int()?;
        let selector = SortedNumericSelectorType::from_ordinal(selector_ordinal).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "Can't deserialize SortedNumericSortField - unknown selector type {selector_ordinal}"
            ))
        })?;
        let missing_value = if input.read_int()? == 1 {
            Some(match numeric_type {
                SortFieldType::Int => MissingValue::Int(input.read_int()?),
                SortFieldType::Long => MissingValue::Long(input.read_long()?),
                SortFieldType::Float => {
                    MissingValue::Float(NumericUtils::sortable_int_to_float(input.read_int()?))
                }
                SortFieldType::Double => {
                    MissingValue::Double(NumericUtils::sortable_long_to_double(input.read_long()?))
                }
                other => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Cannot deserialize a missing value for numeric type {other}"
                    )))
                }
            })
        } else {
            None
        };
        Self::with_missing_value(&field, numeric_type, reverse, selector, missing_value)
    }
}

impl From<SortedNumericSortField> for SortField {
    fn from(value: SortedNumericSortField) -> Self {
        value.into_sort_field()
    }
}

impl fmt::Display for SortedNumericSortField {
    /// Equivalent to `SortedNumericSortField.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sort_field)
    }
}
