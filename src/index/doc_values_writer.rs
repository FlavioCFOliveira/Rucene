//! Per-field buffers of doc-values, flushed through the codec when the
//! segment flushes.
//!
//! Equivalent to `org.apache.lucene.index.DocValuesWriter` and its five
//! concrete subclasses:
//!
//! * [`NumericDocValuesWriter`] — `index/NumericDocValuesWriter.java`
//! * [`BinaryDocValuesWriter`] — `index/BinaryDocValuesWriter.java`
//! * [`SortedDocValuesWriter`] — `index/SortedDocValuesWriter.java`
//! * [`SortedNumericDocValuesWriter`] — `index/SortedNumericDocValuesWriter.java`
//! * [`SortedSetDocValuesWriter`] — `index/SortedSetDocValuesWriter.java`
//!
//! together with `index/DocsWithFieldSet.java`, which this crate already ports
//! as [`crate::codecs::DocsWithFieldSet`] (the kNN-vector writer needed it
//! first). That port is faithful to Java — the same dense-case cardinality
//! optimisation, the same migration to a `FixedBitSet` on the first doc gap,
//! the same `addRange` — so it is reused rather than duplicated.
//!
//! # Responsibility
//!
//! Each writer owns the doc values of **one field** of the segment currently
//! being built. It buffers one value (or one value set) per document that
//! carried the field, in increasing document order; sorts where the
//! [`DocValuesType`] requires it (the sorted variants); builds the dictionary
//! and the ordinal mapping for the sorted variants; and hands everything to a
//! [`DocValuesConsumer`] when the segment flushes. It writes no files itself —
//! the codec owns the format.
//!
//! # Lifecycle
//!
//! The Java lifecycle, from `IndexingChain`, is:
//!
//! 1. **Creation.** `initializeFieldInfo` creates the writer for every field
//!    whose `docValuesType != NONE`, indexed or not
//!    (`IndexingChain.java:1351-1369`).
//! 2. **One call per field instance.** `processField` calls `indexDocValue`
//!    after `invertAndStore` for *every instance* of a field with doc values
//!    (`IndexingChain.java:1386-1391`). Single-valued types
//!    (NUMERIC/BINARY/SORTED) reject a second instance within one document
//!    with the "appears more than once in this document" error; multi-valued
//!    types (SORTED_NUMERIC/SORTED_SET) accumulate.
//! 3. **Flush.** `IndexingChain.writeDocValues` walks the per-field hash table
//!    — in *table order*, not field-number order, which fixes the order of the
//!    field entries inside the `.dvm` file — and flushes each writer through a
//!    lazily created `DocValuesConsumer` (`IndexingChain.java:439-497`). Each
//!    writer hands the consumer an anonymous `DocValuesProducer` serving its
//!    buffers; the sorted variants hand a *singleton* producer when every
//!    document turned out to carry exactly one value, which is what makes the
//!    codec pick the single-valued file layout.
//!
//! # Invariants
//!
//! * Document ids are non-decreasing. A repeat of a single-valued type is the
//!   "appears more than once" error; a backwards id is the same failure. Java
//!   asserts for the multi-valued writers; this port refuses.
//! * At flush time, every writer's value buffer has exactly as many entries as
//!   `docs_with_field.cardinality()`, and the two are read back in lockstep.
//! * The dictionary of the sorted variants is exactly the set of distinct
//!   values in unsigned byte order, ordinal 0 first — which is what makes the
//!   ordinals of two writers over the same documents agree with Lucene's.
//! * The per-doc value order of SORTED_NUMERIC is ascending, and of
//!   SORTED_SET is ascending with duplicates removed — both applied before
//!   any ordinals exist, exactly as in the Java `finishCurrentDoc` methods.
//!
//! # Divergences from Java, and why
//!
//! * Java buffers with `PackedLongValues` (delta-packed) and `PagedBytes`;
//!   this port uses plain `Vec`s and reports to the shared counter what it
//!   actually holds. Same divergence as [`crate::index::NormValuesWriter`].
//! * Java shares one `docValuesBytePool` between the sorted writers of a
//!   segment; here each sorted writer owns its own [`ByteBlockPool`] and
//!   [`BytesRefHash`]. Java shares to amortise block allocation; per-writer
//!   pools keep ownership simple (the dictionary is materialised out of the
//!   writer's own pool at flush) at the cost of one block per sorted field.
//! * Java's `flush` takes a `Sorter.DocMap` and re-orders the values of a
//!   segment with an index sort. `NumericDocValuesWriter` and
//!   `SortedDocValuesWriter` port `sortDocValues`/`getDocValuesProducer`
//!   faithfully. For the binary, sorted-numeric and sorted-set writers, Java
//!   builds `BinaryDVs`, `LongValues` and `DocOrds` — the values of every
//!   document laid out by its *new* number — and iterates them with
//!   `SortingBinaryDocValues`, `SortingSortedNumericDocValues` and
//!   `SortingSortedSetDocValues`. This port's buffered producers already hold
//!   the values per document, so the same layout is produced by re-ordering
//!   those per-document entries by new document number
//!   ([`sort_per_document`]); the buffered iterators then visit the documents
//!   in increasing new order with each document's own values, which is what
//!   the `Sorting*` iterators visit. The `maxCount` of the sorted-set writer is
//!   only consumed by `DocOrds`'s packed layout, so it is not tracked.
//! * The batch paths (`addDenseValues`, `addOrdinalTuples`,
//!   `addDenseOrdinalValues`) belong to the column-batch indexing row path,
//!   which this crate does not have; the per-instance path is the only one.
//! * Java wraps single-valued producers in `DocValues.singleton(...)`; the
//!   Rust wrappers (`SingletonSortedNumericDocValues`,
//!   `SingletonSortedSetDocValues`) store `Box<dyn Trait>` without
//!   `Send`/`Sync` and so cannot serve a producer whose iterators must be
//!   `Send + Sync`. Two small local wrappers below provide the same
//!   single-value views over `Send + Sync` iterators.
//! * Java charges the `2 * Integer.BYTES` ordMap/rehash headroom per new
//!   dictionary entry directly to the counter; here it is folded into the
//!   writer's delta-tracked footprint, so the counter total is the same but
//!   the refund at `Drop` covers it.

#![deny(unsafe_code)]

use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::codecs::doc_values::{DocValuesConsumer, DocValuesProducer};
use crate::codecs::DocsWithFieldSet;
use crate::document::column::{
    BytesRefValuesCursor, LongValuesCursor, OrdinalsCursor, OrdinalsTupleCursor,
};
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{
    BinaryDocValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use crate::index::doc_values::{DocValuesIterator, DocValuesSkipper, NumericDocValues};
use crate::index::index_sorter::SorterDocMap;
use crate::index::{DocValuesType, FieldInfo, IndexableField};
use crate::search::bit_set_util;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::MAX_TERM_LENGTH;
use crate::util::{ArrayUtil, ByteBlockPool, BytesRef, BytesRefHash, FixedBitSet};

/// The doc-values writer of one field, dispatched on its [`DocValuesType`].
///
/// Equivalent to the `DocValuesWriter` reference in `IndexingChain.PerField`:
/// Java stores one of five subclasses behind the abstract class and dispatches
/// in `indexDocValue` with a switch; this port does the same with an enum,
/// because the call site dispatches on the type anyway.
#[derive(Debug)]
pub enum DocValuesWriter {
    /// One numeric value per document — `NumericDocValuesWriter`.
    Numeric(NumericDocValuesWriter),
    /// One binary value per document — `BinaryDocValuesWriter`.
    Binary(BinaryDocValuesWriter),
    /// One binary value per document, dictionary-encoded —
    /// `SortedDocValuesWriter`.
    Sorted(SortedDocValuesWriter),
    /// Any number of numeric values per document —
    /// `SortedNumericDocValuesWriter`.
    SortedNumeric(SortedNumericDocValuesWriter),
    /// Any number of distinct binary values per document —
    /// `SortedSetDocValuesWriter`.
    SortedSet(SortedSetDocValuesWriter),
}

impl DocValuesWriter {
    /// Creates the writer matching `field_info`'s doc-values type.
    ///
    /// Equivalent to the `DocValuesType` switch of
    /// `IndexingChain.initializeFieldInfo` (`IndexingChain.java:1351-1369`).
    ///
    /// # Panics
    ///
    /// Only when `field_info` carries [`DocValuesType::NONE`], which the call
    /// site never does: Java throws an
    /// `AssertionError("unrecognized DocValues.Type")` in the same place.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        match field_info.doc_values_type {
            DocValuesType::NUMERIC => Self::Numeric(NumericDocValuesWriter::new(
                field_info,
                Arc::clone(&iw_bytes_used),
            )),
            DocValuesType::BINARY => Self::Binary(BinaryDocValuesWriter::new(
                field_info,
                Arc::clone(&iw_bytes_used),
            )),
            DocValuesType::SORTED => Self::Sorted(SortedDocValuesWriter::new(
                field_info,
                Arc::clone(&iw_bytes_used),
            )),
            DocValuesType::SORTED_NUMERIC => Self::SortedNumeric(
                SortedNumericDocValuesWriter::new(field_info, Arc::clone(&iw_bytes_used)),
            ),
            DocValuesType::SORTED_SET => Self::SortedSet(SortedSetDocValuesWriter::new(
                field_info,
                Arc::clone(&iw_bytes_used),
            )),
            DocValuesType::NONE => {
                unreachable!("NONE fields never get a doc-values writer")
            }
        }
    }

    /// Buffers the doc value of one field instance of one document.
    ///
    /// Equivalent to `IndexingChain.indexDocValue(int, PerField,
    /// DocValuesType, IndexableField)` (`IndexingChain.java:1659-1684`): the
    /// value is pulled off the field exactly where Java pulls it.
    ///
    /// Java reaches `field.numericValue().longValue()` for SORTED_NUMERIC
    /// without a null check and dies of a `NullPointerException`; this port
    /// reports the same situation the way the NUMERIC branch does, with the
    /// shared "null value not allowed" message, and converts every numeric
    /// flavour with Java's `Number.longValue()` semantics.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a value is missing or
    /// rejected by the concrete writer, and propagates whatever the writer
    /// raises.
    pub fn add_value(&mut self, doc_id: i32, field: &dyn IndexableField) -> Result<()> {
        match self {
            Self::Numeric(writer) => {
                let Some(value) = field.numeric_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{}\": null value not allowed",
                        field.name()
                    )));
                };
                writer.add_value(doc_id, java_long_value(&value))
            }
            Self::Binary(writer) => {
                let Some(value) = field.binary_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{}\": null value not allowed",
                        field.name()
                    )));
                };
                writer.add_value(doc_id, value.slice())
            }
            Self::Sorted(writer) => {
                let Some(value) = field.binary_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field \"{}\": null value not allowed",
                        field.name()
                    )));
                };
                writer.add_value(doc_id, value.slice())
            }
            Self::SortedNumeric(writer) => {
                let Some(value) = field.numeric_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field=\"{}\": null value not allowed",
                        field.name()
                    )));
                };
                writer.add_value(doc_id, java_long_value(&value))
            }
            Self::SortedSet(writer) => {
                let Some(value) = field.binary_value() else {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field \"{}\": null value not allowed",
                        field.name()
                    )));
                };
                writer.add_value(doc_id, value.slice())
            }
        }
    }

    /// Returns a read-only view over the values buffered so far, for the
    /// index sorter to compare.
    ///
    /// Equivalent to `DocValuesWriter.getDocValues()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the concrete writer raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        match self {
            Self::Numeric(writer) => writer.get_doc_values(),
            Self::Binary(writer) => writer.get_doc_values(),
            Self::Sorted(writer) => writer.get_doc_values(),
            Self::SortedNumeric(writer) => writer.get_doc_values(),
            Self::SortedSet(writer) => writer.get_doc_values(),
        }
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `DocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)`: `sort_map` is the permutation the
    /// index sort gave the segment, or `None` when it was not sorted.
    ///
    /// # Errors
    ///
    /// Propagates whatever the concrete writer raises; every concrete writer
    /// refuses a second flush, because two metadata entries for one field
    /// would make the segment unreadable.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        match self {
            Self::Numeric(writer) => writer.flush(consumer, sort_map),
            Self::Binary(writer) => writer.flush(consumer, sort_map),
            Self::Sorted(writer) => writer.flush(consumer, sort_map),
            Self::SortedNumeric(writer) => writer.flush(consumer, sort_map),
            Self::SortedSet(writer) => writer.flush(consumer, sort_map),
        }
    }

    /// Approximate heap held by the plain buffers of the writer.
    ///
    /// The `ByteBlockPool` and `BytesRefHash` of the sorted variants charge
    /// the shared counter directly, so they are not part of this figure.
    pub fn ram_bytes_used(&self) -> i64 {
        match self {
            Self::Numeric(writer) => writer.ram_bytes_used(),
            Self::Binary(writer) => writer.ram_bytes_used(),
            Self::Sorted(writer) => writer.ram_bytes_used(),
            Self::SortedNumeric(writer) => writer.ram_bytes_used(),
            Self::SortedSet(writer) => writer.ram_bytes_used(),
        }
    }
}

/// Converts a [`NumericValue`] the way Java's `Number.longValue()` does at
/// `IndexingChain.indexDocValue` — every numeric type widens, floats truncate
/// toward zero, `NaN` becomes `0`.
///
/// The float conversions are JLS numeric conversions (`float`→`long`,
/// `double`→`long`), which saturate at the type bounds and map `NaN` to `0`;
/// Rust's `as` casts are specified to do exactly that.
/// The error a flush-time producer answers to `get_skipper`.
///
/// Java's flush-time producers extend `EmptyDocValuesProducer`, whose
/// `getSkipper` throws `UnsupportedOperationException`
/// (`index/EmptyDocValuesProducer.java:54-57`).
fn unsupported_skipper() -> LuceneError {
    LuceneError::UnsupportedOperation(
        "a flush-time doc-values producer does not provide a doc-values skipper".to_string(),
    )
}

fn java_long_value(value: &crate::document::NumericValue) -> i64 {
    match value {
        crate::document::NumericValue::Int(v) => i64::from(*v),
        crate::document::NumericValue::Long(v) => *v,
        crate::document::NumericValue::Float(v) => *v as i64,
        crate::document::NumericValue::Double(v) => *v as i64,
    }
}

// -----------------------------------------------------------------------------
// Shared producer/iterator machinery
// -----------------------------------------------------------------------------

/// Materialises the document ids of a [`DocsWithFieldSet`].
///
/// [`DocsWithFieldSet::iterator`] borrows the set, while a producer must
/// outlive it, so the ids are copied once at flush.
fn materialize_docs(docs_with_field: &DocsWithFieldSet) -> Result<Vec<i32>> {
    let mut docs = Vec::with_capacity(docs_with_field.cardinality() as usize);
    let mut iterator = docs_with_field.iterator()?;
    loop {
        let doc = iterator.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        docs.push(doc);
    }
    Ok(docs)
}

