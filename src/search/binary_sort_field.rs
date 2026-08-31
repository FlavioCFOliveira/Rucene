//! Sorting on raw binary doc values, ported from
//! `org.apache.lucene.search.BinarySortField`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::search::sort::{MissingValue, SortField, SortFieldKind, SortFieldType};
use crate::store::{DataInput, DataOutput};

/// Sorts by the raw bytes of a binary doc-values field.
///
/// Equivalent to `org.apache.lucene.search.BinarySortField`, a subclass of
/// `SortField` that declares
/// [`SortFieldType::Custom`](crate::search::SortFieldType::Custom) and builds a
/// [`TermValComparator`](crate::search::TermValComparator) over the field's
/// binary doc values; see
/// [`SortedNumericSortField`](crate::search::SortedNumericSortField) for how
/// the subclass identity is carried.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinarySortField {
    sort_field: SortField,
}

impl BinarySortField {
    /// The name this sort field's `SortFieldProvider` is registered under.
    ///
    /// Equivalent to `BinarySortField.Provider.NAME`.
    pub const NAME: &'static str = "BinarySortField";

    /// Creates a sort, possibly reversed, with no missing value.
    ///
    /// Equivalent to `new BinarySortField(String, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`with_provider_name`](Self::with_provider_name).
    pub fn new(field: &str, reverse: bool) -> Result<Self> {
        Self::with_missing_value(field, reverse, None)
    }

    /// Creates a sort, possibly reversed, with an explicit missing value.
    ///
    /// Equivalent to `new BinarySortField(String, boolean, Object)`.
    ///
    /// # Errors
    ///
    /// As [`with_provider_name`](Self::with_provider_name).
    pub fn with_missing_value(
        field: &str,
        reverse: bool,
        missing_value: Option<MissingValue>,
    ) -> Result<Self> {
        Self::with_provider_name(field, reverse, missing_value, Self::NAME)
    }

    /// Creates a sort, possibly reversed, that a subclass persists under its
    /// own provider name.
    ///
    /// Equivalent to the `protected BinarySortField(String, boolean, Object,
    /// String)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when the missing value is neither of the two string
    /// sentinels, and propagates the [`SortField`] construction error, which
    /// cannot trigger for a named field.
    pub fn with_provider_name(
        field: &str,
        reverse: bool,
        missing_value: Option<MissingValue>,
        provider_name: &str,
    ) -> Result<Self> {
        if !matches!(
            missing_value,
            None | Some(MissingValue::StringFirst) | Some(MissingValue::StringLast)
        ) {
            return Err(LuceneError::IllegalArgument(
                "missing value for BinarySortField must be null, STRING_FIRST or STRING_LAST"
                    .to_string(),
            ));
        }
        Ok(Self {
            sort_field: SortField::with_kind(
                Some(field.to_string()),
                SortFieldType::Custom,
                reverse,
                missing_value,
                SortFieldKind::Binary {
                    provider_name: provider_name.to_string(),
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
    /// Equivalent to the `assert sf instanceof BinarySortField` and the cast
    /// that `Provider.writeSortField` performs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the sort field is not one
    /// of this kind.
    pub fn from_sort_field(sort_field: SortField) -> Result<Self> {
        if !matches!(sort_field.kind(), SortFieldKind::Binary { .. }) {
            return Err(LuceneError::IllegalArgument(format!(
                "expected a BinarySortField, got {sort_field}"
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

    /// Returns the name this sort field is persisted under.
    ///
    /// Equivalent to reading the `private final String providerName` field.
    pub fn provider_name(&self) -> &str {
        match self.sort_field.kind() {
            SortFieldKind::Binary { provider_name } => provider_name,
            _ => Self::NAME,
        }
    }

    /// Writes this sort field's payload, without the provider name.
    ///
    /// Equivalent to the private `BinarySortField.serialize(DataOutput)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while writing.
    pub fn serialize(&self, output: &mut dyn DataOutput) -> Result<()> {
        output.write_string(self.get_field().unwrap_or(""))?;
        output.write_int(if self.sort_field.reverse() { 1 } else { 0 })?;
        match self.sort_field.missing_value() {
            Some(MissingValue::StringFirst) => output.write_int(1)?,
            Some(MissingValue::StringLast) => output.write_int(2)?,
            _ => output.write_int(0)?,
        }
        Ok(())
    }

    /// Reads a sort field written by [`serialize`](Self::serialize).
    ///
    /// Equivalent to `BinarySortField.Provider.readSortField(DataInput)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading.
    pub fn read_sort_field(input: &mut dyn DataInput) -> Result<Self> {
        let field = input.read_string()?;
        let reverse = input.read_int()? == 1;
        let missing_value = match input.read_int()? {
            1 => Some(MissingValue::StringFirst),
            2 => Some(MissingValue::StringLast),
            _ => None,
        };
        Self::with_missing_value(&field, reverse, missing_value)
    }
}

impl From<BinarySortField> for SortField {
    fn from(value: BinarySortField) -> Self {
        value.into_sort_field()
    }
}

impl fmt::Display for BinarySortField {
    /// Equivalent to `BinarySortField.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sort_field)
    }
}
