//! Per-field buffer of index-time normalization values.
//!
//! Equivalent to `org.apache.lucene.index.NormValuesWriter`.
//!
//! # Responsibility
//!
//! [`NormValuesWriter`] owns exactly one thing: the norms of **one field** of
//! the segment currently being built. It buffers one `i64` per document that
//! carried the field, in increasing document order, and hands them to a
//! [`NormsConsumer`] when the segment flushes. It computes nothing — the norm
//! value arrives already encoded, from
//! [`Similarity::compute_norm`](crate::search::Similarity::compute_norm) — and
//! it writes nothing itself; the codec owns the file format.
//!
//! # Lifecycle
//!
//! The Java lifecycle, from `IndexingChain`, is:
//!
//! 1. **Creation.** `PerField.setInvertState()` creates the writer the first
//!    time an indexed field is seen in the segment, and *only* when
//!    `fieldInfo.omitsNorms() == false` (`IndexingChain.java:1837-1841`). The
//!    comment there is load-bearing: *"Even if no documents actually succeed in
//!    setting a norm, we still write norms for this segment"*.
//! 2. **One value per document.** `PerField.finish(docID)` runs once per
//!    document in which the field appeared, after every value of a multi-valued
//!    field has been inverted (`IndexingChain.java:1853-1869`). It adds a norm
//!    of `0` when the field appeared but produced no tokens, and otherwise the
//!    similarity's value — refusing a similarity that returns `0` for a
//!    non-empty field. Documents that never mention the field get no value at
//!    all, which is what makes a norms field sparse.
//! 3. **Flush.** `IndexingChain.writeNorms` walks the field infos and, for each
//!    field with `omitsNorms() == false && indexOptions != NONE`, calls
//!    `finish(maxDoc)` and then `flush(...)` (`IndexingChain.java:503-532`).
//!
//! # Invariants
//!
//! * Document ids are strictly increasing. A repeat is the "appears more than
//!   once in this document" error, which is how Lucene catches a chain that
//!   called `finish` twice for one document.
//! * `pending.len() == docs_with_field.cardinality()` at all times: the two
//!   buffers are appended together and read back in lockstep.
//! * `finish(max_doc)` is a no-op, exactly as in Java
//!   (`NormValuesWriter.java:69`). Unlike doc values, norms are **not** filled
//!   in for the documents that are missing: the file format records which
//!   documents have a value, and the reader reports the rest as absent.
//!
//! The norms are buffered in a `PackedLongValues.deltaPackedBuilder(COMPACT)`,
//! as Java's are, and the RAM charged is that builder's and the
//! docs-with-field set's (`NormValuesWriter.java:37-44, 62-66`).
//!
//! [`NormValuesWriter::flush`] takes the segment's `Sorter.DocMap`, as Java's
//! does, and re-orders the norms when the segment has an index sort.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::codecs::norms::{NormsConsumer, NormsProducer};
use crate::codecs::DocsWithFieldSet;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{DocValuesIterator, NumericDocValues};
use crate::index::doc_values_writer::{
    delta_packed_builder, NumericDVs, NumericDocValuesWriter, SortingNumeric,
};
use crate::index::index_sorter::SorterDocMap;
use crate::index::FieldInfo;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::packed::{
    DeltaPackedLongValues, DeltaPackedLongValuesBuilder, OwnedPackedLongValuesIterator,
    PackedLongValuesBuilderOps,
};

/// Buffers one norm value per document for a single field, then flushes them
/// when the segment flushes.
///
/// Equivalent to `org.apache.lucene.index.NormValuesWriter`.
#[derive(Debug)]
pub struct NormValuesWriter {
    field_info: FieldInfo,
    docs_with_field: DocsWithFieldSet,
    /// `PackedLongValues.deltaPackedBuilder(PackedInts.COMPACT)`, until
    /// [`Self::flush`] builds it.
    pending: Option<DeltaPackedLongValuesBuilder>,
    last_doc_id: i32,
    /// The number of bytes this writer has already reported to
    /// [`Self::iw_bytes_used`], so that an update can report the delta.
    bytes_used: i64,
    iw_bytes_used: Arc<AtomicI64>,
    /// Whether [`Self::flush`] has already handed the buffers over.
    flushed: bool,
}