/// Iterates the docs and one value per doc held in memory.
///
/// Equivalent to `NumericDocValuesWriter.BufferedNumericDocValues`. Like its
/// Java counterpart it is forward-only: `advance` and `advance_exact` are not
/// supported, because the only consumer is the codec's sequential passes.
struct BufferedNumeric {
    docs: Arc<Vec<i32>>,
    values: Arc<Vec<i64>>,
    /// Index of the current document, or `-1` before the first `next_doc` and
    /// `docs.len()` once exhausted.
    index: i64,
}

impl BufferedNumeric {
    fn new(docs: Arc<Vec<i32>>, values: Arc<Vec<i64>>) -> Self {
        Self {
            docs,
            values,
            index: -1,
        }
    }
}

impl fmt::Debug for BufferedNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedNumeric")
            .field("count", &self.docs.len())
            .field("index", &self.index)
            .finish()
    }
}

impl DocIdSetIterator for BufferedNumeric {
    fn doc_id(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.index as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        // `index` runs from -1 to `docs.len()` and then stops, so that calling
        // `next_doc` on an exhausted iterator keeps answering `NO_MORE_DOCS`
        // rather than running off the end.
        if self.index < self.docs.len() as i64 {
            self.index += 1;
        }
        Ok(self.doc_id())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedNumeric does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for BufferedNumeric {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedNumeric does not support advance_exact".to_string(),
        ))
    }
}

impl NumericDocValues for BufferedNumeric {
    fn long_value(&self) -> Result<i64> {
        if self.index < 0 || self.index as usize >= self.values.len() {
            return Err(LuceneError::IllegalState(
                "long_value called with no current document".to_string(),
            ));
        }
        Ok(self.values[self.index as usize])
    }
}

/// Iterates the buffered binary values of one field.
///
/// Equivalent to `BinaryDocValuesWriter.BufferedBinaryDocValues`.
struct BufferedBinary {
    docs: Arc<Vec<i32>>,
    values: Arc<Vec<Vec<u8>>>,
    index: i64,
}

impl BufferedBinary {
    fn new(docs: Arc<Vec<i32>>, values: Arc<Vec<Vec<u8>>>) -> Self {
        debug_assert_eq!(docs.len(), values.len());
        Self {
            docs,
            values,
            index: -1,
        }
    }
}

impl fmt::Debug for BufferedBinary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedBinary")
            .field("count", &self.docs.len())
            .field("index", &self.index)
            .finish()
    }
}

impl DocIdSetIterator for BufferedBinary {
    fn doc_id(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.index as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.index < self.docs.len() as i64 {
            self.index += 1;
        }
        Ok(self.doc_id())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedBinary does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for BufferedBinary {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedBinary does not support advance_exact".to_string(),
        ))
    }
}

impl BinaryDocValues for BufferedBinary {
    fn binary_value(&self) -> Result<BytesRef> {
        if self.index < 0 || self.index as usize >= self.values.len() {
            return Err(LuceneError::IllegalState(
                "binary_value called with no current document".to_string(),
            ));
        }
        Ok(BytesRef::new(self.values[self.index as usize].clone()))
    }
}

/// Iterates the buffered sorted values of one field.
///
/// Equivalent to `SortedDocValuesWriter.BufferedSortedDocValues`: the ords are
/// already the final ordinals (dictionary order), and `lookup_ord` walks the
/// dictionary.
struct BufferedSorted {
    docs: Arc<Vec<i32>>,
    /// Final ordinal (dictionary order) of each document that carried the
    /// field, in document order.
    ords: Arc<Vec<i32>>,
    dictionary: Arc<Vec<Vec<u8>>>,
    index: i64,
}

impl BufferedSorted {
    fn new(docs: Arc<Vec<i32>>, ords: Arc<Vec<i32>>, dictionary: Arc<Vec<Vec<u8>>>) -> Self {
        debug_assert_eq!(docs.len(), ords.len());
        Self {
            docs,
            ords,
            dictionary,
            index: -1,
        }
    }
}

impl fmt::Debug for BufferedSorted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedSorted")
            .field("count", &self.docs.len())
            .field("index", &self.index)
            .finish()
    }
}

impl DocIdSetIterator for BufferedSorted {
    fn doc_id(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.index as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.index < self.docs.len() as i64 {
            self.index += 1;
        }
        Ok(self.doc_id())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSorted does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocValuesIterator for BufferedSorted {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSorted does not support advance_exact".to_string(),
        ))
    }
}

impl SortedDocValues for BufferedSorted {
    fn ord_value(&self) -> Result<i32> {
        if self.index < 0 || self.index as usize >= self.ords.len() {
            return Err(LuceneError::IllegalState(
                "ord_value called with no current document".to_string(),
            ));
        }
        Ok(self.ords[self.index as usize])
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.dictionary.len() as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let value = self.dictionary.get(ord as usize).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "ord={ord} is out of bounds 0 .. {}",
                self.dictionary.len()
            ))
        })?;
        Ok(BytesRef::new(value.clone()))
    }
}

/// Shared iteration state of the multi-valued buffered iterators: docs in
/// document order, a flat value stream and a per-doc count.
struct MultiValueCursor {
    docs: Arc<Vec<i32>>,
    counts: Arc<Vec<i32>>,
    /// Absolute index of the current document's first unread value.
    value_index: i64,
    /// Values still unread for the current document.
    value_upto: i64,
    index: i64,
}

impl MultiValueCursor {
    fn new(docs: Arc<Vec<i32>>, counts: Arc<Vec<i32>>) -> Self {
        debug_assert_eq!(docs.len(), counts.len());
        Self {
            docs,
            counts,
            value_index: 0,
            value_upto: 0,
            index: -1,
        }
    }

    fn doc_id(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.index as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.index >= 0 && (self.index as usize) < self.docs.len() {
            // Skip the values the consumer never read, as Java's
            // `BufferedSortedNumericDocValues.nextDoc` does.
            self.value_index += self.value_upto;
        }
        if self.index < self.docs.len() as i64 {
            self.index += 1;
        }
        if (self.index as usize) < self.docs.len() {
            self.value_upto = i64::from(self.counts[self.index as usize]);
        }
        Ok(self.doc_id())
    }

    fn doc_value_count(&self) -> Result<i32> {
        if self.index < 0 || self.index as usize >= self.docs.len() {
            return Err(LuceneError::IllegalState(
                "doc_value_count called with no current document".to_string(),
            ));
        }
        Ok(self.counts[self.index as usize])
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl fmt::Debug for MultiValueCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiValueCursor")
            .field("count", &self.docs.len())
            .field("index", &self.index)
            .field("value_index", &self.value_index)
            .field("value_upto", &self.value_upto)
            .finish()
    }
}

/// Iterates the buffered sorted-numeric values of one field.
///
/// Equivalent to `SortedNumericDocValuesWriter.BufferedSortedNumericDocValues`.
struct BufferedSortedNumeric {
    cursor: MultiValueCursor,
    values: Arc<Vec<i64>>,
}

impl BufferedSortedNumeric {
    fn new(docs: Arc<Vec<i32>>, counts: Arc<Vec<i32>>, values: Arc<Vec<i64>>) -> Self {
        Self {
            cursor: MultiValueCursor::new(docs, counts),
            values,
        }
    }
}

impl DocIdSetIterator for BufferedSortedNumeric {
    fn doc_id(&self) -> i32 {
        self.cursor.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.cursor.next_doc()
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedNumeric does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.cursor.cost()
    }
}

impl DocValuesIterator for BufferedSortedNumeric {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedNumeric does not support advance_exact".to_string(),
        ))
    }
}

impl SortedNumericDocValues for BufferedSortedNumeric {
    fn next_value(&mut self) -> Result<i64> {
        if self.cursor.value_upto <= 0 || self.cursor.index < 0 {
            return Err(LuceneError::IllegalState(
                "next_value called with no current document or no values left".to_string(),
            ));
        }
        let value = self.values[self.cursor.value_index as usize];
        self.cursor.value_index += 1;
        self.cursor.value_upto -= 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.cursor.doc_value_count()
    }
}

/// Iterates the buffered sorted-set values of one field.
///
/// Equivalent to `SortedSetDocValuesWriter.BufferedSortedSetDocValues` after
/// the per-doc ordinals have been mapped and sorted: Java does both inside the
/// iterator's `nextDoc`, this port does them once at flush; the output is the
/// same stream.
struct BufferedSortedSet {
    cursor: MultiValueCursor,
    /// Final, per-doc-sorted ordinals of every document, concatenated.
    ords: Arc<Vec<i64>>,
    dictionary: Arc<Vec<Vec<u8>>>,
}

impl BufferedSortedSet {
    fn new(
        docs: Arc<Vec<i32>>,
        counts: Arc<Vec<i32>>,
        ords: Arc<Vec<i64>>,
        dictionary: Arc<Vec<Vec<u8>>>,
    ) -> Self {
        Self {
            cursor: MultiValueCursor::new(docs, counts),
            ords,
            dictionary,
        }
    }
}

impl DocIdSetIterator for BufferedSortedSet {
    fn doc_id(&self) -> i32 {
        self.cursor.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.cursor.next_doc()
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedSet does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.cursor.cost()
    }
}

impl DocValuesIterator for BufferedSortedSet {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedSet does not support advance_exact".to_string(),
        ))
    }
}

impl SortedSetDocValues for BufferedSortedSet {
    fn next_ord(&mut self) -> Result<i64> {
        if self.cursor.value_upto <= 0 || self.cursor.index < 0 {
            return Err(LuceneError::IllegalState(
                "next_ord called with no current document or no values left".to_string(),
            ));
        }
        let ord = self.ords[self.cursor.value_index as usize];
        self.cursor.value_index += 1;
        self.cursor.value_upto -= 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.cursor.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        let value = self.dictionary.get(ord as usize).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "ord={ord} is out of bounds 0 .. {}",
                self.dictionary.len()
            ))
        })?;
        Ok(BytesRef::new(value.clone()))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.dictionary.len() as i64)
    }
}

/// Multi-valued view over a `Send + Sync` [`NumericDocValues`] iterator.
///
/// Equivalent to `DocValues.singleton(NumericDocValues)` as it appears in the
/// anonymous producers of `NumericDocValuesWriter.getDocValuesProducer` and
/// `SortedNumericDocValuesWriter.flush`; see the module documentation for why
/// the shared wrapper cannot be reused here.
struct SingletonNumericAsSortedNumeric {
    inner: Box<dyn NumericDocValues>,
}

impl SingletonNumericAsSortedNumeric {
    fn new(inner: Box<dyn NumericDocValues>) -> Self {
        Self { inner }
    }
}

impl DocIdSetIterator for SingletonNumericAsSortedNumeric {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for SingletonNumericAsSortedNumeric {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedNumericDocValues for SingletonNumericAsSortedNumeric {
    fn next_value(&mut self) -> Result<i64> {
        self.inner.long_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }
}

impl fmt::Debug for SingletonNumericAsSortedNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingletonNumericAsSortedNumeric")
            .field("doc", &self.inner.doc_id())
            .finish()
    }
}

/// Multi-valued view over a `Send + Sync` [`SortedDocValues`] iterator.
///
/// Equivalent to `DocValues.singleton(SortedDocValues)` as it appears in the
/// anonymous producer of `SortedSetDocValuesWriter.flush` for the
/// single-valued case.
struct SingletonSortedAsSortedSet {
    inner: Box<dyn SortedDocValues>,
    /// The single ordinal of the current document, fetched when the iterator
    /// advanced, exactly as `SingletonSortedSetDocValues` does.
    ord: i64,
}

impl SingletonSortedAsSortedSet {
    fn new(inner: Box<dyn SortedDocValues>) -> Self {
        Self { inner, ord: -1 }
    }
}

impl DocIdSetIterator for SingletonSortedAsSortedSet {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        if doc != NO_MORE_DOCS {
            self.ord = i64::from(self.inner.ord_value()?);
        }
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.ord = i64::from(self.inner.ord_value()?);
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for SingletonSortedAsSortedSet {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.inner.advance_exact(target)? {
            self.ord = i64::from(self.inner.ord_value()?);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl SortedSetDocValues for SingletonSortedAsSortedSet {
    fn next_ord(&mut self) -> Result<i64> {
        Ok(self.ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.inner.lookup_ord(ord as i32)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.inner.get_value_count().map(i64::from)
    }
}

impl fmt::Debug for SingletonSortedAsSortedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingletonSortedAsSortedSet")
            .field("doc", &self.inner.doc_id())
            .finish()
    }
}

/// Shared "this producer serves another field" plumbing of the buffered
/// producers below, mirroring the anonymous producers' guards.
macro_rules! producer_guard {
    ($type:ty) => {
        impl $type {
            fn check_field(&self, field: &FieldInfo) -> Result<()> {
                if field.number != self.field_number {
                    return Err(LuceneError::IllegalArgument(format!(
                        "wrong fieldInfo: expected {} ({}), got {} ({})",
                        self.field_name, self.field_number, field.name, field.number
                    )));
                }
                Ok(())
            }

            fn unsupported(&self, kind: &str) -> LuceneError {
                LuceneError::UnsupportedOperation(format!(
                    "this producer only serves the {} values of field {} ({})",
                    kind, self.field_name, self.field_number
                ))
            }
        }
    };
}

producer_guard!(BufferedNumericProducer);
producer_guard!(BufferedBinaryProducer);
producer_guard!(BufferedSortedProducer);
producer_guard!(BufferedSortedNumericProducer);
producer_guard!(BufferedSortedSetProducer);

// -----------------------------------------------------------------------------
// NumericDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers one numeric value per document for a single field.
///
/// Equivalent to `org.apache.lucene.index.NumericDocValuesWriter`.
#[derive(Debug)]
pub struct NumericDocValuesWriter {
    field_info: FieldInfo,
    docs_with_field: DocsWithFieldSet,
    pending: Vec<i64>,
    last_doc_id: i32,
    /// The number of bytes this writer has already reported to
    /// [`Self::iw_bytes_used`], so that an update can report the delta.
    bytes_used: i64,
    iw_bytes_used: Arc<AtomicI64>,
    flushed: bool,
}

impl NumericDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new NumericDocValuesWriter(FieldInfo, Counter)`.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            field_info,
            docs_with_field,
            pending: Vec::new(),
            last_doc_id: -1,
            bytes_used,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addValue(int, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` is not strictly
    /// greater than the last document added — which in Lucene means the field
    /// appeared twice in one document — and
    /// [`LuceneError::IllegalState`] when the field has already been flushed.
    pub fn add_value(&mut self, doc_id: i32, value: i64) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        self.pending.push(value);
        self.docs_with_field.add(doc_id)?;
        self.update_bytes_used();
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Buffers `value` as the doc value of `count` consecutive documents
    /// starting at `first_doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addRepeatValues(int, long, int)`,
    /// which the column path uses for the parent field: every document of a
    /// batch is its own parent, so they all take the same value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the last document added, and
    /// [`LuceneError::IllegalState`] when the field has already been flushed.
    pub fn add_repeat_values(&mut self, first_doc_id: i32, value: i64, count: i32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        self.check_appendable(first_doc_id)?;
        self.pending
            .extend(std::iter::repeat_n(value, count as usize));
        self.docs_with_field
            .add_range(first_doc_id, first_doc_id + count)?;
        self.update_bytes_used();
        self.last_doc_id = first_doc_id + count - 1;
        Ok(())
    }

    /// Buffers every value of `cursor` as the doc values of the consecutive
    /// documents starting at `first_doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addDenseValues(int,
    /// LongValuesCursor)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the last document added,
    /// [`LuceneError::IllegalState`] when the field has already been flushed,
    /// and whatever reading the cursor raises.
    pub fn add_dense_values(
        &mut self,
        first_doc_id: i32,
        cursor: &mut dyn LongValuesCursor,
    ) -> Result<()> {
        let num_values = cursor.size();
        if num_values == 0 {
            return Ok(());
        }
        self.check_appendable(first_doc_id)?;
        let start = self.pending.len();
        self.pending.resize(start + num_values as usize, 0);
        cursor.fill_doc_values(&mut self.pending, start, num_values as usize)?;
        self.docs_with_field
            .add_range(first_doc_id, first_doc_id + num_values)?;
        self.update_bytes_used();
        self.last_doc_id = first_doc_id + num_values - 1;
        Ok(())
    }

    /// The two checks every `add*` runs before buffering: the field must not
    /// have been flushed, and the document must come after the last one.
    fn check_appendable(&self, doc_id: i32) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        Ok(())
    }

