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
//! together with `index/DocsWithFieldSet.java`, which this crate ports as
//! [`crate::codecs::DocsWithFieldSet`] (the kNN-vector writer needed it first).
//!
//! # Responsibility
//!
//! Each writer owns the doc values of **one field** of the segment currently
//! being built. It buffers one value (or one value set) per document that
//! carried the field, in increasing document order, in the structures Java
//! uses — `PackedLongValues` builders for the numbers, lengths, term ids and
//! per-document counts, a `PagedBytes` for binary values, and a
//! `BytesRefHash` over the indexing chain's shared `docValuesBytePool` for the
//! dictionary of the sorted variants — and charges the shared counter exactly
//! what Java charges. When the segment flushes it hands everything to a
//! [`DocValuesConsumer`] through a producer whose iterators read those same
//! structures. It writes no files itself — the codec owns the format.
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
//! # Divergences from Java, and why
//!
//! * **A backwards document is refused at the call by the multi-valued
//!   writers.** Java only asserts `docID >= currentDoc` in
//!   `SortedNumericDocValuesWriter.addValue` and
//!   `SortedSetDocValuesWriter.addValue`; with assertions off, the backwards
//!   document is refused later, when `finishCurrentDoc` adds it to the
//!   document set, which throws `IllegalArgumentException("Out of order doc
//!   ids: last=..., next=...")` (`DocsWithFieldSet.java:46-50`). This port
//!   returns the `IllegalArgument` error at the call, before the value is
//!   buffered.
//! * **A second flush is refused.** Java would hand the consumer the field
//!   twice, writing two metadata entries for one field; every writer here
//!   refuses the second call.
//! * **The counter is given back when a writer is dropped.** Java throws the
//!   whole `Counter` away with the indexing chain. Rust drops writers
//!   individually — a document that fails validation can leave one behind —
//!   so each writer refunds what it charged in `Drop`.
//! * **The shared pool and the sorted writers' hashes are reference-counted.**
//!   Java's producers keep references to the writer's `hash` and, through it,
//!   to the chain's `docValuesBytePool`, and read each value with
//!   `hash.get(sortedValues[ord], scratch)`. Here the chain holds the pool as
//!   an `Arc<ByteBlockPool>` and each sorted writer its hash as an
//!   `Arc<BytesRefHash>`; both are mutated through `Arc::get_mut` while
//!   documents are added, and shared, read-only, by the producers `finish`
//!   hands out — the same structures Java reads, with no copy. Adding a value
//!   while a producer over the pool is alive is refused with an
//!   `IllegalState` error; Java never does it, as the producers only exist
//!   while the segment flushes.
//! * Java wraps single-valued producers in `DocValues.singleton(...)`; the
//!   Rust wrappers (`SingletonSortedNumericDocValues`,
//!   `SingletonSortedSetDocValues`) store `Box<dyn Trait>` without
//!   `Send`/`Sync`. Two small local wrappers below provide the same
//!   single-value views.

#![deny(unsafe_code)]

use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::codecs::doc_values::{DocValuesConsumer, DocValuesProducer};
use crate::codecs::state::SegmentWriteState;
use crate::codecs::DocsWithFieldSet;
use crate::document::column::{LongValuesCursor, OrdinalsCursor, OrdinalsTupleCursor};
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{
    BinaryDocValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
};
use crate::index::doc_values::{DocValuesIterator, DocValuesSkipper, NumericDocValues};
use crate::index::documents_writer::{SharedIndexingScratch, INTS_SCRATCH_SIZE};
use crate::index::index_sorter::SorterDocMap;
use crate::index::{DocValuesType, FieldInfo, IndexableField};
use crate::search::bit_set_util;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, DataOutput};
use crate::util::bytes_ref_array::SortableBytesRefArray;
use crate::util::bytes_ref_hash::DEFAULT_CAPACITY;
use crate::util::packed::{
    DeltaPackedLongValues, DeltaPackedLongValuesBuilder, GrowableWriter,
    OwnedPackedLongValuesIterator, PackedInts, PackedIntsMutable, PackedIntsReader,
    PackedLongValues, PackedLongValuesAccess, PackedLongValuesBuilder, PackedLongValuesBuilderOps,
};
use crate::util::paged_bytes::{PagedBytes, PagedBytesDataInput};
use crate::util::MAX_TERM_LENGTH;
use crate::util::{
    Accountable, ArrayUtil, ByteBlockPool, BytesRef, BytesRefArray, BytesRefHash, Counter,
    FixedBitSet, RamUsageEstimator,
};

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
    /// The sorted variants intern their values into the chain's shared
    /// `docValuesBytePool`, which every call that touches the dictionary
    /// receives as an argument.
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
    /// value is pulled off the field exactly where Java pulls it. `pool` is the
    /// chain's `docValuesBytePool`, which the sorted variants intern into.
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
    pub fn add_value(
        &mut self,
        doc_id: i32,
        field: &dyn IndexableField,
        pool: &mut Arc<ByteBlockPool>,
    ) -> Result<()> {
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
                writer.add_value(doc_id, value.slice(), pool)
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
                writer.add_value(doc_id, value.slice(), pool)
            }
        }
    }

    /// Returns a read-only view over the values buffered so far, for the
    /// index sorter to compare.
    ///
    /// Equivalent to `DocValuesWriter.getDocValues()`. As in Java, this
    /// finishes the writer: the buffers are built into their final form and
    /// no further value can be added.
    ///
    /// # Errors
    ///
    /// Propagates whatever the concrete writer raises.
    pub fn get_doc_values(
        &mut self,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        match self {
            Self::Numeric(writer) => writer.get_doc_values(),
            Self::Binary(writer) => writer.get_doc_values(),
            Self::Sorted(writer) => writer.get_doc_values(pool),
            Self::SortedNumeric(writer) => writer.get_doc_values(),
            Self::SortedSet(writer) => writer.get_doc_values(pool),
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
        state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<()> {
        match self {
            Self::Numeric(writer) => writer.flush(state, sort_map, consumer),
            Self::Binary(writer) => writer.flush(state, sort_map, consumer),
            Self::Sorted(writer) => writer.flush(state, sort_map, consumer, pool),
            Self::SortedNumeric(writer) => writer.flush(state, sort_map, consumer),
            Self::SortedSet(writer) => writer.flush(state, sort_map, consumer, pool),
        }
    }

    /// The bytes this writer has charged to the shared counter.
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

/// Converts a [`NumericValue`](crate::document::NumericValue) the way Java's
/// `Number.longValue()` does at `IndexingChain.indexDocValue` — every numeric
/// type widens, floats truncate toward zero, `NaN` becomes `0`.
///
/// The float conversions are JLS numeric conversions (`float`→`long`,
/// `double`→`long`), which saturate at the type bounds and map `NaN` to `0`;
/// Rust's `as` casts are specified to do exactly that.
fn java_long_value(value: &crate::document::NumericValue) -> i64 {
    match value {
        crate::document::NumericValue::Int(v) => i64::from(*v),
        crate::document::NumericValue::Long(v) => *v,
        crate::document::NumericValue::Float(v) => *v as i64,
        crate::document::NumericValue::Double(v) => *v as i64,
    }
}

/// The page size of every builder the writers create.
///
/// Java's `PackedLongValues.deltaPackedBuilder(float)` and
/// `packedBuilder(float)` use `PackedLongValues.DEFAULT_PAGE_SIZE`.
const PAGE_SIZE: usize = PackedLongValues::DEFAULT_PAGE_SIZE as usize;

/// `PackedLongValues.deltaPackedBuilder(PackedInts.COMPACT)`.
fn delta_packed_builder() -> DeltaPackedLongValuesBuilder {
    PackedLongValues::delta_packed_builder(PAGE_SIZE, PackedInts::COMPACT)
        .expect("INVARIANT: the default page size is a legal page size")
}

/// `PackedLongValues.packedBuilder(acceptableOverheadRatio)`.
fn packed_builder(acceptable_overhead_ratio: f32) -> PackedLongValuesBuilder {
    PackedLongValues::packed_builder(PAGE_SIZE, acceptable_overhead_ratio)
        .expect("INVARIANT: the default page size is a legal page size")
}

/// The "Cannot be reused after build()" failure of a builder that has already
/// been built, which is what Java's `Builder.add` throws once `pending` is
/// `null`.
fn already_built() -> LuceneError {
    LuceneError::IllegalState("Cannot be reused after build()".to_string())
}

/// The error a writer reports when it is used after its field was flushed.
fn already_flushed(field_info: &FieldInfo) -> LuceneError {
    LuceneError::IllegalState(format!(
        "the doc values of field \"{}\" have already been flushed",
        field_info.name
    ))
}

/// The "appears more than once in this document" error of the single-valued
/// writers.
fn appears_more_than_once(field_info: &FieldInfo) -> LuceneError {
    LuceneError::IllegalArgument(format!(
        "DocValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
        field_info.name
    ))
}

/// The error the multi-valued writers report for a backwards document, where
/// Java only asserts (see the module documentation).
fn backwards_document(field_info: &FieldInfo, doc_id: i32, current_doc: i32) -> LuceneError {
    LuceneError::IllegalArgument(format!(
        "DocValuesField \"{}\" values must be added in increasing document order (doc {doc_id} after {current_doc})",
        field_info.name
    ))
}

/// Mutable access to a writer's document set while it is still being built.
///
/// The set lives in an [`Arc`] so that the producers handed out at flush can
/// share it, as Java's iterators share the writer's `DocsWithFieldSet`; until
/// then the writer is its only owner.
fn docs_mut(docs_with_field: &mut Arc<DocsWithFieldSet>) -> &mut DocsWithFieldSet {
    Arc::make_mut(docs_with_field)
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

/// The methods every flush-time producer inherits from Java's
/// `EmptyDocValuesProducer` (`index/EmptyDocValuesProducer.java`): every kind
/// other than the one it serves is unsupported, as are the skipper and the
/// merge instance's specialisations.
macro_rules! empty_producer_methods {
    (numeric) => {
        fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
            Err(self.unsupported("numeric"))
        }
    };
    (binary) => {
        fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Err(self.unsupported("binary"))
        }
    };
    (sorted) => {
        fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
            Err(self.unsupported("sorted"))
        }
    };
    (sorted_numeric) => {
        fn get_sorted_numeric(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Err(self.unsupported("sorted numeric"))
        }
    };
    (sorted_set) => {
        fn get_sorted_set(
            &self,
            _field: &FieldInfo,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Err(self.unsupported("sorted set"))
        }
    };
    (common) => {
        /// Inherited from `EmptyDocValuesProducer.getSkipper`
        /// (`index/EmptyDocValuesProducer.java:54-57`): unsupported.
        fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Err(unsupported_skipper())
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
            Ok(Box::new(self.clone()))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    };
}

// -----------------------------------------------------------------------------
// NumericDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers one numeric value per document for a single field.
///
/// Equivalent to `org.apache.lucene.index.NumericDocValuesWriter`
/// (`index/NumericDocValuesWriter.java:32-312`): the values are appended to a
/// delta-packed `PackedLongValues` builder, and the documents to a
/// [`DocsWithFieldSet`].
#[derive(Debug)]
pub struct NumericDocValuesWriter {
    /// `pending`, a `PackedLongValues.deltaPackedBuilder(PackedInts.COMPACT)`;
    /// `None` once built into `final_values`.
    pending: Option<DeltaPackedLongValuesBuilder>,
    /// `finalValues`, built the first time the values are read.
    final_values: Option<Arc<DeltaPackedLongValues>>,
    iw_bytes_used: Arc<AtomicI64>,
    /// What this writer has charged to `iw_bytes_used`, so an update reports
    /// the delta. Equivalent to `NumericDocValuesWriter.bytesUsed`.
    bytes_used: i64,
    docs_with_field: Arc<DocsWithFieldSet>,
    field_info: FieldInfo,
    last_doc_id: i32,
    flushed: bool,
}