impl NormValuesWriter {
    /// Creates a writer for `field_info`, charging its initial footprint to
    /// `iw_bytes_used`.
    ///
    /// Equivalent to `new NormValuesWriter(FieldInfo, Counter)`.
    pub fn new(field_info: FieldInfo, iw_bytes_used: Arc<AtomicI64>) -> Self {
        let docs_with_field = DocsWithFieldSet::new();
        let pending = delta_packed_builder();
        let bytes_used = pending.ram_bytes_used() + docs_with_field.ram_bytes_used();
        iw_bytes_used.fetch_add(bytes_used, Ordering::AcqRel);
        Self {
            field_info,
            docs_with_field,
            pending: Some(pending),
            last_doc_id: -1,
            bytes_used,
            iw_bytes_used,
            flushed: false,
        }
    }

    /// Returns the field these norms belong to.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// Buffers `value` as the norm of `doc_id`.
    ///
    /// Equivalent to `NormValuesWriter.addValue(int, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `doc_id` is not strictly
    /// greater than the last document added, which in Lucene means the field
    /// was finished twice within one document, and
    /// [`LuceneError::IllegalState`] when the field has already been flushed.
    pub fn add_value(&mut self, doc_id: i32, value: i64) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::illegal_state(format!(
                "the norms of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        if doc_id <= self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "Norm for \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        self.pending
            .as_mut()
            .ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "the norms of field \"{}\" have already been flushed",
                    self.field_info.name
                ))
            })?
            .add(value)?;
        self.docs_with_field.add(doc_id)?;
        self.update_bytes_used();
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Number of documents that have a norm buffered.
    pub fn num_docs_with_field(&self) -> i32 {
        self.docs_with_field.cardinality()
    }

    /// Reports the delta between the footprint already charged and the current
    /// one.
    ///
    /// Equivalent to `NormValuesWriter.updateBytesUsed()`.
    fn update_bytes_used(&mut self) {
        let new_bytes_used = self.ram_bytes_used();
        self.iw_bytes_used
            .fetch_add(new_bytes_used - self.bytes_used, Ordering::AcqRel);
        self.bytes_used = new_bytes_used;
    }

    /// Heap held by the buffers.
    ///
    /// Equivalent to `pending.ramBytesUsed() + docsWithField.ramBytesUsed()`
    /// (`NormValuesWriter.java:62-66`).
    pub fn ram_bytes_used(&self) -> i64 {
        self.pending
            .as_ref()
            .map_or(0, PackedLongValuesBuilderOps::ram_bytes_used)
            + self.docs_with_field.ram_bytes_used()
    }

    /// Completes buffering for a segment of `max_doc` documents.
    ///
    /// Equivalent to `NormValuesWriter.finish(int)`, which is empty: unlike doc
    /// values, norms are not filled in for the documents that never carried the
    /// field. The method exists so that the call site can mirror Lucene's, and
    /// so that a future format that does need a completion step has a place to
    /// put it.
    pub fn finish(&mut self, _max_doc: i32) {}

    /// Hands the buffered norms to `consumer`.
    ///
    /// Equivalent to `NormValuesWriter.flush(SegmentWriteState, Sorter.DocMap,
    /// NormsConsumer)` (`NormValuesWriter.java:70-105`): `sort_map` is the
    /// permutation the index sort gave the segment, or `None` when it was not
    /// sorted. Java declares the `SegmentWriteState` only to read `maxDoc`,
    /// which is the sort map's size, so this port does not take one.
    ///
    /// Flushing consumes the buffers, so the heap they held is given back to
    /// the shared counter here rather than when the writer is dropped, and the
    /// values are handed over rather than copied.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the field has already been
    /// flushed — writing it twice would put two metadata entries for one field
    /// into the same segment. Java reaches the same conclusion by way of
    /// `PackedLongValues.Builder`, which refuses to be built twice.
    /// Otherwise propagates whatever the consumer raises while writing the
    /// field.
    pub fn flush(
        &mut self,
        consumer: &mut dyn NormsConsumer,
        sort_map: Option<&SorterDocMap>,
    ) -> Result<()> {
        if self.flushed {
            return Err(LuceneError::illegal_state(format!(
                "the norms of field \"{}\" have already been flushed",
                self.field_info.name
            )));
        }
        self.flushed = true;
        // `pending.build()` (`NormValuesWriter.java:72`).
        let pending = self.pending.take().ok_or_else(|| {
            LuceneError::illegal_state(format!(
                "the norms of field \"{}\" have already been flushed",
                self.field_info.name
            ))
        })?;
        let values = Arc::new(pending.build()?);
        let docs_with_field = Arc::new(std::mem::take(&mut self.docs_with_field));
        let mut producer = BufferedNormsProducer {
            field_number: self.field_info.number,
            field_name: self.field_info.name.clone(),
            docs_with_field: Arc::clone(&docs_with_field),
            values: Arc::clone(&values),
            sorted: None,
        };
        // `NumericDocValuesWriter.sortDocValues(maxDoc, sortMap, new
        // BufferedNorms(...), sortMap.size() == docsWithField.cardinality())`
        // (`NormValuesWriter.java:73-83`); the producer then hands out
        // `SortingNumericDocValues` over the sorted values.
        if let Some(sort_map) = sort_map {
            let mut old_norms = BufferedNorms::new(&values, &docs_with_field);
            let dense = sort_map.size() == docs_with_field.cardinality();
            producer.sorted = Some(Arc::new(NumericDocValuesWriter::sort_doc_values(
                sort_map.size(),
                sort_map,
                &mut old_norms,
                dense,
            )?));
        }
        drop(docs_with_field);
        self.update_bytes_used();
        consumer.add_norms_field(&self.field_info, &producer)
    }
}