    /// Number of documents that have a value buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Equivalent to `NumericDocValuesWriter.updateBytesUsed`.
    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the buffers.
    ///
    /// Equivalent to `pending.ramBytesUsed() + docsWithField.ramBytesUsed()`,
    /// with the `Vec<i64>` standing in for `PackedLongValues`.
    pub fn ram_bytes_used(&self) -> i64 {
        self.pending.capacity() as i64 * std::mem::size_of::<i64>() as i64
            + self.docs_with_field.ram_bytes_used()
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `NumericDocValuesWriter.flush` without the doc map.
    ///
    /// Flushing consumes the buffers, so the heap they held is given back to
    /// the shared counter here rather than when the writer is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed — writing it twice would put two metadata entries for one field
    /// into the same segment. Otherwise propagates whatever the consumer
    /// raises while writing the field.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        let values = std::mem::take(&mut self.pending);
        let docs_with_field = std::mem::take(&mut self.docs_with_field);
        let producer =
            Self::get_doc_values_producer(&self.field_info, values, &docs_with_field, sort_map)?;
        drop(docs_with_field);
        self.update_bytes_used();
        consumer.add_numeric_field(&self.field_info, &*producer)
    }
}

impl NumericDocValuesWriter {
    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `NumericDocValuesWriter.getDocValues()`, which
    /// `IndexingChain.maybeSortSegment` reads to order the segment before
    /// anything is written.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's view reads the writer's live
    /// buffers. Rust's borrow rules make a shared read-only view over buffers
    /// the writer still owns awkward to express, so the view is taken over a
    /// copy. The values are identical; the cost is one copy per sorted field,
    /// paid only when the segment is index-sorted.
    ///
    /// # Errors
    ///
    /// Propagates whatever materialising the document set raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        Self::get_doc_values_producer(
            &self.field_info,
            self.pending.clone(),
            &self.docs_with_field,
            None,
        )
    }

    /// Builds the ordered, per-new-document view of `old_doc_values` that an
    /// index sort produces.
    ///
    /// Equivalent to `NumericDocValuesWriter.sortDocValues(int, Sorter.DocMap,
    /// NumericDocValues, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever iterating `old_doc_values` raises.
    pub(crate) fn sort_doc_values(
        max_doc: i32,
        sort_map: &SorterDocMap,
        old_doc_values: &mut dyn NumericDocValues,
        dense: bool,
    ) -> Result<NumericDVs> {
        let mut docs_with_field = if dense {
            None
        } else {
            Some(FixedBitSet::new(max_doc as usize))
        };

        let mut values = vec![0i64; max_doc as usize];
        loop {
            let doc_id = old_doc_values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = sort_map.old_to_new(doc_id);
            if let Some(docs_with_field) = docs_with_field.as_mut() {
                docs_with_field.set(new_doc_id as usize);
            }
            values[new_doc_id as usize] = old_doc_values.long_value()?;
        }
        Ok(NumericDVs {
            values,
            docs_with_field,
            max_doc,
        })
    }

    /// Builds the [`DocValuesProducer`] `flush` and `get_doc_values` hand to
    /// the codec: the plain buffered view when `sort_map` is `None`, or a view
    /// reordered by the index sort otherwise.
    ///
    /// Equivalent to `NumericDocValuesWriter.getDocValuesProducer(FieldInfo,
    /// PackedLongValues, DocsWithFieldSet, Sorter.DocMap)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::sort_doc_values`] raises.
    pub(crate) fn get_doc_values_producer(
        field_info: &FieldInfo,
        values: Vec<i64>,
        docs_with_field: &DocsWithFieldSet,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        let mut producer = BufferedNumericProducer::new(field_info, docs_with_field, values)?;
        if let Some(sort_map) = sort_map {
            let mut old_doc_values =
                BufferedNumeric::new(Arc::clone(&producer.docs), Arc::clone(&producer.values));
            let dense = sort_map.size() == docs_with_field.cardinality();
            producer.sorted = Some(Arc::new(Self::sort_doc_values(
                sort_map.size(),
                sort_map,
                &mut old_doc_values,
                dense,
            )?));
        }
        Ok(Arc::new(producer))
    }
}

impl Drop for NumericDocValuesWriter {
    /// Gives back the bytes this writer charged to the shared counter.
    ///
    /// Java has no equivalent: `DocumentsWriterPerThread` throws its whole
    /// `Counter` away with the chain. Rust drops writers individually — a
    /// document that fails validation can leave one behind — so the counter is
    /// balanced here instead of drifting upwards.
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
    }
}

/// Serves the buffered numeric values of one field to a [`DocValuesConsumer`].
///
/// Equivalent to the anonymous `DocValuesProducer` of
/// `NumericDocValuesWriter.flush`, including its "wrong fieldInfo" guard.
#[derive(Debug)]
struct BufferedNumericProducer {
    field_number: i32,
    field_name: String,
    docs: Arc<Vec<i32>>,
    values: Arc<Vec<i64>>,
    /// The field reordered by an index sort, when one applies.
    sorted: Option<Arc<NumericDVs>>,
}

impl BufferedNumericProducer {
    fn new(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        values: Vec<i64>,
    ) -> Result<Self> {
        debug_assert_eq!(docs_with_field.cardinality() as usize, values.len());
        Ok(Self {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            docs: Arc::new(materialize_docs(docs_with_field)?),
            values: Arc::new(values),
            sorted: None,
        })
    }
}

impl DocValuesProducer for BufferedNumericProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            self.check_field(field)?;
            match &self.sorted {
                None => Ok(Box::new(BufferedNumeric::new(
                    Arc::clone(&self.docs),
                    Arc::clone(&self.values),
                ))),
                Some(sorted) => Ok(Box::new(SortingNumeric::new(Arc::clone(sorted)))),
            }
        };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = { Err(self.unsupported("binary")) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = { Err(self.unsupported("sorted")) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> =
            { Err(self.unsupported("sorted numeric")) };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = { Err(self.unsupported("sorted set")) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`) by the flush-time producer
    /// of the matching `*DocValuesWriter`: unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported_skipper())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs: Arc::clone(&self.docs),
            values: Arc::clone(&self.values),
            sorted: self.sorted.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// The numeric values of a field once an index sort has reordered the
/// segment: one value per new document ID, plus which new document IDs
/// actually carry the field.
///
/// Equivalent to `NumericDocValuesWriter.NumericDVs`.
#[derive(Debug)]
pub(crate) struct NumericDVs {
    values: Vec<i64>,
    /// `None` when every new document carries the field (the dense case),
    /// exactly like Java's `docsWithField == null`.
    docs_with_field: Option<FixedBitSet>,
    max_doc: i32,
}

impl NumericDVs {
    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    /// Equivalent to `NumericDVs.advanceExact(int)`.
    fn advance_exact(&self, target: i32) -> bool {
        match &self.docs_with_field {
            Some(docs_with_field) => docs_with_field.get(target as usize),
            None => true,
        }
    }

    /// Equivalent to `NumericDVs.advance(int)`. Only called when `target` is
    /// less than [`Self::max_doc`].
    fn advance(&self, target: i32) -> i32 {
        match &self.docs_with_field {
            Some(docs_with_field) => bit_set_util::next_set_bit(docs_with_field, target as usize),
            None => target,
        }
    }

    /// Equivalent to `NumericDVs.cost()`.
    fn cost(&self) -> i64 {
        match &self.docs_with_field {
            Some(docs_with_field) => docs_with_field.cardinality() as i64,
            None => self.max_doc as i64,
        }
    }
}

/// Iterates the numeric doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `NumericDocValuesWriter.SortingNumericDocValues`.
pub(crate) struct SortingNumeric {
    dvs: Arc<NumericDVs>,
    doc_id: i32,
    /// Lazily computed and cached, like Java's `cost` field (sentinel `-1`).
    cost: Cell<Option<i64>>,
}

impl SortingNumeric {
    pub(crate) fn new(dvs: Arc<NumericDVs>) -> Self {
        Self {
            dvs,
            doc_id: -1,
            cost: Cell::new(None),
        }
    }
}

impl fmt::Debug for SortingNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingNumeric")
            .field("doc_id", &self.doc_id)
            .finish()
    }
}

impl DocIdSetIterator for SortingNumeric {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id = if self.doc_id + 1 == self.dvs.max_doc() {
            NO_MORE_DOCS
        } else {
            self.dvs.advance(self.doc_id + 1)
        };
        Ok(self.doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use next_doc() instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        if let Some(cost) = self.cost.get() {
            return cost;
        }
        let cost = self.dvs.cost();
        self.cost.set(Some(cost));
        cost
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let up_to = up_to.min(self.dvs.max_doc());
        if up_to > self.doc_id {
            match &self.dvs.docs_with_field {
                None => {
                    bit_set.set_range((self.doc_id - offset) as usize, (up_to - offset) as usize);
                    self.doc_id = up_to;
                }
                Some(docs_with_field) => {
                    FixedBitSet::or_range(
                        docs_with_field,
                        self.doc_id as usize,
                        bit_set,
                        (self.doc_id - offset) as usize,
                        (up_to - self.doc_id) as usize,
                    );
                    self.doc_id = if up_to < self.dvs.max_doc() {
                        self.dvs.advance(up_to)
                    } else {
                        NO_MORE_DOCS
                    };
                }
            }
        }
        Ok(())
    }
}

impl DocValuesIterator for SortingNumeric {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.dvs.advance_exact(target))
    }
}

impl NumericDocValues for SortingNumeric {
    fn long_value(&self) -> Result<i64> {
        Ok(self.dvs.values[self.doc_id as usize])
    }
}

// -----------------------------------------------------------------------------
// BinaryDocValuesWriter
// -----------------------------------------------------------------------------

/// Java's `BinaryDocValuesWriter.MAX_LENGTH = ArrayUtil.MAX_ARRAY_LENGTH`.
const BINARY_MAX_LENGTH: usize = ArrayUtil::MAX_ARRAY_LENGTH;

/// Buffers one binary value per document for a single field.
///
/// Equivalent to `org.apache.lucene.index.BinaryDocValuesWriter`: values are
/// concatenated into one flat byte buffer (Java's `PagedBytes`) with one
/// length per document (Java's `lengths` builder).
#[derive(Debug)]
pub struct BinaryDocValuesWriter {
    field_info: FieldInfo,
    data: Vec<u8>,
    lengths: Vec<i64>,
    docs_with_field: DocsWithFieldSet,
    last_doc_id: i32,
    max_length: usize,
    bytes_used: i64,
    iw_bytes_used: Arc<AtomicI64>,
    flushed: bool,
}

impl BinaryDocValuesWriter {
    /// Creates a writer for `field_info`.
    ///
    /// Equivalent to `new BinaryDocValuesWriter(FieldInfo, Counter)`.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            field_info,
            data: Vec::new(),
            lengths: Vec::new(),
            docs_with_field,
            last_doc_id: -1,
            max_length: 0,
            bytes_used,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `BinaryDocValuesWriter.addValue(int, BytesRef)`. Note the
    /// validation order, which differs from the other writers: document order
    /// first, then the null check (performed by the caller, which reads the
    /// value off the field), then the length check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` repeats or goes
    /// backwards, or when `value` is longer than [`BINARY_MAX_LENGTH`], and
    /// [`LuceneError::IllegalState`] when the field has already been flushed.
    pub fn add_value(&mut self, doc_id: i32, value: &[u8]) -> Result<()> {
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        if value.len() > BINARY_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {BINARY_MAX_LENGTH}",
                self.field_info.name
            )));
        }

        self.max_length = self.max_length.max(value.len());
        self.lengths.push(value.len() as i64);
        self.data.extend_from_slice(value);
        self.docs_with_field.add(doc_id)?;
        self.update_bytes_used();

        self.last_doc_id = doc_id;
        Ok(())
    }

    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the buffers.
    ///
    /// Equivalent to `lengths.ramBytesUsed() + bytes.ramBytesUsed() +
    /// docsWithField.ramBytesUsed()`.
    pub fn ram_bytes_used(&self) -> i64 {
        self.lengths.capacity() as i64 * std::mem::size_of::<i64>() as i64
            + self.data.capacity() as i64
            + self.docs_with_field.ram_bytes_used()
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `BinaryDocValuesWriter.flush` without the doc map.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        let lengths = std::mem::take(&mut self.lengths);
        let data = std::mem::take(&mut self.data);
        let docs_with_field = std::mem::take(&mut self.docs_with_field);
        let mut producer =
            BufferedBinaryProducer::new(&self.field_info, &docs_with_field, &lengths, &data)?;
        // `new BinaryDVs(maxDoc, sortMap, ...)` and `SortingBinaryDocValues`
        // (`BinaryDocValuesWriter.java:135-157`).
        if let Some(sort_map) = sort_map {
            let (docs, values) =
                sort_per_document(&producer.docs, producer.values.to_vec(), sort_map);
            producer.docs = Arc::new(docs);
            producer.values = Arc::new(values);
        }
        drop((docs_with_field, lengths, data));
        self.update_bytes_used();
        consumer.add_binary_field(&self.field_info, &producer)
    }
}