impl NumericDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new NumericDocValuesWriter(FieldInfo, Counter)`
    /// (`NumericDocValuesWriter.java:42-49`).
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let pending = delta_packed_builder();
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = pending.ram_bytes_used() + docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            pending: Some(pending),
            final_values: None,
            iw_bytes_used,
            bytes_used,
            docs_with_field: Arc::new(docs_with_field),
            field_info,
            last_doc_id: -1,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    fn pending_mut(&mut self) -> Result<&mut DeltaPackedLongValuesBuilder> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        self.pending.as_mut().ok_or_else(already_built)
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addValue(int, long)`
    /// (`NumericDocValuesWriter.java:51-63`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` is not strictly
    /// greater than the last document added — which in Lucene means the field
    /// appeared twice in one document — and [`LuceneError::IllegalState`] when
    /// the values have already been built or flushed.
    pub fn add_value(&mut self, doc_id: i32, value: i64) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if doc_id <= self.last_doc_id {
            return Err(appears_more_than_once(&self.field_info));
        }
        self.pending_mut()?.add(value)?;
        docs_mut(&mut self.docs_with_field).add(doc_id)?;
        self.update_bytes_used();
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Buffers `value` as the doc value of `count` consecutive documents
    /// starting at `first_doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addRepeatValues(int, long, int)`
    /// (`NumericDocValuesWriter.java:65-79`), which the column path uses for the
    /// parent field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the last document added (Java asserts it), and
    /// [`LuceneError::IllegalState`] when the values have already been built or
    /// flushed.
    pub fn add_repeat_values(&mut self, first_doc_id: i32, value: i64, count: i32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        self.check_appendable(first_doc_id)?;
        self.pending_mut()?.add_repeated(value, count)?;
        docs_mut(&mut self.docs_with_field).add_range(first_doc_id, first_doc_id + count)?;
        self.update_bytes_used();
        self.last_doc_id = first_doc_id + count - 1;
        Ok(())
    }

    /// Buffers every value of `cursor` as the doc values of the consecutive
    /// documents starting at `first_doc_id`.
    ///
    /// Equivalent to `NumericDocValuesWriter.addDenseValues(int,
    /// LongValuesCursor)` (`NumericDocValuesWriter.java:81-95`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the last document added,
    /// [`LuceneError::IllegalState`] when the values have already been built or
    /// flushed, and whatever reading the cursor raises.
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
        self.pending_mut()?.add_cursor(cursor)?;
        docs_mut(&mut self.docs_with_field).add_range(first_doc_id, first_doc_id + num_values)?;
        self.update_bytes_used();
        self.last_doc_id = first_doc_id + num_values - 1;
        Ok(())
    }

    /// The two checks the batch paths run before buffering: the field must not
    /// have been flushed, and the document must come after the last one (Java
    /// asserts `firstDocID > lastDocID`).
    fn check_appendable(&self, doc_id: i32) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if doc_id <= self.last_doc_id {
            return Err(appears_more_than_once(&self.field_info));
        }
        Ok(())
    }

    /// Number of documents that have a value buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Equivalent to `NumericDocValuesWriter.updateBytesUsed`
    /// (`NumericDocValuesWriter.java:97-101`).
    fn update_bytes_used(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let new_bytes_used = pending.ram_bytes_used() + self.docs_with_field.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// The bytes this writer has charged to the shared counter:
    /// `pending.ramBytesUsed() + docsWithField.ramBytesUsed()` as of the last
    /// update.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used
    }

    /// Builds `finalValues` the first time it is needed.
    fn final_values(&mut self) -> Result<Arc<DeltaPackedLongValues>> {
        if self.final_values.is_none() {
            let pending = self.pending.take().ok_or_else(already_built)?;
            self.final_values = Some(Arc::new(pending.build()?));
        }
        Ok(Arc::clone(
            self.final_values
                .as_ref()
                .expect("INVARIANT: built just above"),
        ))
    }

    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `NumericDocValuesWriter.getDocValues()`
    /// (`NumericDocValuesWriter.java:103-109`), which builds `finalValues` and
    /// iterates them; `IndexingChain.maybeSortSegment` reads it to order the
    /// segment before anything is written.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the values raises.
    pub fn get_doc_values(&mut self) -> Result<Arc<dyn DocValuesProducer>> {
        let values = self.final_values()?;
        Self::get_doc_values_producer(
            &self.field_info,
            values,
            Arc::clone(&self.docs_with_field),
            None,
        )
    }

    /// Builds the ordered, per-new-document view of `old_doc_values` that an
    /// index sort produces.
    ///
    /// Equivalent to `NumericDocValuesWriter.sortDocValues(int, Sorter.DocMap,
    /// NumericDocValues, boolean)` (`NumericDocValuesWriter.java:111-133`).
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

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `NumericDocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)` (`NumericDocValuesWriter.java:135-143`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed — writing it twice would put two metadata entries for one field
    /// into the same segment. Otherwise propagates whatever the consumer
    /// raises while writing the field.
    pub fn flush(
        &mut self,
        _state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
    ) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        let values = self.final_values()?;
        self.flushed = true;
        let producer = Self::get_doc_values_producer(
            &self.field_info,
            values,
            Arc::clone(&self.docs_with_field),
            sort_map,
        )?;
        consumer.add_numeric_field(&self.field_info, &*producer)
    }

    /// Builds the [`DocValuesProducer`] `flush` and `get_doc_values` hand to
    /// the codec: the plain buffered view when `sort_map` is `None`, or a view
    /// reordered by the index sort otherwise.
    ///
    /// Equivalent to `NumericDocValuesWriter.getDocValuesProducer(FieldInfo,
    /// PackedLongValues, DocsWithFieldSet, Sorter.DocMap)`
    /// (`NumericDocValuesWriter.java:145-175`).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::sort_doc_values`] raises.
    pub(crate) fn get_doc_values_producer(
        field_info: &FieldInfo,
        values: Arc<DeltaPackedLongValues>,
        docs_with_field: Arc<DocsWithFieldSet>,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        let sorted = match sort_map {
            Some(sort_map) => {
                let mut old_values = BufferedNumeric::new(Arc::clone(&values), &docs_with_field);
                let dense = sort_map.size() == docs_with_field.cardinality();
                Some(Arc::new(Self::sort_doc_values(
                    sort_map.size(),
                    sort_map,
                    &mut old_values,
                    dense,
                )?))
            }
            None => None,
        };
        Ok(Arc::new(BufferedNumericProducer {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            values,
            docs_with_field,
            sorted,
        }))
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
/// Equivalent to the anonymous `EmptyDocValuesProducer` of
/// `NumericDocValuesWriter.getDocValuesProducer`, including its "wrong
/// fieldInfo" guard.
#[derive(Debug, Clone)]
struct BufferedNumericProducer {
    field_number: i32,
    field_name: String,
    values: Arc<DeltaPackedLongValues>,
    docs_with_field: Arc<DocsWithFieldSet>,
    /// The field reordered by an index sort, when one applies.
    sorted: Option<Arc<NumericDVs>>,
}

producer_guard!(BufferedNumericProducer);

impl DocValuesProducer for BufferedNumericProducer {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.check_field(field)?;
        Ok(Some(match &self.sorted {
            None => Box::new(BufferedNumeric::new(
                Arc::clone(&self.values),
                &self.docs_with_field,
            )),
            Some(sorted) => Box::new(SortingNumeric::new(Arc::clone(sorted))),
        }))
    }

    empty_producer_methods!(binary);
    empty_producer_methods!(sorted);
    empty_producer_methods!(sorted_numeric);
    empty_producer_methods!(sorted_set);
    empty_producer_methods!(common);
}

/// Iterates the docs and one value per doc held in memory.
///
/// Equivalent to `NumericDocValuesWriter.BufferedNumericDocValues`
/// (`NumericDocValuesWriter.java:178-232`). Like its Java counterpart it is
/// forward-only: `advance` and `advance_exact` are not supported.
struct BufferedNumeric {
    iter: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
    value: i64,
}

impl BufferedNumeric {
    fn new(values: Arc<DeltaPackedLongValues>, docs_with_field: &Arc<DocsWithFieldSet>) -> Self {
        Self {
            iter: OwnedPackedLongValuesIterator::new(values),
            docs_with_field: DocsWithFieldSet::shared_iterator(docs_with_field),
            value: 0,
        }
    }
}

impl fmt::Debug for BufferedNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedNumeric")
            .field("doc_id", &self.docs_with_field.doc_id())
            .finish()
    }
}

impl DocIdSetIterator for BufferedNumeric {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.docs_with_field.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            self.value = self.iter.next_value();
        }
        Ok(doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedNumericDocValues does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.docs_with_field.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.docs_with_field.doc_id_run_end()
    }
}

impl DocValuesIterator for BufferedNumeric {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedNumericDocValues does not support advanceExact".to_string(),
        ))
    }
}

impl NumericDocValues for BufferedNumeric {
    fn long_value(&self) -> Result<i64> {
        Ok(self.value)
    }
}

/// The numeric values of a field once an index sort has reordered the
/// segment: one value per new document ID, plus which new document IDs
/// actually carry the field.
///
/// Equivalent to `NumericDocValuesWriter.NumericDVs`
/// (`NumericDocValuesWriter.java:283-312`).
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
/// Equivalent to `NumericDocValuesWriter.SortingNumericDocValues`
/// (`NumericDocValuesWriter.java:234-281`).
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
            "use nextDoc() instead".to_string(),
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
        // needed in IndexSorter#{Long|Int|Double|Float}Sorter
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

/// Java's `BinaryDocValuesWriter.BLOCK_BITS`: 4 kB pages for the `PagedBytes`.
const BINARY_BLOCK_BITS: u32 = 12;

/// Buffers one binary value per document for a single field.
///
/// Equivalent to `org.apache.lucene.index.BinaryDocValuesWriter`
/// (`index/BinaryDocValuesWriter.java:37-263`): the values are appended to a
/// `PagedBytes` of 4 kB pages and their lengths to a delta-packed
/// `PackedLongValues` builder.
#[derive(Debug)]
pub struct BinaryDocValuesWriter {
    bytes: PagedBytes,
    /// `lengths`, a `PackedLongValues.deltaPackedBuilder(PackedInts.COMPACT)`;
    /// `None` once built into `final_lengths`.
    lengths: Option<DeltaPackedLongValuesBuilder>,
    final_lengths: Option<Arc<DeltaPackedLongValues>>,
    /// The frozen bytes, read sequentially by every iterator: Java's
    /// `bytes.getDataInput()`, taken once at freeze time.
    bytes_input: Option<PagedBytesDataInput<'static>>,
    docs_with_field: Arc<DocsWithFieldSet>,
    field_info: FieldInfo,
    iw_bytes_used: Arc<AtomicI64>,
    bytes_used: i64,
    last_doc_id: i32,
    max_length: usize,
    flushed: bool,
}

impl BinaryDocValuesWriter {
    /// Creates a writer for `field_info`.
    ///
    /// Equivalent to `new BinaryDocValuesWriter(FieldInfo, Counter)`
    /// (`BinaryDocValuesWriter.java:56-65`), which charges the lengths builder
    /// and the document set but not the empty `PagedBytes`.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let lengths = delta_packed_builder();
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = lengths.ram_bytes_used() + docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            bytes: PagedBytes::new(BINARY_BLOCK_BITS),
            lengths: Some(lengths),
            final_lengths: None,
            bytes_input: None,
            docs_with_field: Arc::new(docs_with_field),
            field_info,
            iw_bytes_used,
            bytes_used,
            last_doc_id: -1,
            max_length: 0,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `BinaryDocValuesWriter.addValue(int, BytesRef)`
    /// (`BinaryDocValuesWriter.java:67-97`). Note the validation order, which
    /// differs from the other writers: document order first, then the null
    /// check (performed by the caller, which reads the value off the field),
    /// then the length check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` repeats or goes
    /// backwards, or when `value` is longer than [`BINARY_MAX_LENGTH`], and
    /// [`LuceneError::IllegalState`] when the field has already been built or
    /// flushed.
    pub fn add_value(&mut self, doc_id: i32, value: &[u8]) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if doc_id <= self.last_doc_id {
            return Err(appears_more_than_once(&self.field_info));
        }
        if value.len() > BINARY_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {BINARY_MAX_LENGTH}",
                self.field_info.name
            )));
        }
        let lengths = self.lengths.as_mut().ok_or_else(already_built)?;
        self.max_length = self.max_length.max(value.len());
        lengths.add(value.len() as i64)?;
        self.bytes
            .get_data_output()?
            .write_bytes(value, 0, value.len())?;
        docs_mut(&mut self.docs_with_field).add(doc_id)?;
        self.update_bytes_used();

        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Equivalent to `BinaryDocValuesWriter.updateBytesUsed`
    /// (`BinaryDocValuesWriter.java:99-104`).
    fn update_bytes_used(&mut self) {
        let Some(lengths) = self.lengths.as_ref() else {
            return;
        };
        let new_bytes_used = lengths.ram_bytes_used()
            + self.bytes.ram_bytes_used()
            + self.docs_with_field.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// The bytes this writer has charged to the shared counter as of the last
    /// update.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used
    }

    /// Freezes the `PagedBytes` exactly once.
    ///
    /// Equivalent to `BinaryDocValuesWriter.freezeBytes()`
    /// (`BinaryDocValuesWriter.java:106-114`): `getDocValues()` may be called
    /// before `flush()` (when index sorting on this field reads the in-RAM
    /// values), so `flush()` cannot rely on being the first to freeze.
    fn freeze_bytes(&mut self) -> Result<()> {
        if self.bytes_input.is_none() {
            self.bytes.freeze(false)?;
            self.bytes_input = Some(self.bytes.get_shared_data_input()?);
        }
        Ok(())
    }

    /// Builds `finalLengths` the first time it is needed.
    fn final_lengths(&mut self) -> Result<Arc<DeltaPackedLongValues>> {
        if self.final_lengths.is_none() {
            let lengths = self.lengths.take().ok_or_else(already_built)?;
            self.final_lengths = Some(Arc::new(lengths.build()?));
        }
        Ok(Arc::clone(
            self.final_lengths
                .as_ref()
                .expect("INVARIANT: built just above"),
        ))
    }

    /// The buffered view `getDocValues()` and `flush()` build.
    fn buffered(&self, lengths: &Arc<DeltaPackedLongValues>) -> BufferedBinary {
        BufferedBinary::new(
            Arc::clone(lengths),
            self.max_length,
            self.bytes_input
                .clone()
                .expect("INVARIANT: the bytes are frozen before any view is built"),
            &self.docs_with_field,
        )
    }

    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `BinaryDocValuesWriter.getDocValues()`
    /// (`BinaryDocValuesWriter.java:116-124`).
    ///
    /// # Errors
    ///
    /// Propagates whatever building the lengths or freezing the bytes raises.
    pub fn get_doc_values(&mut self) -> Result<Arc<dyn DocValuesProducer>> {
        let lengths = self.final_lengths()?;
        self.freeze_bytes()?;
        Ok(Arc::new(self.producer(lengths, None)))
    }

    fn producer(
        &self,
        lengths: Arc<DeltaPackedLongValues>,
        sorted: Option<Arc<BinaryDVs>>,
    ) -> BufferedBinaryProducer {
        BufferedBinaryProducer {
            field_number: self.field_info.number,
            field_name: self.field_info.name.clone(),
            lengths,
            max_length: self.max_length,
            bytes_input: self
                .bytes_input
                .clone()
                .expect("INVARIANT: the bytes are frozen before any view is built"),
            docs_with_field: Arc::clone(&self.docs_with_field),
            sorted,
        }
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `BinaryDocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)` (`BinaryDocValuesWriter.java:126-162`):
    /// with an index sort, the values are first copied into `BinaryDVs` by new
    /// document number.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
    ) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        self.freeze_bytes()?;
        let lengths = self.final_lengths()?;
        self.flushed = true;
        let sorted = match sort_map {
            Some(sort_map) => Some(Arc::new(BinaryDVs::new(
                state.segment_info.max_doc()?,
                sort_map,
                &mut self.buffered(&lengths),
            )?)),
            None => None,
        };
        let producer = self.producer(lengths, sorted);
        consumer.add_binary_field(&self.field_info, &producer)
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
/// Equivalent to the anonymous `EmptyDocValuesProducer` of
/// `BinaryDocValuesWriter.flush`, including its "wrong fieldInfo" guard.
#[derive(Debug, Clone)]
struct BufferedBinaryProducer {
    field_number: i32,
    field_name: String,
    lengths: Arc<DeltaPackedLongValues>,
    max_length: usize,
    bytes_input: PagedBytesDataInput<'static>,
    docs_with_field: Arc<DocsWithFieldSet>,
    sorted: Option<Arc<BinaryDVs>>,
}

