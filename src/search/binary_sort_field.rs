//! Sorting on raw binary doc values, ported from
//! `org.apache.lucene.search.BinarySortField`.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::index_sorter::{
    BinaryComparator, BinaryDocValuesProvider, BinarySorter, IndexSorter,
};
use crate::index::{BinaryDocValues, DocValues, LeafReader};
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

    /// Returns the per-document sort key, as [`BinaryDocValues`] whose value
    /// is the bytes to sort on.
    ///
    /// The default returns the field's values unchanged; a subclass derives a
    /// key from the stored bytes and/or other doc-values fields of the segment.
    /// Values are read in increasing docID order, so any underlying doc values
    /// must be advanced with `next_doc()` only, since at flush time the
    /// in-memory doc values do not support `advance_exact`.
    ///
    /// Equivalent to `org.apache.lucene.search.BinarySortField#getSortKeyDocValues`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field exists with another
    /// doc-values type, and propagates any I/O error.
    pub fn get_sort_key_doc_values(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Box<dyn BinaryDocValues>> {
        DocValues::get_binary(reader, self.get_field().unwrap_or(""))
    }

    /// Returns the [`IndexSorter`] that sorts a segment by this field.
    ///
    /// Equivalent to `BinarySortField.getIndexSorter()`
    /// (`BinarySortField.java:178-182`): an `IndexSorter.BinarySorter` under
    /// this sort field's provider name and reverse flag, comparing with
    /// [`comparator`](Self::comparator) and reading
    /// [`get_sort_key_doc_values`](Self::get_sort_key_doc_values).
    pub fn get_index_sorter(&self) -> Arc<dyn IndexSorter> {
        binary_index_sorter(
            self.get_field().unwrap_or(""),
            self.sort_field.reverse(),
            self.sort_field.missing_value(),
            self.provider_name(),
        )
    }

    /// Returns the unsigned byte-order comparator that treats a missing value
    /// (`None`) as smaller than any value, or larger when the missing value is
    /// [`MissingValue::StringLast`].
    ///
    /// Equivalent to the private `BinarySortField.comparator()`
    /// (`BinarySortField.java:163-176`); `BytesRef.compareTo` is the unsigned
    /// lexicographic order Rust's slice ordering gives.
    pub fn comparator(&self) -> BinaryComparator {
        binary_comparator(self.sort_field.missing_value())
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

/// The comparator of a binary sort whose missing value is `missing_value`.
///
/// Equivalent to the body of the private `BinarySortField.comparator()`.
fn binary_comparator(missing_value: Option<MissingValue>) -> BinaryComparator {
    let missing_last = missing_value == Some(MissingValue::StringLast);
    Arc::new(move |a: Option<&[u8]>, b: Option<&[u8]>| match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (None, None) => Ordering::Equal,
        (a, _) => {
            let c = if a.is_none() {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            if missing_last {
                c.reverse()
            } else {
                c
            }
        }
    })
}

/// Builds the index sorter of a binary sort field.
///
/// Shared by [`BinarySortField::get_index_sorter`] and
/// [`SortField::get_index_sorter`], which dispatches on the subclass identity a
/// [`SortField`] carries; see [`SortFieldKind`].
pub(crate) fn binary_index_sorter(
    field: &str,
    reverse: bool,
    missing_value: Option<MissingValue>,
    provider_name: &str,
) -> Arc<dyn IndexSorter> {
    let field = field.to_string();
    // `this::getSortKeyDocValues`, whose default is `DocValues.getBinary`.
    let values_provider: BinaryDocValuesProvider =
        Arc::new(move |reader: &dyn LeafReader| DocValues::get_binary(reader, &field));
    Arc::new(BinarySorter::new(
        provider_name,
        reverse,
        binary_comparator(missing_value),
        values_provider,
    ))
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