impl BinaryDocValuesWriter {
    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `BinaryDocValuesWriter.getDocValues()`; the same
    /// divergence as [`NumericDocValuesWriter::get_doc_values`] applies.
    ///
    /// # Errors
    ///
    /// Propagates whatever materialising the document set raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        Ok(Arc::new(BufferedBinaryProducer::new(
            &self.field_info,
            &self.docs_with_field,
            &self.lengths,
            &self.data,
        )?))
    }
}

impl Drop for BinaryDocValuesWriter {
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
    }
}

/// Serves the buffered binary values of one field to a [`DocValuesConsumer`].
///
/// Equivalent to the anonymous `DocValuesProducer` of
/// `BinaryDocValuesWriter.flush`, including its "wrong fieldInfo" guard.
#[derive(Debug)]
struct BufferedBinaryProducer {
    field_number: i32,
    field_name: String,
    docs: Arc<Vec<i32>>,
    /// One value per document that carried the field, in document order.
    values: Arc<Vec<Vec<u8>>>,
}

impl BufferedBinaryProducer {
    fn new(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        lengths: &[i64],
        data: &[u8],
    ) -> Result<Self> {
        debug_assert_eq!(docs_with_field.cardinality() as usize, lengths.len());
        let mut values = Vec::with_capacity(lengths.len());
        let mut cursor = 0usize;
        for &length in lengths {
            let end = cursor + length as usize;
            debug_assert!(end <= data.len());
            values.push(data[cursor..end].to_vec());
            cursor = end;
        }
        debug_assert_eq!(cursor, data.len());
        Ok(Self {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            docs: Arc::new(materialize_docs(docs_with_field)?),
            values: Arc::new(values),
        })
    }
}

impl DocValuesProducer for BufferedBinaryProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = { Err(self.unsupported("numeric")) };
        Ok(Some(values?))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = {
            self.check_field(field)?;
            Ok(Box::new(BufferedBinary::new(
                Arc::clone(&self.docs),
                Arc::clone(&self.values),
            )))
        };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = { Err(self.unsupported("sorted")) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> =
            { Err(self.unsupported("sorted numeric")) };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = { Err(self.unsupported("sorted set")) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`) by the flush-time producer
    /// of the matching `*DocValuesWriter`: unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported_skipper())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs: Arc::clone(&self.docs),
            values: Arc::clone(&self.values),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Index-sort re-ordering of the materialised per-document values
// -----------------------------------------------------------------------------

/// Re-orders one entry per document — `per_doc[i]` belongs to `docs[i]` — into
/// the order the index sort gives the documents, returning the new document
/// numbers in increasing order with their entries.
///
/// This is the layout `BinaryDocValuesWriter.BinaryDVs`,
/// `SortedNumericDocValuesWriter.LongValues` and
/// `SortedSetDocValuesWriter.DocOrds` build — every document's values filed
/// under `sortMap.oldToNew(docID)` — so iterating the result visits exactly
/// what the matching `Sorting*DocValues` iterator visits.
fn sort_per_document<T>(
    docs: &[i32],
    per_doc: Vec<T>,
    sort_map: &SorterDocMap,
) -> (Vec<i32>, Vec<T>) {
    debug_assert_eq!(docs.len(), per_doc.len());
    let mut entries: Vec<(i32, T)> = docs
        .iter()
        .map(|&doc| sort_map.old_to_new(doc))
        .zip(per_doc)
        .collect();
    entries.sort_unstable_by_key(|&(doc, _)| doc);
    entries.into_iter().unzip()
}

/// Splits a flat value stream into one slice per document, using the
/// per-document counts, or one value per document when there are none.
fn split_per_document<T: Clone>(values: &[T], counts: Option<&Vec<i32>>) -> Vec<Vec<T>> {
    match counts {
        None => values.iter().map(|value| vec![value.clone()]).collect(),
        Some(counts) => {
            let mut cursor = 0usize;
            counts
                .iter()
                .map(|&count| {
                    let end = cursor + count as usize;
                    let slice = values[cursor..end].to_vec();
                    cursor = end;
                    slice
                })
                .collect()
        }
    }
}

// -----------------------------------------------------------------------------
// Shared sorted-writer plumbing
// -----------------------------------------------------------------------------

/// The largest value a sorted writer accepts: Java's `BYTE_BLOCK_SIZE - 2`.
const SORTED_MAX_LENGTH: usize = MAX_TERM_LENGTH;

/// Serves the buffered sorted values of one field to a [`DocValuesConsumer`].
///
/// Equivalent to the anonymous `DocValuesProducer` of
/// `SortedDocValuesWriter.flush`.
#[derive(Debug)]
struct BufferedSortedProducer {
    field_number: i32,
    field_name: String,
    docs: Arc<Vec<i32>>,
    /// Final ordinal of each document, in document order.
    ords: Arc<Vec<i32>>,
    /// Distinct values in unsigned byte order; ordinal 0 first.
    dictionary: Arc<Vec<Vec<u8>>>,
    /// The field reordered by an index sort, when one applies: one ordinal
    /// per new document ID, `-1` where the sorted document has none.
    sorted: Option<Arc<Vec<i32>>>,
}

impl BufferedSortedProducer {
    fn new(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        ords: Vec<i32>,
        dictionary: Vec<Vec<u8>>,
    ) -> Result<Self> {
        debug_assert_eq!(docs_with_field.cardinality() as usize, ords.len());
        Ok(Self {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            docs: Arc::new(materialize_docs(docs_with_field)?),
            ords: Arc::new(ords),
            dictionary: Arc::new(dictionary),
            sorted: None,
        })
    }
}

impl DocValuesProducer for BufferedSortedProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = { Err(self.unsupported("numeric")) };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = { Err(self.unsupported("binary")) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = {
            self.check_field(field)?;
            match &self.sorted {
                None => Ok(Box::new(BufferedSorted::new(
                    Arc::clone(&self.docs),
                    Arc::clone(&self.ords),
                    Arc::clone(&self.dictionary),
                ))),
                Some(sorted) => Ok(Box::new(SortingSorted::new(
                    Arc::clone(&self.dictionary),
                    self.docs.len() as i64,
                    Arc::clone(sorted),
                ))),
            }
        };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> =
            { Err(self.unsupported("sorted numeric")) };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = { Err(self.unsupported("sorted set")) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`) by the flush-time producer
    /// of the matching `*DocValuesWriter`: unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported_skipper())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs: Arc::clone(&self.docs),
            ords: Arc::clone(&self.ords),
            dictionary: Arc::clone(&self.dictionary),
            sorted: self.sorted.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Iterates the sorted doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `SortedDocValuesWriter.SortingSortedDocValues`. Java keeps a
/// reference to the unsorted `SortedDocValues` only to forward `cost()`,
/// `lookupOrd()` and `getValueCount()`; this port keeps the dictionary and the
/// unsorted cost directly instead of the whole buffered iterator.
struct SortingSorted {
    dictionary: Arc<Vec<Vec<u8>>>,
    cost: i64,
    /// One ordinal per new document ID, `-1` where that document has none.
    ords: Arc<Vec<i32>>,
    doc_id: i32,
}

impl SortingSorted {
    fn new(dictionary: Arc<Vec<Vec<u8>>>, cost: i64, ords: Arc<Vec<i32>>) -> Self {
        Self {
            dictionary,
            cost,
            ords,
            doc_id: -1,
        }
    }
}

impl fmt::Debug for SortingSorted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingSorted")
            .field("doc_id", &self.doc_id)
            .finish()
    }
}

impl DocIdSetIterator for SortingSorted {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            self.doc_id += 1;
            if self.doc_id as usize == self.ords.len() {
                self.doc_id = NO_MORE_DOCS;
                break;
            }
            if self.ords[self.doc_id as usize] != -1 {
                break;
            }
        }
        Ok(self.doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use next_doc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl DocValuesIterator for SortingSorted {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(self.ords[target as usize] != -1)
    }
}

impl SortedDocValues for SortingSorted {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords[self.doc_id as usize])
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.dictionary.len() as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        let value = self.dictionary.get(ord as usize).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "ord={ord} is out of bounds 0 .. {}",
                self.dictionary.len()
            ))
        })?;
        Ok(BytesRef::new(value.clone()))
    }
}

/// Serves the buffered sorted-numeric values of one field.
///
/// Equivalent to the anonymous `DocValuesProducer` of
/// `SortedNumericDocValuesWriter.flush`: when every document turned out to
/// carry exactly one value (`counts == None`) it serves a singleton view, the
/// same producer Java hands over, and otherwise the multi-valued view.
#[derive(Debug)]
struct BufferedSortedNumericProducer {
    field_number: i32,
    field_name: String,
    docs: Arc<Vec<i32>>,
    values: Arc<Vec<i64>>,
    /// `None` when every document carries exactly one value — Java's
    /// `valueCounts == null` singleton case.
    counts: Option<Arc<Vec<i32>>>,
}

impl BufferedSortedNumericProducer {
    fn new(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        values: Vec<i64>,
        counts: Option<Vec<i32>>,
    ) -> Result<Self> {
        if let Some(counts) = &counts {
            debug_assert_eq!(docs_with_field.cardinality() as usize, counts.len());
            debug_assert_eq!(
                counts.iter().map(|&count| i64::from(count)).sum::<i64>(),
                values.len() as i64
            );
        } else {
            debug_assert_eq!(docs_with_field.cardinality() as usize, values.len());
        }
        Ok(Self {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            docs: Arc::new(materialize_docs(docs_with_field)?),
            values: Arc::new(values),
            counts: counts.map(Arc::new),
        })
    }
}