producer_guard!(BufferedBinaryProducer);

impl DocValuesProducer for BufferedBinaryProducer {
    fn get_binary(&self, field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        self.check_field(field)?;
        Ok(Some(match &self.sorted {
            None => Box::new(BufferedBinary::new(
                Arc::clone(&self.lengths),
                self.max_length,
                self.bytes_input.clone(),
                &self.docs_with_field,
            )),
            Some(sorted) => Box::new(SortingBinary::new(Arc::clone(sorted))),
        }))
    }

    empty_producer_methods!(numeric);
    empty_producer_methods!(sorted);
    empty_producer_methods!(sorted_numeric);
    empty_producer_methods!(sorted_set);
    empty_producer_methods!(common);
}

/// Iterates the buffered binary values of one field.
///
/// Equivalent to `BinaryDocValuesWriter.BufferedBinaryDocValues`
/// (`BinaryDocValuesWriter.java:165-224`): each document's length comes off the
/// lengths iterator and its bytes off the sequential `PagedBytes` input.
struct BufferedBinary {
    /// `value`, a `BytesRefBuilder` grown to `maxLength` once.
    value: Vec<u8>,
    length: usize,
    lengths_iterator: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
    bytes_iterator: PagedBytesDataInput<'static>,
}

impl BufferedBinary {
    fn new(
        lengths: Arc<DeltaPackedLongValues>,
        max_length: usize,
        bytes_iterator: PagedBytesDataInput<'static>,
        docs_with_field: &Arc<DocsWithFieldSet>,
    ) -> Self {
        Self {
            value: vec![0u8; max_length],
            length: 0,
            lengths_iterator: OwnedPackedLongValuesIterator::new(lengths),
            docs_with_field: DocsWithFieldSet::shared_iterator(docs_with_field),
            bytes_iterator,
        }
    }
}

impl fmt::Debug for BufferedBinary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedBinary")
            .field("doc_id", &self.docs_with_field.doc_id())
            .finish()
    }
}

impl DocIdSetIterator for BufferedBinary {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.docs_with_field.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            let length = usize::try_from(self.lengths_iterator.next_value()).map_err(|_| {
                LuceneError::IllegalState("integer overflow in a binary value length".to_string())
            })?;
            self.length = length;
            self.bytes_iterator.read_bytes(&mut self.value, 0, length)?;
        }
        Ok(doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedBinaryDocValues does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.docs_with_field.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.docs_with_field.doc_id_run_end()
    }
}

impl DocValuesIterator for BufferedBinary {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedBinaryDocValues does not support advanceExact".to_string(),
        ))
    }
}

impl BinaryDocValues for BufferedBinary {
    fn binary_value(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.value[..self.length].to_vec()))
    }
}

/// The binary values of a field laid out by new document number.
///
/// Equivalent to `BinaryDocValuesWriter.BinaryDVs`
/// (`BinaryDocValuesWriter.java:274-292`): `offsets[newDocID]` is one plus the
/// value's index in a `BytesRefArray` charged to a fresh counter, `0` meaning
/// the document has no value.
#[derive(Debug)]
struct BinaryDVs {
    offsets: Vec<i32>,
    values: BytesRefArray,
}

impl BinaryDVs {
    fn new(
        max_doc: i32,
        sort_map: &SorterDocMap,
        old_values: &mut dyn BinaryDocValues,
    ) -> Result<Self> {
        let mut offsets = vec![0i32; max_doc as usize];
        let mut values = BytesRefArray::new(Arc::new(Counter::new_counter()));
        let mut offset = 1; // 0 means no values for this document
        loop {
            let doc_id = old_values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = sort_map.old_to_new(doc_id);
            values.append(&old_values.binary_value()?)?;
            offsets[new_doc_id as usize] = offset;
            offset += 1;
        }
        Ok(Self { offsets, values })
    }
}

/// Iterates the binary doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `BinaryDocValuesWriter.SortingBinaryDocValues`
/// (`BinaryDocValuesWriter.java:226-272`).
struct SortingBinary {
    dvs: Arc<BinaryDVs>,
    doc_id: i32,
}

impl SortingBinary {
    fn new(dvs: Arc<BinaryDVs>) -> Self {
        Self { dvs, doc_id: -1 }
    }
}

impl fmt::Debug for SortingBinary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingBinary")
            .field("doc_id", &self.doc_id)
            .finish()
    }
}

impl DocIdSetIterator for SortingBinary {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            self.doc_id += 1;
            if self.doc_id as usize == self.dvs.offsets.len() {
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            if self.dvs.offsets[self.doc_id as usize] > 0 {
                return Ok(self.doc_id);
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.dvs.values.size() as i64
    }
}

impl DocValuesIterator for SortingBinary {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }
}

impl BinaryDocValues for SortingBinary {
    fn binary_value(&self) -> Result<BytesRef> {
        self.dvs
            .values
            .get((self.dvs.offsets[self.doc_id as usize] - 1) as usize)
    }
}

// -----------------------------------------------------------------------------
// SortedDocValuesWriter
// -----------------------------------------------------------------------------

/// The largest value a sorted writer accepts: Java's `BYTE_BLOCK_SIZE - 2`.
const SORTED_MAX_LENGTH: usize = MAX_TERM_LENGTH;

/// The dictionary of a finished sorted variant.
///
/// Equivalent to what Java's buffered producers hold after `finish()`: the
/// writer's `hash` over the chain's `docValuesBytePool`, and the
/// `finalSortedValues` and `finalOrdMap` arrays (`SortedDocValuesWriter.java:214-226`,
/// `SortedSetDocValuesWriter.java:222-236`). The hash and the pool are shared,
/// not copied: a value is read with `hash.get(sortedValues[ord], scratch)`, as
/// `BufferedSortedDocValues.lookupOrd` (`SortedDocValuesWriter.java:364-368`)
/// and `BufferedSortedSetDocValues.lookupOrd`
/// (`SortedSetDocValuesWriter.java:412-416`) do.
#[derive(Debug, Clone)]
struct Dictionary {
    hash: Arc<BytesRefHash>,
    pool: Arc<ByteBlockPool>,
    /// `sortedValues`: the term id of each ordinal.
    sorted_values: Arc<Vec<i32>>,
    /// `ordMap[termID] = ord`.
    ord_map: Arc<Vec<i32>>,
}

impl Dictionary {
    /// `sortedValues = hash.sort()` and `ordMap[sortedValues[ord]] = ord`.
    fn build(hash: &Arc<BytesRefHash>, pool: &Arc<ByteBlockPool>) -> Self {
        let sorted_values = hash.sort(pool);
        let mut ord_map = vec![0i32; sorted_values.len()];
        for (ord, &term_id) in sorted_values.iter().enumerate() {
            ord_map[term_id as usize] = ord as i32;
        }
        Self {
            hash: Arc::clone(hash),
            pool: Arc::clone(pool),
            sorted_values: Arc::new(sorted_values),
            ord_map: Arc::new(ord_map),
        }
    }

    /// `valueCount`: the number of distinct values.
    fn len(&self) -> usize {
        self.sorted_values.len()
    }

    /// `hash.get(sortedValues[ord], scratch)`, refusing an out-of-range
    /// ordinal (Java indexes the array and throws).
    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        usize::try_from(ord)
            .ok()
            .and_then(|ord| self.sorted_values.get(ord))
            .map(|&term_id| BytesRef::new(self.hash.get(&self.pool, term_id).to_vec()))
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "ord={ord} is out of bounds 0 .. {}",
                    self.len() as i64 - 1
                ))
            })
    }
}

/// The writer's hash while values are being added.
///
/// Producers share the hash only once `finish` ran, after which `check_open`
/// refuses every addition, so the hash is never shared here.
fn hash_mut(hash: &mut Arc<BytesRefHash>) -> Result<&mut BytesRefHash> {
    Arc::get_mut(hash).ok_or_else(already_built)
}

/// The chain's `docValuesBytePool` while values are being added.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] while a producer over the pool is
/// alive (see the module documentation).
fn pool_mut(pool: &mut Arc<ByteBlockPool>) -> Result<&mut ByteBlockPool> {
    Arc::get_mut(pool).ok_or_else(|| {
        LuceneError::IllegalState("the doc-values byte pool is read by a live producer".to_string())
    })
}

/// The ordinal-to-term-id table of a batch path:
/// `dictSize <= SharedIndexingScratch.INTS_SCRATCH_SIZE ? scratch.intsScratch()
/// : new int[dictSize]`, then `Arrays.fill(ordToHash, 0, dictSize, -1)`
/// (`SortedDocValuesWriter.java:123-128`, `SortedSetDocValuesWriter.java:115-120`).
fn ord_to_hash_table<'a>(
    dict_size: usize,
    scratch: &'a mut SharedIndexingScratch,
    owned: &'a mut Vec<i32>,
) -> &'a mut [i32] {
    let table = if dict_size <= INTS_SCRATCH_SIZE {
        &mut scratch.ints_scratch()[..dict_size]
    } else {
        *owned = vec![0i32; dict_size];
        &mut owned[..]
    };
    table.fill(-1);
    table
}

/// Buffers one binary value per document and builds the value dictionary.
///
/// Equivalent to `org.apache.lucene.index.SortedDocValuesWriter`
/// (`index/SortedDocValuesWriter.java:45-404`). Values are interned into a
/// [`BytesRefHash`] over the indexing chain's shared `docValuesBytePool`;
/// `pending`, a delta-packed `PackedLongValues` builder, holds the hash term id
/// of every document; at finish the term ids are ranked by unsigned byte order
/// (the hash's sort) to produce the final ordinals.
#[derive(Debug)]
pub struct SortedDocValuesWriter {
    hash: Arc<BytesRefHash>,
    pending: Option<DeltaPackedLongValuesBuilder>,
    docs_with_field: Arc<DocsWithFieldSet>,
    iw_bytes_used: Arc<AtomicI64>,
    /// What `updateBytesUsed` has charged: the builder and the document set.
    bytes_used: i64,
    /// What the writer charged the counter directly: `2 * Integer.BYTES` per
    /// distinct value. Only kept so that `Drop` can give it back.
    direct_charges: i64,
    field_info: FieldInfo,
    last_doc_id: i32,
    final_ords: Option<Arc<DeltaPackedLongValues>>,
    dictionary: Option<Dictionary>,
    flushed: bool,
}

