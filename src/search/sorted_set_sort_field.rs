//! Sorting on a multi-valued term field, ported from
//! `org.apache.lucene.search.SortedSetSortField`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::search::sort::{MissingValue, SortField, SortFieldKind, SortFieldType};
use crate::search::sorted_set_selector::SortedSetSelectorType;
use crate::store::{DataInput, DataOutput};

/// Sorts by a field that indexes several terms per document, reducing each
/// document's ordinals to one with a
/// [`SortedSetSelectorType`](crate::search::SortedSetSelectorType).
///
/// Equivalent to `org.apache.lucene.search.SortedSetSortField`, a subclass of
/// `SortField` that declares
/// [`SortFieldType::Custom`](crate::search::SortFieldType::Custom) and
/// overrides the comparator it builds; see
/// [`SortedNumericSortField`](crate::search::SortedNumericSortField) for how
/// the subclass identity is carried.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SortedSetSortField {
    sort_field: SortField,
}

impl SortedSetSortField {
    /// The name this sort field's `SortFieldProvider` is registered under.
    ///
    /// Equivalent to `SortedSetSortField.Provider.NAME`.
    pub const NAME: &'static str = "SortedSetSortField";

    /// Creates a sort, possibly reversed, selecting the minimum term of each
    /// document.
    ///
    /// Equivalent to `new SortedSetSortField(String, boolean)`.
    ///
    /// # Errors
    ///
    /// As [`with_missing_value`](Self::with_missing_value).
    pub fn new(field: &str, reverse: bool) -> Result<Self> {
        Self::with_selector(field, reverse, SortedSetSelectorType::MIN)
    }

    /// Creates a sort, possibly reversed, with an explicit selector.
    ///
    /// Equivalent to
    /// `new SortedSetSortField(String, boolean, SortedSetSelector.Type)`.
    ///
    /// # Errors
    ///
    /// As [`with_missing_value`](Self::with_missing_value).
    pub fn with_selector(
        field: &str,
        reverse: bool,
        selector: SortedSetSelectorType,
    ) -> Result<Self> {
        Self::with_missing_value(field, reverse, selector, None)
    }

    /// Creates a sort, possibly reversed, with an explicit selector and
    /// missing value.
    ///
    /// Equivalent to
    /// `new SortedSetSortField(String, boolean, SortedSetSelector.Type, Object)`.
    ///
    /// # Errors
    ///
    /// Propagates the [`SortField`] construction error, which cannot trigger
    /// for a named field.
    pub fn with_missing_value(
        field: &str,
        reverse: bool,
        selector: SortedSetSelectorType,
        missing_value: Option<MissingValue>,
    ) -> Result<Self> {
        Ok(Self {
            sort_field: SortField::with_kind(
                Some(field.to_string()),
                SortFieldType::Custom,
                reverse,
                missing_value,
                SortFieldKind::SortedSet { selector },
            )?,
        })
    }

    /// Returns this sort field as a [`SortField`].
    /// Returns the [`IndexSorter`](crate::index::index_sorter::IndexSorter)
    /// that sorts a segment by this field.
    ///
    /// Equivalent to `SortedSetSortField.getIndexSorter()`. The sorter reads
    /// the sorted-set doc values and reduces each document to the one term the
    /// selector picks.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field is unnamed.
    pub fn get_index_sorter(
        &self,
    ) -> Result<std::sync::Arc<dyn crate::index::index_sorter::IndexSorter>> {
        use crate::index::index_sorter::StringSorter;
        let field = self
            .get_field()
            .ok_or_else(|| {
                LuceneError::IllegalArgument("a sort field must name a field".to_string())
            })?
            .to_string();
        let selector = self.get_selector();
        Ok(std::sync::Arc::new(StringSorter::new(
            Self::NAME,
            matches!(
                self.sort_field.missing_value(),
                Some(crate::search::MissingValue::StringLast)
            ),
            self.sort_field.reverse(),
            std::sync::Arc::new(move |reader: &dyn crate::index::LeafReader| {
                let Some(values) = reader.get_sorted_set_doc_values(&field)? else {
                    return Ok(None);
                };
                Ok(Some(
                    crate::search::sorted_set_selector::SortedSetSelector::wrap(values, selector)?,
                ))
            }),
        )))
    }