impl DocValuesProducer for BufferedSortedNumericProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = { Err(self.unsupported("numeric")) };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = { Err(self.unsupported("binary")) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = { Err(self.unsupported("sorted")) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> = {
            self.check_field(field)?;
            if let Some(counts) = &self.counts {
                Ok(Box::new(BufferedSortedNumeric::new(
                    Arc::clone(&self.docs),
                    Arc::clone(counts),
                    Arc::clone(&self.values),
                )))
            } else {
                Ok(Box::new(SingletonNumericAsSortedNumeric::new(Box::new(
                    BufferedNumeric::new(Arc::clone(&self.docs), Arc::clone(&self.values)),
                ))))
            }
        };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = { Err(self.unsupported("sorted set")) };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`) by the flush-time producer
    /// of the matching `*DocValuesWriter`: unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported_skipper())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs: Arc::clone(&self.docs),
            values: Arc::clone(&self.values),
            counts: self.counts.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Serves the buffered sorted-set values of one field.
///
/// Equivalent to the anonymous `DocValuesProducer` of
/// `SortedSetDocValuesWriter.flush`: the singleton case serves a
/// single-value view over the sorted doc values, the multi-valued case serves
/// the buffered per-doc ordinal sets.
#[derive(Debug)]
struct BufferedSortedSetProducer {
    field_number: i32,
    field_name: String,
    docs: Arc<Vec<i32>>,
    /// Final, per-doc-sorted ordinals of every document, concatenated.
    ords: Arc<Vec<i64>>,
    /// One count per document, when any document carries more than one value.
    ord_counts: Option<Arc<Vec<i32>>>,
    dictionary: Arc<Vec<Vec<u8>>>,
}

impl BufferedSortedSetProducer {
    fn new(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        ords: Vec<i64>,
        ord_counts: Option<Vec<i32>>,
        dictionary: Vec<Vec<u8>>,
    ) -> Result<Self> {
        debug_assert_eq!(
            docs_with_field.cardinality() as usize,
            ord_counts.as_ref().map_or(ords.len(), Vec::len)
        );
        Ok(Self {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            docs: Arc::new(materialize_docs(docs_with_field)?),
            ords: Arc::new(ords),
            ord_counts: ord_counts.map(Arc::new),
            dictionary: Arc::new(dictionary),
        })
    }
}

impl DocValuesProducer for BufferedSortedSetProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = { Err(self.unsupported("numeric")) };
        Ok(Some(values?))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        let values: Result<Box<dyn BinaryDocValues>> = { Err(self.unsupported("binary")) };
        Ok(Some(values?))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        let values: Result<Box<dyn SortedDocValues>> = { Err(self.unsupported("sorted")) };
        Ok(Some(values?))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let values: Result<Box<dyn SortedNumericDocValues>> =
            { Err(self.unsupported("sorted numeric")) };
        Ok(Some(values?))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let values: Result<Box<dyn SortedSetDocValues>> = {
            self.check_field(field)?;
            if let Some(counts) = &self.ord_counts {
                Ok(Box::new(BufferedSortedSet::new(
                    Arc::clone(&self.docs),
                    Arc::clone(counts),
                    Arc::clone(&self.ords),
                    Arc::clone(&self.dictionary),
                )))
            } else {
                Ok(Box::new(SingletonSortedAsSortedSet::new(Box::new(
                    BufferedSorted::new(
                        Arc::clone(&self.docs),
                        // The singleton view reports `i32` ordinals straight from
                        // the per-doc stream.
                        Arc::new(self.ords.iter().map(|&ord| ord as i32).collect()),
                        Arc::clone(&self.dictionary),
                    ),
                ))))
            }
        };
        Ok(Some(values?))
    }

    /// Inherited from `EmptyDocValuesProducer.getSkipper`
    /// (`index/EmptyDocValuesProducer.java:54-57`) by the flush-time producer
    /// of the matching `*DocValuesWriter`: unsupported.
    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported_skipper())
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs: Arc::clone(&self.docs),
            ords: Arc::clone(&self.ords),
            ord_counts: self.ord_counts.clone(),
            dictionary: Arc::clone(&self.dictionary),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// SortedDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers one binary value per document and builds the value dictionary.
///
/// Equivalent to `org.apache.lucene.index.SortedDocValuesWriter`. Values are
/// interned into a [`BytesRefHash`] over the writer's own [`ByteBlockPool`];
/// `pending` holds hash term ids, and at flush the term ids are ranked by
/// unsigned byte order (the hash's sort) to produce the final ordinals.
#[derive(Debug)]
pub struct SortedDocValuesWriter {
    field_info: FieldInfo,
    pool: ByteBlockPool,
    hash: BytesRefHash,
    /// Hash term id of every buffered value, in document order.
    pending: Vec<i32>,
    docs_with_field: DocsWithFieldSet,
    last_doc_id: i32,
    bytes_used: i64,
    /// Headroom Java charges per distinct dictionary entry (the `ordMap`
    /// rehash reserve); tracked as part of `bytes_used` — see the module
    /// documentation.
    ord_map_charge: i64,
    iw_bytes_used: Arc<AtomicI64>,
    flushed: bool,
}

impl SortedDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedDocValuesWriter(FieldInfo, Counter, pool,
    /// scratch)` without the shared pool and the batch scratch.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            pool: ByteBlockPool::new(Arc::clone(&iw_bytes_used)),
            hash: BytesRefHash::new(Arc::clone(&iw_bytes_used)),
            field_info,
            pending: Vec::new(),
            docs_with_field,
            last_doc_id: -1,
            bytes_used,
            ord_map_charge: 0,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `SortedDocValuesWriter.addValue(int, BytesRef)`, with the
    /// null check performed by the caller (see [`DocValuesWriter::add_value`]).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` is not strictly
    /// greater than the last document added, or when `value` is longer than
    /// [`SORTED_MAX_LENGTH`], and [`LuceneError::IllegalState`] when the field
    /// has already been flushed.
    pub fn add_value(&mut self, doc_id: i32, value: &[u8]) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        if value.len() > SORTED_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {SORTED_MAX_LENGTH}",
                self.field_info.name
            )));
        }

        self.add_one_value(value)?;
        self.docs_with_field.add(doc_id)?;
        self.update_bytes_used();
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Buffers one dictionary-encoded value per document the tuple cursor
    /// names, at `base_doc_id` plus the batch-local id.
    ///
    /// Equivalent to `SortedDocValuesWriter.addOrdinalTuples(int, List,
    /// OrdinalsTupleCursor)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a document repeats or an
    /// ordinal is out of range, [`LuceneError::IllegalState`] when the field
    /// has already been flushed, and whatever reading the cursor raises.
    pub fn add_ordinal_tuples(
        &mut self,
        base_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsTupleCursor,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        let mut ord_to_hash = vec![-1i32; dictionary.len()];
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            let doc_id = base_doc_id + batch_doc_id;
            if doc_id <= self.last_doc_id {
                return Err(LuceneError::IllegalArgument(format!(
                    "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                    self.field_info.name
                )));
            }
            let ord = cursor.ord_value();
            let id = self.lookup_or_translate(ord, dictionary, &mut ord_to_hash)?;
            self.pending.push(id);
            self.docs_with_field.add(doc_id)?;
            self.last_doc_id = doc_id;
        }
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers one dictionary-encoded value per consecutive document starting
    /// at `first_doc_id`.
    ///
    /// Equivalent to `SortedDocValuesWriter.addDenseOrdinalValues(int, List,
    /// OrdinalsCursor)`, which does one hash lookup per distinct dictionary
    /// entry actually used rather than one per document.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the range does not start
    /// after the last document written or an ordinal is out of range,
    /// [`LuceneError::IllegalState`] when the field has already been flushed,
    /// and whatever reading the cursor raises.
    pub fn add_dense_ordinal_values(
        &mut self,
        first_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsCursor,
    ) -> Result<()> {
        let n = cursor.size();
        if n == 0 {
            return Ok(());
        }
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if first_doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\": firstDocID={first_doc_id} must be > lastDocID={}",
                self.field_info.name, self.last_doc_id
            )));
        }
        let mut ord_to_hash = vec![-1i32; dictionary.len()];
        let mut processed = 0i32;
        // Java commits whatever was processed even when a lookup throws, in a
        // `finally`; the same is done here by recording the range before the
        // error is returned.
        let mut outcome = Ok(());
        while processed < n {
            let ord = match cursor.next_ord() {
                Ok(ord) => ord,
                Err(e) => {
                    outcome = Err(e);
                    break;
                }
            };
            match self.lookup_or_translate(ord, dictionary, &mut ord_to_hash) {
                Ok(id) => self.pending.push(id),
                Err(e) => {
                    outcome = Err(e);
                    break;
                }
            }
            processed += 1;
        }
        if processed > 0 {
            self.docs_with_field
                .add_range(first_doc_id, first_doc_id + processed)?;
            self.last_doc_id = first_doc_id + processed - 1;
        }
        self.update_bytes_used();
        outcome
    }

    /// Interns the dictionary entry `ord` names, remembering the term id it was
    /// given so the same entry costs one hash lookup per batch.
    ///
    /// Equivalent to the private `SortedDocValuesWriter.lookupOrTranslate(int,
    /// List, int[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `ord` is outside the
    /// dictionary, and whatever interning the value raises.
    fn lookup_or_translate(
        &mut self,
        ord: i32,
        dictionary: &[BytesRef],
        ord_to_hash: &mut [i32],
    ) -> Result<i32> {
        if ord < 0 || ord as usize >= dictionary.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\": ordinal {ord} is out of range [0, {})",
                self.field_info.name,
                dictionary.len()
            )));
        }
        let mut id = ord_to_hash[ord as usize];
        if id < 0 {
            id = self
                .hash
                .add(&mut self.pool, dictionary[ord as usize].slice())?;
            if id < 0 {
                id = -id - 1;
            } else {
                self.ord_map_charge += 2 * std::mem::size_of::<i32>() as i64;
            }
            ord_to_hash[ord as usize] = id;
        }
        Ok(id)
    }

    /// Equivalent to `SortedDocValuesWriter.addOneValue`.
    fn add_one_value(&mut self, value: &[u8]) -> Result<()> {
        let term_id = self.hash.add(&mut self.pool, value)?;
        let term_id = if term_id < 0 {
            -term_id - 1
        } else {
            // Java reserves 2 * Integer.BYTES per unique value for the rehash
            // headroom and the flush-time ordMap slot; folded into the
            // delta-tracked footprint here.
            self.ord_map_charge += 2 * std::mem::size_of::<i32>() as i64;
            term_id
        };
        self.pending.push(term_id);
        Ok(())
    }

    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the plain buffers.
    ///
    /// The pool and the hash charge the shared counter directly on growth and
    /// are refunded at `Drop`, so they are not part of this figure.
    pub fn ram_bytes_used(&self) -> i64 {
        self.pending.capacity() as i64 * std::mem::size_of::<i32>() as i64
            + self.ord_map_charge
            + self.docs_with_field.ram_bytes_used()
    }

    /// Number of documents that have a value buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedDocValuesWriter.flush` without the doc map. The
    /// dictionary is materialised here, once, in unsigned byte order; the
    /// ordinals handed to the consumer are the ranks in that order, exactly
    /// what Java's `finalOrdMap` translation produces.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;

        let dictionary = self.materialize_dictionary()?;
        let ords: Vec<i32> = self
            .pending
            .iter()
            .map(|&term_id| dictionary.ord_map[term_id as usize])
            .collect();
        let pending = std::mem::take(&mut self.pending);
        let docs_with_field = std::mem::take(&mut self.docs_with_field);
        let producer = Self::get_doc_values_producer(
            &self.field_info,
            &docs_with_field,
            ords,
            dictionary.values,
            sort_map,
        )?;
        drop((docs_with_field, pending, dictionary.ord_map));
        self.update_bytes_used();
        consumer.add_sorted_field(&self.field_info, &*producer)
    }

    /// Sorts the distinct values by unsigned byte order and builds the
    /// ordinal map.
    ///
    /// Equivalent to `SortedDocValuesWriter.finish`: `hash.sort()` yields the
    /// term ids in dictionary order, and `ordMap[termID] = ord` inverts it.
    fn materialize_dictionary(&self) -> Result<Dictionary> {
        let sorted = self.hash.sort(&self.pool);
        let mut dictionary = Vec::with_capacity(sorted.len());
        let mut ord_map = vec![0i32; sorted.len()];
        for (ord, &term_id) in sorted.iter().enumerate() {
            ord_map[term_id as usize] = ord as i32;
            let start = self.hash.byte_start(term_id);
            dictionary.push(self.pool.term_bytes(start).to_vec());
        }
        Ok(Dictionary {
            values: dictionary,
            ord_map,
        })
    }
}

impl SortedDocValuesWriter {
    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedDocValuesWriter.getDocValues()`; the same
    /// divergence as [`NumericDocValuesWriter::get_doc_values`] applies. The
    /// dictionary is built here as it is at flush, because the ordinals a
    /// sorter compares are dictionary ordinals.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the dictionary raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        let dictionary = self.materialize_dictionary()?;
        let ords: Vec<i32> = self
            .pending
            .iter()
            .map(|&term_id| dictionary.ord_map[term_id as usize])
            .collect();
        Self::get_doc_values_producer(
            &self.field_info,
            &self.docs_with_field,
            ords,
            dictionary.values,
            None,
        )
    }

    /// Builds the ordered, per-new-document ordinal view that an index sort
    /// produces: `-1` where the sorted document carries no value.
    ///
    /// Equivalent to the private `SortedDocValuesWriter.sortDocValues(int,
    /// Sorter.DocMap, SortedDocValues)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever iterating `old_values` raises.
    pub(crate) fn sort_doc_values(
        max_doc: i32,
        sort_map: &SorterDocMap,
        old_values: &mut dyn SortedDocValues,
    ) -> Result<Vec<i32>> {
        let mut ords = vec![-1i32; max_doc as usize];
        loop {
            let doc_id = old_values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = sort_map.old_to_new(doc_id);
            ords[new_doc_id as usize] = old_values.ord_value()?;
        }
        Ok(ords)
    }

    /// Builds the [`DocValuesProducer`] `flush` and `get_doc_values` hand to
    /// the codec: the plain buffered view when `sort_map` is `None`, or a view
    /// reordered by the index sort otherwise.
    ///
    /// Equivalent to `SortedDocValuesWriter.getDocValuesProducer(FieldInfo,
    /// BytesRefHash, PackedLongValues, int[], int[], DocsWithFieldSet,
    /// Sorter.DocMap)`, with `hash`/`ords`/`sortedValues`/`ordMap` already
    /// resolved to the final dictionary and per-document ordinals.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::sort_doc_values`] raises.
    pub(crate) fn get_doc_values_producer(
        field_info: &FieldInfo,
        docs_with_field: &DocsWithFieldSet,
        ords: Vec<i32>,
        dictionary: Vec<Vec<u8>>,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        let mut producer =
            BufferedSortedProducer::new(field_info, docs_with_field, ords, dictionary)?;
        if let Some(sort_map) = sort_map {
            let mut old_values = BufferedSorted::new(
                Arc::clone(&producer.docs),
                Arc::clone(&producer.ords),
                Arc::clone(&producer.dictionary),
            );
            producer.sorted = Some(Arc::new(Self::sort_doc_values(
                sort_map.size(),
                sort_map,
                &mut old_values,
            )?));
        }
        Ok(Arc::new(producer))
    }
}

impl Drop for SortedDocValuesWriter {
    /// Gives back everything this writer charged to the shared counter: the
    /// delta-tracked buffers, the hash's own arrays and every pool block.
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
        self.hash.release_accounting();
        self.pool.reset();
    }
}

/// The dictionary of a flushed sorted variant: distinct values in unsigned
/// byte order plus the term-id → ordinal map.
struct Dictionary {
    values: Vec<Vec<u8>>,
    ord_map: Vec<i32>,
}

// -----------------------------------------------------------------------------
// SortedNumericDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers any number of numeric values per document, sorted per document.
///
/// Equivalent to `org.apache.lucene.index.SortedNumericDocValuesWriter`.
#[derive(Debug)]
pub struct SortedNumericDocValuesWriter {
    field_info: FieldInfo,
    /// Flat stream of all values, in document order.
    pending: Vec<i64>,
    /// One count per document, only once some document carries more than one
    /// value — Java's lazily created `pendingCounts`.
    pending_counts: Option<Vec<i32>>,
    docs_with_field: DocsWithFieldSet,
    current_doc: i32,
    current_values: Vec<i64>,
    bytes_used: i64,
    iw_bytes_used: Arc<AtomicI64>,
    flushed: bool,
}

impl SortedNumericDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedNumericDocValuesWriter(FieldInfo, Counter)`.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            field_info,
            pending: Vec::new(),
            pending_counts: None,
            docs_with_field,
            current_doc: -1,
            current_values: Vec::new(),
            bytes_used,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as one more value of `doc_id`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.addValue(int, long)`: the
    /// values of a document accumulate, and moving to a new document commits
    /// the previous one. Java only asserts `docID >= currentDoc`; this port
    /// refuses a backwards document id, because with the assertion off Java
    /// would silently corrupt the value stream.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` goes backwards,
    /// and [`LuceneError::IllegalState`] when the field has already been
    /// flushed.
    pub fn add_value(&mut self, doc_id: i32, value: i64) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if doc_id < self.current_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" values must be added in increasing document order (doc {} after {})",
                self.field_info.name, doc_id, self.current_doc
            )));
        }
        if doc_id != self.current_doc {
            self.finish_current_doc()?;
            self.current_doc = doc_id;
        }
        self.current_values.push(value);
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers every value of `cursor` as the single value of each consecutive
    /// document starting at `first_doc_id`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.addDenseValues(int,
    /// LongValuesCursor)`. Each value is one document's whole value list, so
    /// nothing needs buffering or sorting; a count of one is recorded per
    /// document when some earlier document was multi-valued.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the current document,
    /// [`LuceneError::IllegalState`] when the field has already been flushed,
    /// and whatever reading the cursor raises.
    pub fn add_dense_values(
        &mut self,
        first_doc_id: i32,
        cursor: &mut dyn LongValuesCursor,
    ) -> Result<()> {
        let num_values = cursor.size();
        if num_values == 0 {
            return Ok(());
        }
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if first_doc_id <= self.current_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" values must be added in increasing document order (doc {first_doc_id} after {})",
                self.field_info.name, self.current_doc
            )));
        }
        self.finish_current_doc()?;

        // Write the values straight to the flat stream: each is one document's
        // whole list, so there is nothing to buffer and nothing to sort.
        let start = self.pending.len();
        self.pending.resize(start + num_values as usize, 0);
        cursor.fill_doc_values(&mut self.pending, start, num_values as usize)?;

        // When the per-document counts are already being tracked — because some
        // earlier document was multi-valued — record one per dense document.
        if let Some(counts) = &mut self.pending_counts {
            counts.extend(std::iter::repeat_n(1i32, num_values as usize));
        }

        let end_doc_id = first_doc_id + num_values;
        self.docs_with_field.add_range(first_doc_id, end_doc_id)?;

        // Leave `current_doc` on the last document written, so ordering holds;
        // `current_values` stays empty, because nothing was buffered.
        self.current_doc = end_doc_id - 1;
        self.update_bytes_used();
        Ok(())
    }

    /// Equivalent to `SortedNumericDocValuesWriter.finishCurrentDoc`: sorts the
    /// current document's values, commits them to the flat stream and records
    /// the count — backfilling one count per already-seen document when the
    /// first multi-valued document appears.
    fn finish_current_doc(&mut self) -> Result<()> {
        if self.current_doc == -1 {
            return Ok(());
        }
        if self.current_values.is_empty() {
            // The document was committed directly, by `add_dense_values`.
            return Ok(());
        }
        let count = self.current_values.len();
        if count > 1 {
            self.current_values.sort_unstable();
        }
        self.pending.extend_from_slice(&self.current_values);
        if let Some(counts) = &mut self.pending_counts {
            counts.push(count as i32);
        } else if count != 1 {
            // Java fills one count per document already recorded in
            // `docsWithField` — which still excludes the current document —
            // and then records the multi-valued count that triggered the
            // builder.
            let mut counts = vec![1i32; self.docs_with_field.cardinality() as usize];
            counts.push(count as i32);
            self.pending_counts = Some(counts);
        }
        self.current_values.clear();
        self.docs_with_field.add(self.current_doc)?;
        Ok(())
    }

    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the plain buffers.
    pub fn ram_bytes_used(&self) -> i64 {
        self.pending.capacity() as i64 * std::mem::size_of::<i64>() as i64
            + self.pending_counts.as_ref().map_or(0, |counts| {
                counts.capacity() as i64 * std::mem::size_of::<i32>() as i64
            })
            + self.current_values.capacity() as i64 * std::mem::size_of::<i64>() as i64
            + self.docs_with_field.ram_bytes_used()
    }

    /// Number of documents that have values buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.flush`: when every document
    /// carries exactly one value the producer serves a singleton view — the
    /// same shape Java hands over, and what makes the codec pick the
    /// single-valued file layout.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        self.finish_current_doc()?;
        let values = std::mem::take(&mut self.pending);
        let counts = self.pending_counts.take();
        let docs_with_field = std::mem::take(&mut self.docs_with_field);
        let mut producer =
            BufferedSortedNumericProducer::new(&self.field_info, &docs_with_field, values, counts)?;
        // `new LongValues(maxDoc, sortMap, ...)` and
        // `SortingSortedNumericDocValues` (`SortedNumericDocValuesWriter.java:230-253`).
        if let Some(sort_map) = sort_map {
            let per_doc = split_per_document(&producer.values, producer.counts.as_deref());
            let (docs, per_doc) = sort_per_document(&producer.docs, per_doc, sort_map);
            producer.counts = producer
                .counts
                .as_ref()
                .map(|_| Arc::new(per_doc.iter().map(|values| values.len() as i32).collect()));
            producer.docs = Arc::new(docs);
            producer.values = Arc::new(per_doc.concat());
        }
        drop(docs_with_field);
        self.update_bytes_used();
        consumer.add_sorted_numeric_field(&self.field_info, &producer)
    }
}

