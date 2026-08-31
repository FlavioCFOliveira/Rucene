//! Doc-values accessors, inlining the static helpers of
//! `org.apache.lucene.index.DocValues`.
//!
//! Lucene reaches doc values from a leaf through
//! `DocValues.getNumeric(LeafReader, String)` and its siblings, which return an
//! empty instance when the field simply has no values and raise
//! `IllegalStateException` when the field exists with the wrong doc-values
//! type. [`crate::index::DocValues`] does not yet expose those statics — it
//! only offers the empty instances they fall back on — so this module holds
//! them for the search package. They are crate-private: the public home for
//! them is `crate::index::DocValues`, and they should move there once that type
//! gains them.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::{
    BinaryDocValues, DocValues, DocValuesType, LeafReader, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};

/// Raises the `IllegalStateException` that `DocValues.checkField` raises when
/// `field` exists but was not indexed with one of the `expected` doc-values
/// types.
///
/// Equivalent to the private `DocValues.checkField(LeafReader, String,
/// DocValuesType...)`, which does nothing when the field is absent from the
/// leaf.
fn check_field(reader: &dyn LeafReader, field: &str, expected: &[DocValuesType]) -> Result<()> {
    let field_infos = reader.get_field_infos();
    let Some(info) = field_infos.field_info(field) else {
        return Ok(());
    };
    let actual = info.get_doc_values_type();
    let expectation = if expected.len() == 1 {
        format!("(expected={:?}", expected[0])
    } else {
        format!("(expected one of {expected:?}")
    };
    Err(LuceneError::IllegalState(format!(
        "unexpected docvalues type {actual:?} for field '{field}' {expectation}). \
         Re-index with correct docvalues type."
    )))
}

/// Returns the numeric doc values of `field`, or an empty instance when the
/// field has none.
///
/// Equivalent to `DocValues.getNumeric(LeafReader, String)`.
pub(crate) fn get_numeric(
    reader: &dyn LeafReader,
    field: &str,
) -> Result<Box<dyn NumericDocValues>> {
    match reader.get_numeric_doc_values(field)? {
        Some(dv) => Ok(dv),
        None => {
            check_field(reader, field, &[DocValuesType::NUMERIC])?;
            Ok(Box::new(DocValues::empty_numeric()))
        }
    }
}

/// Returns the binary doc values of `field`, or an empty instance when the
/// field has none.
///
/// Equivalent to `DocValues.getBinary(LeafReader, String)`.
pub(crate) fn get_binary(reader: &dyn LeafReader, field: &str) -> Result<Box<dyn BinaryDocValues>> {
    match reader.get_binary_doc_values(field)? {
        Some(dv) => Ok(dv),
        None => {
            check_field(reader, field, &[DocValuesType::BINARY])?;
            Ok(Box::new(DocValues::empty_binary()))
        }
    }
}

/// Returns the sorted doc values of `field`, or an empty instance when the
/// field has none.
///
/// Equivalent to `DocValues.getSorted(LeafReader, String)`.
pub(crate) fn get_sorted(reader: &dyn LeafReader, field: &str) -> Result<Box<dyn SortedDocValues>> {
    match reader.get_sorted_doc_values(field)? {
        Some(dv) => Ok(dv),
        None => {
            check_field(reader, field, &[DocValuesType::SORTED])?;
            Ok(Box::new(DocValues::empty_sorted()))
        }
    }
}

/// Returns the sorted-numeric doc values of `field`, wrapping a single-valued
/// numeric field when necessary, or an empty instance when the field has none.
///
/// Equivalent to `DocValues.getSortedNumeric(LeafReader, String)`.
pub(crate) fn get_sorted_numeric(
    reader: &dyn LeafReader,
    field: &str,
) -> Result<Box<dyn SortedNumericDocValues>> {
    if let Some(dv) = reader.get_sorted_numeric_doc_values(field)? {
        return Ok(dv);
    }
    match reader.get_numeric_doc_values(field)? {
        Some(single) => Ok(Box::new(DocValues::singleton_numeric(single))),
        None => {
            check_field(
                reader,
                field,
                &[DocValuesType::SORTED_NUMERIC, DocValuesType::NUMERIC],
            )?;
            Ok(Box::new(DocValues::empty_sorted_numeric()))
        }
    }
}

/// Returns the sorted-set doc values of `field`, wrapping a single-valued
/// sorted field when necessary, or an empty instance when the field has none.
///
/// Equivalent to `DocValues.getSortedSet(LeafReader, String)`.
pub(crate) fn get_sorted_set(
    reader: &dyn LeafReader,
    field: &str,
) -> Result<Box<dyn SortedSetDocValues>> {
    if let Some(dv) = reader.get_sorted_set_doc_values(field)? {
        return Ok(dv);
    }
    match reader.get_sorted_doc_values(field)? {
        Some(sorted) => Ok(Box::new(DocValues::singleton_sorted(sorted))),
        None => {
            check_field(
                reader,
                field,
                &[DocValuesType::SORTED, DocValuesType::SORTED_SET],
            )?;
            Ok(Box::new(DocValues::empty_sorted_set()))
        }
    }
}
