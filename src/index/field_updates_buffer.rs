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
///
/// Three of Lucene's members exist only to serve that packed representation and
/// have no counterpart here:
///
/// * `getArrayIndex(int, int)` resolves `min(arrayLength - 1, index)` so that an
///   array holding a single, repeated value can be read at any ordinal; with one
///   record per update the ordinal is always the record's own index.
/// * `sizeOfString(String)` accounts the field names Java keeps in an elided
///   `String[]`; [`ram_bytes_used`](Accountable::ram_bytes_used) charges each
///   record's own field name instead.
/// * `assertTermAndDocInOrder()` validates the sort state `finish()` builds when
///   the buffer qualifies for Lucene's sorted-terms optimisation. This port
///   never sorts, so `FieldUpdatesBuffer.BufferedUpdateIterator.isSortedTerms()`
///   is always `false` for it and there is no state to validate.
#[derive(Debug)]
pub struct FieldUpdatesBuffer {
    updates: Vec<BufferedUpdate>,
    is_numeric: bool,
    finished: bool,
    /// Smallest and largest numeric value seen, which Lucene uses to pick the
    /// narrowest encoding when the buffer is written.
    min_numeric: i64,
    max_numeric: i64,
    /// RAM the buffered records occupy, kept up to date by every append.
    ///
    /// Equivalent to the share of `FieldUpdatesBuffer.bytesUsed` — the
    /// `Counter` Java increments in `add` and `append` — that the records
    /// account for; [`ram_bytes_used`](Accountable::ram_bytes_used) adds the
    /// shallow size to it, so the answer costs O(1) however many updates are
    /// buffered.
    bytes_used: i64,
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
            bytes_used: 0,
        };
        // `fields = new String[] {initialValue.term.field}`
        // (`FieldUpdatesBuffer.java:73`): the field a buffered update records
        // is the field its *term* is looked up in, not the doc-values field it
        // updates.
        buffer.push(
            initial_value.term.field().to_string(),
            initial_value.term.bytes().clone(),
            doc_up_to,
            initial_value.value,
        )?;
        Ok(buffer)
    }

    /// Appends one update to the buffer.
    ///
    /// Equivalent to `FieldUpdatesBuffer.append(Term)` followed by
    /// `FieldUpdatesBuffer.add(String, int, int, boolean)`, which Java keeps
    /// apart only because `append` returns the ordinal the parallel arrays are
    /// then indexed by. This port stores one record per update, so there is no
    /// ordinal to hand around and the two steps are one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the buffer is finished.
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
        self.bytes_used += Self::record_bytes(&field, &term, &value);
        self.updates.push(BufferedUpdate {
            field,
            term,
            doc_up_to,
            value,
        });
        Ok(())
    }

    /// What one record is charged: the per-record header plus the bytes its
    /// field name, term and value occupy.
    fn record_bytes(field: &str, term: &BytesRef, value: &DocValuesUpdateValue) -> i64 {
        BYTES_PER_UPDATE + field.len() as i64 + term.length as i64 + value.value_size_in_bytes()
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
    /// Equivalent to `FieldUpdatesBuffer.addNoValue(Term, int)`, which appends
    /// the term and then calls the private `add(String, int, int, boolean)`
    /// with `hasValue` false. Java reads the field off `term.field`; this port
    /// takes it separately because [`Term`] is shared with the query side and
    /// the caller already has it — it must be `term.field()`, the field the
    /// term is looked up in, not the doc-values field being cleared.
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

    /// Returns the smallest numeric value buffered, or `None` when none was.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getMinNumeric()`, which asserts the
    /// buffer is numeric and answers `0` when no update carried a value —
    /// `minNumeric` and `maxNumeric` are still at their `Long.MAX_VALUE` /
    /// `Long.MIN_VALUE` seeds. `None` is that case, said in the type rather
    /// than in a magic `0` a caller could mistake for a real minimum; the
    /// caller that needs Java's literal answer writes `unwrap_or(0)`.
    pub fn min_numeric_value(&self) -> Option<i64> {
        (self.min_numeric != i64::MAX).then_some(self.min_numeric)
    }

    /// Returns the largest numeric value buffered, or `None` when none was.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getMaxNumeric()`; see
    /// [`min_numeric_value`](Self::min_numeric_value) for why `None` stands in
    /// for Java's `0`.
    pub fn max_numeric_value(&self) -> Option<i64> {
        (self.max_numeric != i64::MIN).then_some(self.max_numeric)
    }

    /// Returns whether every numeric update in the buffer carries the same
    /// value.
    ///
    /// Equivalent to `FieldUpdatesBuffer.hasSingleValue()`, which answers
    /// `isNumeric && numericValues.length == 1`. Java seeds `numericValues` with
    /// the initial update's value, or with `0` when the initial update carries
    /// none (`FieldUpdatesBuffer.java:88-98`), and `addUpdate(Term, long, int)`
    /// grows the array — for good — the first time a value differs from that
    /// seed (`FieldUpdatesBuffer.java:178-188`); updates without a value never
    /// touch it. This port keeps one record per update (the divergence declared
    /// on the type), so the same question is answered by comparing every
    /// numeric update with that seed: a buffer that starts with a reset and
    /// later carries a non-zero value is *not* single-valued, exactly as in
    /// Java. A binary buffer answers `false`, because its `numericValues` is
    /// null and `isNumeric` is false.
    pub fn has_single_value(&self) -> bool {
        if !self.is_numeric {
            return false;
        }
        let seed = match self.updates.first().map(|update| &update.value) {
            Some(DocValuesUpdateValue::Numeric(value)) => *value,
            _ => 0,
        };
        self.updates.iter().all(|update| match update.value {
            DocValuesUpdateValue::Numeric(value) => value == seed,
            _ => true,
        })
    }

    /// Returns the numeric value of the update at `index`.
    ///
    /// Equivalent to `FieldUpdatesBuffer.getNumericValue(int)`, which answers
    /// `0` for an update that carries no value and otherwise reads the packed
    /// array through `getArrayIndex`. With one record per update the ordinal is
    /// the record's own index, which is why `getArrayIndex` has no counterpart
    /// here.
    ///
    /// Out of range, and for a binary buffer, the answer is `0` — the value
    /// Java's `hasValues` bit yields for an update with none.
    pub fn get_numeric_value(&self, index: usize) -> i64 {
        match self.updates.get(index).map(|update| &update.value) {
            Some(DocValuesUpdateValue::Numeric(value)) => *value,
            _ => 0,
        }
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
    /// Java charges a shallow size plus the RAM of its `BytesRefArray` and its
    /// packed value arrays to a `Counter` as each update is appended. This port
    /// charges the same shallow size plus, per record, a fixed header cost and
    /// the bytes the field name, the term and the value actually occupy, and
    /// keeps that sum incrementally as Java does, so reading it is O(1).
    fn ram_bytes_used(&self) -> i64 {
        SELF_SHALLOW_SIZE + self.bytes_used
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The sum the buffer charged before rmp #235, recomputed record by record.
    fn summed(buffer: &FieldUpdatesBuffer) -> i64 {
        SELF_SHALLOW_SIZE
            + buffer
                .updates
                .iter()
                .map(|u| FieldUpdatesBuffer::record_bytes(&u.field, &u.term, &u.value))
                .sum::<i64>()
    }

    fn term(field: &str, value: &str) -> Term {
        Term::new(field, BytesRef::new(value.as_bytes().to_vec()))
    }

    #[test]
    fn incremental_ram_matches_the_record_by_record_sum() {
        let first = DocValuesUpdate {
            doc_values_type: crate::index::DocValuesType::NUMERIC,
            term: term("id", "a"),
            field: "price".to_string(),
            doc_id_up_to: 3,
            value: DocValuesUpdateValue::Numeric(7),
        };
        let mut numeric = FieldUpdatesBuffer::new(first, 3).unwrap();
        assert_eq!(numeric.ram_bytes_used(), summed(&numeric));
        for i in 0..50 {
            let t = term(if i % 3 == 0 { "id" } else { "group" }, &format!("t{i}"));
            if i % 5 == 0 {
                numeric.add_reset(t.field(), &t, i).unwrap();
            } else {
                numeric.add_numeric_update(&t, i64::from(i), i).unwrap();
            }
            assert_eq!(numeric.ram_bytes_used(), summed(&numeric));
        }

        let first = DocValuesUpdate {
            doc_values_type: crate::index::DocValuesType::BINARY,
            term: term("id", "b"),
            field: "blob".to_string(),
            doc_id_up_to: 1,
            value: DocValuesUpdateValue::Binary(BytesRef::new(vec![1, 2, 3])),
        };
        let mut binary = FieldUpdatesBuffer::new(first, 1).unwrap();
        for i in 0..20 {
            let t = term("id", &format!("b{i}"));
            binary
                .add_binary_update(&t, BytesRef::new(vec![0; i as usize]), i)
                .unwrap();
            assert_eq!(binary.ram_bytes_used(), summed(&binary));
        }
    }
}