impl SortedNumericDocValuesWriter {
    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.getDocValues()`; the same
    /// divergence as [`NumericDocValuesWriter::get_doc_values`] applies.
    ///
    /// # Errors
    ///
    /// Propagates whatever materialising the document set raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        Ok(Arc::new(BufferedSortedNumericProducer::new(
            &self.field_info,
            &self.docs_with_field,
            self.pending.clone(),
            self.pending_counts.clone(),
        )?))
    }
}

impl Drop for SortedNumericDocValuesWriter {
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
    }
}

// -----------------------------------------------------------------------------
// SortedSetDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers any number of distinct binary values per document.
///
/// Equivalent to `org.apache.lucene.index.SortedSetDocValuesWriter`: values
/// are interned into the dictionary hash, the term ids of the current
/// document are sorted and deduplicated when the document is committed, and
/// the final ordinals are mapped at flush.
#[derive(Debug)]
pub struct SortedSetDocValuesWriter {
    field_info: FieldInfo,
    pool: ByteBlockPool,
    hash: BytesRefHash,
    /// Flat stream of deduplicated term ids, in document order.
    pending: Vec<i32>,
    /// One count of distinct values per document, only once some document
    /// carries more than one — Java's lazily created `pendingCounts`.
    pending_counts: Option<Vec<i32>>,
    docs_with_field: DocsWithFieldSet,
    current_doc: i32,
    current_values: Vec<i32>,
    /// Largest per-document distinct count, seen so far.
    max_count: i32,
    bytes_used: i64,
    /// Headroom Java charges per unique value; folded into `bytes_used`.
    ord_map_charge: i64,
    iw_bytes_used: Arc<AtomicI64>,
    flushed: bool,
}

impl SortedSetDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedSetDocValuesWriter(FieldInfo, Counter, pool,
    /// scratch)` without the shared pool and the batch paths.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            pool: ByteBlockPool::new(Arc::clone(&iw_bytes_used)),
            hash: BytesRefHash::new(Arc::clone(&iw_bytes_used)),
            field_info,
            pending: Vec::new(),
            pending_counts: None,
            docs_with_field,
            current_doc: -1,
            current_values: Vec::new(),
            max_count: 0,
            bytes_used,
            ord_map_charge: 0,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as one more value of `doc_id`.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.addValue(int, BytesRef)` with
    /// the null check performed by the caller: repeated values of one document
    /// accumulate and are deduplicated on commit. Java asserts
    /// `docID >= currentDoc`; this port refuses a backwards document id, for
    /// the same reason as [`SortedNumericDocValuesWriter::add_value`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` goes backwards
    /// or the value is too long, and [`LuceneError::IllegalState`] when the
    /// field has already been flushed.
    pub fn add_value(&mut self, doc_id: i32, value: &[u8]) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if value.len() > SORTED_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {SORTED_MAX_LENGTH}",
                self.field_info.name
            )));
        }
        if doc_id < self.current_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" values must be added in increasing document order (doc {} after {})",
                self.field_info.name, doc_id, self.current_doc
            )));
        }
        if doc_id != self.current_doc {
            self.finish_current_doc()?;
            self.current_doc = doc_id;
        }

        let term_id = self.hash.add(&mut self.pool, value)?;
        if term_id < 0 {
            // `-termID - 1`: the value is already in the dictionary.
        } else {
            self.ord_map_charge += 2 * std::mem::size_of::<i32>() as i64;
        }
        let term_id = if term_id < 0 { -term_id - 1 } else { term_id };
        self.current_values.push(term_id);
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers one dictionary-encoded value per tuple the cursor names, at
    /// `base_doc_id` plus the batch-local id.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.addOrdinalTuples(int, List,
    /// OrdinalsTupleCursor)`. Several tuples may name the same document; their
    /// values accumulate and are deduplicated when the document is committed.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a document goes backwards
    /// or an ordinal is out of range, [`LuceneError::IllegalState`] when the
    /// field has already been flushed, and whatever reading the cursor raises.
    pub fn add_ordinal_tuples(
        &mut self,
        base_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsTupleCursor,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        let mut ord_to_hash = vec![-1i32; dictionary.len()];
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            let doc_id = base_doc_id + batch_doc_id;
            if doc_id < self.current_doc {
                return Err(LuceneError::IllegalArgument(format!(
                    "DocValuesField \"{}\" values must be added in increasing document order (doc {doc_id} after {})",
                    self.field_info.name, self.current_doc
                )));
            }
            if doc_id != self.current_doc {
                self.finish_current_doc()?;
                self.current_doc = doc_id;
            }
            let ord = cursor.ord_value();
            if ord < 0 || ord as usize >= dictionary.len() {
                return Err(LuceneError::IllegalArgument(format!(
                    "DocValuesField \"{}\": ordinal {ord} is out of range [0, {})",
                    self.field_info.name,
                    dictionary.len()
                )));
            }
            let mut hash_id = ord_to_hash[ord as usize];
            if hash_id < 0 {
                hash_id = self
                    .hash
                    .add(&mut self.pool, dictionary[ord as usize].slice())?;
                if hash_id < 0 {
                    hash_id = -hash_id - 1;
                } else {
                    self.ord_map_charge += 2 * std::mem::size_of::<i32>() as i64;
                }
                ord_to_hash[ord as usize] = hash_id;
            }
            self.current_values.push(hash_id);
        }
        self.update_bytes_used();
        Ok(())
    }

    /// Equivalent to `SortedSetDocValuesWriter.finishCurrentDoc`: sorts the
    /// current term ids, deduplicates them into the flat stream and records
    /// the distinct count — backfilling one count per already-seen document
    /// when the first multi-valued document appears.
    fn finish_current_doc(&mut self) -> Result<()> {
        if self.current_doc == -1 {
            return Ok(());
        }
        self.current_values.sort_unstable();
        let mut count = 0i32;
        let mut last_value = -1i32;
        for &term_id in &self.current_values {
            if term_id != last_value {
                self.pending.push(term_id);
                count += 1;
            }
            last_value = term_id;
        }
        if let Some(counts) = &mut self.pending_counts {
            counts.push(count);
        } else if count != 1 {
            let mut counts = vec![1i32; self.docs_with_field.cardinality() as usize];
            counts.push(count);
            self.pending_counts = Some(counts);
        }
        self.max_count = self.max_count.max(count);
        self.current_values.clear();
        self.docs_with_field.add(self.current_doc)?;
        Ok(())
    }

    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Approximate heap held by the plain buffers; the pool and the hash
    /// charge the shared counter directly.
    pub fn ram_bytes_used(&self) -> i64 {
        self.pending.capacity() as i64 * std::mem::size_of::<i32>() as i64
            + self.pending_counts.as_ref().map_or(0, |counts| {
                counts.capacity() as i64 * std::mem::size_of::<i32>() as i64
            })
            + self.current_values.capacity() as i64 * std::mem::size_of::<i32>() as i64
            + self.docs_with_field.ram_bytes_used()
    }

    /// Number of documents that have values buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.flush`: the per-document
    /// ordinals are mapped through the dictionary order and sorted eagerly
    /// (Java sorts them inside the buffered iterator, producing the same
    /// stream), and the singleton case wraps the sorted view, which is what
    /// makes the codec pick the single-valued file layout.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        consumer: &mut dyn DocValuesConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::IllegalState(format!(
                "the doc values of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        self.finish_current_doc()?;

        let dictionary = self.materialize_dictionary()?;
        // One count per document; when no document turned out multi-valued
        // every count is one and the flat stream maps one to one.
        let mut ord_counts = self.pending_counts.take().unwrap_or_default();
        if ord_counts.is_empty() {
            ord_counts = vec![1; self.pending.len()];
        }
        let mut ords: Vec<i64> = Vec::with_capacity(self.pending.len());
        let mut cursor = 0usize;
        for &count in &ord_counts {
            let end = cursor + count as usize;
            for &term_id in &self.pending[cursor..end] {
                ords.push(i64::from(dictionary.ord_map[term_id as usize]));
            }
            // Java's buffered iterator sorts each document's mapped ordinals
            // in `nextDoc`; here the same stream is sorted eagerly at flush.
            ords[cursor..end].sort_unstable();
            cursor = end;
        }

        let docs_with_field = std::mem::take(&mut self.docs_with_field);
        let mut producer = BufferedSortedSetProducer::new(
            &self.field_info,
            &docs_with_field,
            ords,
            if ord_counts.is_empty() {
                None
            } else {
                Some(ord_counts)
            },
            dictionary.values,
        )?;
        // `new DocOrds(maxDoc, sortMap, ...)` and `SortingSortedSetDocValues`
        // (`SortedSetDocValuesWriter.java:290-314`).
        if let Some(sort_map) = sort_map {
            let per_doc = split_per_document(&producer.ords, producer.ord_counts.as_deref());
            let (docs, per_doc) = sort_per_document(&producer.docs, per_doc, sort_map);
            producer.ord_counts = producer
                .ord_counts
                .as_ref()
                .map(|_| Arc::new(per_doc.iter().map(|ords| ords.len() as i32).collect()));
            producer.docs = Arc::new(docs);
            producer.ords = Arc::new(per_doc.concat());
        }
        drop(docs_with_field);
        self.update_bytes_used();
        consumer.add_sorted_set_field(&self.field_info, &producer)
    }

    /// Sorts the distinct values by unsigned byte order and builds the
    /// term-id → ordinal map. Equivalent to `SortedSetDocValuesWriter.finish`.
    fn materialize_dictionary(&self) -> Result<Dictionary> {
        let sorted = self.hash.sort(&self.pool);
        let mut values = Vec::with_capacity(sorted.len());
        let mut ord_map = vec![0i32; sorted.len()];
        for (ord, &term_id) in sorted.iter().enumerate() {
            ord_map[term_id as usize] = ord as i32;
            let start = self.hash.byte_start(term_id);
            values.push(self.pool.term_bytes(start).to_vec());
        }
        Ok(Dictionary { values, ord_map })
    }
}