impl SortedDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedDocValuesWriter(FieldInfo, Counter,
    /// ByteBlockPool, SharedIndexingScratch)` (`SortedDocValuesWriter.java:58-71`):
    /// the hash is a `BytesRefHash(pool, DEFAULT_CAPACITY,
    /// DirectBytesStartArray(DEFAULT_CAPACITY, iwBytesUsed))`. The pool itself is
    /// the chain's, handed to every call that interns or reads values.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let hash = Arc::new(BytesRefHash::with_direct_bytes_start(
            DEFAULT_CAPACITY,
            Arc::clone(&iw_bytes_used),
        ));
        let pending = delta_packed_builder();
        let docs_with_field = DocsWithFieldSet::new();
        let bytes_used = pending.ram_bytes_used() + docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            hash,
            pending: Some(pending),
            docs_with_field: Arc::new(docs_with_field),
            iw_bytes_used,
            bytes_used,
            direct_charges: 0,
            field_info,
            last_doc_id: -1,
            final_ords: None,
            dictionary: None,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    fn check_open(&self) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if self.pending.is_none() {
            return Err(already_built());
        }
        Ok(())
    }

    /// Charges `bytes` straight to the counter, as Java's
    /// `iwBytesUsed.addAndGet` does.
    fn charge_directly(&mut self, bytes: i64) {
        self.iw_bytes_used.fetch_add(bytes, Ordering::AcqRel);
        self.direct_charges += bytes;
    }

    /// Buffers `value` as the doc value of `doc_id`.
    ///
    /// Equivalent to `SortedDocValuesWriter.addValue(int, BytesRef)`
    /// (`SortedDocValuesWriter.java:73-95`), with the null check performed by
    /// the caller (see [`DocValuesWriter::add_value`]). As in Java, the
    /// document set grows after `addOneValue` has updated the counter, so its
    /// growth is charged with the next value.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` is not strictly
    /// greater than the last document added, or when `value` is longer than
    /// [`SORTED_MAX_LENGTH`], and [`LuceneError::IllegalState`] when the field
    /// has already been built or flushed.
    pub fn add_value(
        &mut self,
        doc_id: i32,
        value: &[u8],
        pool: &mut Arc<ByteBlockPool>,
    ) -> Result<()> {
        self.check_open()?;
        if doc_id <= self.last_doc_id {
            return Err(appears_more_than_once(&self.field_info));
        }
        if value.len() > SORTED_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {SORTED_MAX_LENGTH}",
                self.field_info.name
            )));
        }
        self.add_one_value(value, pool)?;
        docs_mut(&mut self.docs_with_field).add(doc_id)?;
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Equivalent to `SortedDocValuesWriter.addOneValue(BytesRef)`
    /// (`SortedDocValuesWriter.java:97-112`).
    fn add_one_value(&mut self, value: &[u8], pool: &mut Arc<ByteBlockPool>) -> Result<()> {
        let mut term_id = hash_mut(&mut self.hash)?.add(pool_mut(pool)?, value)?;
        if term_id < 0 {
            term_id = -term_id - 1;
        } else {
            // reserve additional space for each unique value:
            // 1. when indexing, when hash is 50% full, rehash() suddenly needs 2*size ints.
            // 2. when flushing, we need 1 int per value (slot in the ordMap).
            self.charge_directly(2 * i64::from(i32::BITS / 8));
        }
        self.pending
            .as_mut()
            .ok_or_else(already_built)?
            .add(i64::from(term_id))?;
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers one dictionary-encoded value per document the tuple cursor
    /// names, at `base_doc_id` plus the batch-local id.
    ///
    /// Equivalent to `SortedDocValuesWriter.addOrdinalTuples(int, List,
    /// OrdinalsTupleCursor)` (`SortedDocValuesWriter.java:114-147`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a document repeats or an
    /// ordinal is out of range, [`LuceneError::IllegalState`] when the field
    /// has already been built or flushed, and whatever reading the cursor
    /// raises.
    pub fn add_ordinal_tuples(
        &mut self,
        base_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsTupleCursor,
        pool: &mut Arc<ByteBlockPool>,
        scratch: &mut SharedIndexingScratch,
    ) -> Result<()> {
        self.check_open()?;
        let mut owned = Vec::new();
        let ord_to_hash = ord_to_hash_table(dictionary.len(), scratch, &mut owned);
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            let doc_id = base_doc_id + batch_doc_id;
            if doc_id <= self.last_doc_id {
                return Err(appears_more_than_once(&self.field_info));
            }
            let ord = cursor.ord_value();
            let id = self.lookup_or_translate(ord, dictionary, ord_to_hash, pool)?;
            self.pending
                .as_mut()
                .ok_or_else(already_built)?
                .add(i64::from(id))?;
            docs_mut(&mut self.docs_with_field).add(doc_id)?;
            self.last_doc_id = doc_id;
        }
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers one dictionary-encoded value per consecutive document starting
    /// at `first_doc_id`.
    ///
    /// Equivalent to `SortedDocValuesWriter.addDenseOrdinalValues(int, List,
    /// OrdinalsCursor)` (`SortedDocValuesWriter.java:149-182`), which does one
    /// hash lookup per distinct dictionary entry actually used rather than one
    /// per document.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the range does not start
    /// after the last document written or an ordinal is out of range,
    /// [`LuceneError::IllegalState`] when the field has already been built or
    /// flushed, and whatever reading the cursor raises.
    pub fn add_dense_ordinal_values(
        &mut self,
        first_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsCursor,
        pool: &mut Arc<ByteBlockPool>,
        scratch: &mut SharedIndexingScratch,
    ) -> Result<()> {
        let n = cursor.size();
        if n == 0 {
            return Ok(());
        }
        self.check_open()?;
        if first_doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\": firstDocID={first_doc_id} must be > lastDocID={}",
                self.field_info.name, self.last_doc_id
            )));
        }
        let mut owned = Vec::new();
        let ord_to_hash = ord_to_hash_table(dictionary.len(), scratch, &mut owned);
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
            let added = self
                .lookup_or_translate(ord, dictionary, ord_to_hash, pool)
                .and_then(|id| {
                    self.pending
                        .as_mut()
                        .ok_or_else(already_built)?
                        .add(i64::from(id))
                });
            if let Err(e) = added {
                outcome = Err(e);
                break;
            }
            processed += 1;
        }
        if processed > 0 {
            docs_mut(&mut self.docs_with_field)
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
    /// List, int[])` (`SortedDocValuesWriter.java:184-206`).
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
        pool: &mut Arc<ByteBlockPool>,
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
            id =
                hash_mut(&mut self.hash)?.add(pool_mut(pool)?, dictionary[ord as usize].slice())?;
            if id < 0 {
                id = -id - 1;
            } else {
                self.charge_directly(2 * i64::from(i32::BITS / 8));
            }
            ord_to_hash[ord as usize] = id;
        }
        Ok(id)
    }

    /// Equivalent to `SortedDocValuesWriter.updateBytesUsed`
    /// (`SortedDocValuesWriter.java:208-212`).
    fn update_bytes_used(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let new_bytes_used = pending.ram_bytes_used() + self.docs_with_field.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// The bytes this writer has charged to the shared counter, other than the
    /// hash's own slot array.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used + self.direct_charges
    }

    /// Number of documents that have a value buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Builds the final ordinals and the dictionary, once.
    ///
    /// Equivalent to `SortedDocValuesWriter.finish()`
    /// (`SortedDocValuesWriter.java:214-226`): `updateBytesUsed()`, then
    /// `hash.sort()`, `pending.build()` and the ordinal map.
    fn finish(&mut self, pool: &Arc<ByteBlockPool>) -> Result<()> {
        if self.dictionary.is_none() {
            self.update_bytes_used();
            let dictionary = Dictionary::build(&self.hash, pool);
            let pending = self.pending.take().ok_or_else(already_built)?;
            self.final_ords = Some(Arc::new(pending.build()?));
            self.dictionary = Some(dictionary);
        }
        Ok(())
    }

    /// The finished ordinals and dictionary; [`Self::finish`] must have run.
    fn finished(&self) -> (Arc<DeltaPackedLongValues>, &Dictionary) {
        (
            Arc::clone(self.final_ords.as_ref().expect("INVARIANT: finish() ran")),
            self.dictionary.as_ref().expect("INVARIANT: finish() ran"),
        )
    }

    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedDocValuesWriter.getDocValues()`
    /// (`SortedDocValuesWriter.java:228-232`).
    ///
    /// # Errors
    ///
    /// Propagates whatever building the ordinals raises.
    pub fn get_doc_values(
        &mut self,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        self.finish(pool)?;
        let (ords, dictionary) = self.finished();
        Self::get_doc_values_producer(
            &self.field_info,
            dictionary,
            ords,
            Arc::clone(&self.docs_with_field),
            None,
        )
    }

    /// Builds the ordered, per-new-document ordinal view that an index sort
    /// produces: `-1` where the sorted document carries no value.
    ///
    /// Equivalent to the private `SortedDocValuesWriter.sortDocValues(int,
    /// Sorter.DocMap, SortedDocValues)` (`SortedDocValuesWriter.java:234-244`).
    ///
    /// # Errors
    ///
    /// Propagates whatever iterating `old_values` raises.
    fn sort_doc_values(
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

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedDocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)` (`SortedDocValuesWriter.java:246-253`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        _state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        self.finish(pool)?;
        self.flushed = true;
        let (ords, dictionary) = self.finished();
        let producer = Self::get_doc_values_producer(
            &self.field_info,
            dictionary,
            ords,
            Arc::clone(&self.docs_with_field),
            sort_map,
        )?;
        consumer.add_sorted_field(&self.field_info, &*producer)
    }

    /// Builds the [`DocValuesProducer`] `flush` and `get_doc_values` hand to
    /// the codec: the plain buffered view when `sort_map` is `None`, or a view
    /// reordered by the index sort otherwise.
    ///
    /// Equivalent to the static `SortedDocValuesWriter.getDocValuesProducer(
    /// FieldInfo, BytesRefHash, PackedLongValues, int[], int[],
    /// DocsWithFieldSet, Sorter.DocMap)` (`SortedDocValuesWriter.java:255-290`),
    /// which `SortedSetDocValuesWriter` also calls for a single-valued field —
    /// with plain packed term ids rather than delta-packed ones, hence the type
    /// parameter.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::sort_doc_values`] raises.
    fn get_doc_values_producer<V>(
        field_info: &FieldInfo,
        dictionary: &Dictionary,
        ords: Arc<V>,
        docs_with_field: Arc<DocsWithFieldSet>,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<Arc<dyn DocValuesProducer>>
    where
        V: PackedLongValuesAccess + Send + Sync + fmt::Debug + 'static,
    {
        let mut producer = BufferedSortedProducer {
            field_number: field_info.number,
            field_name: field_info.name.clone(),
            dictionary: dictionary.clone(),
            ords,
            docs_with_field,
            sorted: None,
        };
        if let Some(sort_map) = sort_map {
            let mut old_values = producer.buffered();
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
    /// delta-tracked buffers, the direct charges and the hash's slot array.
    /// The pool's blocks belong to the chain, which releases them; a producer
    /// still holding the hash only reads it.
    fn drop(&mut self) {
        self.iw_bytes_used.fetch_sub(
            self.bytes_used + self.direct_charges + self.hash.charged_bytes(),
            Ordering::AcqRel,
        );
    }
}

/// Serves the buffered sorted values of one field to a [`DocValuesConsumer`].
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` of
/// `SortedDocValuesWriter.getDocValuesProducer`.
#[derive(Debug)]
struct BufferedSortedProducer<V> {
    field_number: i32,
    field_name: String,
    dictionary: Dictionary,
    ords: Arc<V>,
    docs_with_field: Arc<DocsWithFieldSet>,
    sorted: Option<Arc<Vec<i32>>>,
}

impl<V> Clone for BufferedSortedProducer<V> {
    fn clone(&self) -> Self {
        Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            dictionary: self.dictionary.clone(),
            ords: Arc::clone(&self.ords),
            docs_with_field: Arc::clone(&self.docs_with_field),
            sorted: self.sorted.clone(),
        }
    }
}

impl<V: PackedLongValuesAccess + Send + Sync + 'static> BufferedSortedProducer<V> {
    fn buffered(&self) -> BufferedSorted<V> {
        BufferedSorted::new(
            self.dictionary.clone(),
            Arc::clone(&self.ords),
            &self.docs_with_field,
        )
    }

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

impl<V> DocValuesProducer for BufferedSortedProducer<V>
where
    V: PackedLongValuesAccess + Send + Sync + fmt::Debug + 'static,
{
    fn get_sorted(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        self.check_field(field)?;
        let buf = self.buffered();
        Ok(Some(match &self.sorted {
            None => Box::new(buf),
            Some(sorted) => Box::new(SortingSorted::new(Box::new(buf), Arc::clone(sorted))),
        }))
    }

    empty_producer_methods!(numeric);
    empty_producer_methods!(binary);
    empty_producer_methods!(sorted_numeric);
    empty_producer_methods!(sorted_set);
    empty_producer_methods!(common);
}

/// Iterates the buffered sorted values of one field.
///
/// Equivalent to `SortedDocValuesWriter.BufferedSortedDocValues`
/// (`SortedDocValuesWriter.java:292-362`): each document's term id comes off
/// the ordinals iterator and is mapped through `ordMap`.
struct BufferedSorted<V: PackedLongValuesAccess> {
    dictionary: Dictionary,
    ord: i32,
    iter: OwnedPackedLongValuesIterator<V>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
}

impl<V: PackedLongValuesAccess> BufferedSorted<V> {
    fn new(
        dictionary: Dictionary,
        doc_to_ord: Arc<V>,
        docs_with_field: &Arc<DocsWithFieldSet>,
    ) -> Self {
        Self {
            dictionary,
            ord: 0,
            iter: OwnedPackedLongValuesIterator::new(doc_to_ord),
            docs_with_field: DocsWithFieldSet::shared_iterator(docs_with_field),
        }
    }
}

impl<V: PackedLongValuesAccess> fmt::Debug for BufferedSorted<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedSorted")
            .field("doc_id", &self.docs_with_field.doc_id())
            .field("ord", &self.ord)
            .finish()
    }
}

impl<V: PackedLongValuesAccess> DocIdSetIterator for BufferedSorted<V> {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.docs_with_field.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            let term_id = to_int_exact(self.iter.next_value())?;
            self.ord = self.dictionary.ord_map[term_id as usize];
        }
        Ok(doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedDocValues does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.docs_with_field.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.docs_with_field.doc_id_run_end()
    }
}

impl<V: PackedLongValuesAccess> DocValuesIterator for BufferedSorted<V> {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedDocValues does not support advanceExact".to_string(),
        ))
    }
}

impl<V: PackedLongValuesAccess> SortedDocValues for BufferedSorted<V> {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ord)
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(self.dictionary.len() as i32)
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.dictionary.lookup_ord(i64::from(ord))
    }
}

/// `Math.toIntExact(long)`.
fn to_int_exact(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| LuceneError::IllegalState("integer overflow".to_string()))
}

/// Iterates the sorted doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `SortedDocValuesWriter.SortingSortedDocValues`
/// (`SortedDocValuesWriter.java:364-403`).
struct SortingSorted {
    in_values: Box<dyn SortedDocValues>,
    ords: Arc<Vec<i32>>,
    doc_id: i32,
}

impl SortingSorted {
    fn new(in_values: Box<dyn SortedDocValues>, ords: Arc<Vec<i32>>) -> Self {
        Self {
            in_values,
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
            // skip missing docs
        }
        Ok(self.doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.in_values.cost()
    }
}

impl DocValuesIterator for SortingSorted {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        // needed in IndexSorter#StringSorter
        self.doc_id = target;
        Ok(self.ords[target as usize] != -1)
    }
}

impl SortedDocValues for SortingSorted {
    fn ord_value(&self) -> Result<i32> {
        Ok(self.ords[self.doc_id as usize])
    }

    fn get_value_count(&self) -> Result<i32> {
        self.in_values.get_value_count()
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.in_values.lookup_ord(ord)
    }
}

// -----------------------------------------------------------------------------
// Single-valued views of the multi-valued kinds
// -----------------------------------------------------------------------------

/// Presents a single-valued numeric iterator as a sorted-numeric one.
///
/// Equivalent to `DocValues.singleton(NumericDocValues)`.
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
            .field("doc_id", &self.inner.doc_id())
            .finish()
    }
}

/// Presents a single-valued sorted iterator as a sorted-set one.
///
/// Equivalent to `DocValues.singleton(SortedDocValues)`: one ordinal per
/// document, served once.
struct SingletonSortedAsSortedSet {
    inner: Box<dyn SortedDocValues>,
}

impl SingletonSortedAsSortedSet {
    fn new(inner: Box<dyn SortedDocValues>) -> Self {
        Self { inner }
    }
}

impl DocIdSetIterator for SingletonSortedAsSortedSet {
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

impl DocValuesIterator for SingletonSortedAsSortedSet {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedSetDocValues for SingletonSortedAsSortedSet {
    fn next_ord(&mut self) -> Result<i64> {
        Ok(i64::from(self.inner.ord_value()?))
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.inner.lookup_ord(to_int_exact(ord)?)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(i64::from(self.inner.get_value_count()?))
    }
}

impl fmt::Debug for SingletonSortedAsSortedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingletonSortedAsSortedSet")
            .field("doc_id", &self.inner.doc_id())
            .finish()
    }
}

/// The producer a multi-valued writer hands over when every document carried
/// exactly one value: the single-valued producer of the matching single-valued
/// writer, viewed through `DocValues.singleton`.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer`s of
/// `SortedNumericDocValuesWriter.flush` (`SortedNumericDocValuesWriter.java:205-218`)
/// and `SortedSetDocValuesWriter.flush` (`SortedSetDocValuesWriter.java:276-288`).
#[derive(Debug, Clone)]
struct SingletonProducer {
    field_number: i32,
    field_name: String,
    single_value_producer: Arc<dyn DocValuesProducer>,
}

impl SingletonProducer {
    fn unsupported(&self, kind: &str) -> LuceneError {
        LuceneError::UnsupportedOperation(format!(
            "this producer only serves the single-valued view of field {} ({}), not its {} values",
            self.field_name, self.field_number, kind
        ))
    }
}

impl DocValuesProducer for SingletonProducer {
    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        let numeric = self.single_value_producer.get_numeric(field)?;
        Ok(numeric.map(|numeric| {
            Box::new(SingletonNumericAsSortedNumeric::new(numeric))
                as Box<dyn SortedNumericDocValues>
        }))
    }

    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        let sorted = self.single_value_producer.get_sorted(field)?;
        Ok(sorted.map(|sorted| {
            Box::new(SingletonSortedAsSortedSet::new(sorted)) as Box<dyn SortedSetDocValues>
        }))
    }

    empty_producer_methods!(numeric);
    empty_producer_methods!(binary);
    empty_producer_methods!(sorted);
    empty_producer_methods!(common);
}

// -----------------------------------------------------------------------------
// SortedNumericDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers any number of numeric values per document, sorted per document.
///
/// Equivalent to `org.apache.lucene.index.SortedNumericDocValuesWriter`
/// (`index/SortedNumericDocValuesWriter.java:36-404`): every value goes to a
/// delta-packed `pending` builder, and a second delta-packed builder of
/// per-document counts is started the first time a document does not carry
/// exactly one value.
#[derive(Debug)]
pub struct SortedNumericDocValuesWriter {
    /// Stream of all values; `None` once built.
    pending: Option<DeltaPackedLongValuesBuilder>,
    /// Count of values per doc, created lazily.
    pending_counts: Option<DeltaPackedLongValuesBuilder>,
    docs_with_field: Arc<DocsWithFieldSet>,
    iw_bytes_used: Arc<AtomicI64>,
    /// Tracks `pending`, `pendingCounts`, the document set and `currentValues`.
    bytes_used: i64,
    field_info: FieldInfo,
    current_doc: i32,
    /// `currentValues`, a `long[]` that starts at 8 and grows by
    /// `ArrayUtil.grow`; only `[..current_upto]` is meaningful.
    current_values: Vec<i64>,
    current_upto: usize,
    final_values: Option<Arc<DeltaPackedLongValues>>,
    final_values_count: Option<Arc<DeltaPackedLongValues>>,
    flushed: bool,
}

impl SortedNumericDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedNumericDocValuesWriter(FieldInfo, Counter)`
    /// (`SortedNumericDocValuesWriter.java:50-60`).
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let pending = delta_packed_builder();
        let docs_with_field = DocsWithFieldSet::new();
        let current_values = vec![0i64; 8];
        let bytes_used = pending.ram_bytes_used()
            + docs_with_field.ram_bytes_used()
            + RamUsageEstimator::size_of_long(&current_values);
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            pending: Some(pending),
            pending_counts: None,
            docs_with_field: Arc::new(docs_with_field),
            iw_bytes_used,
            bytes_used,
            field_info,
            current_doc: -1,
            current_values,
            current_upto: 0,
            final_values: None,
            final_values_count: None,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    fn check_open(&self) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if self.pending.is_none() {
            return Err(already_built());
        }
        Ok(())
    }

    /// Buffers `value` as one more value of `doc_id`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.addValue(int, long)`
    /// (`SortedNumericDocValuesWriter.java:62-70`): the values of a document
    /// accumulate, and moving to a new document commits the previous one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` goes backwards
    /// (Java asserts), and [`LuceneError::IllegalState`] when the field has
    /// already been built or flushed.
    pub fn add_value(&mut self, doc_id: i32, value: i64) -> Result<()> {
        self.check_open()?;
        if doc_id < self.current_doc {
            return Err(backwards_document(
                &self.field_info,
                doc_id,
                self.current_doc,
            ));
        }
        if doc_id != self.current_doc {
            self.finish_current_doc()?;
            self.current_doc = doc_id;
        }
        self.add_one_value(value);
        self.update_bytes_used();
        Ok(())
    }

    /// Buffers every value of `cursor` as the single value of each consecutive
    /// document starting at `first_doc_id`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.addDenseValues(int,
    /// LongValuesCursor)` (`SortedNumericDocValuesWriter.java:72-99`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `first_doc_id` is not
    /// strictly greater than the current document (Java asserts),
    /// [`LuceneError::IllegalState`] when the field has already been built or
    /// flushed, and whatever reading the cursor raises.
    pub fn add_dense_values(
        &mut self,
        first_doc_id: i32,
        cursor: &mut dyn LongValuesCursor,
    ) -> Result<()> {
        let num_values = cursor.size();
        if num_values == 0 {
            return Ok(());
        }
        self.check_open()?;
        if first_doc_id <= self.current_doc {
            return Err(backwards_document(
                &self.field_info,
                first_doc_id,
                self.current_doc,
            ));
        }
        self.finish_current_doc()?;

        // Write values directly to pending — each value is one doc, single-valued.
        // No currentValues[] buffering, no sorting needed.
        self.pending
            .as_mut()
            .ok_or_else(already_built)?
            .add_cursor(cursor)?;
        // If pendingCounts is active (some earlier doc was multi-valued),
        // record count=1 for each dense doc.
        if let Some(counts) = self.pending_counts.as_mut() {
            for _ in 0..num_values {
                counts.add(1)?;
            }
        }
        let end_doc_id = first_doc_id + num_values;
        // Bulk-add consecutive doc-ids
        docs_mut(&mut self.docs_with_field).add_range(first_doc_id, end_doc_id)?;
        // Set currentDoc to last written doc so ordering is maintained.
        // currentUpto stays 0 — nothing buffered.
        self.current_doc = end_doc_id - 1;
        self.update_bytes_used();
        Ok(())
    }

    /// Finalizes the current document: sorts its values, commits them to
    /// `pending` and records the count — starting the counts builder, with one
    /// count per document already recorded, when the first document that does
    /// not carry exactly one value appears.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.finishCurrentDoc()`
    /// (`SortedNumericDocValuesWriter.java:101-128`).
    fn finish_current_doc(&mut self) -> Result<()> {
        if self.current_doc == -1 {
            return Ok(());
        }
        if self.current_upto == 0 {
            return Ok(()); // doc already committed directly (e.g., via addDenseValues)
        }
        let upto = self.current_upto;
        if upto > 1 {
            self.current_values[..upto].sort_unstable();
        }
        let pending = self.pending.as_mut().ok_or_else(already_built)?;
        for &value in &self.current_values[..upto] {
            pending.add(value)?;
        }
        // record the number of values for this doc
        if let Some(counts) = self.pending_counts.as_mut() {
            counts.add(upto as i64)?;
        } else if upto != 1 {
            let mut counts = delta_packed_builder();
            for _ in 0..self.docs_with_field.cardinality() {
                counts.add(1)?;
            }
            counts.add(upto as i64)?;
            self.pending_counts = Some(counts);
        }
        self.current_upto = 0;
        docs_mut(&mut self.docs_with_field).add(self.current_doc)?;
        Ok(())
    }

    /// Equivalent to `SortedNumericDocValuesWriter.addOneValue(long)`
    /// (`SortedNumericDocValuesWriter.java:130-137`).
    fn add_one_value(&mut self, value: i64) {
        if self.current_upto == self.current_values.len() {
            let new_len = ArrayUtil::oversize(self.current_values.len() + 1, 8);
            self.current_values.resize(new_len, 0);
        }
        self.current_values[self.current_upto] = value;
        self.current_upto += 1;
    }

    /// Equivalent to `SortedNumericDocValuesWriter.updateBytesUsed()`
    /// (`SortedNumericDocValuesWriter.java:139-147`).
    fn update_bytes_used(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let new_bytes_used = pending.ram_bytes_used()
            + self
                .pending_counts
                .as_ref()
                .map_or(0, |counts| counts.ram_bytes_used())
            + self.docs_with_field.ram_bytes_used()
            + RamUsageEstimator::size_of_long(&self.current_values);
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// The bytes this writer has charged to the shared counter as of the last
    /// update.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used
    }

    /// Number of documents that have values buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Builds `finalValues` and `finalValuesCount` the first time they are
    /// needed: the current document is finished, then both builders built.
    fn finish(&mut self) -> Result<()> {
        if self.final_values.is_none() {
            self.finish_current_doc()?;
            let pending = self.pending.take().ok_or_else(already_built)?;
            self.final_values = Some(Arc::new(pending.build()?));
            self.final_values_count = match self.pending_counts.take() {
                Some(counts) => Some(Arc::new(counts.build()?)),
                None => None,
            };
        }
        Ok(())
    }

    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.getDocValues()`
    /// (`SortedNumericDocValuesWriter.java:149-158`).
    ///
    /// # Errors
    ///
    /// Propagates whatever finishing the current document or building the
    /// values raises.
    pub fn get_doc_values(&mut self) -> Result<Arc<dyn DocValuesProducer>> {
        self.finish()?;
        Ok(Arc::new(self.producer(None)))
    }

    fn producer(&self, sorted: Option<Arc<LongValues>>) -> BufferedSortedNumericProducer {
        BufferedSortedNumericProducer {
            field_number: self.field_info.number,
            field_name: self.field_info.name.clone(),
            values: Arc::clone(self.final_values.as_ref().expect("INVARIANT: finish() ran")),
            value_counts: self.final_values_count.clone(),
            docs_with_field: Arc::clone(&self.docs_with_field),
            sorted,
        }
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedNumericDocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)` (`SortedNumericDocValuesWriter.java:192-253`):
    /// when every document carries exactly one value the field is handed over
    /// through `NumericDocValuesWriter.getDocValuesProducer` viewed as a
    /// singleton — the same shape Java hands over, and what makes the codec
    /// pick the single-valued file layout; otherwise, with an index sort, the
    /// values are first copied into `LongValues` by new document number.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
    ) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        self.finish()?;
        self.flushed = true;
        let values = Arc::clone(self.final_values.as_ref().expect("INVARIANT: finish() ran"));
        let Some(value_counts) = self.final_values_count.clone() else {
            let single_value_producer = NumericDocValuesWriter::get_doc_values_producer(
                &self.field_info,
                values,
                Arc::clone(&self.docs_with_field),
                sort_map,
            )?;
            let producer = SingletonProducer {
                field_number: self.field_info.number,
                field_name: self.field_info.name.clone(),
                single_value_producer,
            };
            return consumer.add_sorted_numeric_field(&self.field_info, &producer);
        };
        let sorted = match sort_map {
            Some(sort_map) => Some(Arc::new(LongValues::new(
                state.segment_info.max_doc()?,
                sort_map,
                &mut BufferedSortedNumeric::new(values, value_counts, &self.docs_with_field),
                PackedInts::FASTEST,
            )?)),
            None => None,
        };
        let producer = self.producer(sorted);
        consumer.add_sorted_numeric_field(&self.field_info, &producer)
    }
}

