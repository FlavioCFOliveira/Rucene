//! `FieldUpdatesBuffer` ported from `org.apache.lucene.index`.
//!
//! Buffers a run of doc-values updates compactly, so the delete queue can carry
//! many of them without a per-update object.

use crate::error::{LuceneError, Result};
use crate::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
use crate::index::Term;
use crate::util::{Accountable, BytesRef};

/// Shallow size Lucene charges for the buffer object itself.
///
/// Equivalent to `FieldUpdatesBuffer.SELF_SHALLOW_SIZE`.
const SELF_SHALLOW_SIZE: i64 = 96;

/// What one buffered record costs beyond the bytes of its field and term.
///
/// This port stores a record per update rather than Lucene's parallel arrays
/// (see the type's divergence note), so the per-record constant covers the
/// `String`, `BytesRef` and enum headers instead of Lucene's packed slots.
const BYTES_PER_UPDATE: i64 = 84;

/// One buffered update, as the buffer stores it.
#[derive(Debug, Clone)]
struct BufferedUpdate {
    field: String,
    term: BytesRef,
    doc_up_to: i32,
    value: DocValuesUpdateValue,
}

/// A run of updates to one kind of doc-values field.
///
/// Equivalent to `org.apache.lucene.index.FieldUpdatesBuffer`.
///
/// **Divergence from Lucene 10.5.0.** Java packs the updates into parallel
/// primitive arrays plus a `BytesRefArray`, and elides a field name or a value
/// that repeats, so a long run of updates to the same field costs almost
/// nothing. This port keeps one record per update. The iteration order, the
/// `doc_up_to` semantics and the numeric/binary split are unchanged; only the
/// memory footprint differs.
#[derive(Debug)]
pub struct FieldUpdatesBuffer {
    updates: Vec<BufferedUpdate>,
    is_numeric: bool,
    finished: bool,
    /// Smallest and largest numeric value seen, which Lucene uses to pick the
    /// narrowest encoding when the buffer is written.
    min_numeric: i64,
    max_numeric: i64,
}

impl FieldUpdatesBuffer {
    /// Creates a buffer seeded with `initial_value`.
    ///
    /// Equivalent to the `FieldUpdatesBuffer` constructor, which also takes the
    /// first update.
    pub fn new(initial_value: DocValuesUpdate, doc_up_to: i32) -> Result<Self> {
        let is_numeric = matches!(
            initial_value.value,
            DocValuesUpdateValue::Numeric(_) | DocValuesUpdateValue::None
        );
        let mut buffer = Self {
            updates: Vec::new(),
            is_numeric,
            finished: false,
            min_numeric: i64::MAX,
            max_numeric: i64::MIN,
        };
        buffer.push(
            initial_value.field.clone(),
            initial_value.term.bytes().clone(),
            doc_up_to,
            initial_value.value,
        )?;
        Ok(buffer)
    }