    /// Returns the [`SortField`] this sort field extends.
    ///
    /// Java's `SortedSetSortField` *is* a `SortField`; this port composes one,
    /// and this accessor exposes it.
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// Re-wraps a [`SortField`] that carries this subclass's identity.
    ///
    /// Equivalent to the `assert sf instanceof SortedSetSortField` and the cast
    /// that `Provider.writeSortField` performs.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the sort field is not one
    /// of this kind.
    pub fn from_sort_field(sort_field: SortField) -> Result<Self> {
        if !matches!(sort_field.kind(), SortFieldKind::SortedSet { .. }) {
            return Err(LuceneError::IllegalArgument(format!(
                "expected a SortedSetSortField, got {sort_field}"
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

    /// Returns the selector in use.
    ///
    /// Equivalent to `SortedSetSortField.getSelector()`.
    pub fn get_selector(&self) -> SortedSetSelectorType {
        match self.sort_field.kind() {
            SortFieldKind::SortedSet { selector } => *selector,
            _ => SortedSetSelectorType::MIN,
        }
    }

    /// Writes this sort field's payload, without the provider name.
    ///
    /// Equivalent to the private `SortedSetSortField.serialize(DataOutput)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while writing.
    pub fn serialize(&self, output: &mut dyn DataOutput) -> Result<()> {
        output.write_string(self.get_field().unwrap_or(""))?;
        output.write_int(if self.sort_field.reverse() { 1 } else { 0 })?;
        output.write_int(self.get_selector().ordinal())?;
        match self.sort_field.missing_value() {
            Some(MissingValue::StringFirst) => output.write_int(1)?,
            Some(MissingValue::StringLast) => output.write_int(2)?,
            _ => output.write_int(0)?,
        }
        Ok(())
    }

    /// Reads the selector type of a serialised sort field.
    ///
    /// Equivalent to the private
    /// `SortedSetSortField.readSelectorType(DataInput)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an unknown selector, and
    /// propagates any I/O error.
    fn read_selector_type(input: &mut dyn DataInput) -> Result<SortedSetSelectorType> {
        let selector_ordinal = input.read_int()?;
        SortedSetSelectorType::from_ordinal(selector_ordinal).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "Cannot deserialize SortedSetSortField: unknown selector type {selector_ordinal}"
            ))
        })
    }

    /// Reads a sort field written by [`serialize`](Self::serialize).
    ///
    /// Equivalent to `SortedSetSortField.Provider.readSortField(DataInput)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an unknown selector, and
    /// propagates any I/O error.
    pub fn read_sort_field(input: &mut dyn DataInput) -> Result<Self> {
        let field = input.read_string()?;
        let reverse = input.read_int()? == 1;
        let selector = Self::read_selector_type(input)?;
        let missing_value = match input.read_int()? {
            1 => Some(MissingValue::StringFirst),
            2 => Some(MissingValue::StringLast),
            _ => None,
        };
        Self::with_missing_value(&field, reverse, selector, missing_value)
    }
}

impl From<SortedSetSortField> for SortField {
    fn from(value: SortedSetSortField) -> Self {
        value.into_sort_field()
    }
}

impl fmt::Display for SortedSetSortField {
    /// Equivalent to `SortedSetSortField.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sort_field)
    }
}

/// The `SortFieldProvider` for [`SortedSetSortField`], registered as [`SortedSetSortField::NAME`].
///
/// Equivalent to the nested class `SortedSetSortField.Provider` (`SortedSetSortField.java:83-127`).
#[derive(Debug, Default, Clone, Copy)]
pub struct Provider;

impl Provider {
    /// Creates a new provider.
    ///
    /// Equivalent to `new SortedSetSortField.Provider()`.
    pub fn new() -> Self {
        Self
    }
}

impl crate::util::spi::NamedSPI for Provider {
    fn get_name(&self) -> &str {
        SortedSetSortField::NAME
    }
}

impl crate::index::index_sorter::SortFieldProvider for Provider {
    /// Equivalent to `SortedSetSortField.Provider.readSortField(DataInput)`.
    fn read_sort_field(
        &self,
        input: &mut dyn crate::store::DataInput,
    ) -> Result<crate::search::SortField> {
        Ok(SortedSetSortField::read_sort_field(input)?.into_sort_field())
    }

    /// Equivalent to `SortedSetSortField.Provider.writeSortField(SortField, DataOutput)`:
    /// the field must be a `SortedSetSortField` (Java asserts it and casts).
    fn write_sort_field(
        &self,
        sort_field: &crate::search::SortField,
        output: &mut dyn crate::store::DataOutput,
    ) -> Result<()> {
        SortedSetSortField::from_sort_field(sort_field.clone())?.serialize(output)
    }
}