impl Drop for SortedNumericDocValuesWriter {
    fn drop(&mut self) {
        self.iw_bytes_used
            .fetch_sub(self.bytes_used, Ordering::AcqRel);
    }
}

/// Serves the buffered sorted-numeric values of one field.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` of
/// `SortedNumericDocValuesWriter.flush` (`SortedNumericDocValuesWriter.java:237-252`)
/// and to `getValues(values, valueCounts, docsWithField)`, which views a field
/// without counts as a singleton.
#[derive(Debug, Clone)]
struct BufferedSortedNumericProducer {
    field_number: i32,
    field_name: String,
    values: Arc<DeltaPackedLongValues>,
    value_counts: Option<Arc<DeltaPackedLongValues>>,
    docs_with_field: Arc<DocsWithFieldSet>,
    sorted: Option<Arc<LongValues>>,
}

producer_guard!(BufferedSortedNumericProducer);

impl BufferedSortedNumericProducer {
    /// Equivalent to the private `SortedNumericDocValuesWriter.getValues(
    /// PackedLongValues, PackedLongValues, DocsWithFieldSet)`
    /// (`SortedNumericDocValuesWriter.java:182-190`).
    fn get_values(&self) -> Box<dyn SortedNumericDocValues> {
        match &self.value_counts {
            None => Box::new(SingletonNumericAsSortedNumeric::new(Box::new(
                BufferedNumeric::new(Arc::clone(&self.values), &self.docs_with_field),
            ))),
            Some(value_counts) => Box::new(BufferedSortedNumeric::new(
                Arc::clone(&self.values),
                Arc::clone(value_counts),
                &self.docs_with_field,
            )),
        }
    }
}

