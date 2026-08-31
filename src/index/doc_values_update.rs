//! `DocValuesUpdate` ported from `org.apache.lucene.index`.
//!
//! One in-place update to a doc-values field, carried from the writer's delete
//! queue to the segments it applies to.

use crate::error::Result;
use crate::index::{DocValuesType, Term};
use crate::store::DataOutput;
use crate::util::BytesRef;

/// The largest `doc_id_up_to` an update can carry, meaning "every document".
///
/// Equivalent to the `BufferedUpdates.MAX_INT` Java initialises `docIDUpTo`
/// with.
pub const MAX_DOC_ID_UP_TO: i32 = i32::MAX;

/// The value an update writes, or its absence.
///
/// **Divergence from Lucene 10.5.0.** Java models the two value kinds as the
/// subclasses `NumericDocValuesUpdate` and `BinaryDocValuesUpdate`, each with a
/// typed field plus a `hasValue` flag. Rust expresses the same three states —
/// a numeric value, a binary value, no value — as one enum, which makes the
/// "no value" case unrepresentable alongside a value rather than merely
/// discouraged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocValuesUpdateValue {
    /// A numeric value.
    Numeric(i64),
    /// A binary value.
    Binary(BytesRef),
    /// The field is cleared for the matching documents.
    None,
}

impl DocValuesUpdateValue {
    /// Returns whether the update carries a value.
    ///
    /// Equivalent to `DocValuesUpdate.hasValue()`.
    pub fn has_value(&self) -> bool {
        !matches!(self, DocValuesUpdateValue::None)
    }

    /// Returns the RAM the value itself occupies.
    ///
    /// Equivalent to `DocValuesUpdate.valueSizeInBytes()`.
    pub fn value_size_in_bytes(&self) -> i64 {
        match self {
            DocValuesUpdateValue::Numeric(_) => 8,
            DocValuesUpdateValue::Binary(bytes) => 64 + bytes.length as i64,
            DocValuesUpdateValue::None => 0,
        }
    }

    /// Renders the value the way Lucene's `valueToString()` does.
    ///
    /// Crate-visible, matching the package-private visibility of Java's
    /// `DocValuesUpdate.valueToString()`, which only the delete queue's node
    /// rendering uses.
    pub(crate) fn value_to_string(&self) -> String {
        match self {
            DocValuesUpdateValue::Numeric(value) => value.to_string(),
            DocValuesUpdateValue::Binary(bytes) => format!("{bytes:?}"),
            DocValuesUpdateValue::None => "null".to_string(),
        }
    }
}

/// An in-place update to one doc-values field, applying to every document the
/// term matches up to `doc_id_up_to`.
///
/// Equivalent to `org.apache.lucene.index.DocValuesUpdate`.
#[derive(Clone, Debug)]
pub struct DocValuesUpdate {
    /// Which kind of doc values the update targets.
    pub doc_values_type: DocValuesType,
    /// The term selecting the documents to update.
    pub term: Term,
    /// The field being updated.
    pub field: String,
    /// The update applies to documents below this number only.
    pub doc_id_up_to: i32,
    /// The value written, or `None` to clear the field.
    pub value: DocValuesUpdateValue,
}

/// Fixed overhead Lucene charges per update, before the term, field and value.
const RAW_SIZE_IN_BYTES: i64 = 8 * 16 + 8 * 8 + 8 * 4;

impl DocValuesUpdate {
    /// Creates an update.
    pub fn new(
        doc_values_type: DocValuesType,
        term: Term,
        field: impl Into<String>,
        doc_id_up_to: i32,
        value: DocValuesUpdateValue,
    ) -> Self {
        Self {
            doc_values_type,
            term,
            field: field.into(),
            doc_id_up_to,
            value,
        }
    }

    /// Creates a numeric update covering every document the term matches.
    pub fn numeric(term: Term, field: impl Into<String>, value: i64) -> Self {
        Self::new(
            DocValuesType::NUMERIC,
            term,
            field,
            MAX_DOC_ID_UP_TO,
            DocValuesUpdateValue::Numeric(value),
        )
    }

    /// Creates a binary update covering every document the term matches.
    pub fn binary(term: Term, field: impl Into<String>, value: BytesRef) -> Self {
        Self::new(
            DocValuesType::BINARY,
            term,
            field,
            MAX_DOC_ID_UP_TO,
            DocValuesUpdateValue::Binary(value),
        )
    }

    /// Returns whether the update carries a value.
    pub fn has_value(&self) -> bool {
        self.value.has_value()
    }

    /// Returns the RAM this update occupies.
    ///
    /// Equivalent to `DocValuesUpdate.sizeInBytes()`.
    pub fn size_in_bytes(&self) -> i64 {
        RAW_SIZE_IN_BYTES
            + self.term.field().len() as i64 * 2
            + self.term.bytes().length as i64
            + self.field.len() as i64 * 2
            + self.value.value_size_in_bytes()
            + 1
    }

    /// Writes the update's value to `output`.
    ///
    /// Equivalent to `DocValuesUpdate.writeTo(DataOutput)`, which the frozen
    /// buffer uses to serialise a packet.
    pub fn write_to(&self, output: &mut dyn DataOutput) -> Result<()> {
        match &self.value {
            DocValuesUpdateValue::Numeric(value) => output.write_z_long(*value),
            DocValuesUpdateValue::Binary(bytes) => {
                output.write_v_int(bytes.length as i32)?;
                output.write_bytes(bytes.slice(), 0, bytes.length)
            }
            DocValuesUpdateValue::None => Ok(()),
        }
    }
}

impl std::fmt::Display for DocValuesUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "term={:?},field={},value={},docIDUpTo={}",
            self.term,
            self.field,
            self.value.value_to_string(),
            self.doc_id_up_to
        )
    }
}