impl Drop for NormValuesWriter {
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

// ---------------------------------------------------------------------------
// The producer handed to the codec
// ---------------------------------------------------------------------------

/// Serves the buffered norms of one field to a [`NormsConsumer`].
///
/// Equivalent to the anonymous `NormsProducer` of `NormValuesWriter.flush`
/// (`NormValuesWriter.java:84-104`), including its "wrong fieldInfo" guard:
/// every [`NormsProducer::get_norms`] call returns a fresh `BufferedNorms`
/// over the built `PackedLongValues` and the docs-with-field set.
#[derive(Debug)]
struct BufferedNormsProducer {
    field_number: i32,
    field_name: String,
    docs_with_field: Arc<DocsWithFieldSet>,
    values: Arc<DeltaPackedLongValues>,
    /// The values re-ordered by the index sort, when the segment is sorted.
    sorted: Option<Arc<NumericDVs>>,
}

impl NormsProducer for BufferedNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        let values: Result<Box<dyn NumericDocValues>> = {
            if field.number != self.field_number {
                return Err(LuceneError::IllegalArgument(format!(
                    "wrong fieldInfo: expected {} ({}), got {} ({})",
                    self.field_name, self.field_number, field.name, field.number
                )));
            }
            match &self.sorted {
                None => Ok(Box::new(BufferedNorms::new(
                    &self.values,
                    &self.docs_with_field,
                ))),
                Some(sorted) => Ok(Box::new(SortingNumeric::new(Arc::clone(sorted)))),
            }
        };
        Ok(Some(values?))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(Self {
            field_number: self.field_number,
            field_name: self.field_name.clone(),
            docs_with_field: Arc::clone(&self.docs_with_field),
            values: Arc::clone(&self.values),
            sorted: self.sorted.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Iterates the norms held in memory.
///
/// Equivalent to `NormValuesWriter.BufferedNorms`
/// (`NormValuesWriter.java:110-153`): it walks the docs-with-field set and
/// takes the next packed value for every document it lands on. Like its Java
/// counterpart it is forward-only — `advance` and `advance_exact` are not
/// supported — and `long_value` answers the last value it read.
struct BufferedNorms {
    iter: OwnedPackedLongValuesIterator<DeltaPackedLongValues>,
    docs_with_field: Box<dyn DocIdSetIterator + Send + Sync>,
    value: i64,
}

impl fmt::Debug for BufferedNorms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedNorms")
            .field("doc_id", &self.docs_with_field.doc_id())
            .finish()
    }
}

impl BufferedNorms {
    fn new(values: &Arc<DeltaPackedLongValues>, docs_with_field: &Arc<DocsWithFieldSet>) -> Self {
        Self {
            iter: OwnedPackedLongValuesIterator::new(Arc::clone(values)),
            docs_with_field: DocsWithFieldSet::shared_iterator(docs_with_field),
            value: 0,
        }
    }
}

impl DocIdSetIterator for BufferedNorms {
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
            "BufferedNorms does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.docs_with_field.cost()
    }
}