impl DocValuesProducer for BufferedSortedNumericProducer {
    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        self.check_field(field)?;
        let buf = self.get_values();
        Ok(Some(match &self.sorted {
            None => buf,
            Some(sorted) => Box::new(SortingSortedNumeric::new(buf, Arc::clone(sorted))),
        }))
    }

    empty_producer_methods!(numeric);
    empty_producer_methods!(binary);
    empty_producer_methods!(sorted);
    empty_producer_methods!(sorted_set);
    empty_producer_methods!(common);
}

/// The sorted-numeric values of a field laid out by new document number.
///
/// Equivalent to `SortedNumericDocValuesWriter.LongValues`
/// (`SortedNumericDocValuesWriter.java:160-180`): `offsets[newDocID]` is one plus
/// the position of the document's count in `values`, which holds each
/// document's count followed by its values; `0` means no values.
#[derive(Debug)]
struct LongValues {
    offsets: Vec<i64>,
    values: PackedLongValues,
}

impl LongValues {
    fn new(
        max_doc: i32,
        sort_map: &SorterDocMap,
        old_values: &mut dyn SortedNumericDocValues,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        let mut offsets = vec![0i64; max_doc as usize];
        let mut values_builder = packed_builder(acceptable_overhead_ratio);
        let mut offset_index = 1i64; // 0 means the doc has no values
        loop {
            let doc_id = old_values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = sort_map.old_to_new(doc_id);
            let num_values = old_values.doc_value_count()?;
            values_builder.add(i64::from(num_values))?;
            offsets[new_doc_id as usize] = offset_index;
            offset_index += 1;
            for _ in 0..num_values {
                values_builder.add(old_values.next_value()?)?;
                offset_index += 1;
            }
        }
        Ok(Self {
            offsets,
            values: values_builder.build()?,
        })
    }
}

/// Iterates the buffered values of a field some document of which carries
/// other than one value.
///
/// Equivalent to `SortedNumericDocValuesWriter.BufferedSortedNumericDocValues`
/// (`SortedNumericDocValuesWriter.java:255-325`).
struct BufferedSortedNumeric {
    values_iter: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    value_counts_iter: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
    value_count: i32,
    value_upto: i32,
}

impl BufferedSortedNumeric {
    fn new(
        values: Arc<DeltaPackedLongValues>,
        value_counts: Arc<DeltaPackedLongValues>,
        docs_with_field: &Arc<DocsWithFieldSet>,
    ) -> Self {
        Self {
            values_iter: OwnedPackedLongValuesIterator::new(values),
            value_counts_iter: OwnedPackedLongValuesIterator::new(value_counts),
            docs_with_field: DocsWithFieldSet::shared_iterator(docs_with_field),
            value_count: 0,
            value_upto: 0,
        }
    }
}

impl fmt::Debug for BufferedSortedNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedSortedNumeric")
            .field("doc_id", &self.docs_with_field.doc_id())
            .field("value_count", &self.value_count)
            .finish()
    }
}

impl DocIdSetIterator for BufferedSortedNumeric {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        for _ in self.value_upto..self.value_count {
            self.values_iter.next_value();
        }
        let doc_id = self.docs_with_field.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            self.value_count = to_int_exact(self.value_counts_iter.next_value())?;
            self.value_upto = 0;
        }
        Ok(doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedNumericDocValues does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.docs_with_field.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.docs_with_field.doc_id_run_end()
    }
}

impl DocValuesIterator for BufferedSortedNumeric {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedNumericDocValues does not support advanceExact".to_string(),
        ))
    }
}

impl SortedNumericDocValues for BufferedSortedNumeric {
    fn next_value(&mut self) -> Result<i64> {
        self.value_upto += 1;
        Ok(self.values_iter.next_value())
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.value_count)
    }
}

/// Iterates the sorted-numeric doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `SortedNumericDocValuesWriter.SortingSortedNumericDocValues`
/// (`SortedNumericDocValuesWriter.java:327-389`).
struct SortingSortedNumeric {
    in_values: Box<dyn SortedNumericDocValues>,
    values: Arc<LongValues>,
    doc_id: i32,
    upto: i64,
    num_values: i32,
}

impl SortingSortedNumeric {
    fn new(in_values: Box<dyn SortedNumericDocValues>, values: Arc<LongValues>) -> Self {
        Self {
            in_values,
            values,
            doc_id: -1,
            upto: 0,
            num_values: -1,
        }
    }
}

impl fmt::Debug for SortingSortedNumeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingSortedNumeric")
            .field("doc_id", &self.doc_id)
            .finish()
    }
}

impl DocIdSetIterator for SortingSortedNumeric {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            self.doc_id += 1;
            if self.doc_id as usize >= self.values.offsets.len() {
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            if self.values.offsets[self.doc_id as usize] > 0 {
                break;
            }
        }
        self.upto = self.values.offsets[self.doc_id as usize];
        self.num_values = to_int_exact(self.values.values.get(self.upto - 1))?;
        Ok(self.doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.in_values.cost()
    }
}