    fn push(
        &mut self,
        field: String,
        term: BytesRef,
        doc_up_to: i32,
        value: DocValuesUpdateValue,
    ) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "cannot add to a finished FieldUpdatesBuffer".to_string(),
            ));
        }
        if let DocValuesUpdateValue::Numeric(v) = value {
            self.min_numeric = self.min_numeric.min(v);
            self.max_numeric = self.max_numeric.max(v);
        }
        self.updates.push(BufferedUpdate {
            field,
            term,
            doc_up_to,
            value,
        });
        Ok(())
    }

    /// Buffers a numeric update.
    ///
    /// Equivalent to `FieldUpdatesBuffer.addUpdate(Term, long, int)`.
    pub fn add_numeric_update(&mut self, term: &Term, value: i64, doc_up_to: i32) -> Result<()> {
        if !self.is_numeric {
            return Err(LuceneError::IllegalState(
                "cannot add a numeric update to a binary FieldUpdatesBuffer".to_string(),
            ));
        }
        self.push(
            term.field().to_string(),
            term.bytes().clone(),
            doc_up_to,
            DocValuesUpdateValue::Numeric(value),
        )
    }

    /// Buffers a binary update.
    ///
    /// Equivalent to `FieldUpdatesBuffer.addUpdate(Term, BytesRef, int)`.
    pub fn add_binary_update(
        &mut self,
        term: &Term,
        value: BytesRef,
        doc_up_to: i32,
    ) -> Result<()> {
        if self.is_numeric {
            return Err(LuceneError::IllegalState(
                "cannot add a binary update to a numeric FieldUpdatesBuffer".to_string(),
            ));
        }
        self.push(
            term.field().to_string(),
            term.bytes().clone(),
            doc_up_to,
            DocValuesUpdateValue::Binary(value),
        )
    }

    /// Buffers an update that clears the field.
    ///
    /// Equivalent to `FieldUpdatesBuffer.add(String, int, int, boolean)` called
    /// with `hasValue` false.
    pub fn add_reset(&mut self, field: &str, term: &Term, doc_up_to: i32) -> Result<()> {
        self.push(
            field.to_string(),
            term.bytes().clone(),
            doc_up_to,
            DocValuesUpdateValue::None,
        )
    }

    /// Seals the buffer; no further update may be added.
    ///
    /// Equivalent to `FieldUpdatesBuffer.finish()`.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Returns whether the buffer holds numeric updates.
    ///
    /// Equivalent to `FieldUpdatesBuffer.isNumeric()`.
    pub fn is_numeric(&self) -> bool {
        self.is_numeric
    }

    /// Returns how many updates are buffered.
    ///
    /// Equivalent to `FieldUpdatesBuffer.numUpdates`.
    pub fn num_updates(&self) -> usize {
        self.updates.len()
    }

    /// Returns the smallest numeric value buffered, or `None` for a binary
    /// buffer.
    pub fn min_numeric_value(&self) -> Option<i64> {
        (self.min_numeric != i64::MAX).then_some(self.min_numeric)
    }

    /// Returns the largest numeric value buffered, or `None` for a binary
    /// buffer.
    pub fn max_numeric_value(&self) -> Option<i64> {
        (self.max_numeric != i64::MIN).then_some(self.max_numeric)
    }

    /// Iterates the buffered updates in the order they were added.
    ///
    /// Equivalent to `FieldUpdatesBuffer.iterator()`.
    pub fn iter(&self) -> impl Iterator<Item = BufferedUpdateRef<'_>> {
        self.updates.iter().map(|update| BufferedUpdateRef {
            field: &update.field,
            term: &update.term,
            doc_up_to: update.doc_up_to,
            value: &update.value,
        })
    }
}

impl Accountable for FieldUpdatesBuffer {
    /// Equivalent to `FieldUpdatesBuffer.ramBytesUsed()`.
    ///
    /// Java sums a shallow size with the RAM of its `BytesRefArray` and its
    /// packed value arrays. This port charges the same shallow size plus, per
    /// record, a fixed header cost and the bytes the field name, the term and
    /// the value actually occupy.
    fn ram_bytes_used(&self) -> i64 {
        SELF_SHALLOW_SIZE
            + self
                .updates
                .iter()
                .map(|update| {
                    BYTES_PER_UPDATE
                        + update.field.len() as i64
                        + update.term.length as i64
                        + update.value.value_size_in_bytes()
                })
                .sum::<i64>()
    }
}

/// A borrowed view of one buffered update.
///
/// Equivalent to what `FieldUpdatesBuffer.BufferedUpdateIterator` exposes.
#[derive(Debug)]
pub struct BufferedUpdateRef<'a> {
    /// The field being updated.
    pub field: &'a str,
    /// The term selecting the documents.
    pub term: &'a BytesRef,
    /// The update applies to documents below this number only.
    pub doc_up_to: i32,
    /// The value written, or `None` to clear the field.
    pub value: &'a DocValuesUpdateValue,
}