impl SortedSetDocValuesWriter {
    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.getDocValues()`; the same
    /// divergence as [`NumericDocValuesWriter::get_doc_values`] applies. The
    /// document under construction is not included: Java calls `finishCurrentDoc`
    /// before reading, and this view is taken between documents, where there is
    /// none in flight.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the dictionary raises.
    pub fn get_doc_values(&self) -> Result<Arc<dyn DocValuesProducer>> {
        let dictionary = self.materialize_dictionary()?;
        let mut ord_counts = self.pending_counts.clone().unwrap_or_default();
        if ord_counts.is_empty() {
            ord_counts = vec![1; self.pending.len()];
        }
        let mut ords: Vec<i64> = Vec::with_capacity(self.pending.len());
        let mut cursor = 0usize;
        for &count in &ord_counts {
            let end = cursor + count as usize;
            for &term_id in &self.pending[cursor..end] {
                ords.push(i64::from(dictionary.ord_map[term_id as usize]));
            }
            ords[cursor..end].sort_unstable();
            cursor = end;
        }
        Ok(Arc::new(BufferedSortedSetProducer::new(
            &self.field_info,
            &self.docs_with_field,
            ords,
            self.pending_counts.clone(),
            dictionary.values,
        )?))
    }
}

impl Drop for SortedSetDocValuesWriter {
    /// Gives back the delta-tracked buffers, the hash's arrays and the pool
    /// blocks.
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
        self.hash.release_accounting();
        self.pool.reset();
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Analyzer, TokenStream};
    use crate::document::{FieldType, InvertableType, NumericValue, StoredValue};
    use crate::index::IndexableFieldType;

    fn field(name: &str, number: i32, doc_values_type: DocValuesType) -> FieldInfo {
        let mut info = FieldInfo::new(name, number);
        info.doc_values_type = doc_values_type;
        info
    }

    fn counter() -> Arc<AtomicI64> {
        Arc::new(AtomicI64::new(0))
    }

    /// Everything a consumer can be handed, captured generically so a test can
    /// assert on it without going through a codec.
    #[derive(Debug, PartialEq)]
    enum Recorded {
        Numeric(Vec<(i32, i64)>),
        Binary(Vec<(i32, Vec<u8>)>),
        Sorted {
            docs_ords: Vec<(i32, i32)>,
            dictionary: Vec<Vec<u8>>,
        },
        SortedNumeric(Vec<(i32, Vec<i64>)>),
        SortedSet {
            docs_ords: Vec<(i32, Vec<i64>)>,
            dictionary: Vec<Vec<u8>>,
        },
    }

    /// A sorted-set view read back: the ordinals of each document that has
    /// any, and the dictionary those ordinals index.
    type SortedSetView = (Vec<(i32, Vec<i64>)>, Vec<Vec<u8>>);

    /// Walks a sorted-set (or singleton-wrapped) view into `(doc, ords)` pairs
    /// plus the dictionary, the way the Lucene 9.0 consumer reads it: fresh
    /// iterator for the documents, then one `lookup_ord` per ordinal.
    fn collect_sorted_set(
        values: &dyn DocValuesProducer,
        field: &FieldInfo,
    ) -> Result<SortedSetView> {
        let mut sorted_set = values.get_sorted_set(field)?.unwrap();
        let mut docs_ords = Vec::new();
        loop {
            let doc = sorted_set.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            let count = sorted_set.doc_value_count()?;
            let mut ords = Vec::with_capacity(count as usize);
            for _ in 0..count {
                ords.push(sorted_set.next_ord()?);
            }
            docs_ords.push((doc, ords));
        }
        let dictionary = (0..sorted_set.get_value_count()?)
            .map(|ord| sorted_set.lookup_ord(ord).map(|value| value.bytes))
            .collect::<Result<Vec<_>>>()?;
        Ok((docs_ords, dictionary))
    }

    /// Collects everything a consumer is handed, so a test can assert on it
    /// without going through a codec. Each getter is walked with fresh
    /// iterators, exactly as the Lucene 9.0 codec does across its passes.
    #[derive(Debug, Default)]
    struct RecordingConsumer {
        fields: Vec<(String, Recorded)>,
        closed: bool,
    }

    impl DocValuesConsumer for RecordingConsumer {
        fn add_numeric_field(
            &mut self,
            field: &FieldInfo,
            values: &dyn DocValuesProducer,
        ) -> Result<()> {
            let mut collected = Vec::new();
            let mut numeric = values.get_numeric(field)?.unwrap();
            loop {
                let doc = numeric.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                collected.push((doc, numeric.long_value()?));
            }
            self.fields
                .push((field.name.clone(), Recorded::Numeric(collected)));
            Ok(())
        }

        fn add_binary_field(
            &mut self,
            field: &FieldInfo,
            values: &dyn DocValuesProducer,
        ) -> Result<()> {
            let mut collected = Vec::new();
            let mut binary = values.get_binary(field)?.unwrap();
            loop {
                let doc = binary.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                collected.push((doc, binary.binary_value()?.bytes));
            }
            self.fields
                .push((field.name.clone(), Recorded::Binary(collected)));
            Ok(())
        }

        fn add_sorted_field(
            &mut self,
            field: &FieldInfo,
            values: &dyn DocValuesProducer,
        ) -> Result<()> {
            let mut sorted = values.get_sorted(field)?.unwrap();
            let mut docs_ords = Vec::new();
            loop {
                let doc = sorted.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                docs_ords.push((doc, sorted.ord_value()?));
            }
            let dictionary = (0..sorted.get_value_count()?)
                .map(|ord| sorted.lookup_ord(ord).map(|value| value.bytes))
                .collect::<Result<Vec<_>>>()?;
            self.fields.push((
                field.name.clone(),
                Recorded::Sorted {
                    docs_ords,
                    dictionary,
                },
            ));
            Ok(())
        }

        fn add_sorted_numeric_field(
            &mut self,
            field: &FieldInfo,
            values: &dyn DocValuesProducer,
        ) -> Result<()> {
            let mut collected = Vec::new();
            let mut numeric = values.get_sorted_numeric(field)?.unwrap();
            loop {
                let doc = numeric.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                let count = numeric.doc_value_count()?;
                let mut values = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    values.push(numeric.next_value()?);
                }
                collected.push((doc, values));
            }
            self.fields
                .push((field.name.clone(), Recorded::SortedNumeric(collected)));
            Ok(())
        }

        fn add_sorted_set_field(
            &mut self,
            field: &FieldInfo,
            values: &dyn DocValuesProducer,
        ) -> Result<()> {
            let (docs_ords, dictionary) = collect_sorted_set(values, field)?;
            self.fields.push((
                field.name.clone(),
                Recorded::SortedSet {
                    docs_ords,
                    dictionary,
                },
            ));
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            self.closed = true;
            Ok(())
        }
    }

    /// A minimal [`IndexableField`] whose value the test controls, so the
    /// `indexDocValue` dispatch — including the null checks Java performs
    /// before the concrete writers ever see the value — can be exercised
    /// directly.
    #[derive(Debug)]
    struct TestDocValuesField {
        name: String,
        field_type: FieldType,
        numeric: Option<NumericValue>,
        binary: Option<BytesRef>,
    }

    impl TestDocValuesField {
        fn numeric(name: &str, doc_values_type: DocValuesType, value: NumericValue) -> Self {
            let mut field_type = FieldType::new();
            field_type.set_doc_values_type(doc_values_type).unwrap();
            Self {
                name: name.to_string(),
                field_type,
                numeric: Some(value),
                binary: None,
            }
        }

        fn binary(name: &str, doc_values_type: DocValuesType, value: &[u8]) -> Self {
            let mut field = Self::numeric(name, doc_values_type, NumericValue::Long(0));
            // The numeric placeholder is removed: a binary field carries bytes
            // only.
            field.numeric = None;
            field.binary = Some(BytesRef::new(value.to_vec()));
            field
        }

        fn without_value(mut self) -> Self {
            self.numeric = None;
            self.binary = None;
            self
        }
    }

    impl std::fmt::Display for TestDocValuesField {
        /// Test double; it renders under its own name. `Display` is required
        /// because `IndexableField` carries it as a supertrait, standing in for
        /// the `toString()` every Java field inherits.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("TestDocValuesField")
        }
    }

    impl IndexableField for TestDocValuesField {
        fn name(&self) -> &str {
            &self.name
        }

        fn field_type(&self) -> &dyn IndexableFieldType {
            &self.field_type
        }

        fn token_stream(
            &self,
            _analyzer: &dyn Analyzer,
            _reuse: Option<&mut dyn TokenStream>,
        ) -> Box<dyn TokenStream> {
            Box::new(crate::analysis::StringTokenStream::new(String::new()).unwrap())
        }

        fn binary_value(&self) -> Option<BytesRef> {
            self.binary.clone()
        }

        fn string_value(&self) -> Option<String> {
            None
        }

        fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
            None
        }

        fn numeric_value(&self) -> Option<NumericValue> {
            self.numeric
        }

        fn stored_value(&self) -> Result<Option<StoredValue>> {
            Ok(None)
        }

        fn invertable_type(&self) -> Option<InvertableType> {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Numeric
    // -----------------------------------------------------------------------

    #[test]
    fn numeric_values_reach_the_consumer_in_document_order() {
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        for (doc, value) in [(0, 5i64), (3, -1), (7, 120)] {
            writer.add_value(doc, value).unwrap();
        }
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields,
            vec![(
                "count".to_string(),
                Recorded::Numeric(vec![(0, 5), (3, -1), (7, 120)])
            )]
        );
    }

    #[test]
    fn a_numeric_writer_that_saw_no_document_flushes_an_empty_field() {
        // A field that exists in the segment's FieldInfos but carried no value
        // must still reach the consumer, so the metadata records it as empty
        // rather than dropping the field entirely.
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields,
            vec![("count".to_string(), Recorded::Numeric(Vec::new()))]
        );
    }

    #[test]
    fn a_repeated_numeric_document_is_the_appears_more_than_once_error() {
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        writer.add_value(4, 1).unwrap();
        let error = writer.add_value(4, 2).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("appears more than once") && m.contains("count")),
            "unexpected error: {error:?}"
        );
        // A doc id that goes backwards is the same failure, and the rejected
        // values were not buffered.
        assert!(writer.add_value(3, 2).is_err());
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.fields[0].1, Recorded::Numeric(vec![(4, 1)]));
    }

    #[test]
    fn flushing_a_numeric_field_twice_is_refused() {
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        writer.add_value(0, 1).unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let error = writer.flush(&mut consumer, None).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalState(ref m) if m.contains("already been flushed")),
            "unexpected error: {error:?}"
        );
        assert_eq!(consumer.fields.len(), 1);
    }

    #[test]
    fn the_numeric_producer_replays_and_refuses_foreign_fields() {
        // The Lucene 9.0 consumer walks the producer three times: once for the
        // count and range, once for the docs-with-field set and once for the
        // packed values. Each call must start from the beginning, and a
        // different field must hit the "wrong fieldInfo" guard.
        #[derive(Debug, Default)]
        struct ProbingConsumer {
            passes: Vec<Vec<(i32, i64)>>,
            wrong_field: Option<String>,
            sorted: Option<String>,
        }
        impl DocValuesConsumer for ProbingConsumer {
            fn add_numeric_field(
                &mut self,
                field: &FieldInfo,
                values: &dyn DocValuesProducer,
            ) -> Result<()> {
                for _ in 0..3 {
                    let mut collected = Vec::new();
                    let mut numeric = values.get_numeric(field)?.unwrap();
                    loop {
                        let doc = numeric.next_doc()?;
                        if doc == NO_MORE_DOCS {
                            break;
                        }
                        collected.push((doc, numeric.long_value()?));
                    }
                    self.passes.push(collected);
                }
                let other = FieldInfo::new("other", 7);
                self.wrong_field = Some(
                    values
                        .get_numeric(&other)
                        .err()
                        .map_or_else(String::new, |error| error.to_string()),
                );
                self.sorted = Some(
                    values
                        .get_sorted(field)
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_default(),
                );
                Ok(())
            }

            fn add_binary_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                unreachable!("numeric writers only call add_numeric_field")
            }

            fn add_sorted_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn add_sorted_numeric_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn add_sorted_set_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        for doc in 0..5 {
            writer.add_value(doc * 2, doc as i64 + 1).unwrap();
        }
        let mut consumer = ProbingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.passes.len(), 3);
        assert_eq!(consumer.passes[0], consumer.passes[1]);
        assert_eq!(consumer.passes[1], consumer.passes[2]);
        assert_eq!(consumer.passes[0].len(), 5);
        assert!(
            consumer.wrong_field.unwrap().contains("wrong fieldInfo"),
            "a foreign field must hit the wrong-fieldInfo guard"
        );
        assert!(
            consumer.sorted.unwrap().contains("sorted"),
            "the numeric producer must refuse the sorted view"
        );
    }

    #[test]
    fn the_numeric_iterator_is_forward_only_and_exhausts_cleanly() {
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        writer.add_value(2, 9).unwrap();

        #[derive(Debug, Default)]
        struct ProbingConsumer {
            advance: Option<String>,
            advance_exact: Option<String>,
            after: Vec<i32>,
            long_value_error: Option<String>,
        }
        impl DocValuesConsumer for ProbingConsumer {
            fn add_numeric_field(
                &mut self,
                field: &FieldInfo,
                values: &dyn DocValuesProducer,
            ) -> Result<()> {
                let mut numeric = values.get_numeric(field)?.unwrap();
                self.advance = Some(numeric.advance(0).unwrap_err().to_string());
                self.advance_exact = Some(numeric.advance_exact(0).unwrap_err().to_string());
                assert_eq!(numeric.next_doc()?, 2);
                for _ in 0..4 {
                    self.after.push(numeric.next_doc()?);
                }
                self.long_value_error = Some(numeric.long_value().unwrap_err().to_string());
                Ok(())
            }

            fn add_binary_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn add_sorted_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn add_sorted_numeric_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn add_sorted_set_field(
                &mut self,
                _field: &FieldInfo,
                _values: &dyn DocValuesProducer,
            ) -> Result<()> {
                Err(LuceneError::UnsupportedOperation("unused".to_string()))
            }

            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut consumer = ProbingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert!(consumer.advance.unwrap().contains("advance"));
        assert!(consumer.advance_exact.unwrap().contains("advance_exact"));
        assert_eq!(consumer.after, vec![NO_MORE_DOCS; 4]);
        assert!(consumer
            .long_value_error
            .unwrap()
            .contains("no current document"));
    }

    // -----------------------------------------------------------------------
    // Binary
    // -----------------------------------------------------------------------

    #[test]
    fn binary_values_including_the_empty_value_reach_the_consumer() {
        let mut writer =
            BinaryDocValuesWriter::new(field("blob", 0, DocValuesType::BINARY), counter());
        writer.add_value(0, b"first").unwrap();
        // The empty byte string is a legitimate value, not a missing one: it
        // must be recorded, with the document still counted as having a value.
        writer.add_value(1, b"").unwrap();
        writer.add_value(5, b"tail").unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields,
            vec![(
                "blob".to_string(),
                Recorded::Binary(vec![
                    (0, b"first".to_vec()),
                    (1, Vec::new()),
                    (5, b"tail".to_vec())
                ])
            )]
        );
    }

    #[test]
    fn a_binary_value_repeated_in_the_same_document_is_refused() {
        let mut writer =
            BinaryDocValuesWriter::new(field("blob", 0, DocValuesType::BINARY), counter());
        writer.add_value(2, b"one").unwrap();
        let error = writer.add_value(2, b"two").unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("appears more than once") && m.contains("blob")),
            "unexpected error: {error:?}"
        );
        assert!(writer.add_value(1, b"backwards").is_err());
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields[0].1,
            Recorded::Binary(vec![(2, b"one".to_vec())])
        );
    }

    // -----------------------------------------------------------------------
    // Sorted
    // -----------------------------------------------------------------------

    #[test]
    fn the_sorted_dictionary_is_in_unsigned_byte_order_and_ordinals_follow_it() {
        // b"\xff" sorts *after* b"\x01" in unsigned byte order, although a
        // signed comparison would put it first — this is exactly the ordering
        // BytesRefHash.sort produces, and what Lucene's dictionary guarantees.
        let mut writer =
            SortedDocValuesWriter::new(field("tag", 0, DocValuesType::SORTED), counter());
        writer.add_value(0, b"\xff").unwrap();
        writer.add_value(1, b"abc").unwrap();
        writer.add_value(2, b"\x01").unwrap();
        writer.add_value(4, b"abc").unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let Recorded::Sorted {
            docs_ords,
            dictionary,
        } = &consumer.fields[0].1
        else {
            panic!("expected a sorted field, got {:?}", consumer.fields[0]);
        };
        assert_eq!(
            dictionary,
            &vec![b"\x01".to_vec(), b"abc".to_vec(), b"\xff".to_vec()]
        );
        assert_eq!(docs_ords, &[(0, 2), (1, 1), (2, 0), (4, 1)]);
        assert_eq!(consumer.fields.len(), 1);
        assert_eq!(consumer.fields[0].0, "tag");
    }

    #[test]
    fn a_sorted_value_repeated_in_one_document_is_refused() {
        let mut writer =
            SortedDocValuesWriter::new(field("tag", 0, DocValuesType::SORTED), counter());
        writer.add_value(3, b"one").unwrap();
        let error = writer.add_value(3, b"two").unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("appears more than once") && m.contains("only one value is allowed")),
            "unexpected error: {error:?}"
        );
        // A doc id that goes backwards is the same failure.
        assert!(writer.add_value(2, b"two").is_err());
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let Recorded::Sorted { docs_ords, .. } = &consumer.fields[0].1 else {
            panic!("expected a sorted field");
        };
        assert_eq!(docs_ords.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Sorted numeric
    // -----------------------------------------------------------------------

    #[test]
    fn sorted_numeric_values_are_sorted_per_document_and_counted() {
        let mut writer = SortedNumericDocValuesWriter::new(
            field("vals", 0, DocValuesType::SORTED_NUMERIC),
            counter(),
        );
        // One value for the first documents, then a multi-valued document
        // whose values arrive unsorted; the writer must sort them.
        writer.add_value(0, 1).unwrap();
        writer.add_value(2, 5).unwrap();
        writer.add_value(2, 3).unwrap();
        writer.add_value(2, 4).unwrap();
        writer.add_value(5, 7).unwrap();
        writer.add_value(6, 9).unwrap();
        writer.add_value(6, 8).unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields,
            vec![(
                "vals".to_string(),
                Recorded::SortedNumeric(vec![
                    (0, vec![1]),
                    (2, vec![3, 4, 5]),
                    (5, vec![7]),
                    (6, vec![8, 9])
                ])
            )]
        );
    }

    #[test]
    fn an_all_single_valued_sorted_numeric_field_reports_singleton_views() {
        // When every document carries exactly one value the producer serves a
        // singleton view with docValueCount == 1 — the shape Java hands over,
        // and what makes the codec write the single-valued file layout.
        let mut writer = SortedNumericDocValuesWriter::new(
            field("values", 0, DocValuesType::SORTED_NUMERIC),
            counter(),
        );
        for doc in 0..4 {
            writer.add_value(doc, i64::from(doc) * 10).unwrap();
        }
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields[0].1,
            Recorded::SortedNumeric(vec![
                (0, vec![0]),
                (1, vec![10]),
                (2, vec![20]),
                (3, vec![30])
            ])
        );
    }

    #[test]
    fn a_sorted_numeric_writer_refuses_a_backwards_document() {
        // Java only asserts `docID >= currentDoc`; with assertions off a
        // backwards id would silently corrupt the value stream, so this port
        // refuses it.
        let mut writer = SortedNumericDocValuesWriter::new(
            field("values", 0, DocValuesType::SORTED_NUMERIC),
            counter(),
        );
        writer.add_value(3, 1).unwrap();
        writer.add_value(3, 2).unwrap();
        let error = writer.add_value(2, 3).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("increasing document order") && m.contains("values")),
            "unexpected error: {error:?}"
        );
        // The rejected document was not committed; the earlier document keeps
        // its values.
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields[0].1,
            Recorded::SortedNumeric(vec![(3, vec![1, 2])])
        );
    }

    // -----------------------------------------------------------------------
    // Sorted set
    // -----------------------------------------------------------------------

    #[test]
    fn sorted_set_values_are_deduplicated_sorted_per_document_and_dictionary_ordered() {
        // Two documents: the first carries a duplicate and an unordered pair,
        // the second reuses one value. The dictionary must be the distinct
        // value set in unsigned byte order, each document's ords deduplicated,
        // sorted and mapped into that dictionary.
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(0, b"b").unwrap();
        writer.add_value(0, b"a").unwrap();
        writer.add_value(0, b"b").unwrap();
        writer.add_value(2, b"c").unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let Recorded::SortedSet {
            docs_ords,
            dictionary,
        } = &consumer.fields[0].1
        else {
            panic!("expected a sorted-set field");
        };
        assert_eq!(
            dictionary,
            &vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]
        );
        assert_eq!(docs_ords, &vec![(0, vec![0, 1]), (2, vec![2])]);
        // Every document's ords come back ascending — the codec relies on it.
        assert!(docs_ords
            .iter()
            .all(|(_, ords)| ords.windows(2).all(|w| w[0] < w[1])));
    }

    #[test]
    fn an_all_single_valued_sorted_set_field_reports_singleton_views() {
        // Same contract as the sorted-numeric singleton: one distinct value per
        // document makes the producer serve the singleton view, and the codec
        // then picks the single-valued layout.
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(0, b"x").unwrap();
        writer.add_value(1, b"y").unwrap();
        writer.add_value(3, b"x").unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let Recorded::SortedSet {
            docs_ords,
            dictionary,
        } = &consumer.fields[0].1
        else {
            panic!("expected a sorted-set field");
        };
        assert_eq!(dictionary, &vec![b"x".to_vec(), b"y".to_vec()]);
        assert_eq!(docs_ords, &vec![(0, vec![0]), (1, vec![1]), (3, vec![0])]);
    }

    #[test]
    fn a_sorted_set_writer_refuses_a_backwards_document() {
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(3, b"one").unwrap();
        writer.add_value(3, b"two").unwrap();
        let error = writer.add_value(2, b"three").unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("increasing document order")),
            "unexpected error: {error:?}"
        );
        // The rejected document was not committed; the earlier document keeps
        // both of its distinct values.
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let Recorded::SortedSet { docs_ords, .. } = &consumer.fields[0].1 else {
            panic!("expected a sorted-set field");
        };
        assert_eq!(docs_ords.len(), 1);
        assert_eq!(docs_ords[0].1.len(), 2);
    }

    // -----------------------------------------------------------------------
    // indexDocValue dispatch
    // -----------------------------------------------------------------------

    #[test]
    fn the_dispatch_pulls_the_value_where_java_pulls_it() {
        // `IndexingChain.indexDocValue` reads a long off the NUMERIC and
        // SORTED_NUMERIC branches and bytes off the BINARY, SORTED and
        // SORTED_SET branches; the enum must route to the right writer.
        let counter = counter();
        let mut writer =
            DocValuesWriter::new(field("n", 0, DocValuesType::NUMERIC), Arc::clone(&counter));
        writer
            .add_value(
                0,
                &TestDocValuesField::numeric("n", DocValuesType::NUMERIC, NumericValue::Int(42)),
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.fields[0].1, Recorded::Numeric(vec![(0, 42)]));
    }

    #[test]
    fn numeric_flavours_widen_like_java_number_long_value() {
        // Java's `Number.longValue()` truncates floats toward zero and maps
        // NaN to 0; the Rust `as` casts are specified to do the same, and the
        // dispatch must apply them.
        let cases = [
            (NumericValue::Float(1.9), 1i64),
            (NumericValue::Float(-1.9), -1),
            (NumericValue::Double(2.9), 2),
            (NumericValue::Double(-3.5), -3),
            (NumericValue::Float(f32::NAN), 0),
            (NumericValue::Double(f64::NAN), 0),
            (NumericValue::Int(-7), -7),
        ];
        for (value, expected) in cases {
            let mut writer = DocValuesWriter::new(field("n", 0, DocValuesType::NUMERIC), counter());
            writer
                .add_value(
                    0,
                    &TestDocValuesField::numeric("n", DocValuesType::NUMERIC, value),
                )
                .unwrap();
            let mut consumer = RecordingConsumer::default();
            writer.flush(&mut consumer, None).unwrap();
            assert_eq!(
                consumer.fields[0].1,
                Recorded::Numeric(vec![(0, expected)]),
                "wrong longValue conversion for {value:?}"
            );
        }
    }

    #[test]
    fn a_missing_value_is_refused_before_the_writer_sees_it() {
        // `indexDocValue` null-checks NUMERIC and BINARY; the message shape
        // differs: the NUMERIC branch uses `field="name":` and the SORTED
        // branch `field "name":` — both are locked in here.
        let cases = [
            (
                DocValuesType::NUMERIC,
                TestDocValuesField::numeric("n", DocValuesType::NUMERIC, NumericValue::Long(1))
                    .without_value(),
            ),
            (
                DocValuesType::BINARY,
                TestDocValuesField::binary("b", DocValuesType::BINARY, b"").without_value(),
            ),
            (
                DocValuesType::SORTED,
                TestDocValuesField::binary("s", DocValuesType::SORTED, b"").without_value(),
            ),
        ];
        for (doc_values_type, field_value) in cases {
            let mut writer = DocValuesWriter::new(field("f", 0, doc_values_type), counter());
            let error = writer.add_value(0, &field_value).unwrap_err();
            assert!(
                matches!(error, LuceneError::IllegalArgument(ref m)
                    if m.contains("null value not allowed")),
                "unexpected error for {doc_values_type:?}: {error:?}"
            );
        }
    }

    #[test]
    fn sorted_numeric_dispatch_converts_like_the_numeric_branch() {
        // Java reaches `numericValue().longValue()` for SORTED_NUMERIC without
        // a null check; this port reports the NUMERIC-style message instead
        // (see the module documentation) and applies the same widening.
        let mut writer =
            DocValuesWriter::new(field("sn", 0, DocValuesType::SORTED_NUMERIC), counter());
        writer
            .add_value(
                0,
                &TestDocValuesField::numeric(
                    "sn",
                    DocValuesType::SORTED_NUMERIC,
                    NumericValue::Double(9.7),
                ),
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields[0].1,
            Recorded::SortedNumeric(vec![(0, vec![9])])
        );
    }

    #[test]
    fn the_dispatch_routes_each_type_to_its_recorded_kind() {
        // One round trip per DocValuesType through the enum, proving the
        // `indexDocValue` switch and the flush dispatch agree.
        let mut numeric = DocValuesWriter::new(field("f", 0, DocValuesType::NUMERIC), counter());
        numeric
            .add_value(
                0,
                &TestDocValuesField::numeric(
                    "numeric",
                    DocValuesType::NUMERIC,
                    NumericValue::Long(3),
                ),
            )
            .unwrap();
        let mut binary = DocValuesWriter::new(field("b", 1, DocValuesType::BINARY), counter());
        binary
            .add_value(
                0,
                &TestDocValuesField::binary("b", DocValuesType::BINARY, b"bytes"),
            )
            .unwrap();
        let mut sorted = DocValuesWriter::new(field("s", 2, DocValuesType::SORTED), counter());
        sorted
            .add_value(
                0,
                &TestDocValuesField::binary("s", DocValuesType::SORTED, b"sorted"),
            )
            .unwrap();
        let mut sorted_set =
            DocValuesWriter::new(field("ss", 3, DocValuesType::SORTED_SET), counter());
        sorted_set
            .add_value(
                0,
                &TestDocValuesField::binary("ss", DocValuesType::SORTED_SET, b"set"),
            )
            .unwrap();

        let mut consumer = RecordingConsumer::default();
        for writer in [&mut numeric, &mut binary, &mut sorted, &mut sorted_set] {
            writer.flush(&mut consumer, None).unwrap();
        }
        assert!(matches!(consumer.fields[0].1, Recorded::Numeric(_)));
        assert!(matches!(consumer.fields[1].1, Recorded::Binary(_)));
        assert!(matches!(consumer.fields[2].1, Recorded::Sorted { .. }));
        assert_eq!(consumer.fields[0].0, "f");
        assert_eq!(consumer.fields[1].0, "b");
        assert_eq!(consumer.fields[2].0, "s");
        assert_eq!(consumer.fields[3].0, "ss");
    }

    #[test]
    fn flushing_the_enum_twice_is_refused() {
        let mut writer =
            DocValuesWriter::new(field("f", 0, DocValuesType::SORTED_NUMERIC), counter());
        writer
            .add_value(
                0,
                &TestDocValuesField::numeric(
                    "f",
                    DocValuesType::SORTED_NUMERIC,
                    NumericValue::Long(1),
                ),
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let error = writer.flush(&mut consumer, None).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalState(ref m) if m.contains("already been flushed")),
            "unexpected error: {error:?}"
        );
        assert!(writer
            .add_value(
                1,
                &TestDocValuesField::numeric(
                    "f",
                    DocValuesType::SORTED_NUMERIC,
                    NumericValue::Long(2)
                )
            )
            .is_err());
    }

    // -----------------------------------------------------------------------
    // Accounting
    // -----------------------------------------------------------------------

    #[test]
    fn the_shared_byte_counter_grows_with_the_buffers_and_is_given_back() {
        // Every concrete writer charges the shared counter for what it holds
        // and gives it all back at Drop — including the sorted variants, whose
        // ByteBlockPool and BytesRefHash charge the counter directly.
        let counter = counter();
        {
            let mut writer = NumericDocValuesWriter::new(
                field("n", 0, DocValuesType::NUMERIC),
                Arc::clone(&counter),
            );
            let empty = counter.load(Ordering::Acquire);
            assert!(empty > 0, "the empty buffers already cost something");
            for doc in 0..1_000 {
                writer.add_value(doc, 1).unwrap();
            }
            let full = counter.load(Ordering::Acquire);
            assert!(
                full > empty,
                "a thousand buffered values must be reported: {empty} -> {full}"
            );
            assert_eq!(full, writer.ram_bytes_used());
        }
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "dropping the numeric writer must give the bytes back"
        );
    }

    #[test]
    fn the_sorted_writer_refunds_the_pool_and_the_hash_at_drop() {
        // The sorted writers charge their ByteBlockPool and BytesRefHash to the
        // shared counter on growth, outside the delta-tracked footprint; the
        // Drop impl must refund those too, not just the plain buffers.
        let counter = counter();
        {
            let mut writer = SortedDocValuesWriter::new(
                field("s", 0, DocValuesType::SORTED),
                Arc::clone(&counter),
            );
            for doc in 0..500 {
                writer
                    .add_value(doc, format!("value-{doc}").as_bytes())
                    .unwrap();
            }
            assert!(
                counter.load(Ordering::Acquire) > 0,
                "the sorted writer must charge pool, hash and buffers"
            );
        }
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "dropping the sorted writer must give every byte back"
        );
    }

    #[test]
    fn flushing_gives_the_plain_buffers_back_to_the_counter() {
        let counter = counter();
        let mut writer = NumericDocValuesWriter::new(
            field("n", 0, DocValuesType::NUMERIC),
            Arc::clone(&counter),
        );
        for doc in 0..1_000 {
            writer.add_value(doc, 1).unwrap();
        }
        let full = counter.load(Ordering::Acquire);
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let after = counter.load(Ordering::Acquire);
        assert!(
            after < full,
            "flushing must release the buffers: {full} -> {after}"
        );
        assert_eq!(
            consumer.fields[0].1,
            Recorded::Numeric((0..1_000).map(|d| (d, 1)).collect())
        );
    }

    #[test]
    fn two_sorted_writers_share_one_counter_without_losing_bytes() {
        // Two sorted writers each own a pool and a hash; the shared counter
        // must account for both, and dropping one must not disturb the other's
        // charges.
        let counter = counter();
        let mut left =
            SortedDocValuesWriter::new(field("l", 0, DocValuesType::SORTED), Arc::clone(&counter));
        let mut right = SortedSetDocValuesWriter::new(
            field("r", 1, DocValuesType::SORTED_SET),
            Arc::clone(&counter),
        );
        for doc in 0..200 {
            left.add_value(doc, format!("left-{doc}").as_bytes())
                .unwrap();
            right
                .add_value(doc, format!("right-{doc}").as_bytes())
                .unwrap();
        }
        let both = counter.load(Ordering::Acquire);
        assert!(
            both > left.ram_bytes_used() + right.ram_bytes_used(),
            "the pools and hashes must be part of the total"
        );
        drop(left);
        let after_left = counter.load(Ordering::Acquire);
        assert!(
            after_left < both,
            "dropping one writer must refund its share"
        );
        drop(right);
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "dropping both writers must give every byte back"
        );
    }
}