impl DocValuesIterator for SortingSortedNumeric {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        self.upto = self.values.offsets[target as usize];
        if self.upto > 0 {
            self.num_values = to_int_exact(self.values.values.get(self.upto - 1))?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl SortedNumericDocValues for SortingSortedNumeric {
    fn next_value(&mut self) -> Result<i64> {
        let value = self.values.values.get(self.upto);
        self.upto += 1;
        Ok(value)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.num_values)
    }
}

// -----------------------------------------------------------------------------
// SortedSetDocValuesWriter
// -----------------------------------------------------------------------------

/// Buffers any number of distinct binary values per document.
///
/// Equivalent to `org.apache.lucene.index.SortedSetDocValuesWriter`
/// (`index/SortedSetDocValuesWriter.java:46-524`): values are interned into a
/// [`BytesRefHash`] over the chain's shared pool, the term ids of the current
/// document are sorted and deduplicated when the document is committed into
/// a packed `pending` builder, and a delta-packed builder of per-document
/// counts is started the first time a document does not carry exactly one
/// distinct value.
#[derive(Debug)]
pub struct SortedSetDocValuesWriter {
    hash: Arc<BytesRefHash>,
    /// Stream of all term ids, a `PackedLongValues.packedBuilder(COMPACT)`;
    /// `None` once built.
    pending: Option<PackedLongValuesBuilder>,
    /// Term ids per doc, created lazily.
    pending_counts: Option<DeltaPackedLongValuesBuilder>,
    docs_with_field: Arc<DocsWithFieldSet>,
    iw_bytes_used: Arc<AtomicI64>,
    /// Tracks `pending`, `pendingCounts`, the document set and `currentValues`.
    bytes_used: i64,
    /// What the writer charged the counter directly (`2 * Integer.BYTES` per
    /// distinct value, and the growth of `currentValues`); kept so that `Drop`
    /// can give it back.
    direct_charges: i64,
    field_info: FieldInfo,
    current_doc: i32,
    /// `currentValues`, an `int[]` that starts at 8 and grows by
    /// `ArrayUtil.grow`; only `[..current_upto]` is meaningful.
    current_values: Vec<i32>,
    current_upto: usize,
    max_count: i32,
    final_ords: Option<Arc<PackedLongValues>>,
    final_ord_counts: Option<Arc<DeltaPackedLongValues>>,
    dictionary: Option<Dictionary>,
    flushed: bool,
}

impl SortedSetDocValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new SortedSetDocValuesWriter(FieldInfo, Counter,
    /// ByteBlockPool, SharedIndexingScratch)` (`SortedSetDocValuesWriter.java:65-81`).
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let hash = Arc::new(BytesRefHash::with_direct_bytes_start(
            DEFAULT_CAPACITY,
            Arc::clone(&iw_bytes_used),
        ));
        let pending = packed_builder(PackedInts::COMPACT);
        let docs_with_field = DocsWithFieldSet::new();
        let current_values = vec![0i32; 8];
        let bytes_used = pending.ram_bytes_used()
            + docs_with_field.ram_bytes_used()
            + RamUsageEstimator::size_of_int(&current_values);
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            hash,
            pending: Some(pending),
            pending_counts: None,
            docs_with_field: Arc::new(docs_with_field),
            iw_bytes_used,
            bytes_used,
            direct_charges: 0,
            field_info,
            current_doc: -1,
            current_values,
            current_upto: 0,
            max_count: 0,
            final_ords: None,
            final_ord_counts: None,
            dictionary: None,
            flushed: false,
        }
    }

    /// Returns the field these values belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    fn check_open(&self) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        if self.pending.is_none() {
            return Err(already_built());
        }
        Ok(())
    }

    /// Charges `bytes` straight to the counter, as Java's
    /// `iwBytesUsed.addAndGet` does.
    fn charge_directly(&mut self, bytes: i64) {
        self.iw_bytes_used.fetch_add(bytes, Ordering::AcqRel);
        self.direct_charges += bytes;
    }

    /// Buffers `value` as one more value of `doc_id`.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.addValue(int, BytesRef)`
    /// (`SortedSetDocValuesWriter.java:83-105`) with the null check performed
    /// by the caller: repeated values of one document accumulate and are
    /// deduplicated on commit.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` goes backwards
    /// (Java asserts) or the value is too long, and
    /// [`LuceneError::IllegalState`] when the field has already been built or
    /// flushed.
    pub fn add_value(
        &mut self,
        doc_id: i32,
        value: &[u8],
        pool: &mut Arc<ByteBlockPool>,
    ) -> Result<()> {
        self.check_open()?;
        if doc_id < self.current_doc {
            return Err(backwards_document(
                &self.field_info,
                doc_id,
                self.current_doc,
            ));
        }
        if value.len() > SORTED_MAX_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "DocValuesField \"{}\" is too large, must be <= {SORTED_MAX_LENGTH}",
                self.field_info.name
            )));
        }
        if doc_id != self.current_doc {
            self.finish_current_doc()?;
            self.current_doc = doc_id;
        }
        self.add_one_value(value, pool)?;
        self.update_bytes_used();
        Ok(())
    }

    /// Appends `term_id` to the current document's values, growing
    /// `currentValues` as Java does — including the direct charge of the new
    /// slots, which Java makes on top of `updateBytesUsed`'s `sizeOf`.
    fn push_current_value(&mut self, term_id: i32) {
        if self.current_upto == self.current_values.len() {
            let new_len = ArrayUtil::oversize(self.current_values.len() + 1, 4);
            self.current_values.resize(new_len, 0);
            self.charge_directly((new_len - self.current_upto) as i64 * i64::from(i32::BITS / 8));
        }
        self.current_values[self.current_upto] = term_id;
        self.current_upto += 1;
    }

    /// Buffers one dictionary-encoded value per tuple the cursor names, at
    /// `base_doc_id` plus the batch-local id.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.addOrdinalTuples(int, List,
    /// OrdinalsTupleCursor)` (`SortedSetDocValuesWriter.java:107-153`). Several
    /// tuples may name the same document; their values accumulate and are
    /// deduplicated when the document is committed.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a document goes backwards
    /// or an ordinal is out of range, [`LuceneError::IllegalState`] when the
    /// field has already been built or flushed, and whatever reading the
    /// cursor raises.
    pub fn add_ordinal_tuples(
        &mut self,
        base_doc_id: i32,
        dictionary: &[BytesRef],
        cursor: &mut dyn OrdinalsTupleCursor,
        pool: &mut Arc<ByteBlockPool>,
        scratch: &mut SharedIndexingScratch,
    ) -> Result<()> {
        self.check_open()?;
        let mut owned = Vec::new();
        let ord_to_hash = ord_to_hash_table(dictionary.len(), scratch, &mut owned);
        loop {
            let batch_doc_id = cursor.next_doc()?;
            if batch_doc_id == NO_MORE_DOCS {
                break;
            }
            let doc_id = base_doc_id + batch_doc_id;
            if doc_id < self.current_doc {
                return Err(backwards_document(
                    &self.field_info,
                    doc_id,
                    self.current_doc,
                ));
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
                hash_id = hash_mut(&mut self.hash)?
                    .add(pool_mut(pool)?, dictionary[ord as usize].slice())?;
                if hash_id < 0 {
                    hash_id = -hash_id - 1;
                } else {
                    self.charge_directly(2 * i64::from(i32::BITS / 8));
                }
                ord_to_hash[ord as usize] = hash_id;
            }
            self.push_current_value(hash_id);
        }
        self.update_bytes_used();
        Ok(())
    }

    /// Finalizes the current document: sorts and deduplicates its term ids
    /// into `pending` and records the distinct count — starting the counts
    /// builder, with one count per document already recorded, when the first
    /// document that does not carry exactly one distinct value appears.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.finishCurrentDoc()`
    /// (`SortedSetDocValuesWriter.java:155-189`).
    fn finish_current_doc(&mut self) -> Result<()> {
        if self.current_doc == -1 {
            return Ok(());
        }
        let upto = self.current_upto;
        if upto > 1 {
            self.current_values[..upto].sort_unstable();
        }
        let pending = self.pending.as_mut().ok_or_else(already_built)?;
        let mut last_value = -1i32;
        let mut count = 0i32;
        for &term_id in &self.current_values[..upto] {
            // if it's not a duplicate
            if term_id != last_value {
                pending.add(i64::from(term_id))?; // record the term id
                count += 1;
            }
            last_value = term_id;
        }
        // record the number of unique term ids for this doc
        if let Some(counts) = self.pending_counts.as_mut() {
            counts.add(i64::from(count))?;
        } else if count != 1 {
            let mut counts = delta_packed_builder();
            for _ in 0..self.docs_with_field.cardinality() {
                counts.add(1)?;
            }
            counts.add(i64::from(count))?;
            self.pending_counts = Some(counts);
        }
        self.max_count = self.max_count.max(count);
        self.current_upto = 0;
        docs_mut(&mut self.docs_with_field).add(self.current_doc)?;
        Ok(())
    }

    /// Equivalent to `SortedSetDocValuesWriter.addOneValue(BytesRef)`
    /// (`SortedSetDocValuesWriter.java:191-210`).
    fn add_one_value(&mut self, value: &[u8], pool: &mut Arc<ByteBlockPool>) -> Result<()> {
        let mut term_id = hash_mut(&mut self.hash)?.add(pool_mut(pool)?, value)?;
        if term_id < 0 {
            term_id = -term_id - 1;
        } else {
            // reserve additional space for each unique value:
            // 1. when indexing, when hash is 50% full, rehash() suddenly needs 2*size ints.
            // 2. when flushing, we need 1 int per value (slot in the ordMap).
            self.charge_directly(2 * i64::from(i32::BITS / 8));
        }
        self.push_current_value(term_id);
        Ok(())
    }

    /// Equivalent to `SortedSetDocValuesWriter.updateBytesUsed()`
    /// (`SortedSetDocValuesWriter.java:212-220`).
    fn update_bytes_used(&mut self) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let new_bytes_used = pending.ram_bytes_used()
            + self
                .pending_counts
                .as_ref()
                .map_or(0, |counts| counts.ram_bytes_used())
            + self.docs_with_field.ram_bytes_used()
            + RamUsageEstimator::size_of_int(&self.current_values);
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// The bytes this writer has charged to the shared counter, other than the
    /// hash's own slot array.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used + self.direct_charges
    }

    /// Number of documents that have values buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Builds the final term ids, counts and dictionary, once.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.finish()`
    /// (`SortedSetDocValuesWriter.java:222-236`).
    fn finish(&mut self, pool: &Arc<ByteBlockPool>) -> Result<()> {
        if self.final_ords.is_none() {
            self.finish_current_doc()?;
            let pending = self.pending.take().ok_or_else(already_built)?;
            self.final_ords = Some(Arc::new(pending.build()?));
            self.final_ord_counts = match self.pending_counts.take() {
                Some(counts) => Some(Arc::new(counts.build()?)),
                None => None,
            };
            self.dictionary = Some(Dictionary::build(&self.hash, pool));
        }
        Ok(())
    }

    fn producer(&self, doc_ords: Option<Arc<DocOrds>>) -> BufferedSortedSetProducer {
        let dictionary = self.dictionary.as_ref().expect("INVARIANT: finish() ran");
        BufferedSortedSetProducer {
            field_number: self.field_info.number,
            field_name: self.field_info.name.clone(),
            dictionary: dictionary.clone(),
            ords: Arc::clone(self.final_ords.as_ref().expect("INVARIANT: finish() ran")),
            ord_counts: Arc::clone(
                self.final_ord_counts
                    .as_ref()
                    .expect("INVARIANT: the multi-valued producer is built with counts"),
            ),
            max_count: self.max_count,
            docs_with_field: Arc::clone(&self.docs_with_field),
            doc_ords,
        }
    }

    /// The single-valued producer `SortedDocValuesWriter.getDocValuesProducer`
    /// builds over this writer's term ids, for a field without counts.
    fn single_value_producer(
        &self,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        SortedDocValuesWriter::get_doc_values_producer(
            &self.field_info,
            self.dictionary.as_ref().expect("INVARIANT: finish() ran"),
            Arc::clone(self.final_ords.as_ref().expect("INVARIANT: finish() ran")),
            Arc::clone(&self.docs_with_field),
            sort_map,
        )
    }

    /// Returns a read-only view over the values buffered so far.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.getDocValues()`
    /// (`SortedSetDocValuesWriter.java:238-243`), which finishes the current
    /// document first.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the term ids raises.
    pub fn get_doc_values(
        &mut self,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<Arc<dyn DocValuesProducer>> {
        self.finish(pool)?;
        if self.final_ord_counts.is_none() {
            let single_value_producer = self.single_value_producer(None)?;
            return Ok(Arc::new(SingletonProducer {
                field_number: self.field_info.number,
                field_name: self.field_info.name.clone(),
                single_value_producer,
            }));
        }
        Ok(Arc::new(self.producer(None)))
    }

    /// Hands the buffered values to `consumer`.
    ///
    /// Equivalent to `SortedSetDocValuesWriter.flush(SegmentWriteState,
    /// Sorter.DocMap, DocValuesConsumer)` (`SortedSetDocValuesWriter.java:262-320`):
    /// a field without counts is handed over through
    /// `SortedDocValuesWriter.getDocValuesProducer` viewed as a singleton;
    /// otherwise, with an index sort, the ordinals are first copied into
    /// `DocOrds` by new document number.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed; otherwise propagates whatever the consumer raises.
    pub fn flush(
        &mut self,
        state: &SegmentWriteState<'_>,
        sort_map: Option<&SorterDocMap>,
        consumer: &mut dyn DocValuesConsumer,
        pool: &Arc<ByteBlockPool>,
    ) -> Result<()> {
        if self.flushed {
            return Err(already_flushed(&self.field_info));
        }
        self.finish(pool)?;
        self.flushed = true;
        if self.final_ord_counts.is_none() {
            let single_value_producer = self.single_value_producer(sort_map)?;
            let producer = SingletonProducer {
                field_number: self.field_info.number,
                field_name: self.field_info.name.clone(),
                single_value_producer,
            };
            return consumer.add_sorted_set_field(&self.field_info, &producer);
        }
        let doc_ords = match sort_map {
            Some(sort_map) => {
                let unsorted = self.producer(None);
                Some(Arc::new(DocOrds::new(
                    state.segment_info.max_doc()?,
                    sort_map,
                    &mut unsorted.buffered(),
                    PackedInts::FASTEST,
                    PackedInts::bits_required(i64::from(self.max_count))?,
                )?))
            }
            None => None,
        };
        let producer = self.producer(doc_ords);
        consumer.add_sorted_set_field(&self.field_info, &producer)
    }
}

impl Drop for SortedSetDocValuesWriter {
    /// Gives back the delta-tracked buffers, the direct charges and the hash's
    /// slot array; the pool's blocks belong to the chain.
    fn drop(&mut self) {
        self.iw_bytes_used.fetch_sub(
            self.bytes_used + self.direct_charges + self.hash.charged_bytes(),
            Ordering::AcqRel,
        );
    }
}

/// Serves the buffered sorted-set values of a field some document of which
/// carries other than one distinct value.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` of
/// `SortedSetDocValuesWriter.flush` (`SortedSetDocValuesWriter.java:303-319`).
#[derive(Debug, Clone)]
struct BufferedSortedSetProducer {
    field_number: i32,
    field_name: String,
    dictionary: Dictionary,
    ords: Arc<PackedLongValues>,
    ord_counts: Arc<DeltaPackedLongValues>,
    max_count: i32,
    docs_with_field: Arc<DocsWithFieldSet>,
    doc_ords: Option<Arc<DocOrds>>,
}

producer_guard!(BufferedSortedSetProducer);

impl BufferedSortedSetProducer {
    /// Equivalent to the private `SortedSetDocValuesWriter.getValues(...)`
    /// with counts (`SortedSetDocValuesWriter.java:245-260`).
    fn buffered(&self) -> BufferedSortedSet {
        BufferedSortedSet {
            dictionary: self.dictionary.clone(),
            ords_iter: OwnedPackedLongValuesIterator::new(Arc::clone(&self.ords)),
            ord_counts_iter: OwnedPackedLongValuesIterator::new(Arc::clone(&self.ord_counts)),
            docs_with_field: DocsWithFieldSet::shared_iterator(&self.docs_with_field),
            current_doc: vec![0i32; self.max_count.max(0) as usize],
            ord_count: 0,
            ord_upto: 0,
        }
    }
}

impl DocValuesProducer for BufferedSortedSetProducer {
    fn get_sorted_set(&self, field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        self.check_field(field)?;
        let buf = Box::new(self.buffered());
        Ok(Some(match &self.doc_ords {
            None => buf,
            Some(doc_ords) => Box::new(SortingSortedSet::new(buf, Arc::clone(doc_ords))),
        }))
    }

    empty_producer_methods!(numeric);
    empty_producer_methods!(binary);
    empty_producer_methods!(sorted);
    empty_producer_methods!(sorted_numeric);
    empty_producer_methods!(common);
}

/// Iterates the buffered values of a sorted-set field.
///
/// Equivalent to `SortedSetDocValuesWriter.BufferedSortedSetDocValues`
/// (`SortedSetDocValuesWriter.java:322-418`): each document's term ids are
/// mapped through `ordMap` and sorted as the document is reached.
struct BufferedSortedSet {
    dictionary: Dictionary,
    ords_iter: OwnedPackedLongValuesIterator<PackedLongValues>,
    ord_counts_iter: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
    current_doc: Vec<i32>,
    ord_count: i32,
    ord_upto: usize,
}

impl fmt::Debug for BufferedSortedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedSortedSet")
            .field("doc_id", &self.docs_with_field.doc_id())
            .field("ord_count", &self.ord_count)
            .finish()
    }
}

impl DocIdSetIterator for BufferedSortedSet {
    fn doc_id(&self) -> i32 {
        self.docs_with_field.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc_id = self.docs_with_field.next_doc()?;
        if doc_id != NO_MORE_DOCS {
            self.ord_count = self.ord_counts_iter.next_value() as i32;
            debug_assert!(self.ord_count > 0);
            for i in 0..self.ord_count as usize {
                let term_id = to_int_exact(self.ords_iter.next_value())?;
                self.current_doc[i] = self.dictionary.ord_map[term_id as usize];
            }
            self.current_doc[..self.ord_count as usize].sort_unstable();
            self.ord_upto = 0;
        }
        Ok(doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedSetDocValues does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.docs_with_field.into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.docs_with_field.doc_id_run_end()
    }
}

impl DocValuesIterator for BufferedSortedSet {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedSortedSetDocValues does not support advanceExact".to_string(),
        ))
    }
}

impl SortedSetDocValues for BufferedSortedSet {
    fn next_ord(&mut self) -> Result<i64> {
        let ord = self.current_doc[self.ord_upto];
        self.ord_upto += 1;
        Ok(i64::from(ord))
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(self.ord_count)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.dictionary.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(self.dictionary.len() as i64)
    }
}

/// The ordinals of a sorted-set field laid out by new document number.
///
/// Equivalent to `SortedSetDocValuesWriter.DocOrds`
/// (`SortedSetDocValuesWriter.java:487-523`): `offsets[newDocID]` is one plus
/// the position of the document's first ordinal in `ords`, `0` meaning no
/// values, and `docValueCounts` holds each new document's count.
#[derive(Debug)]
struct DocOrds {
    offsets: Vec<i64>,
    ords: PackedLongValues,
    doc_value_counts: GrowableWriter,
}