impl DocValuesIterator for BufferedNorms {
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::UnsupportedOperation(
            "BufferedNorms does not support advance_exact".to_string(),
        ))
    }
}

impl NumericDocValues for BufferedNorms {
    fn long_value(&self) -> Result<i64> {
        Ok(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexOptions;

    fn field(name: &str, number: i32) -> FieldInfo {
        let mut info = FieldInfo::new(name, number);
        info.index_options = IndexOptions::DOCS_AND_FREQS;
        info
    }

    fn counter() -> Arc<AtomicI64> {
        Arc::new(AtomicI64::new(0))
    }

    /// Collects everything a consumer is handed, so a test can assert on it
    /// without going through a codec.
    #[derive(Debug, Default)]
    struct RecordingConsumer {
        fields: Vec<(String, Vec<(i32, i64)>)>,
        closed: bool,
    }

    impl NormsConsumer for RecordingConsumer {
        fn add_norms_field(&mut self, field: &FieldInfo, values: &dyn NormsProducer) -> Result<()> {
            let mut collected = Vec::new();
            let mut norms = values.get_norms(field)?.unwrap();
            loop {
                let doc = norms.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                collected.push((doc, norms.long_value()?));
            }
            self.fields.push((field.name.clone(), collected));
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            self.closed = true;
            Ok(())
        }
    }

    #[test]
    fn buffered_norms_reach_the_consumer_in_document_order() {
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        for (doc, value) in [(0, 5i64), (3, -1), (7, 120)] {
            writer.add_value(doc, value).unwrap();
        }
        writer.finish(10);
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(
            consumer.fields,
            vec![("body".to_string(), vec![(0, 5), (3, -1), (7, 120)])]
        );
    }

    #[test]
    fn a_repeated_document_is_the_appears_more_than_once_error() {
        // This is how Lucene catches a chain that finished one field twice
        // within a single document.
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(4, 1).unwrap();
        let error = writer.add_value(4, 2).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalArgument(ref m)
                if m.contains("appears more than once") && m.contains("body")),
            "unexpected error: {error:?}"
        );
        // A doc id that goes backwards is the same failure.
        assert!(writer.add_value(3, 2).is_err());
        // The rejected values were not buffered.
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.fields[0].1, vec![(4, 1)]);
    }

    #[test]
    fn a_writer_that_saw_no_document_still_flushes_a_field() {
        // "Even if no documents actually succeed in setting a norm, we still
        // write norms for this segment" (`IndexingChain.java:1839-1840`): the
        // field must reach the consumer so that the metadata records it as
        // empty, which is what tells a reader the field exists with no values.
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.finish(10);
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.fields, vec![("body".to_string(), Vec::new())]);
    }

    #[test]
    fn finish_does_not_fill_the_missing_documents() {
        // Unlike doc values, norms are not padded out to `max_doc`: the format
        // records which documents have a value.
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(0, 1).unwrap();
        writer.add_value(9, 2).unwrap();
        writer.finish(1_000);
        assert_eq!(writer.num_docs_with_field(), 2);
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.fields[0].1, vec![(0, 1), (9, 2)]);
    }

    #[test]
    fn the_producer_refuses_a_field_it_does_not_hold() {
        // Equivalent to the "wrong fieldInfo" guard of the anonymous producer
        // in `NormValuesWriter.flush`.
        #[derive(Debug, Default)]
        struct WrongFieldConsumer {
            error: Option<String>,
        }
        impl NormsConsumer for WrongFieldConsumer {
            fn add_norms_field(
                &mut self,
                _field: &FieldInfo,
                values: &dyn NormsProducer,
            ) -> Result<()> {
                let other = FieldInfo::new("other", 7);
                self.error = match values.get_norms(&other) {
                    Ok(_) => panic!("the wrong field must be refused"),
                    Err(error) => Some(error.to_string()),
                };
                Ok(())
            }
            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(0, 1).unwrap();
        let mut consumer = WrongFieldConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let error = consumer.error.expect("the guard must have fired");
        assert!(
            error.contains("wrong fieldInfo"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_consumer_may_replay_the_values_as_often_as_it_likes() {
        // The Lucene 9.0 consumer walks the producer three times: once for the
        // count and range, once for the docs-with-field set and once for the
        // packed values. Each call must start from the beginning.
        #[derive(Debug, Default)]
        struct ThreePassConsumer {
            passes: Vec<Vec<(i32, i64)>>,
        }
        impl NormsConsumer for ThreePassConsumer {
            fn add_norms_field(
                &mut self,
                field: &FieldInfo,
                values: &dyn NormsProducer,
            ) -> Result<()> {
                for _ in 0..3 {
                    let mut collected = Vec::new();
                    let mut norms = values.get_norms(field)?.unwrap();
                    loop {
                        let doc = norms.next_doc()?;
                        if doc == NO_MORE_DOCS {
                            break;
                        }
                        collected.push((doc, norms.long_value()?));
                    }
                    self.passes.push(collected);
                }
                Ok(())
            }
            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        for doc in 0..5 {
            writer.add_value(doc * 2, doc as i64 + 1).unwrap();
        }
        let mut consumer = ThreePassConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.passes.len(), 3);
        assert_eq!(consumer.passes[0], consumer.passes[1]);
        assert_eq!(consumer.passes[1], consumer.passes[2]);
        assert_eq!(consumer.passes[0].len(), 5);
    }

    #[test]
    fn the_buffered_iterator_is_forward_only() {
        // `BufferedNorms.advance` and `advanceExact` throw
        // `UnsupportedOperationException` in Java, because the only consumer is
        // the codec's sequential passes; anything else is a bug in the caller.
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(0, 1).unwrap();

        #[derive(Debug, Default)]
        struct ProbingConsumer {
            advance: Option<String>,
            advance_exact: Option<String>,
        }
        impl NormsConsumer for ProbingConsumer {
            fn add_norms_field(
                &mut self,
                field: &FieldInfo,
                values: &dyn NormsProducer,
            ) -> Result<()> {
                let mut norms = values.get_norms(field)?.unwrap();
                self.advance = Some(norms.advance(0).unwrap_err().to_string());
                self.advance_exact = Some(norms.advance_exact(0).unwrap_err().to_string());
                Ok(())
            }
            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut consumer = ProbingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert!(consumer.advance.unwrap().contains("advance"));
        assert!(consumer.advance_exact.unwrap().contains("advance_exact"));
    }

    #[test]
    fn the_exhausted_iterator_keeps_reporting_no_more_docs() {
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(2, 9).unwrap();

        #[derive(Debug, Default)]
        struct ExhaustionConsumer {
            after: Vec<i32>,
        }
        impl NormsConsumer for ExhaustionConsumer {
            fn add_norms_field(
                &mut self,
                field: &FieldInfo,
                values: &dyn NormsProducer,
            ) -> Result<()> {
                let mut norms = values.get_norms(field)?.unwrap();
                assert_eq!(norms.next_doc()?, 2);
                for _ in 0..4 {
                    self.after.push(norms.next_doc()?);
                }
                // `BufferedNorms.longValue()` returns the last value it read
                // (`NormValuesWriter.java:149-151`).
                assert_eq!(norms.long_value()?, 9);
                Ok(())
            }
            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let mut consumer = ExhaustionConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert_eq!(consumer.after, vec![NO_MORE_DOCS; 4]);
    }

    #[test]
    fn the_shared_byte_counter_grows_with_the_buffer_and_is_given_back() {
        let counter = counter();
        let start = counter.load(Ordering::Acquire);
        assert_eq!(start, 0);
        {
            let mut writer = NormValuesWriter::new(field("body", 0), Arc::clone(&counter));
            let empty = counter.load(Ordering::Acquire);
            assert!(empty > 0, "the empty buffers already cost something");
            // Enough varied values to pack several pages of the
            // `PackedLongValues` builder, whose first page is allocated up
            // front and charged by the empty writer already.
            for doc in 0..5_000 {
                writer.add_value(doc, i64::from(doc % 97) * 1_000).unwrap();
            }
            let full = counter.load(Ordering::Acquire);
            assert!(
                full > empty,
                "a thousand buffered norms must be reported: {empty} -> {full}"
            );
            assert_eq!(full, writer.ram_bytes_used());
        }
        assert_eq!(
            counter.load(Ordering::Acquire),
            0,
            "dropping the writer must give the bytes back"
        );
    }

    #[test]
    fn two_writers_share_one_counter() {
        let counter = counter();
        let mut body = NormValuesWriter::new(field("body", 0), Arc::clone(&counter));
        let mut title = NormValuesWriter::new(field("title", 1), Arc::clone(&counter));
        for doc in 0..100 {
            body.add_value(doc, 1).unwrap();
            title.add_value(doc, 2).unwrap();
        }
        assert_eq!(
            counter.load(Ordering::Acquire),
            body.ram_bytes_used() + title.ram_bytes_used()
        );
    }

    #[test]
    fn the_field_info_is_the_one_the_writer_was_built_with() {
        let writer = NormValuesWriter::new(field("body", 3), counter());
        assert_eq!(writer.field_info().name, "body");
        assert_eq!(writer.field_info().number, 3);
    }
    #[test]
    fn flushing_twice_is_refused() {
        // Two metadata entries for one field would make the segment unreadable,
        // so the second attempt is an error rather than a second write.
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(0, 1).unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        let error = writer.flush(&mut consumer, None).unwrap_err();
        assert!(
            matches!(error, LuceneError::IllegalState { message: ref m, .. } if m.contains("already been flushed")),
            "unexpected error: {error:?}"
        );
        assert_eq!(consumer.fields.len(), 1);
    }

    #[test]
    fn adding_after_a_flush_is_refused() {
        let mut writer = NormValuesWriter::new(field("body", 0), counter());
        writer.add_value(0, 1).unwrap();
        let mut consumer = RecordingConsumer::default();
        writer.flush(&mut consumer, None).unwrap();
        assert!(writer.add_value(1, 2).is_err());
    }

    #[test]
    fn flushing_gives_the_buffered_bytes_back_to_the_counter() {
        let counter = counter();
        let mut writer = NormValuesWriter::new(field("body", 0), Arc::clone(&counter));
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
        assert_eq!(consumer.fields[0].1.len(), 1_000);
    }
}