impl DocOrds {
    fn new(
        max_doc: i32,
        sort_map: &SorterDocMap,
        old_values: &mut dyn SortedSetDocValues,
        acceptable_overhead_ratio: f32,
        bits_per_value: i32,
    ) -> Result<Self> {
        let mut offsets = vec![0i64; max_doc as usize];
        let mut builder = packed_builder(acceptable_overhead_ratio);
        let mut doc_value_counts =
            GrowableWriter::new(bits_per_value, max_doc, acceptable_overhead_ratio)?;
        let mut ord_offset = 1i64;
        loop {
            let doc_id = old_values.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_doc_id = sort_map.old_to_new(doc_id);
            let start_offset = ord_offset;
            let doc_value_count = old_values.doc_value_count()?;
            ord_offset += i64::from(doc_value_count);
            for _ in 0..doc_value_count {
                builder.add(old_values.next_ord()?)?;
            }
            doc_value_counts.set(new_doc_id, ord_offset - start_offset);
            if start_offset != ord_offset {
                // do we have any values?
                offsets[new_doc_id as usize] = start_offset;
            }
        }
        Ok(Self {
            offsets,
            ords: builder.build()?,
            doc_value_counts,
        })
    }
}

/// Iterates the sorted-set doc values of a field after an index sort has
/// reordered the segment.
///
/// Equivalent to `SortedSetDocValuesWriter.SortingSortedSetDocValues`
/// (`SortedSetDocValuesWriter.java:420-485`).
struct SortingSortedSet {
    in_values: Box<dyn SortedSetDocValues>,
    ords: Arc<DocOrds>,
    doc_id: i32,
    ord_upto: i64,
    count: i32,
}

impl SortingSortedSet {
    fn new(in_values: Box<dyn SortedSetDocValues>, ords: Arc<DocOrds>) -> Self {
        Self {
            in_values,
            ords,
            doc_id: -1,
            ord_upto: 0,
            count: 0,
        }
    }

    /// Equivalent to `SortingSortedSetDocValues.initCount()`.
    fn init_count(&mut self) {
        debug_assert!(self.doc_id >= 0);
        self.ord_upto = self.ords.offsets[self.doc_id as usize] - 1;
        self.count = self.ords.doc_value_counts.get(self.doc_id) as i32;
    }
}

impl fmt::Debug for SortingSortedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SortingSortedSet")
            .field("doc_id", &self.doc_id)
            .finish()
    }
}

impl DocIdSetIterator for SortingSortedSet {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            self.doc_id += 1;
            if self.doc_id as usize == self.ords.offsets.len() {
                self.doc_id = NO_MORE_DOCS;
                return Ok(self.doc_id);
            }
            if self.ords.offsets[self.doc_id as usize] > 0 {
                break;
            }
        }
        self.init_count();
        Ok(self.doc_id)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "use nextDoc instead".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.in_values.cost()
    }
}

impl DocValuesIterator for SortingSortedSet {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        // needed in IndexSorter#StringSorter
        self.doc_id = target;
        self.init_count();
        Ok(self.ords.offsets[self.doc_id as usize] > 0)
    }
}

impl SortedSetDocValues for SortingSortedSet {
    fn next_ord(&mut self) -> Result<i64> {
        let ord = self.ords.ords.get(self.ord_upto);
        self.ord_upto += 1;
        Ok(ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        debug_assert!(self.doc_id >= 0);
        Ok(self.count)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.in_values.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.in_values.get_value_count()
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

    /// The indexing chain's shared `docValuesBytePool`, standing alone.
    fn test_pool() -> Arc<ByteBlockPool> {
        Arc::new(ByteBlockPool::new(counter()))
    }

    /// Runs `f` with a write state for an unsorted segment of `max_doc`
    /// documents, which is all a doc-values writer reads off the state.
    fn with_state<R>(max_doc: i32, f: impl FnOnce(&SegmentWriteState<'_>) -> R) -> R {
        let directory: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::RamDirectory::default());
        let segment_info = crate::index::SegmentInfo::new(
            Arc::clone(&directory),
            crate::util::Version::LATEST,
            Some(crate::util::Version::LATEST),
            "_0".to_string(),
            max_doc,
            false,
            false,
            Arc::new(crate::codecs::Lucene104Codec::new()),
            std::collections::HashMap::new(),
            [7u8; 16],
            std::collections::HashMap::new(),
            Default::default(),
        )
        .expect("segment info");
        let field_infos = crate::index::FieldInfos::default();
        let state = SegmentWriteState::new(
            &crate::util::NoOutputInfoStream,
            directory,
            &segment_info,
            &field_infos,
            Box::leak(Box::new(crate::codecs::stub::BufferedUpdates::default())),
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        f(&state)
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
        assert_eq!(consumer.fields[0].1, Recorded::Numeric(vec![(4, 1)]));
    }

    #[test]
    fn flushing_a_numeric_field_twice_is_refused() {
        let mut writer =
            NumericDocValuesWriter::new(field("count", 0, DocValuesType::NUMERIC), counter());
        writer.add_value(0, 1).unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
        let error = with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap_err();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
            long_value: Option<i64>,
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
                self.long_value = Some(numeric.long_value()?);
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
        assert!(consumer.advance.unwrap().contains("advance"));
        assert!(consumer.advance_exact.unwrap().contains("advanceExact"));
        assert_eq!(consumer.after, vec![NO_MORE_DOCS; 4]);
        // Java's `BufferedNumericDocValues.longValue()` returns the last value
        // read, whatever the position.
        assert_eq!(consumer.long_value, Some(9));
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        let mut pool = test_pool();
        // b"\xff" sorts *after* b"\x01" in unsigned byte order, although a
        // signed comparison would put it first — this is exactly the ordering
        // BytesRefHash.sort produces, and what Lucene's dictionary guarantees.
        let mut writer =
            SortedDocValuesWriter::new(field("tag", 0, DocValuesType::SORTED), counter());
        writer.add_value(0, b"\xff", &mut pool).unwrap();
        writer.add_value(1, b"abc", &mut pool).unwrap();
        writer.add_value(2, b"\x01", &mut pool).unwrap();
        writer.add_value(4, b"abc", &mut pool).unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        let mut pool = test_pool();
        let mut writer =
            SortedDocValuesWriter::new(field("tag", 0, DocValuesType::SORTED), counter());
        writer.add_value(3, b"one", &mut pool).unwrap();
        let error = writer.add_value(3, b"two", &mut pool).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("appears more than once") && m.contains("only one value is allowed")),
            "unexpected error: {error:?}"
        );
        // A doc id that goes backwards is the same failure.
        assert!(writer.add_value(2, b"two", &mut pool).is_err());
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        // Java only asserts `docID >= currentDoc`; with assertions off the
        // backwards id is refused later, when `finishCurrentDoc` adds it to the
        // document set, which throws `IllegalArgumentException("Out of order
        // doc ids: last=..., next=...")` (`DocsWithFieldSet.java:46-50`). This
        // port refuses it at the call.
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
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
        let mut pool = test_pool();
        // Two documents: the first carries a duplicate and an unordered pair,
        // the second reuses one value. The dictionary must be the distinct
        // value set in unsigned byte order, each document's ords deduplicated,
        // sorted and mapped into that dictionary.
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(0, b"b", &mut pool).unwrap();
        writer.add_value(0, b"a", &mut pool).unwrap();
        writer.add_value(0, b"b", &mut pool).unwrap();
        writer.add_value(2, b"c", &mut pool).unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        let mut pool = test_pool();
        // Same contract as the sorted-numeric singleton: one distinct value per
        // document makes the producer serve the singleton view, and the codec
        // then picks the single-valued layout.
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(0, b"x", &mut pool).unwrap();
        writer.add_value(1, b"y", &mut pool).unwrap();
        writer.add_value(3, b"x", &mut pool).unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        let mut pool = test_pool();
        let mut writer =
            SortedSetDocValuesWriter::new(field("tags", 0, DocValuesType::SORTED_SET), counter());
        writer.add_value(3, b"one", &mut pool).unwrap();
        writer.add_value(3, b"two", &mut pool).unwrap();
        let error = writer.add_value(2, b"three", &mut pool).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("increasing document order")),
            "unexpected error: {error:?}"
        );
        // The rejected document was not committed; the earlier document keeps
        // both of its distinct values.
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        let mut pool = test_pool();
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
                &mut pool,
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
        assert_eq!(consumer.fields[0].1, Recorded::Numeric(vec![(0, 42)]));
    }

    #[test]
    fn numeric_flavours_widen_like_java_number_long_value() {
        let mut pool = test_pool();
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
                    &mut pool,
                )
                .unwrap();
            let mut consumer = RecordingConsumer::default();
            with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
            assert_eq!(
                consumer.fields[0].1,
                Recorded::Numeric(vec![(0, expected)]),
                "wrong longValue conversion for {value:?}"
            );
        }
    }

    #[test]
    fn a_missing_value_is_refused_before_the_writer_sees_it() {
        let mut pool = test_pool();
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
            let error = writer.add_value(0, &field_value, &mut pool).unwrap_err();
            assert!(
                matches!(error, LuceneError::IllegalArgument(ref m)
                    if m.contains("null value not allowed")),
                "unexpected error for {doc_values_type:?}: {error:?}"
            );
        }
    }

    #[test]
    fn sorted_numeric_dispatch_converts_like_the_numeric_branch() {
        let mut pool = test_pool();
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
                &mut pool,
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
        assert_eq!(
            consumer.fields[0].1,
            Recorded::SortedNumeric(vec![(0, vec![9])])
        );
    }

    #[test]
    fn the_dispatch_routes_each_type_to_its_recorded_kind() {
        let mut pool = test_pool();
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
                &mut pool,
            )
            .unwrap();
        let mut binary = DocValuesWriter::new(field("b", 1, DocValuesType::BINARY), counter());
        binary
            .add_value(
                0,
                &TestDocValuesField::binary("b", DocValuesType::BINARY, b"bytes"),
                &mut pool,
            )
            .unwrap();
        let mut sorted = DocValuesWriter::new(field("s", 2, DocValuesType::SORTED), counter());
        sorted
            .add_value(
                0,
                &TestDocValuesField::binary("s", DocValuesType::SORTED, b"sorted"),
                &mut pool,
            )
            .unwrap();
        let mut sorted_set =
            DocValuesWriter::new(field("ss", 3, DocValuesType::SORTED_SET), counter());
        sorted_set
            .add_value(
                0,
                &TestDocValuesField::binary("ss", DocValuesType::SORTED_SET, b"set"),
                &mut pool,
            )
            .unwrap();

        let mut consumer = RecordingConsumer::default();
        for writer in [&mut numeric, &mut binary, &mut sorted, &mut sorted_set] {
            with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
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
        let mut pool = test_pool();
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
                &mut pool,
            )
            .unwrap();
        let mut consumer = RecordingConsumer::default();
        with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap();
        let error =
            with_state(0, |state| writer.flush(state, None, &mut consumer, &pool)).unwrap_err();
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
                ),
                &mut pool,
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
                // Distinct values: a constant run packs into zero-bit pages,
                // which cost Java's builder nothing more.
                writer.add_value(doc, i64::from(doc) * 1_000_003).unwrap();
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
    fn the_sorted_writer_refunds_its_buffers_and_the_hash_at_drop() {
        let mut pool = test_pool();
        // The sorted writers charge their BytesRefHash and the per-value
        // headroom to the shared counter directly, outside the delta-tracked
        // footprint; the Drop impl must refund those too, not just the plain
        // buffers. The pool belongs to the indexing chain.
        let counter = counter();
        {
            let mut writer = SortedDocValuesWriter::new(
                field("s", 0, DocValuesType::SORTED),
                Arc::clone(&counter),
            );
            for doc in 0..500 {
                writer
                    .add_value(doc, format!("value-{doc}").as_bytes(), &mut pool)
                    .unwrap();
            }
            assert!(
                counter.load(Ordering::Acquire) > 0,
                "the sorted writer must charge its hash and buffers"
            );
        }
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "dropping the sorted writer must give every byte back"
        );
    }

    #[test]
    fn flushing_keeps_the_charge_as_lucene_does() {
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
        with_state(0, |state| writer.flush(state, None, &mut consumer)).unwrap();
        let after = counter.load(Ordering::Acquire);
        // Java's `flush` builds the values without updating the counter; the
        // charge goes when the indexing chain's counter is discarded.
        assert_eq!(after, full, "flushing does not change the charge");
        assert_eq!(
            consumer.fields[0].1,
            Recorded::Numeric((0..1_000).map(|d| (d, 1)).collect())
        );
    }

    #[test]
    fn two_sorted_writers_share_one_counter_without_losing_bytes() {
        let mut pool = test_pool();
        // Two sorted writers intern into one shared pool, as the indexing
        // chain's `docValuesBytePool` is shared, and each owns a hash; the
        // shared counter must account for both, and dropping one must not
        // disturb the other's charges.
        let counter = counter();
        let mut left =
            SortedDocValuesWriter::new(field("l", 0, DocValuesType::SORTED), Arc::clone(&counter));
        let mut right = SortedSetDocValuesWriter::new(
            field("r", 1, DocValuesType::SORTED_SET),
            Arc::clone(&counter),
        );
        for doc in 0..200 {
            left.add_value(doc, format!("left-{doc}").as_bytes(), &mut pool)
                .unwrap();
            right
                .add_value(doc, format!("right-{doc}").as_bytes(), &mut pool)
                .unwrap();
        }
        let both = counter.load(Ordering::Acquire);
        assert!(
            both > left.ram_bytes_used() + right.ram_bytes_used(),
            "the hashes must be part of the total"
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
